//! Window assembly. Each page is a plain function that returns a widget and
//! registers a refresh closure with the shared state.

mod dashboard;
mod logs;
mod proxies;
mod routing;
mod settings;
mod subscriptions;
mod widgets;

use adw::prelude::*;
use gtk::glib;

use crate::state::{self, AppState};

pub fn build_window(app: &adw::Application) {
    let state = AppState::new();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("MihomoManifold")
        .default_width(1000)
        .default_height(740)
        .width_request(360)
        .height_request(480)
        .build();

    let toaster = adw::ToastOverlay::new();
    state.attach_toaster(&toaster);

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &dashboard::page(&state),
        Some("dashboard"),
        "Dashboard",
        "network-transmit-receive-symbolic",
    );
    stack.add_titled_with_icon(
        &proxies::page(&state),
        Some("proxies"),
        "Nodes",
        "network-workgroup-symbolic",
    );
    stack.add_titled_with_icon(
        &routing::page(&state),
        Some("routing"),
        "Routing",
        "document-properties-symbolic",
    );
    stack.add_titled_with_icon(
        &subscriptions::page(&state),
        Some("subscriptions"),
        "Subscriptions",
        "folder-download-symbolic",
    );
    stack.add_titled_with_icon(
        &logs::page(&state),
        Some("logs"),
        "Logs",
        "utilities-terminal-symbolic",
    );
    stack.add_titled_with_icon(
        &settings::page(&state),
        Some("settings"),
        "Settings",
        "emblem-system-symbolic",
    );

    let header = adw::HeaderBar::new();
    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header.set_title_widget(Some(&switcher));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    // Narrow windows get the switcher at the bottom instead of in the header.
    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    toolbar.add_bottom_bar(&switcher_bar);

    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        600.0,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    breakpoint.add_setter(
        &header,
        "title-widget",
        Some(&None::<gtk::Widget>.to_value()),
    );
    window.add_breakpoint(breakpoint);

    toaster.set_child(Some(&toolbar));
    window.set_content(Some(&toaster));

    // ---- tray ----
    let tray = crate::tray::spawn(false);

    let command_state = state.clone();
    let command_window = window.clone();
    let command_app = app.clone();
    let commands = tray.commands.clone();
    glib::spawn_future_local(async move {
        while let Ok(command) = commands.recv().await {
            match command {
                crate::tray::TrayCommand::Show => command_window.present(),
                crate::tray::TrayCommand::ToggleCore => {
                    if command_state.is_running() {
                        state::stop(&command_state);
                    } else {
                        state::apply(&command_state);
                    }
                }
                crate::tray::TrayCommand::Quit => {
                    // Closing would only hide it once the tray is up.
                    command_window.set_hide_on_close(false);
                    command_app.quit();
                }
            }
        }
    });

    // Only start hiding on close once something is actually showing the icon,
    // or the window would vanish with no way to bring it back.
    let hold_app = app.clone();
    let hold_window = window.clone();
    let started = tray.started.clone();
    glib::spawn_future_local(async move {
        if started.recv().await == Ok(true) {
            hold_window.set_hide_on_close(true);
            // Without a hold the application quits as soon as the last window
            // goes away, taking the core with it.
            std::mem::forget(hold_app.hold());
        }
    });

    let tray_status = tray.status.clone();
    state.subscribe(move |state| {
        let _ = tray_status.try_send(state.is_running());
    });

    // First paint, then find out whether a core is already running.
    state.notify();
    state::refresh_status(&state);

    if state.config.borrow().core.autostart_core {
        state::apply(&state);
    }

    let poll_state = state.clone();
    glib::timeout_add_seconds_local(5, move || {
        state::refresh_status(&poll_state);
        glib::ControlFlow::Continue
    });

    window.present();
}
