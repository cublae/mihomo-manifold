//! Subscription management. Only Clash/Mihomo YAML is accepted; the nodes are
//! cached on disk so the core can still start without network.

use adw::prelude::*;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::api::format_bytes;
use crate::config::Subscription;
use crate::state::{self, AppState};
use crate::ui::widgets;

/// `Key: value` per line, `{hwid}` substituted at request time.
fn parse_headers(text: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if !key.is_empty() {
                headers.insert(key.to_string(), value.trim().to_string());
            }
        }
    }
    headers
}

fn format_headers(headers: &BTreeMap<String, String>) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn editor(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>, existing: Option<Subscription>) {
    let is_new = existing.is_none();
    let subscription = existing.unwrap_or_default();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let general = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder().title("Name").build();
    name.set_text(&subscription.name);
    let url = adw::EntryRow::builder().title("Subscription URL").build();
    url.set_text(&subscription.url);
    general.add(&name);
    general.add(&url);
    content.append(&general);

    let identity = adw::PreferencesGroup::builder()
        .title("Device identity")
        .description("Sends x-hwid, x-device-os, x-ver-os and x-device-model with the request, the way Remnawave counts devices.")
        .build();
    let send_hwid = adw::SwitchRow::builder().title("Send HWID headers").build();
    send_hwid.set_active(subscription.send_hwid);
    identity.add(&send_hwid);

    let current_hwid = state.config.borrow().hwid.value();
    let hwid_row = adw::ActionRow::builder()
        .title("This device")
        .subtitle(&current_hwid)
        .build();
    hwid_row.add_css_class("property");
    identity.add(&hwid_row);
    content.append(&identity);

    let update = adw::PreferencesGroup::builder().title("Updates").build();
    let interval = adw::SpinRow::with_range(0.0, 10080.0, 30.0);
    interval.set_title("Auto-update interval");
    interval.set_subtitle("Minutes; 0 disables automatic updates");
    interval.set_value(subscription.auto_update_minutes as f64);
    update.add(&interval);
    content.append(&update);

    let headers_group = adw::PreferencesGroup::builder()
        .title("Extra headers")
        .description(
            "One per line as `Key: value`. `{hwid}` is replaced with this device's identifier.",
        )
        .build();
    let (headers_scroller, headers_view) =
        widgets::text_area(&format_headers(&subscription.headers), true);
    headers_group.add(&headers_scroller);
    content.append(&headers_group);

    let (dialog, confirm) = widgets::form_dialog(
        if is_new {
            "Add subscription"
        } else {
            "Edit subscription"
        },
        if is_new { "Add" } else { "Save" },
        &content,
    );

    let save_state = state.clone();
    let dialog_for_save = dialog.clone();
    confirm.connect_clicked(move |_| {
        let url_text = url.text().trim().to_string();
        if url_text.is_empty() {
            save_state.toast("A subscription URL is required.");
            return;
        }

        let mut entry = subscription.clone();
        entry.name = {
            let typed = name.text().trim().to_string();
            if typed.is_empty() {
                "Subscription".to_string()
            } else {
                typed
            }
        };
        entry.url = url_text;
        entry.send_hwid = send_hwid.is_active();
        entry.auto_update_minutes = interval.value() as u64;
        entry.headers = parse_headers(&widgets::text_of(&headers_view));

        let id = entry.id.clone();
        {
            let mut cfg = save_state.config.borrow_mut();
            match cfg.subscription_mut(&id) {
                Some(slot) => *slot = entry,
                None => cfg.subscriptions.push(entry),
            }
            if cfg.active_subscription.is_none() {
                cfg.active_subscription = Some(id.clone());
            }
        }
        save_state.commit();
        dialog_for_save.close();
        state::update_subscription(&save_state, &id, false);
    });

    dialog.present(Some(parent.as_ref()));
}

fn confirm_delete(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>, id: String, name: String) {
    let dialog = adw::AlertDialog::builder()
        .heading("Remove subscription?")
        .body(format!(
            "\"{name}\" and its downloaded nodes will be deleted from this machine."
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Remove");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));

    let state = state.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        {
            let mut cfg = state.config.borrow_mut();
            if let Some(sub) = cfg.subscription(&id) {
                let _ = std::fs::remove_file(sub.profile_path());
            }
            cfg.subscriptions.retain(|s| s.id != id);
            if cfg.active_subscription.as_deref() == Some(id.as_str()) {
                cfg.active_subscription = cfg.subscriptions.first().map(|s| s.id.clone());
            }
        }
        state.commit();
    });
    dialog.present(Some(parent.as_ref()));
}

fn status_line(sub: &Subscription) -> String {
    if let Some(err) = &sub.last_error {
        return format!("⚠ {err}");
    }
    let mut parts = Vec::new();
    if sub.node_count > 0 {
        parts.push(format!("{} nodes", sub.node_count));
    }
    if let Some(info) = sub.user_info {
        if info.total > 0 {
            parts.push(format!(
                "{} of {} used, {} left",
                format_bytes(info.used()),
                format_bytes(info.total),
                format_bytes(info.remaining())
            ));
        }
        if info.expire > 0 {
            parts.push(format!("expires {}", widgets::format_expiry(info.expire)));
        }
    }
    match sub.last_updated {
        Some(ts) => parts.push(format!("updated {}", widgets::format_timestamp(ts))),
        None => parts.push("never updated".to_string()),
    }
    parts.join(" · ")
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let (scroller, content) = widgets::page_container();

    state.subscribe(move |state| {
        widgets::clear(&content);

        let group = adw::PreferencesGroup::builder()
            .title("Subscriptions")
            .description("Only the nodes are taken from the provider — routing stays yours.")
            .build();

        let add = widgets::action_button("list-add-symbolic", "Add");
        let add_state = state.clone();
        let add_anchor = content.clone();
        add.connect_clicked(move |_| editor(&add_state, &add_anchor, None));
        group.set_header_suffix(Some(&add));

        let entries: Vec<Subscription> = state.config.borrow().subscriptions.clone();
        let active_id = state.config.borrow().active().map(|s| s.id.clone());

        if entries.is_empty() {
            let empty = adw::ActionRow::builder()
                .title("No subscriptions yet")
                .subtitle(
                    "Add the URL your panel gave you; the HWID headers are sent automatically.",
                )
                .build();
            group.add(&empty);
        }

        let mut radio_anchor: Option<gtk::CheckButton> = None;
        for sub in entries {
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&sub.name))
                .subtitle(status_line(&sub))
                .build();
            if sub.last_error.is_some() {
                row.add_css_class("error");
            }

            let selector = gtk::CheckButton::builder()
                .valign(gtk::Align::Center)
                .tooltip_text("Use this subscription")
                .build();
            match &radio_anchor {
                Some(anchor) => selector.set_group(Some(anchor)),
                None => radio_anchor = Some(selector.clone()),
            }
            selector.set_active(active_id.as_deref() == Some(sub.id.as_str()));

            let select_state = state.clone();
            let select_id = sub.id.clone();
            selector.connect_toggled(move |button| {
                if select_state.is_refreshing() || !button.is_active() {
                    return;
                }
                select_state.config.borrow_mut().active_subscription = Some(select_id.clone());
                select_state.commit();
            });
            row.add_prefix(&selector);

            let refresh = widgets::icon_button("view-refresh-symbolic", "Update now");
            let refresh_state = state.clone();
            let refresh_id = sub.id.clone();
            refresh.connect_clicked(move |_| {
                state::update_subscription(&refresh_state, &refresh_id, false)
            });
            row.add_suffix(&refresh);

            let edit = widgets::icon_button("document-edit-symbolic", "Edit");
            let edit_state = state.clone();
            let edit_sub = sub.clone();
            let edit_anchor = content.clone();
            edit.connect_clicked(move |_| {
                editor(&edit_state, &edit_anchor, Some(edit_sub.clone()))
            });
            row.add_suffix(&edit);

            let delete = widgets::icon_button("user-trash-symbolic", "Remove");
            let delete_state = state.clone();
            let delete_anchor = content.clone();
            let delete_id = sub.id.clone();
            let delete_name = sub.name.clone();
            delete.connect_clicked(move |_| {
                confirm_delete(
                    &delete_state,
                    &delete_anchor,
                    delete_id.clone(),
                    delete_name.clone(),
                )
            });
            row.add_suffix(&delete);

            group.add(&row);
        }

        content.append(&group);
    });

    scroller.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_round_trip() {
        let parsed = parse_headers("X-Token: abc\n# comment\n\nX-Hwid: {hwid}");
        assert_eq!(parsed.get("X-Token").unwrap(), "abc");
        assert_eq!(parsed.get("X-Hwid").unwrap(), "{hwid}");
        assert_eq!(parsed.len(), 2);
        assert_eq!(format_headers(&parsed), "X-Hwid: {hwid}\nX-Token: abc");
    }
}
