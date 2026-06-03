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

use std::path::PathBuf;

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
}
