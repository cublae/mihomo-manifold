# MihomoManifold

A GTK4/libadwaita front-end for the [mihomo](https://github.com/MetaCubeX/mihomo)
proxy core, built around three things: subscriptions that carry a device
identifier, split routing you control, and a core the GUI owns rather than
hides.

## What it does

- **Subscriptions (Clash/Mihomo YAML).** Only the `proxies:` section is taken
  from the provider. Everything else — `tun`, `dns`, `proxy-groups`, `rules` —
  is generated from your own settings, so a provider updating their profile can
  never rewrite your routing. Profiles are cached on disk, so the core still
  starts without network.
- **HWID headers.** Every subscription request carries `x-hwid`, `x-device-os`,
  `x-ver-os` and `x-device-model`, the way Remnawave counts devices. The
  identifier is a UUIDv5 over `/etc/machine-id` under an app-specific namespace,
  so the raw machine-id never leaves the host, and it survives reboots and NixOS
  generation switches. When a panel rejects the device, the app says so and
  shows the exact HWID it sent instead of failing silently.
- **Split routing.** By application (`PROCESS-NAME` / `PROCESS-PATH`), by
  destination (domain, suffix, keyword, regex, IP-CIDR, GEOIP, GEOSITE, port),
  by remote rule provider, plus a raw block for anything the UI does not model.
- **Live control.** Node picker with per-group latency tests, traffic graph and
  the core log, all over the external controller.

### Rule order

The core takes the first match, so the order the sections are written in is the
whole behaviour:

1. raw "before everything" block
2. application rules
3. private-network bypass (`GEOIP,private` and `.local` / `.lan`), if enabled
4. destination rules
5. rule providers
6. raw "just before the default action" block
7. `MATCH,<default target>`

Application rules sit above the geo lists on purpose — below them they would
never fire. Enabling any application rule also switches the core to
`find-process-mode: always`, without which `PROCESS-*` rules silently never
match.

## Installing on NixOS

Try it without installing anything:

```sh
nix run github:cublae/mihomo-manifold
```

That gets you the GUI and a plain HTTP/SOCKS proxy port. TUN needs the NixOS
module, because granting `CAP_NET_ADMIN` is a system-level change.

**flake.nix**

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    mihomo-manifold.url = "github:cublae/mihomo-manifold";
  };

  outputs = { nixpkgs, mihomo-manifold, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        mihomo-manifold.nixosModules.default
      ];
    };
  };
}
```

**configuration.nix** — the privileged half:

```nix
programs.mihomo-manifold = {
  enable = true;
  users = [ "alice" ];   # allowed to use the capability wrapper
};
```

**home.nix** — the app and, optionally, declarative settings:

```nix
imports = [ inputs.mihomo-manifold.homeManagerModules.default ];

programs.mihomo-manifold = {
  enable = true;
  autostart = false;
  defaults = {
    core = { mixed_port = 7890; tun_enabled = true; tun_stack = "gvisor"; };
    hwid.device_model = "ThinkPad X1";
  };
};
```

The NixOS module installs `/run/wrappers/bin/mihomo` with
`cap_net_admin,cap_net_raw,cap_net_bind_service+ep` and points the GUI at it via
`MIHOMO_MANIFOLD_CORE`, so **the GUI itself never runs as root**.

`defaults` is merged *underneath* the mutable config, and the app writes back
only the settings that differ from it — so a value you never touch in the UI
keeps following your Nix configuration, while anything you do change wins.

## Files

| Path | Contents |
| --- | --- |
| `$XDG_CONFIG_HOME/mihomo-manifold/config.json` | settings, subscription URLs (0600) |
| `$XDG_CONFIG_HOME/mihomo-manifold/defaults.json` | declarative defaults from home-manager |
| `$XDG_STATE_HOME/mihomo-manifold/profiles/*.yaml` | downloaded subscriptions (0600) |
| `$XDG_STATE_HOME/mihomo-manifold/core/config.yaml` | the generated config |
| `$XDG_STATE_HOME/mihomo-manifold/core.log` | core stdout/stderr |

Subscription URLs contain access tokens and are stored in plain 0600 files, not
in a keyring — the core needs them in the clear anyway.

## Development

```sh
nix develop
cargo test
cargo run
```

`mihomo-manifold --print-config` renders the config that would be handed to the
core and exits, which pairs well with `mihomo -t -f`:

```sh
mihomo-manifold --print-config > /tmp/c.yaml && mihomo -t -f /tmp/c.yaml
```

### A note on icon themes

GTK4 refuses to draw symbolic SVGs whose paths are wrapped in a `<g>` element,
and several popular third-party icon themes (Tela among them) ship exactly that.
Primary actions therefore carry a text label next to the icon, so they stay
usable no matter which icon theme is active.
