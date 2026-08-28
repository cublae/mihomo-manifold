//! Client for the mihomo external controller. Both `/traffic` and `/logs` are
//! consumed as plain chunked HTTP streams, which the core supports alongside the
//! websocket upgrade — one less dependency, same data.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct DelaySample {
    #[serde(default)]
    pub delay: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub now: Option<String>,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub history: Vec<DelaySample>,
    #[serde(default)]
    pub udp: bool,
}

impl ProxyInfo {
    pub fn last_delay(&self) -> Option<u32> {
        self.history.last().map(|h| h.delay).filter(|d| *d > 0)
    }

    pub fn is_group(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "Selector" | "URLTest" | "Fallback" | "LoadBalance" | "Relay"
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxiesResponse {
    pub proxies: HashMap<String, ProxyInfo>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Traffic {
    #[serde(default)]
    pub up: u64,
    #[serde(default)]
    pub down: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogLine {
    #[serde(rename = "type", default)]
    pub level: String,
    #[serde(default)]
    pub payload: String,
}

#[derive(Clone)]
pub struct ClashApi {
    base: String,
    secret: String,
    client: reqwest::Client,
}

impl ClashApi {
    pub fn new(base_url: &str, secret: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            // No global timeout: the same client serves the endless log stream.
            .no_proxy()
            .build()
            .context("building the controller client")?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_string(),
            secret: secret.to_string(),
            client,
        })
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let req = self.client.get(format!("{}{}", self.base, path));
        if self.secret.is_empty() {
            req
        } else {
            req.bearer_auth(&self.secret)
        }
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        let req = self.client.put(format!("{}{}", self.base, path));
        if self.secret.is_empty() {
            req
        } else {
            req.bearer_auth(&self.secret)
        }
    }

    /// Cheap liveness probe, also used to adopt a core left over from a crash.
    pub async fn version(&self) -> Result<String> {
        let value: serde_json::Value = self
            .get("/version")
            .timeout(Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(value
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string())
    }

    pub async fn proxies(&self) -> Result<ProxiesResponse> {
        Ok(self
            .get("/proxies")
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Pick `node` inside `group`; the core persists it via `store-selected`.
    pub async fn select(&self, group: &str, node: &str) -> Result<()> {
        let path = format!("/proxies/{}", urlencoding::encode(group));
        self.put(&path)
            .timeout(Duration::from_secs(5))
            .json(&serde_json::json!({ "name": node }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Latency for every member of a group in one request.
    pub async fn group_delay(
        &self,
        group: &str,
        test_url: &str,
        timeout_ms: u32,
    ) -> Result<HashMap<String, u32>> {
        let path = format!(
            "/group/{}/delay?timeout={}&url={}",
            urlencoding::encode(group),
            timeout_ms,
            urlencoding::encode(test_url)
        );
        Ok(self
            .get(&path)
            .timeout(Duration::from_millis(timeout_ms as u64 + 5000))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Hot-reload the generated config without restarting the process.
    pub async fn reload(&self, config_path: &str) -> Result<()> {
        self.put("/configs?force=true")
            .timeout(Duration::from_secs(15))
            .json(&serde_json::json!({ "path": config_path }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn stream_lines(&self, path: &str, tx: async_channel::Sender<String>) -> Result<()> {
        let response = self.get(path).send().await?.error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();

        while let Some(chunk) = stream.next().await {
            buffer.extend_from_slice(&chunk?);
            while let Some(newline) = buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line).trim().to_string();
                if line.is_empty() {
                    continue;
                }
                // A closed receiver means the UI went away; stop reading.
                if tx.send(line).await.is_err() {
                    return Ok(());
                }
            }
            // Guard against a peer that never sends a newline.
            if buffer.len() > 1 << 20 {
                buffer.clear();
            }
        }
        Ok(())
    }

    pub async fn traffic_stream(&self, tx: async_channel::Sender<Traffic>) {
        let (raw_tx, raw_rx) = async_channel::bounded::<String>(64);
        let api = self.clone();
        let reader = tokio::spawn(async move { api.stream_lines("/traffic", raw_tx).await });

        while let Ok(line) = raw_rx.recv().await {
            if let Ok(traffic) = serde_json::from_str::<Traffic>(&line) {
                if tx.send(traffic).await.is_err() {
                    break;
                }
            }
        }
        reader.abort();
    }

    pub async fn logs_stream(&self, level: &str, tx: async_channel::Sender<LogLine>) {
        let path = format!("/logs?level={}", urlencoding::encode(level));
        let (raw_tx, raw_rx) = async_channel::bounded::<String>(256);
        let api = self.clone();
        let reader = tokio::spawn(async move { api.stream_lines(&path, raw_tx).await });

        while let Ok(line) = raw_rx.recv().await {
            if let Ok(entry) = serde_json::from_str::<LogLine>(&line) {
                if tx.send(entry).await.is_err() {
                    break;
                }
            }
        }
        reader.abort();
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanises_sizes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
    }

    #[test]
    fn zero_delay_means_unreachable() {
        let info = ProxyInfo {
            name: "n".into(),
            kind: "Vless".into(),
            now: None,
            all: vec![],
            history: vec![DelaySample { delay: 0 }],
            udp: false,
        };
        assert_eq!(info.last_delay(), None);
    }
}
