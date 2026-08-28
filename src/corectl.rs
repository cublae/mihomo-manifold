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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreStatus {
    Stopped,
    /// Running as our child.
    Running,
    /// Reachable on the controller port but not started by this GUI.
    Adopted,
    Failed(String),
}

/// Does the binary have the capabilities TUN needs? Reported as a warning rather
/// than a hard failure, since the core may also be running as root elsewhere.
pub fn tun_capabilities_present(binary: &str) -> bool {
    let path = which(binary);
    let Some(path) = path else { return false };
    // The wrapper lives in /run/wrappers/bin and carries the file capabilities.
    match std::process::Command::new("getcap").arg(&path).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
            text.contains("cap_net_admin")
        }
        // getcap is not always installed; do not cry wolf.
        Err(_) => true,
    }
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
