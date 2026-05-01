//! Radio spectrum overlay.
//!
//! ## Sources
//! - `DummySource`  — animated simulation; always available, no hardware required
//! - `WifiSource`   — Android WiFi scan results via JNI (future; returns channel
//!                    frequencies + RSSI from `WifiManager.getScanResults()`)
//! - `RtlSdrSource` — RTL-SDR USB dongle via librtlsdr (future; real IQ → FFT)
//!
//! ## Adding a real source
//! 1. Implement `SpectrumSource` for your type.
//! 2. Add a match arm to `SpectrumOverlay::set_source`.
//! 3. Add the source name to the console help text in `console/mod.rs`.

use std::sync::Arc;

use tokio::{runtime::Handle, sync::watch, time};

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct SpectrumBand {
    pub freq_mhz: f32,
    pub power_dbm: f32,
    pub label: Option<String>,
}

// ─── SpectrumSource trait ─────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait SpectrumSource: Send + Sync {
    fn name(&self) -> &str;
    async fn scan(&self) -> anyhow::Result<Vec<SpectrumBand>>;
    fn scan_interval_ms(&self) -> u64 {
        1000
    }
}

// ─── Dummy source ─────────────────────────────────────────────────────────────

/// Simulated 2.4 GHz + 5 GHz WiFi spectrum.  Four channels are "active" with
/// stronger, slowly-animated signals; the rest sit near the noise floor.
pub struct DummySource;

#[async_trait::async_trait]
impl SpectrumSource for DummySource {
    fn name(&self) -> &str {
        "dummy"
    }
    fn scan_interval_ms(&self) -> u64 {
        500
    }

    async fn scan(&self) -> anyhow::Result<Vec<SpectrumBand>> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f32();

        const CHANNELS_24: &[(f32, &str)] = &[
            (2412.0, "Ch 1"),  (2417.0, "Ch 2"),  (2422.0, "Ch 3"),
            (2427.0, "Ch 4"),  (2432.0, "Ch 5"),  (2437.0, "Ch 6"),
            (2442.0, "Ch 7"),  (2447.0, "Ch 8"),  (2452.0, "Ch 9"),
            (2457.0, "Ch 10"), (2462.0, "Ch 11"), (2467.0, "Ch 12"),
            (2472.0, "Ch 13"),
        ];
        const CHANNELS_5: &[(f32, &str)] = &[
            (5180.0, "Ch 36"), (5200.0, "Ch 40"), (5220.0, "Ch 44"),
            (5240.0, "Ch 48"), (5260.0, "Ch 52"), (5280.0, "Ch 56"),
            (5300.0, "Ch 60"), (5320.0, "Ch 64"),
        ];
        // "Active" AP frequencies
        const ACTIVE: &[f32] = &[2437.0, 2462.0, 5180.0, 5240.0];

        let mut bands = Vec::new();
        for &(freq, label) in CHANNELS_24.iter().chain(CHANNELS_5.iter()) {
            let base = if ACTIVE.contains(&freq) { -62.0_f32 } else { -94.0_f32 };
            let wave = (t * 0.4 + freq * 0.007).sin() * 4.0
                + (t * 1.9 + freq * 0.013).sin() * 1.5;
            bands.push(SpectrumBand {
                freq_mhz: freq,
                power_dbm: base + wave,
                label: Some(label.to_string()),
            });
        }
        Ok(bands)
    }
}

// ─── Overlay ──────────────────────────────────────────────────────────────────

pub struct SpectrumOverlay {
    pub visible: bool,
    pub source_name: String,
    tx: watch::Sender<Vec<SpectrumBand>>,
    rx: watch::Receiver<Vec<SpectrumBand>>,
    rt: Handle,
}

impl SpectrumOverlay {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = watch::channel(Vec::new());
        let overlay = Self {
            visible: false,
            source_name: "dummy".to_string(),
            tx,
            rx,
            rt,
        };
        overlay.spawn_source(Arc::new(DummySource));
        overlay
    }

    /// Spawn a background task that polls `source` on its preferred interval.
    /// Dropping the old `rx` causes `tx.is_closed()` to fire in the old task,
    /// cleanly stopping it without extra cancellation tokens.
    fn spawn_source(&self, source: Arc<dyn SpectrumSource>) {
        let tx = self.tx.clone();
        let ms = source.scan_interval_ms();
        self.rt.spawn(async move {
            let mut ticker = time::interval(time::Duration::from_millis(ms));
            loop {
                ticker.tick().await;
                if tx.is_closed() {
                    break;
                }
                match source.scan().await {
                    Ok(bands) => {
                        let _ = tx.send(bands);
                    }
                    Err(e) => tracing::warn!("spectrum scan: {e:#}"),
                }
            }
        });
    }

    /// Switch to a named source.  Returns `false` if the name is unknown.
    pub fn set_source(&mut self, name: &str) -> bool {
        let src: Arc<dyn SpectrumSource> = match name {
            "dummy" => Arc::new(DummySource),
            // Future: "wifi" => Arc::new(wifi::WifiSource::new()),
            // Future: "rtlsdr" => Arc::new(rtlsdr::RtlSdrSource::new(...)),
            _ => return false,
        };
        // Replace the channel — dropping old rx stops the old task.
        let (tx, rx) = watch::channel(Vec::new());
        self.tx = tx;
        self.rx = rx;
        self.source_name = name.to_string();
        self.spawn_source(src);
        true
    }

    pub fn draw(&self, ctx: &egui::Context) {
        if !self.visible {
            return;
        }
        let bands = self.rx.borrow().clone();
        if bands.is_empty() {
            return;
        }

        egui::Window::new("##spectrum_overlay")
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -8.0])
            .title_bar(false)
            .resizable(false)
            .fixed_size([500.0, 115.0])
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(8, 14, 8, 220))
                    .rounding(5.0)
                    .inner_margin(egui::Margin::same(6.0)),
            )
            .show(ctx, |ui| {
                draw_bands(ui, &bands, &self.source_name);
            });
    }
}

// ─── egui drawing ─────────────────────────────────────────────────────────────

fn draw_bands(ui: &mut egui::Ui, bands: &[SpectrumBand], source: &str) {
    let rect = ui.max_rect();
    let painter = ui.painter();

    // Header
    painter.text(
        egui::pos2(rect.min.x, rect.min.y),
        egui::Align2::LEFT_TOP,
        format!("RF SPECTRUM  [{}]", source),
        egui::FontId::monospace(9.0),
        egui::Color32::from_gray(115),
    );

    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y + 13.0),
        egui::pos2(rect.max.x - 28.0, rect.max.y - 14.0),
    );

    const DB_MIN: f32 = -100.0;
    const DB_MAX: f32 = -30.0;
    const DB_RANGE: f32 = DB_MAX - DB_MIN;

    // Horizontal grid lines at -90 / -70 / -50 dBm
    for db in [-90.0_f32, -70.0, -50.0] {
        let yn = ((db - DB_MIN) / DB_RANGE).clamp(0.0, 1.0);
        let y = plot.max.y - yn * plot.height();
        painter.line_segment(
            [egui::pos2(plot.min.x, y), egui::pos2(plot.max.x, y)],
            egui::Stroke::new(0.5, egui::Color32::from_gray(35)),
        );
        painter.text(
            egui::pos2(plot.max.x + 3.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{db:.0}"),
            egui::FontId::monospace(7.0),
            egui::Color32::from_gray(75),
        );
    }

    // Frequency band separator (gap between 2.4 GHz and 5 GHz groups)
    let n = bands.len();
    let gap = plot.width() / n as f32;
    let bar_w = (gap * 0.72).max(3.0);

    for (i, band) in bands.iter().enumerate() {
        let cx = plot.min.x + (i as f32 + 0.5) * gap;
        let yn = ((band.power_dbm - DB_MIN) / DB_RANGE).clamp(0.0, 1.0);
        let bar_h = yn * plot.height();

        let bar = egui::Rect::from_min_size(
            egui::pos2(cx - bar_w / 2.0, plot.max.y - bar_h),
            egui::vec2(bar_w, bar_h),
        );
        painter.rect_filled(bar, 1.0, power_color(yn));

        // Channel label below — only active channels or every 4th
        if yn > 0.35 || i % 4 == 0 {
            if let Some(label) = &band.label {
                painter.text(
                    egui::pos2(cx, plot.max.y + 2.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::FontId::monospace(7.0),
                    egui::Color32::from_gray(90),
                );
            }
        }
    }

    // Frequency range label
    if let (Some(first), Some(last)) = (bands.first(), bands.last()) {
        painter.text(
            egui::pos2(rect.min.x, rect.max.y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{:.0} MHz", first.freq_mhz),
            egui::FontId::monospace(7.0),
            egui::Color32::from_gray(70),
        );
        painter.text(
            egui::pos2(plot.max.x, rect.max.y - 1.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{:.0} MHz", last.freq_mhz),
            egui::FontId::monospace(7.0),
            egui::Color32::from_gray(70),
        );
    }
}

fn power_color(t: f32) -> egui::Color32 {
    if t < 0.5 {
        let s = t * 2.0;
        egui::Color32::from_rgb((s * 190.0) as u8, (80.0 + s * 120.0) as u8, 25)
    } else {
        let s = (t - 0.5) * 2.0;
        egui::Color32::from_rgb(190, ((1.0 - s) * 200.0) as u8, 20)
    }
}
