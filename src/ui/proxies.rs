//! Node picker. Everything here reflects live state from the controller, so the
//! page is empty until the core runs.

use adw::prelude::*;
use std::rc::Rc;

use crate::api::{ClashApi, ProxiesResponse};
use crate::runtime;
use crate::state::AppState;
use crate::ui::widgets;

const TEST_URL: &str = "https://cp.cloudflare.com/generate_204";
const TEST_TIMEOUT_MS: u32 = 3000;

fn delay_label(delay: Option<u32>) -> gtk::Label {
    let (text, class) = match delay {
        Some(ms) if ms < 200 => (format!("{ms} ms"), "success"),
        Some(ms) if ms < 600 => (format!("{ms} ms"), "warning"),
        Some(ms) => (format!("{ms} ms"), "error"),
        None => ("—".to_string(), "dim-label"),
    };
    let label = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class(class);
    label.add_css_class("caption");
    label
}

fn placeholder(title: &str, description: &str) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("network-offline-symbolic")
        .title(title)
        .description(description)
        .vexpand(true)
        .build();
    status.upcast()
}

fn populate(state: &Rc<AppState>, container: &gtk::Box, response: ProxiesResponse) {
    widgets::clear(container);

    let configured = state.config.borrow().routing.group_names();
    let mut group_names: Vec<String> = response
        .proxies
        .values()
        .filter(|p| p.is_group() && !p.all.is_empty())
        .map(|p| p.name.clone())
        .collect();

    // Our own groups first, in the order they are generated.
    group_names.sort_by_key(|name| {
        configured
            .iter()
            .position(|c| c == name)
            .unwrap_or(usize::MAX)
    });

    if group_names.is_empty() {
        container.append(&placeholder(
            "No proxy groups",
            "The core is running but reports no groups. Apply the configuration first.",
        ));
        return;
    }

    for group_name in group_names {
        let Some(group) = response.proxies.get(&group_name) else {
            continue;
        };

        let prefs = adw::PreferencesGroup::builder()
            .title(&group_name)
            .description(format!("{} · {} nodes", group.kind, group.all.len()))
            .build();

        let test = widgets::action_button("view-refresh-symbolic", "Test");
        let test_state = state.clone();
        let test_group = group_name.clone();
        test.connect_clicked(move |button| {
            let Some(api) = test_state.api() else { return };
            button.set_sensitive(false);
            let state = test_state.clone();
            let name = test_group.clone();
            let button = button.clone();
            runtime::spawn(
                async move {
                    api.group_delay(&name, TEST_URL, TEST_TIMEOUT_MS)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |result| {
                    button.set_sensitive(true);
                    match result {
                        Ok(_) => state.notify(),
                        Err(err) => state.toast(&format!("Latency test failed: {err}")),
                    }
                },
            );
        });
        prefs.set_header_suffix(Some(&test));

        for member in &group.all {
            let info = response.proxies.get(member);
            let subtitle = info
                .map(|i| i.kind.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let row = adw::ActionRow::builder()
                .title(glib_escape(member))
                .subtitle(subtitle)
                .activatable(true)
                .build();

            if info.is_some_and(|i| i.udp) {
                let udp = gtk::Label::builder()
                    .label("UDP")
                    .valign(gtk::Align::Center)
                    .build();
                udp.add_css_class("dim-label");
                udp.add_css_class("caption");
                row.add_suffix(&udp);
            }
            row.add_suffix(&delay_label(info.and_then(|i| i.last_delay())));

            let selected = group.now.as_deref() == Some(member.as_str());
            if selected {
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.add_css_class("accent");
                row.add_prefix(&check);
            }

            let click_state = state.clone();
            let click_group = group_name.clone();
            let click_member = member.clone();
            row.connect_activated(move |_| {
                let Some(api): Option<ClashApi> = click_state.api() else {
                    return;
                };
                let state = click_state.clone();
                let group = click_group.clone();
                let member = click_member.clone();
                runtime::spawn(
                    async move { api.select(&group, &member).await.map_err(|e| e.to_string()) },
                    move |result| match result {
                        Ok(()) => state.notify(),
                        Err(err) => state.toast(&format!("Could not switch node: {err}")),
                    },
                );
            });

            prefs.add(&row);
        }

        container.append(&prefs);
    }
}

/// Node names can contain markup-looking characters; rows use plain text.
fn glib_escape(text: &str) -> String {
    gtk::glib::markup_escape_text(text).to_string()
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let (scroller, content) = widgets::page_container();

    state.subscribe(move |state| {
        if !state.is_running() {
            widgets::clear(&content);
            content.append(&placeholder(
                "Core is not running",
                "Start the core on the Dashboard to browse and switch nodes.",
            ));
            return;
        }

        let Some(api) = state.api() else { return };
        let state = state.clone();
        let content = content.clone();
        runtime::spawn(
            async move { api.proxies().await.map_err(|e| e.to_string()) },
            move |result| match result {
                Ok(response) => populate(&state, &content, response),
                Err(err) => {
                    widgets::clear(&content);
                    content.append(&placeholder("Controller unreachable", &err));
                }
            },
        );
    });

    scroller.upcast()
}
