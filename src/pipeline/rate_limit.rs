use std::time::Duration;

/// Returned by `DataSource::fetch` when the server responds with HTTP 429.
#[derive(Debug)]
pub struct RateLimitError {
    pub retry_after: Duration,
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rate limited — retry after {:.0}s", self.retry_after.as_secs_f64())
    }
}

impl std::error::Error for RateLimitError {}

/// Extract the backoff delay from a 429 response's headers.
///
/// Checks `Retry-After` (RFC 7231 integer seconds) then the OpenSky-specific
/// `X-Rate-Limit-Retry-After-Seconds`.  Returns `fallback` if neither header
/// is present or parseable.
pub fn extract_retry_after(headers: &reqwest::header::HeaderMap, fallback: Duration) -> Duration {
    for name in &["Retry-After", "X-Rate-Limit-Retry-After-Seconds"] {
        if let Some(val) = headers.get(*name) {
            if let Ok(s) = val.to_str() {
                if let Ok(secs) = s.trim().parse::<f64>() {
                    return Duration::from_secs_f64(secs.max(1.0));
                }
            }
        }
    }
    fallback
}
