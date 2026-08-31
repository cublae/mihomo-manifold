//! Config generation. The subscription only contributes `proxies`; every other
//! section — tun, dns, proxy-groups, rules — is ours, so a provider updating
//! their profile can never rewrite the user's routing.

use anyhow::{Context, Result};
use serde_yaml::{Mapping, Value};

use crate::config::{AppConfig, GroupKind};

fn v<T: Into<Value>>(x: T) -> Value {
    x.into()
}

fn seq<I: IntoIterator<Item = Value>>(items: I) -> Value {
    Value::Sequence(items.into_iter().collect())
}

fn strings<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> Value {
    seq(items.into_iter().map(|s| Value::String(s.into())))
}

fn put(map: &mut Mapping, key: &str, value: Value) {
    map.insert(Value::String(key.to_string()), value);
}

/// Node names in subscription order, used to fill the generated groups.
pub fn proxy_names(proxies: &[Value]) -> Vec<String> {
    proxies
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn tun_section(cfg: &AppConfig) -> Value {
    let mut tun = Mapping::new();
    put(&mut tun, "enable", v(true));
    put(&mut tun, "stack", v(cfg.core.tun_stack.clone()));
    put(&mut tun, "device", v("mihomo-tun"));
    put(&mut tun, "auto-route", v(true));
    // auto-redirect makes the core write its own nftables rules, and only pays
    // off when this host forwards other devices' traffic. On a desktop it buys
    // nothing and adds a second writer to the firewall, so it stays off.
    put(&mut tun, "auto-redirect", v(false));
    put(&mut tun, "auto-detect-interface", v(true));
    put(&mut tun, "strict-route", v(false));
    put(&mut tun, "mtu", v(9000u64));
    put(&mut tun, "dns-hijack", strings(["any:53", "tcp://any:53"]));
    Value::Mapping(tun)
}

fn dns_section(cfg: &AppConfig) -> Value {
    let mut dns = Mapping::new();
    put(&mut dns, "enable", v(true));
    put(&mut dns, "ipv6", v(cfg.core.ipv6));
    put(&mut dns, "listen", v("127.0.0.1:1053"));
    put(&mut dns, "prefer-h3", v(false));
    put(&mut dns, "respect-rules", v(true));
    if cfg.core.fake_ip {
        put(&mut dns, "enhanced-mode", v("fake-ip"));
        put(&mut dns, "fake-ip-range", v("198.18.0.1/16"));
        put(
            &mut dns,
            "fake-ip-filter",
            strings([
                "*.lan",
                "*.local",
                "*.localdomain",
                "localhost",
                "time.*.com",
                "+.pool.ntp.org",
                "+.in-addr.arpa",
                "+.ip6.arpa",
            ]),
        );
    } else {
        put(&mut dns, "enhanced-mode", v("redir-host"));
    }
    put(
        &mut dns,
        "default-nameserver",
        strings(["1.1.1.1", "8.8.8.8"]),
    );
    put(
        &mut dns,
        "nameserver",
        strings(["https://1.1.1.1/dns-query", "https://8.8.8.8/dns-query"]),
    );
    // Resolving the node hostnames themselves must not go through the tunnel.
    put(
        &mut dns,
        "proxy-server-nameserver",
        strings(["https://1.1.1.1/dns-query"]),
    );
    Value::Mapping(dns)
}

fn proxy_groups(cfg: &AppConfig, names: &[String]) -> Value {
    let all_group_names = cfg.routing.group_names();
    let mut groups = Vec::new();

    for spec in &cfg.routing.groups {
        let mut group = Mapping::new();
        put(&mut group, "name", v(spec.name.clone()));
        put(&mut group, "type", v(spec.kind.as_yaml()));

        let mut members: Vec<String> = Vec::new();
        if spec.include_specials {
            members.extend(
                all_group_names
                    .iter()
                    .filter(|n| n.as_str() != spec.name)
                    .cloned(),
            );
            members.push("DIRECT".to_string());
        }

        if spec.filter.trim().is_empty() {
            members.extend(names.iter().cloned());
        } else {
            // Let the core apply the regex over every node it knows about.
            put(&mut group, "include-all-proxies", v(true));
            put(&mut group, "filter", v(spec.filter.clone()));
        }

        // A group with an empty member list makes the core refuse to start.
        if members.is_empty() && spec.filter.trim().is_empty() {
            members.push("DIRECT".to_string());
        }
        put(&mut group, "proxies", strings(members));

        if !matches!(spec.kind, GroupKind::Select) {
            put(&mut group, "url", v(spec.test_url.clone()));
            put(&mut group, "interval", v(spec.interval));
            put(&mut group, "tolerance", v(50u64));
        }
        groups.push(Value::Mapping(group));
    }

    seq(groups)
}

fn rule_providers(cfg: &AppConfig) -> Option<(Value, Vec<String>)> {
    let enabled: Vec<_> = cfg
        .routing
        .rule_providers
        .iter()
        .filter(|p| p.enabled && !p.name.trim().is_empty() && !p.url.trim().is_empty())
        .collect();
    if enabled.is_empty() {
        return None;
    }

    let mut map = Mapping::new();
    let mut rules = Vec::new();
    for provider in enabled {
        let mut entry = Mapping::new();
        put(&mut entry, "type", v("http"));
        put(&mut entry, "url", v(provider.url.clone()));
        put(&mut entry, "behavior", v(provider.behavior.clone()));
        put(&mut entry, "format", v(provider.format.clone()));
        put(&mut entry, "interval", v(provider.interval));
        put(
            &mut entry,
            "path",
            v(format!(
                "./providers/rules/{}.{}",
                provider.name, provider.format
            )),
        );
        map.insert(Value::String(provider.name.clone()), Value::Mapping(entry));

        let mut rule = format!(
            "RULE-SET,{},{}",
            provider.name,
            provider.target.as_rule_target()
        );
        if provider.behavior == "ipcidr" {
            rule.push_str(",no-resolve");
        }
        rules.push(rule);
    }
    Some((Value::Mapping(map), rules))
}

/// Rule order is load-bearing: the core takes the first match, so process rules
/// have to sit above the geo/provider lists or they never fire.
fn rules(cfg: &AppConfig, provider_rules: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();

    out.extend(cfg.routing.raw_prepend.iter().cloned());

    out.extend(
        cfg.routing
            .app_rules
            .iter()
            .filter(|r| r.enabled && !r.value.trim().is_empty())
            .map(|r| r.to_rule()),
    );

    if cfg.core.bypass_private {
        out.push("GEOIP,private,DIRECT,no-resolve".to_string());
        out.push("DOMAIN-SUFFIX,local,DIRECT".to_string());
        out.push("DOMAIN-SUFFIX,lan,DIRECT".to_string());
    }

    out.extend(
        cfg.routing
            .domain_rules
            .iter()
            .filter(|r| r.enabled && !r.value.trim().is_empty())
            .map(|r| r.to_rule(cfg.core.fake_ip)),
    );

    out.extend(provider_rules);
    out.extend(cfg.routing.raw_append.iter().cloned());
    out.push(format!(
        "MATCH,{}",
        cfg.routing.default_target.as_rule_target()
    ));
    out
}

/// Build the full config the core is started with.
pub fn generate(cfg: &AppConfig, proxies: &[Value]) -> Result<String> {
    let mut root = Mapping::new();

    put(&mut root, "mixed-port", v(cfg.core.mixed_port as u64));
    put(&mut root, "allow-lan", v(cfg.core.allow_lan));
    put(&mut root, "bind-address", v("*"));
    put(&mut root, "mode", v("rule"));
    put(&mut root, "log-level", v(cfg.core.log_level.clone()));
    put(&mut root, "ipv6", v(cfg.core.ipv6));
    put(&mut root, "unified-delay", v(true));
    put(&mut root, "tcp-concurrent", v(true));
    put(
        &mut root,
        "external-controller",
        v(cfg.core.controller_addr()),
    );
    put(&mut root, "secret", v(cfg.core.secret.clone()));

    // Matching by process needs the core to look up the owner of every
    // connection; without this the PROCESS-* rules silently never match.
    put(
        &mut root,
        "find-process-mode",
        v(if cfg.routing.uses_process_rules() {
            "always"
        } else {
            "strict"
        }),
    );

    let mut profile = Mapping::new();
    put(&mut profile, "store-selected", v(true));
    put(&mut profile, "store-fake-ip", v(true));
    put(&mut root, "profile", Value::Mapping(profile));

    if cfg.core.tun_enabled {
        put(&mut root, "tun", tun_section(cfg));
    }
    put(&mut root, "dns", dns_section(cfg));

    put(&mut root, "proxies", seq(proxies.iter().cloned()));

    let names = proxy_names(proxies);
    put(&mut root, "proxy-groups", proxy_groups(cfg, &names));

    let provider_rules = match rule_providers(cfg) {
        Some((providers, provider_rules)) => {
            put(&mut root, "rule-providers", providers);
            provider_rules
        }
        None => Vec::new(),
    };

    put(&mut root, "rules", strings(rules(cfg, provider_rules)));

    serde_yaml::to_string(&Value::Mapping(root)).context("serializing generated config")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppMatch, AppRule, DomainRule, MatchKind, Target};

    fn node(name: &str) -> Value {
        let mut m = Mapping::new();
        put(&mut m, "name", v(name));
        put(&mut m, "type", v("vless"));
        Value::Mapping(m)
    }

    #[test]
    fn process_rules_come_before_geo_rules() {
        let mut cfg = AppConfig::default();
        cfg.routing.app_rules.push(AppRule {
            enabled: true,
            match_by: AppMatch::Name,
            value: "steam".into(),
            target: Target::Direct,
            label: "Steam".into(),
        });
        cfg.routing.domain_rules.push(DomainRule {
            enabled: true,
            kind: MatchKind::Geosite,
            value: "youtube".into(),
            target: Target::Group("PROXY".into()),
        });

        let rules = rules(&cfg, Vec::new());
        let steam = rules.iter().position(|r| r.contains("steam")).unwrap();
        let youtube = rules.iter().position(|r| r.contains("youtube")).unwrap();
        assert!(steam < youtube, "process rules must win: {rules:?}");
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn process_rules_switch_on_process_lookup() {
        let mut cfg = AppConfig::default();
        let plain = generate(&cfg, &[node("a")]).unwrap();
        assert!(plain.contains("find-process-mode: strict"));

        cfg.routing.app_rules.push(AppRule {
            value: "telegram-desktop".into(),
            ..Default::default()
        });
        let with_apps = generate(&cfg, &[node("a")]).unwrap();
        assert!(with_apps.contains("find-process-mode: always"));
    }

    #[test]
    fn generated_config_keeps_subscription_nodes_only() {
        let cfg = AppConfig::default();
        let yaml = generate(&cfg, &[node("Amsterdam"), node("Frankfurt")]).unwrap();
        let parsed: Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed["proxies"].as_sequence().unwrap().len(), 2);
        assert!(yaml.contains("Amsterdam"));
        // Groups and rules are ours, not the provider's.
        assert!(parsed["proxy-groups"].as_sequence().unwrap().len() == 2);
    }
}
