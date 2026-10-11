//! Small JSON files in the app's config directory (`~/Library/Application Support/telelogs` on macOS).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

fn path(name: &str) -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("telelogs").join(name))
}

/// The file's contents, or the default if it's missing or can't be read.
pub fn read<T: DeserializeOwned + Default>(name: &str) -> T {
    path(name)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Replaces the file's contents. A failure is reported on stderr; the app carries on without it.
pub fn write<T: Serialize>(name: &str, value: &T) {
    let Some(path) = path(name) else {
        return;
    };
    if let Err(e) = write_to(&path, value) {
        eprintln!("telelogs: saving {}: {e}", path.display());
    }
}

/// Writes through a temporary file, so a crash mid-write can't leave half a file behind.
fn write_to<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)
}
