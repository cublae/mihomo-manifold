//! Lifecycle of the mihomo process. The GUI owns the core as a child process and
//! talks to it over the external controller; when TUN is on, the binary is
//! expected to be the capability wrapper installed by the NixOS module, so the
//! GUI itself never needs privileges.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use crate::config::AppConfig;
use crate::paths;

static CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Where the NixOS module installs the capability wrapper for the core.
pub const NIXOS_WRAPPER: &str = "/run/wrappers/bin/mihomo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreStatus {
    Stopped,
    /// Running as our child.
    Running,
    /// Reachable on the controller port but not started by this GUI.
    Adopted,
    Failed(String),
}

/// Whether the resolved core binary can actually open a TUN device — and if not,
/// which of the two very different reasons applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunReadiness {
    Ready,
    /// No such binary.
    Missing(String),
    /// It exists and carries the capabilities, but this session may not execute
    /// it: on NixOS that means the group membership has not been picked up yet.
    NotPermitted(String),
    /// Executable, but without CAP_NET_ADMIN.
    NoCapabilities(String),
    /// `getcap` is not installed, so there is nothing to check against.
    Unknown,
}

impl TunReadiness {
    /// The warning to show, or `None` when there is nothing to complain about.
    pub fn warning(&self) -> Option<String> {
        match self {
            TunReadiness::Ready | TunReadiness::Unknown => None,
            TunReadiness::Missing(binary) => Some(format!(
                "TUN is on but no core binary was found at {binary}. Set its path in Settings."
            )),
            TunReadiness::NotPermitted(path) => Some(format!(
                "TUN is on and {path} is set up correctly, but this session may not run it. \
                 Log out and back in so the mihomo group applies."
            )),
            TunReadiness::NoCapabilities(path) => Some(format!(
                "TUN is on but {path} has no CAP_NET_ADMIN. \
                 Enable programs.mihomo-manifold.tun in your NixOS configuration."
            )),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            TunReadiness::Ready => "The core binary can create the TUN device.".to_string(),
            TunReadiness::Unknown => {
                "getcap is not installed, so privileges could not be checked.".to_string()
            }
            _ => format!("⚠ {}", self.warning().unwrap_or_default()),
        }
    }
}

/// `exec` is whether the binary could be run at all, `caps` the output of
/// `getcap` when that tool exists. Kept free of I/O so every branch is testable.
fn classify(
    path: String,
    exec: Result<(), std::io::ErrorKind>,
    caps: Option<String>,
) -> TunReadiness {
    match exec {
        Err(std::io::ErrorKind::PermissionDenied) => TunReadiness::NotPermitted(path),
        Err(_) => TunReadiness::Missing(path),
        Ok(()) => match caps {
            Some(text) if text.to_lowercase().contains("cap_net_admin") => TunReadiness::Ready,
            Some(_) => TunReadiness::NoCapabilities(path),
            // getcap is not always installed; do not cry wolf.
            None => TunReadiness::Unknown,
        },
    }
}

pub fn tun_readiness(binary: &str) -> TunReadiness {
    let Some(path) = which(binary) else {
        return TunReadiness::Missing(binary.to_string());
    };

    // Running it is the only honest test of whether we are allowed to: the
    // capability wrapper is mode 0710, so group membership decides.
    let exec = Command::new(&path)
        .arg("-v")
        .output()
        .map(|_| ())
        .map_err(|err| err.kind());

    let caps = Command::new("getcap")
        .arg(&path)
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());

    classify(path, exec, caps)
}

fn which(binary: &str) -> Option<String> {
    if binary.contains('/') {
        return Path::new(binary).exists().then(|| binary.to_string());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.exists())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn is_child_alive() -> bool {
    let mut guard = CHILD.lock().unwrap();
    match guard.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(None) => true,
            // Reap it so a later start does not see a zombie.
            _ => {
                *guard = None;
                false
            }
        },
        None => false,
    }
}

/// Write the generated config and (re)start the core against it.
pub fn start(cfg: &AppConfig, generated_yaml: &str) -> Result<()> {
    let dir = paths::core_dir();
    paths::ensure_dir(&dir).context("creating the core working directory")?;
    let config_path = paths::generated_config();
    paths::write_private(&config_path, generated_yaml).context("writing the generated config")?;

    if is_child_alive() {
        stop();
    }

    let binary = cfg.core.resolve_binary();
    let resolved = which(&binary)
        .ok_or_else(|| anyhow!("mihomo binary not found: {binary}\nSet its path in Settings."))?;

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::core_log())
        .context("opening the core log")?;
    let log_err = log.try_clone()?;

    let child = Command::new(&resolved)
        .arg("-d")
        .arg(&dir)
        .arg("-f")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .with_context(|| format!("starting {resolved}"))?;

    *CHILD.lock().unwrap() = Some(child);
    Ok(())
}

pub fn stop() {
    let mut guard = CHILD.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Last lines of the core log, for the "it would not start" case.
pub fn tail_log(lines: usize) -> String {
    let Ok(content) = std::fs::read_to_string(paths::core_log()) else {
        return String::new();
    };
    let all: Vec<&str> = content.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn group_membership_is_not_a_missing_capability() {
        let readiness = classify(
            "/run/wrappers/bin/mihomo".into(),
            Err(ErrorKind::PermissionDenied),
            None,
        );
        assert_eq!(
            readiness,
            TunReadiness::NotPermitted("/run/wrappers/bin/mihomo".into())
        );
        let warning = readiness.warning().unwrap();
        assert!(warning.contains("Log out and back in"), "{warning}");
        assert!(!warning.contains("CAP_NET_ADMIN"), "{warning}");
    }

    #[test]
    fn capabilities_are_read_from_getcap() {
        let caps = Some(
            "/run/wrappers/bin/mihomo cap_net_bind_service,cap_net_admin,cap_net_raw=ep\n"
                .to_string(),
        );
        assert_eq!(classify("p".into(), Ok(()), caps), TunReadiness::Ready);
        assert_eq!(
            classify("p".into(), Ok(()), Some(String::new())),
            TunReadiness::NoCapabilities("p".into())
        );
    }

    #[test]
    fn no_getcap_means_no_warning() {
        let readiness = classify("p".into(), Ok(()), None);
        assert_eq!(readiness, TunReadiness::Unknown);
        assert!(readiness.warning().is_none());
    }
}
