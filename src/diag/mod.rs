pub mod gpu_timer;
pub mod hardware;
pub mod stats;

pub use gpu_timer::GpuTimer;
pub use hardware::{AdapterCaps, HardwareTier};
pub use stats::{DiagAccumulator, FrameStats, DIAG_WINDOW};

use std::time::Instant;

// ─── RSS helper ───────────────────────────────────────────────────────────────

/// Read resident-set size of the current process, in megabytes.
/// Returns 0.0 on platforms where this is unavailable.
#[cfg(not(target_os = "android"))]
pub fn current_rss_mb() -> f32 {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    // Build a one-shot System that refreshes only the current process's memory.
    let pid = sysinfo::Pid::from(std::process::id() as usize);
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_processes(ProcessRefreshKind::new().with_memory()),
    );
    sys.refresh_process_specifics(pid, ProcessRefreshKind::new().with_memory());
    sys.process(pid)
        .map(|p| p.memory() as f32 / 1_048_576.0)
        .unwrap_or(0.0)
}

#[cfg(target_os = "android")]
pub fn current_rss_mb() -> f32 {
    0.0
}

// ─── DiagCollector ────────────────────────────────────────────────────────────

/// Owned by `App`.  Accumulates per-frame stats and formats the `diag` table.
pub struct DiagCollector {
    pub acc:  DiagAccumulator,
    pub caps: AdapterCaps,
    /// Tracks the wall-clock start of the current frame so app.rs can compute
    /// frame duration without storing extra state.
    pub frame_start: Instant,
    /// Whether to show a compact live overlay in the HUD corner.
    pub overlay: bool,
}

impl DiagCollector {
    pub fn new(caps: AdapterCaps) -> Self {
        Self {
            acc: DiagAccumulator::new(DIAG_WINDOW),
            caps,
            frame_start: Instant::now(),
            overlay: false,
        }
    }

    /// Call at the very beginning of `RedrawRequested` to stamp the frame start.
    pub fn begin_frame(&mut self) {
        self.frame_start = Instant::now();
    }

    /// Call after `render()` returns to record the completed frame.
    pub fn record(
        &mut self,
        gpu_ms:     Option<f64>,
        draw_calls: u32,
        tri_count:  u32,
    ) {
        let wall_ms = self.frame_start.elapsed().as_secs_f64() * 1000.0;
        let cpu_ms  = match gpu_ms {
            Some(g) => (wall_ms - g).max(0.0),
            None    => wall_ms,
        };
        let rss_mb = current_rss_mb();
        self.acc.push(FrameStats {
            wall_ms,
            gpu_ms,
            cpu_ms,
            rss_mb,
            draw_calls,
            tri_count,
        });
    }

    pub fn format_table(&self) -> Vec<String> {
        self.acc.format_table(
            &self.caps.name,
            &self.caps.tier.to_string(),
            &self.caps.backend,
        )
    }

    /// Compact one-liner for the HUD overlay (fps + gpu time).
    pub fn hud_line(&self) -> Option<String> {
        let latest = self.acc.latest()?;
        let gpu = latest
            .gpu_ms
            .map(|g| format!("gpu {g:.1}ms"))
            .unwrap_or_else(|| "gpu N/A".to_string());
        Some(format!("{:.0} fps  {gpu}  cpu {:.1}ms", latest.fps(), latest.cpu_ms))
    }
}
