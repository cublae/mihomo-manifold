# home-manager module. Installs the GUI and writes a *defaults* layer that the
# application merges underneath its own mutable config, so declarative settings
# never fight with what the user changes in the UI.
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.mihomo-manifold;
  jsonFormat = pkgs.formats.json { };
in
{
  options.programs.mihomo-manifold = {
    enable = lib.mkEnableOption "MihomoManifold";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.mihomo-manifold;
      description = "The MihomoManifold package to install.";
    };

    corePath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/run/wrappers/bin/mihomo";
      description = ''
        Path to the mihomo binary. Leave null to use MIHOMO_MANIFOLD_CORE from
        the NixOS module, which points at the capability wrapper when TUN is on.
      '';
    };

    defaults = lib.mkOption {
      type = jsonFormat.type;
      default = { };
      example = lib.literalExpression ''
        {
          core = { tun_enabled = true; mixed_port = 7890; };
          hwid = { device_model = "ThinkPad X1"; };
        }
      '';
      description = ''
        Declarative defaults merged underneath the mutable runtime config at
        `$XDG_CONFIG_HOME/mihomo-manifold/config.json`. Anything the user
        changes in the UI wins over these.
      '';
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Open MihomoManifold at login as a systemd user service. There is no tray
        icon yet, so this shows the window; to have the core come up with it, also
        enable "Start the core when the app opens" in Settings.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."mihomo-manifold/defaults.json" = lib.mkIf (cfg.defaults != { }) {
      source = jsonFormat.generate "mihomo-manifold-defaults.json" cfg.defaults;
    };

    home.sessionVariables = lib.mkIf (cfg.corePath != null) {
      MIHOMO_MANIFOLD_CORE = cfg.corePath;
    };

    systemd.user.services.mihomo-manifold = lib.mkIf cfg.autostart {
      Unit = {
        Description = "MihomoManifold";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
