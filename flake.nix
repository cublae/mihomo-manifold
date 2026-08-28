{
  description = "MihomoManifold — GTK4 GUI for the mihomo proxy core";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        mihomo-manifold = pkgs.callPackage ./nix/package.nix { };
        default = mihomo-manifold;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          strictDeps = false;
          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            pkg-config
            wrapGAppsHook4
          ];
          buildInputs = with pkgs; [
            glib
            gtk4
            libadwaita
            pango
            gdk-pixbuf
            graphene
            cairo
            openssl
            mihomo
          ];
          # cargo needs the sources of the C libs for bindgen-free -sys crates
          env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          shellHook = ''
            # GTK finds its settings schemas through XDG_DATA_DIRS. Without this a
            # `cargo run` binary logs GIO criticals the wrapped package never hits.
            export XDG_DATA_DIRS="${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.adwaita-icon-theme}/share:$XDG_DATA_DIRS"
            echo "MihomoManifold dev shell — cargo $(cargo --version | cut -d' ' -f2), mihomo $(mihomo -v 2>/dev/null | head -1)"
          '';
        };
      });

      nixosModules.default = import ./nix/nixos-module.nix self;
      homeManagerModules.default = import ./nix/hm-module.nix self;

      formatter = forAllSystems (pkgs: pkgs.nixpkgs-fmt);
    };
}
