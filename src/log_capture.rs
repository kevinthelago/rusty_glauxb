use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::{Event, Level, Subscriber};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

// ─── LogEntry ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level:   Level,
    /// Last 2 path components of the source file (e.g. "osm/mod.rs").
    pub file:    String,
    pub line:    u32,
    /// Nearest active span name — the function name when `#[tracing::instrument]`
    /// is used, otherwise empty.
    pub span:    String,
    pub message: String,
}

impl LogEntry {
    /// Format as `[LEVEL][file:line][span] msg` (span bracket omitted when empty).
    pub fn format(&self) -> String {
        if self.span.is_empty() {
            format!("[{}][{}:{}] {}", self.level, self.file, self.line, self.message)
        } else {
            format!("[{}][{}][{}:{}] {}",
                self.level, self.file, self.span, self.line, self.message)
        }
    }

    /// Case-insensitive match against the file path.
    pub fn matches_module(&self, pattern: &str) -> bool {
        self.file.to_lowercase().contains(&pattern.to_lowercase())
    }
}

// ─── Shared buffer ────────────────────────────────────────────────────────────

pub type LogBuffer = Arc<Mutex<VecDeque<LogEntry>>>;

pub fn new_log_buffer() -> LogBuffer {
    Arc::new(Mutex::new(VecDeque::new()))
}

// ─── ConsoleLayer ─────────────────────────────────────────────────────────────

pub struct ConsoleLayer {
    buffer: LogBuffer,
}

impl ConsoleLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

fn shorten_file(path: &str) -> String {
    let parts: Vec<&str> = path.split(|c| c == '/' || c == '\\').collect();
    match parts.len() {
        0 => path.to_string(),
        1 => parts[0].to_string(),
        n => format!("{}/{}", parts[n - 2], parts[n - 1]),
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for ConsoleLayer {
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        struct MsgVisitor(String);
        impl Visit for MsgVisitor {
            fn record_debug(&mut self, field: &Field, val: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    use std::fmt::Write;
                    write!(self.0, "{val:?}").ok();
                }
            }
            fn record_str(&mut self, field: &Field, val: &str) {
                if field.name() == "message" {
                    self.0.push_str(val);
                }
            }
        }

        let mut msg = MsgVisitor(String::new());
        event.record(&mut msg);

        let meta  = event.metadata();
        // Skip events from external crates (log-crate bridge) — they have no source location.
        let Some(raw_file) = meta.file() else { return };
        let level = *meta.level();
        let file  = shorten_file(raw_file);
        let line  = meta.line().unwrap_or(0);
        let span  = ctx.lookup_current()
            .map(|s| s.name().to_string())
            .unwrap_or_default();

        let entry = LogEntry { level, file, line, span, message: msg.0 };

        if let Ok(mut buf) = self.buffer.lock() {
            buf.push_back(entry);
            const CAP: usize = 5_000;
            while buf.len() > CAP {
                buf.pop_front();
            }
        }
    }
}
