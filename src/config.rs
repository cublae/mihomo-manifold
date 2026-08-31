//! Application settings. Persisted as JSON with 0600; the home-manager module
//! may drop a `defaults.json` next to it, which is merged *underneath* so that
//! anything changed in the UI keeps winning over the declarative layer.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::paths;

pub const DEFAULT_USER_AGENT: &str =
    concat!("MihomoManifold/", env!("CARGO_PKG_VERSION"), " clash-meta");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub core: CoreSettings,
    pub hwid: HwidSettings,
    pub subscriptions: Vec<Subscription>,
    pub active_subscription: Option<String>,
    pub routing: RoutingSettings,
}

// ---------------------------------------------------------------- core

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreSettings {
    /// Explicit path to the mihomo binary. Empty means: use MIHOMO_MANIFOLD_CORE,
    /// then fall back to `mihomo` on PATH.
    pub binary: String,
    pub controller_host: String,
    pub controller_port: u16,
    pub secret: String,
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub ipv6: bool,
    pub log_level: String,
    pub tun_enabled: bool,
    pub tun_stack: String,
    pub fake_ip: bool,
    /// Keep LAN and loopback traffic off the tunnel.
    pub bypass_private: bool,
    /// Start the core as soon as the GUI opens.
    pub autostart_core: bool,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            binary: String::new(),
            controller_host: "127.0.0.1".to_string(),
            controller_port: 9097,
            secret: Uuid::new_v4().simple().to_string(),
            mixed_port: 7890,
            allow_lan: false,
            ipv6: false,
            log_level: "info".to_string(),
            tun_enabled: true,
            tun_stack: "gvisor".to_string(),
            fake_ip: true,
            bypass_private: true,
            autostart_core: false,
        }
    }
}

impl CoreSettings {
    pub fn controller_addr(&self) -> String {
        format!("{}:{}", self.controller_host, self.controller_port)
    }

    pub fn controller_url(&self) -> String {
        format!("http://{}", self.controller_addr())
    }

    /// Resolve the core binary: explicit setting, then the environment variable
    /// the Nix modules set (the capability wrapper when TUN is enabled), then PATH.
    pub fn resolve_binary(&self) -> String {
        if !self.binary.trim().is_empty() {
            return self.binary.trim().to_string();
        }
        if let Some(env) = std::env::var_os("MIHOMO_MANIFOLD_CORE") {
            if !env.is_empty() {
                return env.to_string_lossy().into_owned();
            }
        }
        // The NixOS module exports the variable above through the session, which
        // a session started before the rebuild has never seen. Look for the
        // wrapper directly, or TUN would silently fall back to the plain binary
        // on PATH, which cannot open the device.
        if self.tun_enabled && std::path::Path::new(crate::corectl::NIXOS_WRAPPER).exists() {
            return crate::corectl::NIXOS_WRAPPER.to_string();
        }
        "mihomo".to_string()
    }
}

// ---------------------------------------------------------------- hwid

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HwidMode {
    /// UUIDv5 over /etc/machine-id.
    Auto,
    /// Value typed by the user, e.g. to reuse a slot from another machine.
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HwidSettings {
    pub mode: HwidMode,
    pub manual: String,
    pub device_os: String,
    pub ver_os: String,
    pub device_model: String,
    pub user_agent: String,
}

impl Default for HwidSettings {
    fn default() -> Self {
        Self {
            mode: HwidMode::Auto,
            manual: String::new(),
            device_os: "Linux".to_string(),
            ver_os: String::new(),
            device_model: String::new(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

impl HwidSettings {
    /// The value sent as `x-hwid`.
    pub fn value(&self) -> String {
        match self.mode {
            HwidMode::Manual if !self.manual.trim().is_empty() => self.manual.trim().to_string(),
            _ => crate::hwid::derive().unwrap_or_default(),
        }
    }

    pub fn effective_ver_os(&self) -> String {
        if self.ver_os.trim().is_empty() {
            crate::hwid::os_version()
        } else {
            self.ver_os.clone()
        }
    }

    pub fn effective_device_model(&self) -> String {
        if self.device_model.trim().is_empty() {
            crate::hwid::device_model()
        } else {
            self.device_model.clone()
        }
    }

    pub fn effective_user_agent(&self) -> String {
        if self.user_agent.trim().is_empty() {
            DEFAULT_USER_AGENT.to_string()
        } else {
            self.user_agent.clone()
        }
    }

    /// Headers a Remnawave panel expects, plus the UA that selects the format.
    pub fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("user-agent".into(), self.effective_user_agent()),
            ("x-hwid".into(), self.value()),
            ("x-device-os".into(), self.device_os.clone()),
            ("x-ver-os".into(), self.effective_ver_os()),
            ("x-device-model".into(), self.effective_device_model()),
        ]
    }
}

// ---------------------------------------------------------------- subscriptions

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub send_hwid: bool,
    /// Extra request headers; `{hwid}` is substituted before sending.
    pub headers: BTreeMap<String, String>,
    pub auto_update_minutes: u64,
    pub last_updated: Option<i64>,
    pub last_error: Option<String>,
    pub user_info: Option<UserInfo>,
    pub node_count: usize,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: "New subscription".to_string(),
            url: String::new(),
            enabled: true,
            send_hwid: true,
            headers: BTreeMap::new(),
            auto_update_minutes: 360,
            last_updated: None,
            last_error: None,
            user_info: None,
            node_count: 0,
        }
    }
}

impl Subscription {
    pub fn profile_path(&self) -> std::path::PathBuf {
        paths::profiles_dir().join(format!("{}.yaml", self.id))
    }
}

/// Parsed `subscription-userinfo` response header.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    /// Unix seconds; 0 means no expiry reported.
    pub expire: i64,
}

impl UserInfo {
    pub fn used(&self) -> u64 {
        self.upload.saturating_add(self.download)
    }

    pub fn remaining(&self) -> u64 {
        self.total.saturating_sub(self.used())
    }
}

// ---------------------------------------------------------------- routing

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Direct,
    Reject,
    /// Name of a generated proxy group.
    Group(String),
}

impl Target {
    pub fn as_rule_target(&self) -> String {
        match self {
            Target::Direct => "DIRECT".to_string(),
            Target::Reject => "REJECT".to_string(),
            Target::Group(name) => name.clone(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Target::Direct => "Direct".to_string(),
            Target::Reject => "Reject".to_string(),
            Target::Group(name) => name.clone(),
        }
    }
}

/// How an application is matched. `PROCESS-NAME` only works with TUN enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMatch {
    Name,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppRule {
    pub enabled: bool,
    pub match_by: AppMatch,
    /// Process name (`telegram-desktop`) or absolute path.
    pub value: String,
    pub target: Target,
    /// Free-form label shown in the UI, usually the .desktop app name.
    pub label: String,
}

impl Default for AppRule {
    fn default() -> Self {
        Self {
            enabled: true,
            match_by: AppMatch::Name,
            value: String::new(),
            target: Target::Direct,
            label: String::new(),
        }
    }
}

impl AppRule {
    pub fn to_rule(&self) -> String {
        let kind = match self.match_by {
            AppMatch::Name => "PROCESS-NAME",
            AppMatch::Path => "PROCESS-PATH",
        };
        format!("{kind},{},{}", self.value, self.target.as_rule_target())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchKind {
    Domain,
    DomainSuffix,
    DomainKeyword,
    DomainRegex,
    IpCidr,
    Geoip,
    Geosite,
    DstPort,
}

impl MatchKind {
    pub fn as_rule_kind(&self) -> &'static str {
        match self {
            MatchKind::Domain => "DOMAIN",
            MatchKind::DomainSuffix => "DOMAIN-SUFFIX",
            MatchKind::DomainKeyword => "DOMAIN-KEYWORD",
            MatchKind::DomainRegex => "DOMAIN-REGEX",
            MatchKind::IpCidr => "IP-CIDR",
            MatchKind::Geoip => "GEOIP",
            MatchKind::Geosite => "GEOSITE",
            MatchKind::DstPort => "DST-PORT",
        }
    }

    /// Matchers that look at the destination address rather than its name.
    pub fn is_ip_based(&self) -> bool {
        matches!(self, MatchKind::IpCidr | MatchKind::Geoip)
    }

    pub const ALL: [MatchKind; 8] = [
        MatchKind::Domain,
        MatchKind::DomainSuffix,
        MatchKind::DomainKeyword,
        MatchKind::DomainRegex,
        MatchKind::IpCidr,
        MatchKind::Geoip,
        MatchKind::Geosite,
        MatchKind::DstPort,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DomainRule {
    pub enabled: bool,
    pub kind: MatchKind,
    pub value: String,
    pub target: Target,
}

impl Default for DomainRule {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: MatchKind::DomainSuffix,
            value: String::new(),
            target: Target::Direct,
        }
    }
}

impl DomainRule {
    /// `fake_ip` decides whether an address matcher may resolve the destination.
    pub fn to_rule(&self, fake_ip: bool) -> String {
        let base = format!(
            "{},{},{}",
            self.kind.as_rule_kind(),
            self.value,
            self.target.as_rule_target()
        );
        // `no-resolve` keeps an address matcher from looking up the destination.
        // Under fake-ip there is nothing to match against without that lookup —
        // the connection carries a 198.18.x.x placeholder — so an address rule
        // written with it can only ever fire for literal-IP traffic, and a rule
        // like "Russian sites direct" silently never matches a single domain.
        if self.kind.is_ip_based() && !fake_ip {
            format!("{base},no-resolve")
        } else {
            base
        }
    }
}

/// A remote rule list (antifilter, geosite mirrors, …) pulled by the core itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleProvider {
    pub enabled: bool,
    pub name: String,
    pub url: String,
    /// `domain`, `ipcidr` or `classical`.
    pub behavior: String,
    /// `yaml`, `text` or `mrs`.
    pub format: String,
    pub interval: u64,
    pub target: Target,
}

impl Default for RuleProvider {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            url: String::new(),
            behavior: "domain".to_string(),
            format: "yaml".to_string(),
            interval: 86400,
            target: Target::Direct,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupKind {
    Select,
    UrlTest,
    Fallback,
    LoadBalance,
}

impl GroupKind {
    pub fn as_yaml(&self) -> &'static str {
        match self {
            GroupKind::Select => "select",
            GroupKind::UrlTest => "url-test",
            GroupKind::Fallback => "fallback",
            GroupKind::LoadBalance => "load-balance",
        }
    }
}

/// A proxy group we generate. Nodes come from the subscription, everything
/// around them is ours.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupSpec {
    pub name: String,
    pub kind: GroupKind,
    /// Regex over node names; empty means every node.
    pub filter: String,
    /// Prepend DIRECT/REJECT and the other groups as selectable entries.
    pub include_specials: bool,
    pub test_url: String,
    pub interval: u64,
}

impl Default for GroupSpec {
    fn default() -> Self {
        Self {
            name: "PROXY".to_string(),
            kind: GroupKind::Select,
            filter: String::new(),
            include_specials: true,
            test_url: "https://cp.cloudflare.com/generate_204".to_string(),
            interval: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingSettings {
    /// What the final MATCH rule does.
    pub default_target: Target,
    pub groups: Vec<GroupSpec>,
    pub app_rules: Vec<AppRule>,
    pub domain_rules: Vec<DomainRule>,
    pub rule_providers: Vec<RuleProvider>,
    /// Verbatim rules inserted before everything we generate.
    pub raw_prepend: Vec<String>,
    /// Verbatim rules inserted just before the final MATCH.
    pub raw_append: Vec<String>,
}

impl Default for RoutingSettings {
    fn default() -> Self {
        Self {
            default_target: Target::Group("PROXY".to_string()),
            groups: vec![
                GroupSpec::default(),
                GroupSpec {
                    name: "AUTO".to_string(),
                    kind: GroupKind::UrlTest,
                    include_specials: false,
                    ..Default::default()
                },
            ],
            app_rules: Vec::new(),
            domain_rules: Vec::new(),
            rule_providers: Vec::new(),
            raw_prepend: Vec::new(),
            raw_append: Vec::new(),
        }
    }
}

impl RoutingSettings {
    pub fn group_names(&self) -> Vec<String> {
        self.groups.iter().map(|g| g.name.clone()).collect()
    }

    /// Every target the UI can offer, in menu order.
    pub fn available_targets(&self) -> Vec<Target> {
        let mut out = vec![Target::Direct, Target::Reject];
        out.extend(self.groups.iter().map(|g| Target::Group(g.name.clone())));
        out
    }

    pub fn uses_process_rules(&self) -> bool {
        self.app_rules
            .iter()
            .any(|r| r.enabled && !r.value.trim().is_empty())
    }
}

// ---------------------------------------------------------------- load / save

/// Deep-merge `over` into `base`; objects merge key-wise, everything else replaces.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (k, v) in over_map {
                match base_map.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        base_map.insert(k, v);
                    }
                }
            }
        }
        (slot, other) => *slot = other,
    }
}

/// Inverse of [`merge`]: drop every value identical to the declarative layer, so
/// defaults keep applying to settings the user never changed.
fn prune(value: Value, defaults: &Value) -> Value {
    match (value, defaults) {
        (Value::Object(map), Value::Object(defaults)) => {
            let mut kept = serde_json::Map::new();
            for (key, value) in map {
                match defaults.get(&key) {
                    Some(default) if *default == value => {}
                    Some(default) => {
                        let pruned = prune(value, default);
                        // An object that pruned down to nothing carries no override.
                        let empty = pruned.as_object().is_some_and(|o| o.is_empty());
                        if !empty {
                            kept.insert(key, pruned);
                        }
                    }
                    None => {
                        kept.insert(key, value);
                    }
                }
            }
            Value::Object(kept)
        }
        (value, _) => value,
    }
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

impl AppConfig {
    /// defaults.json (declarative) < config.json (what the user changed).
    pub fn load() -> Self {
        let mut merged =
            read_json(&paths::defaults_file()).unwrap_or(Value::Object(Default::default()));
        if let Some(user) = read_json(&paths::config_file()) {
            merge(&mut merged, user);
        }
        match serde_json::from_value::<AppConfig>(merged) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("mihomo-manifold: falling back to defaults, config unreadable: {err}");
                AppConfig::default()
            }
        }
    }

    /// Writes only what differs from the declarative layer. Serializing the whole
    /// struct would shadow every default the moment the user touches one setting,
    /// which is not what "defaults are merged underneath" promises.
    pub fn save(&self) -> Result<()> {
        let full = serde_json::to_value(self).context("serializing config")?;
        let defaults =
            read_json(&paths::defaults_file()).unwrap_or_else(|| Value::Object(Default::default()));
        let json =
            serde_json::to_string_pretty(&prune(full, &defaults)).context("serializing config")?;
        paths::write_private(&paths::config_file(), &json).context("writing config.json")?;
        Ok(())
    }

    pub fn subscription(&self, id: &str) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.id == id)
    }

    pub fn subscription_mut(&mut self, id: &str) -> Option<&mut Subscription> {
        self.subscriptions.iter_mut().find(|s| s.id == id)
    }

    /// The subscription whose nodes are currently fed to the core.
    pub fn active(&self) -> Option<&Subscription> {
        self.active_subscription
            .as_deref()
            .and_then(|id| self.subscription(id))
            .or_else(|| self.subscriptions.iter().find(|s| s.enabled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_config_wins_over_declarative_defaults() {
        let mut base = serde_json::json!({ "core": { "mixed_port": 1080, "ipv6": true } });
        merge(
            &mut base,
            serde_json::json!({ "core": { "mixed_port": 7890 } }),
        );
        assert_eq!(base["core"]["mixed_port"], 7890);
        assert_eq!(base["core"]["ipv6"], true);
    }

    #[test]
    fn saving_does_not_shadow_untouched_defaults() {
        let defaults = serde_json::json!({ "core": { "mixed_port": 1080, "ipv6": true } });
        let full = serde_json::json!({
            "core": { "mixed_port": 1080, "ipv6": false, "log_level": "info" },
            "subscriptions": []
        });

        let pruned = prune(full, &defaults);
        // Untouched: stays the declarative layer's business.
        assert!(pruned["core"].get("mixed_port").is_none());
        // Changed in the UI: written out and wins on the next load.
        assert_eq!(pruned["core"]["ipv6"], false);
        assert_eq!(pruned["core"]["log_level"], "info");

        // And the round trip puts the defaults back.
        let mut reloaded = defaults.clone();
        merge(&mut reloaded, pruned);
        assert_eq!(reloaded["core"]["mixed_port"], 1080);
        assert_eq!(reloaded["core"]["ipv6"], false);
    }

    #[test]
    fn without_a_defaults_file_everything_is_written() {
        let full = serde_json::json!({ "core": { "mixed_port": 7890 } });
        let pruned = prune(full.clone(), &Value::Object(Default::default()));
        assert_eq!(pruned, full);
    }

    #[test]
    fn ip_rules_do_not_resolve() {
        let rule = DomainRule {
            kind: MatchKind::Geoip,
            value: "RU".into(),
            target: Target::Direct,
            enabled: true,
        };
        // Without fake-ip the destination address is already the real one.
        assert_eq!(rule.to_rule(false), "GEOIP,RU,DIRECT,no-resolve");
        // With fake-ip it must be allowed to resolve, or it matches nothing.
        assert_eq!(rule.to_rule(true), "GEOIP,RU,DIRECT");
    }
}
