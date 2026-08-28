//! XDG locations. Config (secrets: subscription URLs with access tokens) and
//! runtime state are kept in plain files with 0600/0700 permissions — see the
//! project decision to skip the Secret Service.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("mihomo-manifold")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("mihomo-manifold")
}

/// Mutable settings written by the UI.
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// Declarative defaults from the home-manager module, merged underneath.
pub fn defaults_file() -> PathBuf {
    config_dir().join("defaults.json")
}

/// Raw YAML as downloaded from each subscription, one file per profile id.
pub fn profiles_dir() -> PathBuf {
    state_dir().join("profiles")
}

/// The working directory handed to the core as `-d`.
pub fn core_dir() -> PathBuf {
    state_dir().join("core")
}

/// The config we generate from our own template and feed to the core.
pub fn generated_config() -> PathBuf {
    core_dir().join("config.yaml")
}

pub fn core_log() -> PathBuf {
    state_dir().join("core.log")
}

/// Create a directory tree with 0700 on every component we own.
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

/// Write a file that only the owner can read. Used for anything holding tokens.
pub fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    drop(f);
    // Re-assert the mode: an existing file keeps its old permissions.
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)
}
