//! MihomoManifold — a GTK4/libadwaita front-end for the mihomo proxy core.

mod api;
mod config;
mod corectl;
mod hwid;
mod paths;
mod runtime;
mod state;
mod subscription;
mod sysproxy;
mod template;
mod tray;
mod ui;

use adw::prelude::*;

const APP_ID: &str = "io.github.cublae.MihomoManifold";

/// `--print-config` renders what the core would be started with and exits, so
/// the result can be checked with `mihomo -t` without opening a window.
fn print_config() -> Result<(), String> {
    let cfg = config::AppConfig::load();
    let sub = cfg
        .active()
        .ok_or_else(|| "no subscription configured".to_string())?;
    let proxies = subscription::load_cached(sub)
        .ok_or_else(|| format!("no downloaded profile for \"{}\"", sub.name))?;
    let yaml = template::generate(&cfg, &proxies).map_err(|e| e.to_string())?;
    println!("{yaml}");
    Ok(())
}

fn main() -> gtk::glib::ExitCode {
    if std::env::args().any(|arg| arg == "--print-config") {
        return match print_config() {
            Ok(()) => gtk::glib::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("mihomo-manifold: {err}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }

    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_activate(ui::build_window);

    // Never leave a core behind when the GUI goes away.
    app.connect_shutdown(|_| corectl::stop());

    app.run()
}
