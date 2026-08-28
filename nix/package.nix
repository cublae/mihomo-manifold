{ lib
, rustPlatform
, pkg-config
, wrapGAppsHook4
, glib
, gtk4
, libadwaita
, openssl
, librsvg
, adwaita-icon-theme
, mihomo
}:

rustPlatform.buildRustPackage {
  pname = "mihomo-manifold";
  version = "0.1.0";

  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ pkg-config wrapGAppsHook4 ];
  buildInputs = [ glib gtk4 libadwaita openssl librsvg adwaita-icon-theme ];

  # The GUI shells out to the core; point it at a known-good binary by default.
  # The NixOS module overrides this with the capability wrapper when TUN is on.
  preFixup = ''
    gappsWrapperArgs+=(--set-default MIHOMO_MANIFOLD_CORE "${mihomo}/bin/mihomo")
  '';

  postInstall = ''
    install -Dm444 data/io.github.cublae.MihomoManifold.desktop \
      -t $out/share/applications
  '';

  meta = with lib; {
    description = "GTK4 GUI for the mihomo proxy core with subscriptions, HWID and split routing";
    mainProgram = "mihomo-manifold";
    platforms = platforms.linux;
    license = licenses.gpl3Plus;
  };
}
