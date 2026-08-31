//! Split routing. The order of the sections on this page is the order the rules
//! are written into config.yaml, because the core takes the first match.

use adw::prelude::*;
use gtk::gio;
use std::rc::Rc;

use crate::config::{AppMatch, AppRule, DomainRule, MatchKind, RuleProvider, Target};
use crate::state::AppState;
use crate::ui::widgets;

fn kind_labels() -> Vec<String> {
    MatchKind::ALL
        .iter()
        .map(|k| k.as_rule_kind().to_string())
        .collect()
}

/// Installed applications, as `(display name, process name)`.
fn installed_apps() -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|info| info.should_show())
        .filter_map(|info| {
            let executable = info.executable();
            let process = executable.file_name()?.to_string_lossy().into_owned();
            Some((info.display_name().to_string(), process))
        })
        .collect();
    apps.sort_by_key(|(name, _)| name.to_lowercase());
    apps.dedup_by(|a, b| a.1 == b.1);
    apps
}

fn app_editor(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let apps = installed_apps();
    let group = adw::PreferencesGroup::builder()
        .title("Application")
        .description("Matching by process requires TUN mode; a plain system proxy cannot see which program opened a connection.")
        .build();

    let mut choices: Vec<String> = vec!["Custom…".to_string()];
    choices.extend(
        apps.iter()
            .map(|(name, process)| format!("{name}  ({process})")),
    );
    let picker = adw::ComboRow::builder()
        .title("Installed application")
        .model(&widgets::string_list(&choices))
        .build();
    group.add(&picker);

    let process = adw::EntryRow::builder()
        .title("Process name or path")
        .build();
    group.add(&process);

    let match_by = adw::ComboRow::builder()
        .title("Match by")
        .model(&widgets::string_list(&["Process name", "Executable path"]))
        .build();
    group.add(&match_by);

    let targets = state.config.borrow().routing.available_targets();
    let target = adw::ComboRow::builder()
        .title("Send through")
        .model(&widgets::string_list(
            &targets.iter().map(Target::label).collect::<Vec<_>>(),
        ))
        .build();
    group.add(&target);
    content.append(&group);

    let picker_process = process.clone();
    let picker_apps = apps.clone();
    picker.connect_selected_notify(move |combo| {
        let index = combo.selected() as usize;
        if index == 0 {
            return;
        }
        if let Some((_, process_name)) = picker_apps.get(index - 1) {
            picker_process.set_text(process_name);
        }
    });

    let (dialog, confirm) = widgets::form_dialog("Add application rule", "Add", &content);
    let save_state = state.clone();
    let dialog_for_save = dialog.clone();
    let targets_for_save = targets.clone();
    confirm.connect_clicked(move |_| {
        let value = process.text().trim().to_string();
        if value.is_empty() {
            save_state.toast("Enter a process name, for example telegram-desktop.");
            return;
        }
        let index = picker.selected() as usize;
        let label = if index > 0 {
            apps.get(index - 1)
                .map(|(name, _)| name.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let rule = AppRule {
            enabled: true,
            match_by: if match_by.selected() == 1 {
                AppMatch::Path
            } else {
                AppMatch::Name
            },
            value,
            target: targets_for_save
                .get(target.selected() as usize)
                .cloned()
                .unwrap_or(Target::Direct),
            label,
        };
        save_state.config.borrow_mut().routing.app_rules.push(rule);
        save_state.commit();
        dialog_for_save.close();
    });

    dialog.present(Some(parent.as_ref()));
}

fn domain_editor(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Destination")
        .description("With fake-ip DNS on, address matchers such as GEOIP resolve the destination — otherwise they would never match a domain.")
        .build();

    let kind = adw::ComboRow::builder()
        .title("Match")
        .model(&widgets::string_list(&kind_labels()))
        .selected(1)
        .build();
    group.add(&kind);

    let value = adw::EntryRow::builder().title("Value").build();
    group.add(&value);

    let targets = state.config.borrow().routing.available_targets();
    let target = adw::ComboRow::builder()
        .title("Send through")
        .model(&widgets::string_list(
            &targets.iter().map(Target::label).collect::<Vec<_>>(),
        ))
        .build();
    group.add(&target);
    content.append(&group);

    let hint = widgets::dim_label(
        "Examples: DOMAIN-SUFFIX github.com · GEOIP RU · GEOSITE category-ads-all · IP-CIDR 10.0.0.0/8 · DST-PORT 25",
    );
    content.append(&hint);

    let (dialog, confirm) = widgets::form_dialog("Add routing rule", "Add", &content);
    let save_state = state.clone();
    let dialog_for_save = dialog.clone();
    confirm.connect_clicked(move |_| {
        let text = value.text().trim().to_string();
        if text.is_empty() {
            save_state.toast("Enter a value to match.");
            return;
        }
        let rule = DomainRule {
            enabled: true,
            kind: MatchKind::ALL[(kind.selected() as usize).min(MatchKind::ALL.len() - 1)],
            value: text,
            target: targets
                .get(target.selected() as usize)
                .cloned()
                .unwrap_or(Target::Direct),
        };
        save_state
            .config
            .borrow_mut()
            .routing
            .domain_rules
            .push(rule);
        save_state.commit();
        dialog_for_save.close();
    });

    dialog.present(Some(parent.as_ref()));
}

fn provider_editor(state: &Rc<AppState>, parent: &impl IsA<gtk::Widget>) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Rule provider")
        .description("A remote list the core downloads and refreshes on its own.")
        .build();

    let name = adw::EntryRow::builder().title("Name").build();
    let url = adw::EntryRow::builder().title("URL").build();
    let behavior = adw::ComboRow::builder()
        .title("Behavior")
        .model(&widgets::string_list(&["domain", "ipcidr", "classical"]))
        .build();
    let format = adw::ComboRow::builder()
        .title("Format")
        .model(&widgets::string_list(&["yaml", "text", "mrs"]))
        .build();
    let interval = adw::SpinRow::with_range(300.0, 604800.0, 300.0);
    interval.set_title("Refresh interval (seconds)");
    interval.set_value(86400.0);

    let targets = state.config.borrow().routing.available_targets();
    let target = adw::ComboRow::builder()
        .title("Send through")
        .model(&widgets::string_list(
            &targets.iter().map(Target::label).collect::<Vec<_>>(),
        ))
        .build();

    group.add(&name);
    group.add(&url);
    group.add(&behavior);
    group.add(&format);
    group.add(&interval);
    group.add(&target);
    content.append(&group);

    let (dialog, confirm) = widgets::form_dialog("Add rule provider", "Add", &content);
    let save_state = state.clone();
    let dialog_for_save = dialog.clone();
    confirm.connect_clicked(move |_| {
        let provider_name = name.text().trim().to_string();
        let provider_url = url.text().trim().to_string();
        if provider_name.is_empty() || provider_url.is_empty() {
            save_state.toast("A provider needs both a name and a URL.");
            return;
        }
        let provider = RuleProvider {
            enabled: true,
            name: provider_name,
            url: provider_url,
            behavior: ["domain", "ipcidr", "classical"][behavior.selected().min(2) as usize]
                .to_string(),
            format: ["yaml", "text", "mrs"][format.selected().min(2) as usize].to_string(),
            interval: interval.value() as u64,
            target: targets
                .get(target.selected() as usize)
                .cloned()
                .unwrap_or(Target::Direct),
        };
        save_state
            .config
            .borrow_mut()
            .routing
            .rule_providers
            .push(provider);
        save_state.commit();
        dialog_for_save.close();
    });

    dialog.present(Some(parent.as_ref()));
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let (scroller, content) = widgets::page_container();

    state.subscribe(move |state| {
        widgets::clear(&content);
        let targets = state.config.borrow().routing.available_targets();

        // ---- ordering explainer ----
        let order = adw::PreferencesGroup::builder()
            .title("Rule order")
            .description(
                "Rules are evaluated top to bottom and the first match wins: \
                 application rules, then the private-network bypass, then destination \
                 rules, then rule providers, and finally the default action.",
            )
            .build();

        let default_target = state.config.borrow().routing.default_target.clone();
        let default_row = adw::ComboRow::builder()
            .title("Everything else")
            .subtitle("The final MATCH rule")
            .model(&widgets::string_list(
                &targets.iter().map(Target::label).collect::<Vec<_>>(),
            ))
            .build();
        if let Some(index) = targets.iter().position(|t| *t == default_target) {
            default_row.set_selected(index as u32);
        }
        let default_state = state.clone();
        let default_targets = targets.clone();
        default_row.connect_selected_notify(move |combo| {
            if default_state.is_refreshing() {
                return;
            }
            let picked = default_targets
                .get(combo.selected() as usize)
                .cloned()
                .unwrap_or(Target::Direct);
            default_state.config.borrow_mut().routing.default_target = picked;
            default_state.save();
        });
        order.add(&default_row);
        content.append(&order);

        // ---- applications ----
        let tun_on = state.config.borrow().core.tun_enabled;
        let apps_group = adw::PreferencesGroup::builder()
            .title("Applications")
            .description(if tun_on {
                "Route individual programs, whatever they connect to."
            } else {
                "⚠ TUN is disabled in Settings — process rules cannot match without it."
            })
            .build();

        let add_app = widgets::action_button("list-add-symbolic", "Add");
        let add_app_state = state.clone();
        let add_app_anchor = content.clone();
        add_app.connect_clicked(move |_| app_editor(&add_app_state, &add_app_anchor));
        apps_group.set_header_suffix(Some(&add_app));

        let app_rules = state.config.borrow().routing.app_rules.clone();
        if app_rules.is_empty() {
            apps_group.add(
                &adw::ActionRow::builder()
                    .title("No application rules")
                    .subtitle("For example: Steam direct, browser through the tunnel.")
                    .build(),
            );
        }
        for (index, rule) in app_rules.iter().enumerate() {
            let title = if rule.label.is_empty() {
                rule.value.clone()
            } else {
                rule.label.clone()
            };
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&title))
                .subtitle(match rule.match_by {
                    AppMatch::Name => format!("PROCESS-NAME {}", rule.value),
                    AppMatch::Path => format!("PROCESS-PATH {}", rule.value),
                })
                .build();

            let dropdown = widgets::target_dropdown(&targets, &rule.target);
            let dropdown_state = state.clone();
            let dropdown_targets = targets.clone();
            dropdown.connect_selected_notify(move |combo| {
                if dropdown_state.is_refreshing() {
                    return;
                }
                let picked = widgets::selected_target(&dropdown_targets, combo);
                if let Some(rule) = dropdown_state
                    .config
                    .borrow_mut()
                    .routing
                    .app_rules
                    .get_mut(index)
                {
                    rule.target = picked;
                }
                dropdown_state.save();
            });
            row.add_suffix(&dropdown);

            let remove = widgets::icon_button("user-trash-symbolic", "Remove");
            let remove_state = state.clone();
            remove.connect_clicked(move |_| {
                remove_state
                    .config
                    .borrow_mut()
                    .routing
                    .app_rules
                    .remove(index);
                remove_state.commit();
            });
            row.add_suffix(&remove);
            apps_group.add(&row);
        }
        content.append(&apps_group);

        // ---- destinations ----
        let domains_group = adw::PreferencesGroup::builder()
            .title("Domains, IP and geo")
            .build();
        let add_domain = widgets::action_button("list-add-symbolic", "Add");
        let add_domain_state = state.clone();
        let add_domain_anchor = content.clone();
        add_domain.connect_clicked(move |_| domain_editor(&add_domain_state, &add_domain_anchor));
        domains_group.set_header_suffix(Some(&add_domain));

        let domain_rules = state.config.borrow().routing.domain_rules.clone();
        if domain_rules.is_empty() {
            domains_group.add(
                &adw::ActionRow::builder()
                    .title("No destination rules")
                    .subtitle("Local traffic already bypasses the tunnel when the private-network bypass is on.")
                    .build(),
            );
        }
        for (index, rule) in domain_rules.iter().enumerate() {
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&rule.value))
                .subtitle(rule.kind.as_rule_kind())
                .build();

            let dropdown = widgets::target_dropdown(&targets, &rule.target);
            let dropdown_state = state.clone();
            let dropdown_targets = targets.clone();
            dropdown.connect_selected_notify(move |combo| {
                if dropdown_state.is_refreshing() {
                    return;
                }
                let picked = widgets::selected_target(&dropdown_targets, combo);
                if let Some(rule) = dropdown_state
                    .config
                    .borrow_mut()
                    .routing
                    .domain_rules
                    .get_mut(index)
                {
                    rule.target = picked;
                }
                dropdown_state.save();
            });
            row.add_suffix(&dropdown);

            let remove = widgets::icon_button("user-trash-symbolic", "Remove");
            let remove_state = state.clone();
            remove.connect_clicked(move |_| {
                remove_state
                    .config
                    .borrow_mut()
                    .routing
                    .domain_rules
                    .remove(index);
                remove_state.commit();
            });
            row.add_suffix(&remove);
            domains_group.add(&row);
        }
        content.append(&domains_group);

        // ---- providers ----
        let providers_group = adw::PreferencesGroup::builder()
            .title("Rule providers")
            .description("Remote lists such as antifilter or a geosite mirror.")
            .build();
        let add_provider = widgets::action_button("list-add-symbolic", "Add");
        let add_provider_state = state.clone();
        let add_provider_anchor = content.clone();
        add_provider
            .connect_clicked(move |_| provider_editor(&add_provider_state, &add_provider_anchor));
        providers_group.set_header_suffix(Some(&add_provider));

        let providers = state.config.borrow().routing.rule_providers.clone();
        for (index, provider) in providers.iter().enumerate() {
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&provider.name))
                .subtitle(format!(
                    "{} · {} · → {}",
                    provider.behavior,
                    provider.url,
                    provider.target.label()
                ))
                .build();
            let remove = widgets::icon_button("user-trash-symbolic", "Remove");
            let remove_state = state.clone();
            remove.connect_clicked(move |_| {
                remove_state
                    .config
                    .borrow_mut()
                    .routing
                    .rule_providers
                    .remove(index);
                remove_state.commit();
            });
            row.add_suffix(&remove);
            providers_group.add(&row);
        }
        content.append(&providers_group);

        // ---- raw ----
        let raw_group = adw::PreferencesGroup::builder()
            .title("Raw rules")
            .description("Written verbatim. The first block goes above everything generated, the second just before the default action.")
            .build();

        let (prepend_scroller, prepend_view) = widgets::text_area(
            &state.config.borrow().routing.raw_prepend.join("\n"),
            true,
        );
        let (append_scroller, append_view) =
            widgets::text_area(&state.config.borrow().routing.raw_append.join("\n"), true);

        let raw_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        raw_box.append(&widgets::dim_label("Before everything"));
        raw_box.append(&prepend_scroller);
        raw_box.append(&widgets::dim_label("Just before the default action"));
        raw_box.append(&append_scroller);

        let save = gtk::Button::with_label("Save raw rules");
        save.add_css_class("suggested-action");
        save.set_halign(gtk::Align::End);
        let save_state = state.clone();
        save.connect_clicked(move |_| {
            let split = |view: &gtk::TextView| {
                widgets::text_of(view)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            };
            {
                let mut cfg = save_state.config.borrow_mut();
                cfg.routing.raw_prepend = split(&prepend_view);
                cfg.routing.raw_append = split(&append_view);
            }
            save_state.save();
            save_state.toast("Raw rules saved — apply the configuration to use them.");
        });
        raw_box.append(&save);
        raw_group.add(&raw_box);
        content.append(&raw_group);
    });

    scroller.upcast()
}
