//! Subscription fetching. Only Clash/Mihomo YAML is accepted, and only its
//! `proxies` section is kept — see `template.rs` for why.

use serde_yaml::Value;
use std::time::Duration;

use crate::config::{HwidSettings, Subscription, UserInfo};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("device limit reached: {0}")]
    DeviceLimit(String),
    #[error("server returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("this is not a Clash/Mihomo YAML profile ({0})")]
    NotClash(String),
    #[error("could not store the profile: {0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub proxies: Vec<Value>,
    pub user_info: Option<UserInfo>,
    pub title: Option<String>,
}

/// `upload=1; download=2; total=3; expire=4`
fn parse_user_info(header: &str) -> UserInfo {
    let mut info = UserInfo::default();
    for part in header.split(';') {
        let (key, value) = match part.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let value = value.trim();
        match key.trim() {
            "upload" => info.upload = value.parse().unwrap_or(0),
            "download" => info.download = value.parse().unwrap_or(0),
            "total" => info.total = value.parse().unwrap_or(0),
            "expire" => info.expire = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    info
}

/// Remnawave answers a rejected device with a 4xx and a JSON body; everything
/// else is reported verbatim so the user can see what the panel said.
fn classify_error(status: u16, body: &str) -> FetchError {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());

    let lowered = message.to_lowercase();
    let looks_like_limit = lowered.contains("device")
        || lowered.contains("hwid")
        || lowered.contains("limit")
        || lowered.contains("устройств");

    if (400..500).contains(&status) && looks_like_limit {
        FetchError::DeviceLimit(message)
    } else {
        FetchError::Http { status, message }
    }
}

fn extract_proxies(raw: &str) -> Result<(Vec<Value>, Option<String>), FetchError> {
    let doc: Value = serde_yaml::from_str(raw)
        .map_err(|e| FetchError::NotClash(format!("YAML parse failed: {e}")))?;

    let proxies = doc
        .get("proxies")
        .ok_or_else(|| FetchError::NotClash("no `proxies:` section".to_string()))?;

    let list = proxies
        .as_sequence()
        .ok_or_else(|| FetchError::NotClash("`proxies:` is not a list".to_string()))?;

    if list.is_empty() {
        return Err(FetchError::NotClash("`proxies:` is empty".to_string()));
    }

    // Some panels put a human-readable name in the profile.
    let title = doc
        .get("profile-name")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok((list.clone(), title))
}

/// Download one subscription and persist its raw YAML next to the other profiles.
pub async fn fetch(sub: Subscription, hwid: HwidSettings) -> Result<Fetched, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let hwid_value = hwid.value();
    let mut request = client.get(sub.url.trim());

    if sub.send_hwid {
        for (name, value) in hwid.headers() {
            request = request.header(name, value);
        }
    } else {
        request = request.header("user-agent", hwid.effective_user_agent());
    }

    // User headers win, and may reference the identifier via {hwid}.
    for (name, value) in &sub.headers {
        request = request.header(name.as_str(), value.replace("{hwid}", &hwid_value));
    }

    let response = request
        .send()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let status = response.status();
    let user_info = response
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(parse_user_info);
    let header_title = response
        .headers()
        .get("profile-title")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let body = response
        .text()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(classify_error(status.as_u16(), &body));
    }

    let (proxies, doc_title) = extract_proxies(&body)?;

    crate::paths::write_private(&sub.profile_path(), &body)
        .map_err(|e| FetchError::Io(e.to_string()))?;

    Ok(Fetched {
        proxies,
        user_info,
        title: header_title.or(doc_title),
    })
}

/// Re-read a previously downloaded profile, so the core can be started offline.
pub fn load_cached(sub: &Subscription) -> Option<Vec<Value>> {
    let raw = std::fs::read_to_string(sub.profile_path()).ok()?;
    extract_proxies(&raw).ok().map(|(proxies, _)| proxies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_traffic_counters() {
        let info = parse_user_info("upload=100; download=200; total=1000; expire=1789000000");
        assert_eq!(info.used(), 300);
        assert_eq!(info.remaining(), 700);
        assert_eq!(info.expire, 1789000000);
    }

    #[test]
    fn device_limit_is_its_own_error() {
        let err = classify_error(403, r#"{"message":"Device limit reached for this user"}"#);
        assert!(matches!(err, FetchError::DeviceLimit(_)));

        let err = classify_error(500, "upstream exploded");
        assert!(matches!(err, FetchError::Http { status: 500, .. }));
    }

    #[test]
    fn rejects_non_clash_payloads() {
        let err = extract_proxies("dm1lc3M6Ly9leUp3Y3lJNi==").unwrap_err();
        assert!(matches!(err, FetchError::NotClash(_)));
    }

    #[test]
    fn keeps_every_node() {
        let yaml = "proxies:\n  - {name: A, type: vless}\n  - {name: B, type: vless}\nrules: [MATCH,DIRECT]\n";
        let (proxies, _) = extract_proxies(yaml).unwrap();
        assert_eq!(proxies.len(), 2);
    }
}
