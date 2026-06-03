//! Platform-aware storage paths.
//!
//! The app runs on desktop (Linux/macOS/Windows) **and** Android, which differ
//! in where an application may write files:
//!
//! - **Desktop** has well-defined config/data directories exposed by the `dirs`
//!   crate (`~/.config`, `~/.local/share`, `%APPDATA%`, …).
//! - **Android** returns `None` from every `dirs` accessor.  Under
//!   `NativeActivity` the process working directory is the app's private
//!   internal-storage path, so we fall back to it.
//!
//! Every persistent file in the app — settings, the pipeline registry,
//! downloaded GeoJSON, and the dataset cache — resolves its location through
//! this module so the platform-specific fallback logic lives in exactly one
//! place.  All paths are namespaced under an [`APP_DIR`] subfolder.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;

/// Folder name under the platform's config/data root that namespaces all of the
/// app's files.
pub const APP_DIR: &str = "rusty_glauxb";

/// Base directory for **persistent app data**: the dataset cache, downloaded
/// GeoJSON, the pipeline registry, and session state.
///
/// Resolves to `<platform-local-data>/rusty_glauxb`, falling back to
/// `<cwd>/rusty_glauxb` on Android (and any platform where `dirs` yields
/// nothing).  The directory is **not** created — call [`ensure_dir`] or one of
/// the `*_file` helpers when you need it to exist.
pub fn data_dir() -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(|| std::env::current_dir().ok())
        .context("cannot locate a local data directory")?;
    Ok(base.join(APP_DIR))
}

/// Base directory for **user configuration** (`settings.toml`).
///
/// Resolves to `<platform-config>/rusty_glauxb`.  On platforms without a
/// distinct config directory (Android) it falls back to [`data_dir`] so config
/// and data share one private folder.
pub fn config_dir() -> anyhow::Result<PathBuf> {
    let base = dirs::config_dir()
        .or_else(dirs::data_local_dir)
        .or_else(|| std::env::current_dir().ok())
        .context("cannot locate a config directory")?;
    Ok(base.join(APP_DIR))
}

/// Root of the on-disk dataset cache: `<data_dir>/cache`.
pub fn cache_dir() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("cache"))
}

/// Create `dir` (and all parents) if missing, returning it unchanged so calls
/// can be chained.
pub fn ensure_dir(dir: PathBuf) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating directory {}", dir.display()))?;
    Ok(dir)
}

/// Path to `name` inside the config directory, ensuring the directory exists.
pub fn config_file(name: &str) -> anyhow::Result<PathBuf> {
    Ok(ensure_dir(config_dir()?)?.join(name))
}

/// Path to `name` directly inside the data directory, ensuring it exists.
/// Use for top-level state files (e.g. `pipelines.json`, `active_downloads.json`).
pub fn data_file(name: &str) -> anyhow::Result<PathBuf> {
    Ok(ensure_dir(data_dir()?)?.join(name))
}

/// Path to `name` inside an (optionally nested) cache subdirectory, ensuring the
/// subdirectory exists.  Pass `""` for a file at the cache root.
///
/// ```text
/// cache_file("downloads", "airports.geojson")
///   -> <data_dir>/cache/downloads/airports.geojson
/// cache_file("", "ne_50m_admin0.geojson")
///   -> <data_dir>/cache/ne_50m_admin0.geojson
/// ```
pub fn cache_file(subdir: &str, name: &str) -> anyhow::Result<PathBuf> {
    let dir = if subdir.is_empty() {
        cache_dir()?
    } else {
        cache_dir()?.join(subdir)
    };
    Ok(ensure_dir(dir)?.join(name))
}

// ─── Cache I/O hardening ────────────────────────────────────────────────────────
//
// Every on-disk cache (the download catalog, Natural Earth data, and user
// pipelines) writes through [`write_atomic`] and reads through
// [`read_if_fresh`] / [`read_stale`].  Centralising the I/O here gives all of
// them the same offline-resilient behaviour: a crash mid-write can never leave a
// truncated file, an empty/partial file is treated as absent, and a stale copy
// is still available when the network is down.

/// Atomically replace `path` with `contents`.
///
/// Writes a sibling temp file (named `<file>.<pid>.tmp`, so it lands on the same
/// filesystem and never collides across processes) then renames it over `path`.
/// `std::fs::rename` is atomic and replaces the destination on every supported
/// platform, so a reader sees either the old file or the complete new one —
/// never a half-written cache.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cache".to_string());
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", std::process::id()));
    // Best-effort cleanup of the temp file if the rename fails.
    let result = std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Read `path` as text if it exists and is younger than `max_age`.
///
/// Returns `None` when the file is missing, older than `max_age`, or empty
/// (an empty file is treated as a corrupt/partial cache, not a valid result).
pub fn read_if_fresh(path: &Path, max_age: Duration) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    // A clock that jumped backwards yields Err → treat as stale.
    let age = SystemTime::now().duration_since(modified).ok()?;
    if age < max_age {
        read_stale(path)
    } else {
        None
    }
}

/// Read `path` as text regardless of age, for use as an offline fallback.
/// Returns `None` if the file is missing or blank so a truncated write never
/// surfaces as valid (empty) content.
pub fn read_stale(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_namespaced_under_app_dir() {
        let data = data_dir().expect("data dir resolves");
        let config = config_dir().expect("config dir resolves");
        assert_eq!(data.file_name().unwrap(), APP_DIR);
        assert_eq!(config.file_name().unwrap(), APP_DIR);
    }

    #[test]
    fn cache_dir_lives_under_data_dir() {
        let cache = cache_dir().expect("cache dir resolves");
        assert!(cache.ends_with(PathBuf::from(APP_DIR).join("cache")));
        assert!(cache.starts_with(data_dir().unwrap()));
    }

    #[test]
    fn cache_file_with_subdir_creates_and_nests() {
        let p = cache_file("downloads", "unit_test.geojson").expect("cache file path");
        assert!(p.parent().unwrap().is_dir(), "subdir should be created");
        assert!(p.ends_with(
            PathBuf::from("cache")
                .join("downloads")
                .join("unit_test.geojson")
        ));
    }

    #[test]
    fn cache_file_empty_subdir_lives_at_cache_root() {
        let p = cache_file("", "unit_test_root.geojson").expect("cache file path");
        assert_eq!(p.parent().unwrap(), cache_dir().unwrap());
        assert!(p.parent().unwrap().is_dir());
    }

    #[test]
    fn config_and_data_files_ensure_parent() {
        let cfg = config_file("unit_test_settings.toml").expect("config file path");
        assert!(cfg.parent().unwrap().is_dir());
        let data = data_file("unit_test_state.json").expect("data file path");
        assert!(data.parent().unwrap().is_dir());
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("glauxb_pp_{}_{}", std::process::id(), name))
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let path = tmp("atomic.txt");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        // Overwrite is atomic and leaves the destination complete.
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        // No leftover temp sibling.
        let leftover = path.with_file_name(format!("atomic.txt.{}.tmp", std::process::id()));
        assert!(!leftover.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_if_fresh_respects_age_and_emptiness() {
        let path = tmp("fresh.txt");
        write_atomic(&path, b"data").unwrap();
        assert_eq!(
            read_if_fresh(&path, Duration::from_secs(3600)).as_deref(),
            Some("data")
        );
        assert!(read_if_fresh(&path, Duration::from_secs(0)).is_none());
        // Empty file → treated as absent even when fresh.
        write_atomic(&path, b"   ").unwrap();
        assert!(read_if_fresh(&path, Duration::from_secs(3600)).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_stale_falls_back_but_skips_missing_and_empty() {
        let path = tmp("stale.txt");
        assert!(read_stale(&path).is_none()); // missing
        write_atomic(&path, b"\n\t ").unwrap();
        assert!(read_stale(&path).is_none()); // blank
        write_atomic(&path, b"payload").unwrap();
        assert_eq!(read_stale(&path).as_deref(), Some("payload"));
        let _ = std::fs::remove_file(&path);
    }
}
