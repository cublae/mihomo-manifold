//! Settings: how the core is launched, TUN, and the device identity sent to the
//! panel. Anything here changes the generated config, so most of it needs an
//! Apply on the dashboard to take effect.

use adw::prelude::*;
use std::rc::Rc;

use crate::config::HwidMode;
use crate::corectl;
use crate::state::{self, AppState};
use crate::ui::widgets;

const LOG_LEVELS: [&str; 5] = ["silent", "error", "warning", "info", "debug"];
const TUN_STACKS: [&str; 3] = ["gvisor", "system", "mixed"];

fn preview_config(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>) {
    let yaml = match state::render_config(state) {
        Ok(yaml) => yaml,
        Err(err) => {
            state.toast(&err);
            return;
        }
    };

    let view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    view.buffer().set_text(&yaml);

    let (dialog, confirm) = widgets::form_dialog("Generated config.yaml", "Copy", &view);
    let copy_target = view.clone();
    let copy_state = state.clone();
    confirm.connect_clicked(move |_| {
        widgets::copy_to_clipboard(&copy_target, &widgets::text_of(&copy_target));
        copy_state.toast("Copied to clipboard");
    });
    dialog.present(Some(parent.as_ref()));
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let (scroller, content) = widgets::page_container();

    state.subscribe(move |state| {
        widgets::clear(&content);

        // ---------------------------------------------------------- core
        let core_group = adw::PreferencesGroup::builder()
            .title("Core")
            .description("Changes take effect the next time you apply the configuration.")
            .build();

        let binary = adw::EntryRow::builder().title("mihomo binary").build();
        binary.set_text(&state.config.borrow().core.binary);
        let resolved = state.config.borrow().core.resolve_binary();
        let binary_hint = adw::ActionRow::builder()
            .title("Resolved to")
            .subtitle(&resolved)
            .build();
        binary_hint.add_css_class("property");
        let binary_state = state.clone();
        binary.connect_changed(move |entry| {
            if binary_state.is_refreshing() {
                return;
            }
            binary_state.config.borrow_mut().core.binary = entry.text().to_string();
            binary_state.save();
        });
        core_group.add(&binary);
        core_group.add(&binary_hint);

        let mixed_port = adw::SpinRow::with_range(1.0, 65535.0, 1.0);
        mixed_port.set_title("Mixed proxy port");
        mixed_port.set_subtitle("HTTP and SOCKS on one port");
        mixed_port.set_value(state.config.borrow().core.mixed_port as f64);
        let port_state = state.clone();
        mixed_port.connect_value_notify(move |row| {
            if port_state.is_refreshing() {
                return;
            }
            port_state.config.borrow_mut().core.mixed_port = row.value() as u16;
            port_state.save();
        });
        core_group.add(&mixed_port);

        let controller_port = adw::SpinRow::with_range(1.0, 65535.0, 1.0);
        controller_port.set_title("Controller port");
        controller_port.set_subtitle("Where the GUI talks to the core");
        controller_port.set_value(state.config.borrow().core.controller_port as f64);
        let controller_state = state.clone();
        controller_port.connect_value_notify(move |row| {
            if controller_state.is_refreshing() {
                return;
            }
            controller_state.config.borrow_mut().core.controller_port = row.value() as u16;
            controller_state.save();
        });
        core_group.add(&controller_port);

        let secret = adw::PasswordEntryRow::builder()
            .title("Controller secret")
            .build();
        secret.set_text(&state.config.borrow().core.secret);
        let regenerate = widgets::icon_button("view-refresh-symbolic", "Generate a new secret");
        let regenerate_state = state.clone();
        regenerate.connect_clicked(move |_| {
            regenerate_state.config.borrow_mut().core.secret =
                uuid::Uuid::new_v4().simple().to_string();
            regenerate_state.commit();
            regenerate_state.toast("New secret generated — restart the core to use it.");
        });
        secret.add_suffix(&regenerate);
        let secret_state = state.clone();
        secret.connect_changed(move |entry| {
            if secret_state.is_refreshing() {
                return;
            }
            secret_state.config.borrow_mut().core.secret = entry.text().to_string();
            secret_state.save();
        });
        core_group.add(&secret);

        let log_level = adw::ComboRow::builder()
            .title("Log level")
            .model(&widgets::string_list(&LOG_LEVELS))
            .build();
        let current_level = state.config.borrow().core.log_level.clone();
        if let Some(index) = LOG_LEVELS.iter().position(|l| *l == current_level) {
            log_level.set_selected(index as u32);
        }
        let level_state = state.clone();
        log_level.connect_selected_notify(move |combo| {
            if level_state.is_refreshing() {
                return;
            }
            level_state.config.borrow_mut().core.log_level =
                LOG_LEVELS[combo.selected().min(4) as usize].to_string();
            level_state.save();
        });
        core_group.add(&log_level);

        let allow_lan = adw::SwitchRow::builder()
            .title("Allow LAN")
            .subtitle("Let other machines use this proxy port")
            .build();
        allow_lan.set_active(state.config.borrow().core.allow_lan);
        let lan_state = state.clone();
        allow_lan.connect_active_notify(move |row| {
            if lan_state.is_refreshing() {
                return;
            }
            lan_state.config.borrow_mut().core.allow_lan = row.is_active();
            lan_state.save();
        });
        core_group.add(&allow_lan);

        let ipv6 = adw::SwitchRow::builder().title("IPv6").build();
        ipv6.set_active(state.config.borrow().core.ipv6);
        let ipv6_state = state.clone();
        ipv6.connect_active_notify(move |row| {
            if ipv6_state.is_refreshing() {
                return;
            }
            ipv6_state.config.borrow_mut().core.ipv6 = row.is_active();
            ipv6_state.save();
        });
        core_group.add(&ipv6);

        let autostart = adw::SwitchRow::builder()
            .title("Start the core when the app opens")
            .build();
        autostart.set_active(state.config.borrow().core.autostart_core);
        let autostart_state = state.clone();
        autostart.connect_active_notify(move |row| {
            if autostart_state.is_refreshing() {
                return;
            }
            autostart_state.config.borrow_mut().core.autostart_core = row.is_active();
            autostart_state.save();
        });
        core_group.add(&autostart);
        content.append(&core_group);

        // ---------------------------------------------------------- tun
        let tun_group = adw::PreferencesGroup::builder()
            .title("TUN")
            .description("Required for routing by application and for UDP traffic.")
            .build();

        let tun_enabled = adw::SwitchRow::builder()
            .title("Capture all traffic (TUN)")
            .build();
        tun_enabled.set_active(state.config.borrow().core.tun_enabled);
        let tun_state = state.clone();
        tun_enabled.connect_active_notify(move |row| {
            if tun_state.is_refreshing() {
                return;
            }
            tun_state.config.borrow_mut().core.tun_enabled = row.is_active();
            tun_state.commit();
        });
        tun_group.add(&tun_enabled);

        let capability_row = adw::ActionRow::builder()
            .title("Privileges")
            .subtitle(corectl::tun_readiness(&resolved).describe())
            .build();
        capability_row.add_css_class("property");
        tun_group.add(&capability_row);

        let stack = adw::ComboRow::builder()
            .title("Network stack")
            .model(&widgets::string_list(&TUN_STACKS))
            .build();
        let current_stack = state.config.borrow().core.tun_stack.clone();
        if let Some(index) = TUN_STACKS.iter().position(|s| *s == current_stack) {
            stack.set_selected(index as u32);
        }
        let stack_state = state.clone();
        stack.connect_selected_notify(move |combo| {
            if stack_state.is_refreshing() {
                return;
            }
            stack_state.config.borrow_mut().core.tun_stack =
                TUN_STACKS[combo.selected().min(2) as usize].to_string();
            stack_state.save();
        });
        tun_group.add(&stack);

        let bypass = adw::SwitchRow::builder()
            .title("Keep local networks off the tunnel")
            .subtitle("Adds a private-IP and .local/.lan bypass above your destination rules")
            .build();
        bypass.set_active(state.config.borrow().core.bypass_private);
        let bypass_state = state.clone();
        bypass.connect_active_notify(move |row| {
            if bypass_state.is_refreshing() {
                return;
            }
            bypass_state.config.borrow_mut().core.bypass_private = row.is_active();
            bypass_state.save();
        });
        tun_group.add(&bypass);

        let fake_ip = adw::SwitchRow::builder()
            .title("fake-ip DNS")
            .subtitle("Faster and needed for reliable domain rules under TUN")
            .build();
        fake_ip.set_active(state.config.borrow().core.fake_ip);
        let fake_ip_state = state.clone();
        fake_ip.connect_active_notify(move |row| {
            if fake_ip_state.is_refreshing() {
                return;
            }
            fake_ip_state.config.borrow_mut().core.fake_ip = row.is_active();
            fake_ip_state.save();
        });
        tun_group.add(&fake_ip);
        content.append(&tun_group);

        // ---------------------------------------------------------- identity
        let identity_group = adw::PreferencesGroup::builder()
            .title("Device identity")
            .description("Sent with every subscription request. Panels that enforce a device limit count these.")
            .build();

        let mode = adw::ComboRow::builder()
            .title("HWID source")
            .model(&widgets::string_list(&[
                "Derived from /etc/machine-id",
                "Entered manually",
            ]))
            .build();
        mode.set_selected(match state.config.borrow().hwid.mode {
            HwidMode::Auto => 0,
            HwidMode::Manual => 1,
        });
        let mode_state = state.clone();
        mode.connect_selected_notify(move |combo| {
            if mode_state.is_refreshing() {
                return;
            }
            mode_state.config.borrow_mut().hwid.mode = if combo.selected() == 1 {
                HwidMode::Manual
            } else {
                HwidMode::Auto
            };
            mode_state.commit();
        });
        identity_group.add(&mode);

        let value = state.config.borrow().hwid.value();
        let value_row = adw::ActionRow::builder()
            .title("Current HWID")
            .subtitle(&value)
            .build();
        value_row.add_css_class("property");
        let copy = widgets::icon_button("edit-copy-symbolic", "Copy");
        let copy_value = value.clone();
        let copy_state = state.clone();
        copy.connect_clicked(move |button| {
            widgets::copy_to_clipboard(button, &copy_value);
            copy_state.toast("HWID copied");
        });
        value_row.add_suffix(&copy);
        identity_group.add(&value_row);

        if state.config.borrow().hwid.mode == HwidMode::Manual {
            let manual = adw::EntryRow::builder().title("HWID").build();
            manual.set_text(&state.config.borrow().hwid.manual);
            let manual_state = state.clone();
            manual.connect_changed(move |entry| {
                if manual_state.is_refreshing() {
                    return;
                }
                manual_state.config.borrow_mut().hwid.manual = entry.text().to_string();
                manual_state.save();
            });
            identity_group.add(&manual);
        }

        let device_os = adw::EntryRow::builder().title("x-device-os").build();
        device_os.set_text(&state.config.borrow().hwid.device_os);
        let device_os_state = state.clone();
        device_os.connect_changed(move |entry| {
            if device_os_state.is_refreshing() {
                return;
            }
            device_os_state.config.borrow_mut().hwid.device_os = entry.text().to_string();
            device_os_state.save();
        });
        identity_group.add(&device_os);

        let ver_os = adw::EntryRow::builder().title("x-ver-os").build();
        ver_os.set_text(&state.config.borrow().hwid.effective_ver_os());
        let ver_os_state = state.clone();
        ver_os.connect_changed(move |entry| {
            if ver_os_state.is_refreshing() {
                return;
            }
            ver_os_state.config.borrow_mut().hwid.ver_os = entry.text().to_string();
            ver_os_state.save();
        });
        identity_group.add(&ver_os);

        let model = adw::EntryRow::builder().title("x-device-model").build();
        model.set_text(&state.config.borrow().hwid.effective_device_model());
        let model_state = state.clone();
        model.connect_changed(move |entry| {
            if model_state.is_refreshing() {
                return;
            }
            model_state.config.borrow_mut().hwid.device_model = entry.text().to_string();
            model_state.save();
        });
        identity_group.add(&model);

        let user_agent = adw::EntryRow::builder().title("User-Agent").build();
        user_agent.set_text(&state.config.borrow().hwid.effective_user_agent());
        let ua_state = state.clone();
        user_agent.connect_changed(move |entry| {
            if ua_state.is_refreshing() {
                return;
            }
            ua_state.config.borrow_mut().hwid.user_agent = entry.text().to_string();
            ua_state.save();
        });
        identity_group.add(&user_agent);
        content.append(&identity_group);

        // ---------------------------------------------------------- advanced
        let advanced = adw::PreferencesGroup::builder().title("Advanced").build();

        let preview = adw::ActionRow::builder()
            .title("Preview generated config.yaml")
            .subtitle("Exactly what the core is fed")
            .activatable(true)
            .build();
        preview.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let preview_state = state.clone();
        let preview_anchor = content.clone();
        preview.connect_activated(move |_| preview_config(&preview_state, &preview_anchor));
        advanced.add(&preview);

        let locations = adw::ActionRow::builder()
            .title("Files")
            .subtitle(format!(
                "{}\n{}",
                crate::paths::config_file().display(),
                crate::paths::state_dir().display()
            ))
            .build();
        locations.add_css_class("property");
        advanced.add(&locations);
        content.append(&advanced);
    });

    scroller.upcast()
}
