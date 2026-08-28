//! Device identity. The HWID is a UUIDv5 derived from /etc/machine-id under an
//! application-specific namespace, so the raw machine-id never leaves the host
//! while the value stays stable across reboots and NixOS generations.

use anyhow::{anyhow, Context, Result};
use uuid::Uuid;

/// Namespace UUID for MihomoManifold. Changing it invalidates every device slot.
const NAMESPACE: &str = "1e0f6d9a-4c3b-4f2a-9c7e-8d5b2a1f6c40";

fn namespace() -> Uuid {
    Uuid::parse_str(NAMESPACE).expect("static namespace uuid is valid")
}

fn read_machine_id() -> Result<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let id = raw.trim().to_string();
            if !id.is_empty() {
                return Ok(id);
            }
        }
    }
    Err(anyhow!(
        "no machine-id found; set an HWID manually in Settings"
    ))
}

/// Stable per-machine identifier sent as `x-hwid`.
pub fn derive() -> Result<String> {
    let machine_id = read_machine_id().context("deriving HWID")?;
    Ok(Uuid::new_v5(&namespace(), machine_id.as_bytes()).to_string())
}

/// Best-effort OS version for `x-ver-os`.
pub fn os_version() -> String {
    if let Ok(release) = std::fs::read_to_string("/etc/os-release") {
        let mut name = None;
        let mut version = None;
        for line in release.lines() {
            let (key, value) = match line.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let value = value.trim_matches('"').to_string();
            match key {
                "NAME" => name = Some(value),
                "VERSION_ID" => version = Some(value),
                _ => {}
            }
        }
        if let Some(name) = name {
            return match version {
                Some(v) => format!("{name} {v}"),
                None => name,
            };
        }
    }
    "Linux".to_string()
}

/// Best-effort machine name for `x-device-model`.
pub fn device_model() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "linux-desktop".to_string())
}
