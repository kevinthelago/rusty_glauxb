use std::collections::VecDeque;

pub const DIAG_WINDOW: usize = 300; // ~5 s at 60 fps

// ─── Per-frame snapshot ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    /// Wall-clock time for the full frame (Instant delta), milliseconds.
    pub wall_ms: f64,
    /// GPU execution time from timestamp queries, milliseconds.  None if the
    /// TIMESTAMP_QUERY feature is absent on this adapter.
    pub gpu_ms: Option<f64>,
    /// Approximate CPU submission time (wall_ms minus gpu_ms when available).
    pub cpu_ms: f64,
    /// Process resident-set size at frame end, megabytes.
    pub rss_mb: f32,
    /// Number of indexed draw calls issued this frame.
    pub draw_calls: u32,
    /// Total triangles submitted across all draw calls.
    pub tri_count: u32,
}

impl FrameStats {
    pub fn fps(&self) -> f64 {
        if self.wall_ms > 0.0 {
            1000.0 / self.wall_ms
        } else {
            0.0
        }
    }
}

// ─── Percentile result ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Percentiles {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

// ─── Rolling accumulator ──────────────────────────────────────────────────────

pub struct DiagAccumulator {
    ring: VecDeque<FrameStats>,
    cap: usize,
}

impl DiagAccumulator {
    pub fn new(cap: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(cap),
            cap,
        }
    }

    pub fn push(&mut self, s: FrameStats) {
        if self.ring.len() >= self.cap {
            self.ring.pop_front();
        }
        self.ring.push_back(s);
    }

    pub fn reset(&mut self) {
        self.ring.clear();
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn latest(&self) -> Option<&FrameStats> {
        self.ring.back()
    }

    pub fn percentiles(&self, f: impl Fn(&FrameStats) -> f64) -> Option<Percentiles> {
        if self.ring.is_empty() {
            return None;
        }
        let mut vals: Vec<f64> = self.ring.iter().map(f).collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = vals.len();
        Some(Percentiles {
            mean,
            p50: vals[n * 50 / 100],
            p95: vals[(n * 95 / 100).min(n - 1)],
            p99: vals[(n * 99 / 100).min(n - 1)],
            max: *vals.last().unwrap(),
        })
    }

    // ── ASCII table ───────────────────────────────────────────────────────────

    pub fn format_table(&self, adapter: &str, tier: &str, backend: &str) -> Vec<String> {
        let mut out = Vec::new();
        let n = self.ring.len();
        if n == 0 {
            out.push("no frames recorded — run the app for a moment first".to_string());
            return out;
        }

        out.push(format!(
            "── diag ── {n} frames ─────────────────────────────────────────"
        ));
        out.push(format!("   adapter   {adapter}"));
        out.push(format!("   tier      {tier}  ({backend})"));
        out.push(String::new());
        out.push(
            "                      mean      p50      p95      p99      max".to_string(),
        );

        // FPS
        if let Some(p) = self.percentiles(|f| f.fps()) {
            out.push(format!(
                "   fps           {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                p.mean, p.p50, p.p95, p.p99, p.max
            ));
        }

        // Frame time
        if let Some(p) = self.percentiles(|f| f.wall_ms) {
            out.push(format!(
                "   frame   ms    {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
                p.mean, p.p50, p.p95, p.p99, p.max
            ));
        }

        // GPU time — only frames that have it
        let gpu_vals: Vec<f64> = self.ring.iter().filter_map(|f| f.gpu_ms).collect();
        if gpu_vals.is_empty() {
            out.push(
                "   gpu     ms    N/A  (TIMESTAMP_QUERY not supported on this adapter)".to_string(),
            );
        } else {
            let mean = gpu_vals.iter().sum::<f64>() / gpu_vals.len() as f64;
            let mut s = gpu_vals.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let ng = s.len();
            out.push(format!(
                "   gpu     ms    {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
                mean,
                s[ng * 50 / 100],
                s[(ng * 95 / 100).min(ng - 1)],
                s[(ng * 99 / 100).min(ng - 1)],
                s.last().unwrap(),
            ));
        }

        // CPU time
        if let Some(p) = self.percentiles(|f| f.cpu_ms) {
            out.push(format!(
                "   cpu     ms    {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
                p.mean, p.p50, p.p95, p.p99, p.max
            ));
        }

        out.push(String::new());

        // RSS — mean and max (low variance, percentiles not meaningful)
        let rss_vals: Vec<f32> = self.ring.iter().map(|f| f.rss_mb).collect();
        let rss_mean = rss_vals.iter().sum::<f32>() / rss_vals.len() as f32;
        let rss_max = rss_vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        out.push(format!(
            "   rss     MB    {:>8.1}        -        -        - {:>8.1}",
            rss_mean, rss_max
        ));

        // Tri count and draw calls from latest frame (stable over time)
        if let Some(latest) = self.latest() {
            out.push(format!(
                "   triangles     {:>8}        -        -        - {:>8}",
                latest.tri_count, latest.tri_count
            ));
            out.push(format!(
                "   draw calls    {:>8}        -        -        - {:>8}",
                latest.draw_calls, latest.draw_calls
            ));
        }

        out.push("───────────────────────────────────────────────────────────────".to_string());
        out
    }
}
