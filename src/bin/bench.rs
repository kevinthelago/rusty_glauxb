/// Headless benchmark binary.
///
/// Usage:
///   bench [--frames N] [--scenario <name>] [--software] [--json]
///
/// Scenarios:
///   globe-only   — globe + world-map, no flights      (default)
///   full-scene   — globe + world-map + 500 flights
///   stress       — globe + world-map + 5 000 flights

use rusty_glauxb::{
    diag::stats::DiagAccumulator,
    geo::{FeatureKind, GeoFeature},
    renderer::headless::HeadlessRenderer,
};

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(run());
}

async fn run() {
    let mut frames: u32   = 300;
    let mut scenario      = "globe-only".to_string();
    let mut force_sw      = false;
    let mut json_out      = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames"   => frames    = args.next().and_then(|v| v.parse().ok()).unwrap_or(300),
            "--scenario" => scenario  = args.next().unwrap_or_else(|| "globe-only".into()),
            "--software" => force_sw  = true,
            "--json"     => json_out  = true,
            other        => eprintln!("unknown flag {other}; ignored"),
        }
    }

    let mut renderer = HeadlessRenderer::new(force_sw)
        .await
        .expect("HeadlessRenderer init");

    let flights = make_flights(&scenario);

    eprintln!(
        "bench: {} frames, scenario={scenario}, adapter={} ({})",
        frames,
        renderer.adapter_caps.name,
        renderer.adapter_caps.backend,
    );

    let cap = frames as usize;
    let mut acc = DiagAccumulator::new(cap);

    // warm-up: 10 frames not counted
    for _ in 0..10 {
        renderer.render_frame(&flights);
    }

    for _ in 0..frames {
        let stats = renderer.render_frame(&flights);
        acc.push(stats);
    }

    if json_out {
        print_json(&acc, &renderer, &scenario, frames);
    } else {
        let caps = &renderer.adapter_caps;
        let lines = acc.format_table(&caps.name, &caps.tier.to_string(), &caps.backend);
        for line in &lines {
            println!("{line}");
        }
        println!("   scenario     {scenario}");
        println!("   frames       {frames}");
    }
}

fn make_flights(scenario: &str) -> Vec<GeoFeature> {
    let count = match scenario {
        "full-scene" => 500,
        "stress"     => 5_000,
        _            => 0,
    };

    (0..count)
        .map(|i| {
            let lat = -80.0 + (i as f32 * 3.7_f32) % 160.0;
            let lon = -180.0 + (i as f32 * 7.3_f32) % 360.0;
            GeoFeature {
                lat,
                lon,
                alt: 10_000.0,
                kind: FeatureKind::Point,
                meta: serde_json::Value::Null,
                coordinates: vec![],
            }
        })
        .collect()
}

fn print_json(acc: &DiagAccumulator, renderer: &HeadlessRenderer, scenario: &str, frames: u32) {
    let fps   = acc.percentiles(|f| f.fps());
    let frame = acc.percentiles(|f| f.wall_ms);
    let cpu   = acc.percentiles(|f| f.cpu_ms);

    // gpu_ms is Optional per frame; use the same percentile helper, treating None as skip.
    // DiagAccumulator::percentiles maps over all frames; frames with no timestamp will return 0.0
    // which skews the numbers.  Derive from wall - cpu instead (consistent with format_table).
    let gpu_mean = frame.as_ref().zip(cpu.as_ref()).map(|(fr, cp)| (fr.mean - cp.mean).max(0.0));

    let caps = &renderer.adapter_caps;
    let obj = serde_json::json!({
        "adapter":   caps.name,
        "backend":   caps.backend,
        "tier":      caps.tier.to_string(),
        "scenario":  scenario,
        "frames":    frames,
        "fps": {
            "mean": fps.as_ref().map(|p| round2(p.mean)),
            "p50":  fps.as_ref().map(|p| round2(p.p50)),
            "p95":  fps.as_ref().map(|p| round2(p.p95)),
            "p99":  fps.as_ref().map(|p| round2(p.p99)),
            "max":  fps.as_ref().map(|p| round2(p.max)),
        },
        "frame_ms": {
            "mean": frame.as_ref().map(|p| round2(p.mean)),
            "p50":  frame.as_ref().map(|p| round2(p.p50)),
            "p95":  frame.as_ref().map(|p| round2(p.p95)),
            "p99":  frame.as_ref().map(|p| round2(p.p99)),
            "max":  frame.as_ref().map(|p| round2(p.max)),
        },
        "gpu_ms_mean": gpu_mean.map(round2),
        "cpu_ms": {
            "mean": cpu.as_ref().map(|p| round2(p.mean)),
        },
        "rss_mb": acc.latest().map(|f| f.rss_mb),
        "draw_calls": acc.latest().map(|f| f.draw_calls),
        "tri_count":  acc.latest().map(|f| f.tri_count),
    });

    println!("{}", serde_json::to_string_pretty(&obj).unwrap());
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
