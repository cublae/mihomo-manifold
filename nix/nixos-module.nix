# NixOS module. Needed only for TUN mode: granting CAP_NET_ADMIN to the core
# is a system-level change and cannot be done from home-manager.
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.mihomo-manifold;
  corePath =
    if cfg.tun.enable
    then "/run/wrappers/bin/mihomo"
    else "${cfg.corePackage}/bin/mihomo";
in
{
  options.programs.mihomo-manifold = {
    enable = lib.mkEnableOption "MihomoManifold, a GTK4 GUI for the mihomo core";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.mihomo-manifold;
      description = "The MihomoManifold package to install.";
    };

    corePackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.mihomo;
      description = "The mihomo core package the GUI drives.";
    };

    tun.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Install a setcap wrapper for the core so it can create the TUN device
        without running the whole GUI as root. Members of the
        `programs.mihomo-manifold.tun.group` group may use it.
      '';
    };

    tun.group = lib.mkOption {
      type = lib.types.str;
      default = "mihomo";
      description = "Group allowed to run the privileged core wrapper.";
    };

    users = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = "Users to add to the TUN group.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package cfg.corePackage ];

    # The GUI reads this to find the core; it points at the wrapper when TUN is on.
    environment.sessionVariables.MIHOMO_MANIFOLD_CORE = corePath;

    users.groups = lib.mkIf cfg.tun.enable { ${cfg.tun.group} = { }; };

    users.users = lib.mkIf cfg.tun.enable (
      lib.genAttrs cfg.users (_: { extraGroups = [ cfg.tun.group ]; })
    );

    security.wrappers = lib.mkIf cfg.tun.enable {
      mihomo = {
        owner = "root";
        group = cfg.tun.group;
        permissions = "u+rx,g+x,o-rwx";
        capabilities = "cap_net_admin,cap_net_raw,cap_net_bind_service+ep";
        source = "${cfg.corePackage}/bin/mihomo";
      };
    };

    boot.kernelModules = lib.mkIf cfg.tun.enable [ "tun" ];
  };
}
