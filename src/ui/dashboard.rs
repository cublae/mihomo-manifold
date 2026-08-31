//! Dashboard: core power, what is currently loaded, and a live traffic graph
//! fed by the controller's `/traffic` stream.

use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use crate::api::format_bytes;
use crate::corectl::CoreStatus;
use crate::runtime;
use crate::state::{self, AppState};
use crate::ui::widgets;

const HISTORY: usize = 120;

struct Traffic {
    samples: RefCell<VecDeque<(u64, u64)>>,
    total_up: Cell<u64>,
    total_down: Cell<u64>,
    streaming: Cell<bool>,
}

impl Traffic {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            samples: RefCell::new(VecDeque::with_capacity(HISTORY)),
            total_up: Cell::new(0),
            total_down: Cell::new(0),
            streaming: Cell::new(false),
        })
    }

    fn push(&self, up: u64, down: u64) {
        let mut samples = self.samples.borrow_mut();
        if samples.len() == HISTORY {
            samples.pop_front();
        }
        samples.push_back((up, down));
        self.total_up.set(self.total_up.get() + up);
        self.total_down.set(self.total_down.get() + down);
    }

    fn reset(&self) {
        self.samples.borrow_mut().clear();
        self.total_up.set(0);
        self.total_down.set(0);
    }
}

fn draw_graph(traffic: &Rc<Traffic>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .height_request(150)
        .hexpand(true)
        .build();
    area.add_css_class("card");

    let data = traffic.clone();
    area.set_draw_func(move |_, cr, width, height| {
        let samples = data.samples.borrow();
        let width = width as f64;
        let height = height as f64;
        let padding = 6.0;

        let peak = samples
            .iter()
            .map(|(up, down)| (*up).max(*down))
            .max()
            .unwrap_or(0)
            .max(1024) as f64;

        let plot =
            |cr: &gtk::cairo::Context, pick: fn(&(u64, u64)) -> u64, rgb: (f64, f64, f64)| {
                if samples.len() < 2 {
                    return;
                }
                let step = (width - padding * 2.0) / (HISTORY - 1) as f64;
                let offset = (HISTORY - samples.len()) as f64 * step;

                cr.move_to(padding + offset, height - padding);
                for (index, sample) in samples.iter().enumerate() {
                    let x = padding + offset + index as f64 * step;
                    let ratio = (pick(sample) as f64 / peak).min(1.0);
                    let y = height - padding - ratio * (height - padding * 2.0);
                    cr.line_to(x, y);
                }
                let last_x = padding + offset + (samples.len() - 1) as f64 * step;
                cr.line_to(last_x, height - padding);
                cr.close_path();

                cr.set_source_rgba(rgb.0, rgb.1, rgb.2, 0.20);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(rgb.0, rgb.1, rgb.2, 0.95);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            };

        plot(cr, |s| s.1, (0.20, 0.52, 0.89)); // download
        plot(cr, |s| s.0, (0.90, 0.49, 0.13)); // upload

        cr.set_source_rgba(0.5, 0.5, 0.5, 0.6);
        cr.select_font_face(
            "sans",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        cr.set_font_size(11.0);
        cr.move_to(padding + 2.0, padding + 12.0);
        let _ = cr.show_text(&format!("peak {}/s", format_bytes(peak as u64)));
    });

    area
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let (scroller, content) = widgets::page_container();
    let traffic = Traffic::new();

    // ---- core status ----
    let status_group = adw::PreferencesGroup::builder().title("Core").build();

    let power = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let status_row = adw::ActionRow::builder()
        .title("Stopped")
        .subtitle("The proxy core is not running")
        .build();
    status_row.add_suffix(&power);
    status_group.add(&status_row);

    let profile_row = adw::ActionRow::builder()
        .title("Active subscription")
        .subtitle("none")
        .build();
    status_group.add(&profile_row);

    // Tunnel or plain proxy. It lives here rather than in Settings because it is
    // the one thing people switch depending on where they are.
    let mode_picker = adw::ComboRow::builder()
        .title("Mode")
        .model(&widgets::string_list(&[
            "Tunnel (TUN) — captures everything",
            "Proxy only — one port, no privileges",
        ]))
        .build();
    status_group.add(&mode_picker);

    let proxy_row = adw::ActionRow::builder().title("Proxy address").build();
    proxy_row.add_css_class("property");
    let copy_proxy = widgets::icon_button("edit-copy-symbolic", "Copy");
    proxy_row.add_suffix(&copy_proxy);
    status_group.add(&proxy_row);

    let mode_row = adw::ActionRow::builder()
        .title("Routing")
        .subtitle("—")
        .build();
    status_group.add(&mode_row);
    content.append(&status_group);

    // ---- traffic ----
    let traffic_group = adw::PreferencesGroup::builder().title("Traffic").build();
    let graph = draw_graph(&traffic);

    let rates = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(24)
        .homogeneous(true)
        .margin_top(10)
        .build();

    let make_stat = |caption: &str| {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        let value = gtk::Label::builder().label("0 B/s").xalign(0.0).build();
        value.add_css_class("title-4");
        let label = widgets::dim_label(caption);
        column.append(&value);
        column.append(&label);
        (column, value)
    };

    let (down_box, down_label) = make_stat("Download");
    let (up_box, up_label) = make_stat("Upload");
    let (session_box, session_label) = make_stat("This session");
    rates.append(&down_box);
    rates.append(&up_box);
    rates.append(&session_box);

    let traffic_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    traffic_box.append(&graph);
    traffic_box.append(&rates);
    traffic_group.add(&traffic_box);
    content.append(&traffic_group);

    // ---- quick actions ----
    let actions_group = adw::PreferencesGroup::builder()
        .title("Quick actions")
        .build();

    let apply_row = adw::ActionRow::builder()
        .title("Apply configuration")
        .subtitle("Regenerate config.yaml from your rules and reload the core")
        .activatable(true)
        .build();
    apply_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let apply_state = state.clone();
    apply_row.connect_activated(move |_| state::apply(&apply_state));
    actions_group.add(&apply_row);

    let update_row = adw::ActionRow::builder()
        .title("Update active subscription")
        .subtitle("Download nodes again and reload")
        .activatable(true)
        .build();
    update_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let update_state = state.clone();
    update_row.connect_activated(move |_| {
        let id = update_state
            .config
            .borrow()
            .active()
            .map(|sub| sub.id.clone());
        match id {
            Some(id) => state::update_subscription(&update_state, &id, true),
            None => update_state.toast("Add a subscription first."),
        }
    });
    actions_group.add(&update_row);
    content.append(&actions_group);

    // ---- wiring ----
    let mode_state = state.clone();
    mode_picker.connect_selected_notify(move |combo| {
        if mode_state.is_refreshing() {
            return;
        }
        let tun = combo.selected() == 0;
        mode_state.config.borrow_mut().core.tun_enabled = tun;
        mode_state.save();
        // Switching mode rewrites the config, so a running core has to take it.
        if mode_state.is_running() {
            state::apply(&mode_state);
        } else {
            mode_state.notify();
        }
    });

    let copy_state = state.clone();
    copy_proxy.connect_clicked(move |button| {
        let address = format!("127.0.0.1:{}", copy_state.config.borrow().core.mixed_port);
        widgets::copy_to_clipboard(button, &address);
        copy_state.toast("Proxy address copied");
    });

    let switch_state = state.clone();
    power.connect_active_notify(move |switch| {
        if switch_state.is_refreshing() {
            return;
        }
        if switch.is_active() {
            state::apply(&switch_state);
        } else {
            state::stop(&switch_state);
        }
    });

    let traffic_for_refresh = traffic.clone();
    state.subscribe(move |state| {
        let running = state.is_running();
        power.set_active(running);

        let status = state.status.borrow().clone();
        let version = state.core_version.borrow().clone();
        match &status {
            CoreStatus::Running => {
                status_row.set_title("Running");
                status_row.set_subtitle(&match &version {
                    Some(v) => format!("mihomo {v}, started by MihomoManifold"),
                    None => "started by MihomoManifold".to_string(),
                });
            }
            CoreStatus::Adopted => {
                status_row.set_title("Running (external)");
                status_row.set_subtitle(
                    "A core was already listening on the controller port; it was adopted.",
                );
            }
            CoreStatus::Stopped => {
                status_row.set_title("Stopped");
                status_row.set_subtitle("The proxy core is not running");
            }
            CoreStatus::Failed(err) => {
                status_row.set_title("Failed to start");
                status_row.set_subtitle(err);
            }
        }

        {
            let cfg = state.config.borrow();
            profile_row.set_subtitle(&match cfg.active() {
                Some(sub) => format!("{} — {} nodes", sub.name, sub.node_count),
                None => "none".to_string(),
            });

            mode_picker.set_selected(if cfg.core.tun_enabled { 0 } else { 1 });
            proxy_row.set_visible(!cfg.core.tun_enabled);
            proxy_row.set_subtitle(&format!(
                "127.0.0.1:{} (HTTP and SOCKS){}",
                cfg.core.mixed_port,
                if cfg.core.set_system_proxy {
                    ", published to the desktop"
                } else {
                    ""
                }
            ));
            mode_row.set_subtitle(&format!(
                "{} · {} rule{} · {} app rule{}",
                // The mode itself is the picker above; this line is about what
                // the tunnel carries.
                if cfg.core.tun_enabled {
                    format!("stack {}", cfg.core.tun_stack)
                } else {
                    format!("port {}", cfg.core.mixed_port)
                },
                cfg.routing.domain_rules.len(),
                if cfg.routing.domain_rules.len() == 1 {
                    ""
                } else {
                    "s"
                },
                cfg.routing.app_rules.len(),
                if cfg.routing.app_rules.len() == 1 {
                    ""
                } else {
                    "s"
                },
            ));
        }

        if !running {
            traffic_for_refresh.reset();
            graph.queue_draw();
            down_label.set_label("0 B/s");
            up_label.set_label("0 B/s");
            session_label.set_label("0 B");
            return;
        }

        if traffic_for_refresh.streaming.get() {
            return;
        }
        let Some(api) = state.api() else { return };
        traffic_for_refresh.streaming.set(true);

        let (tx, rx) = async_channel::bounded::<crate::api::Traffic>(64);
        runtime::runtime().spawn(async move { api.traffic_stream(tx).await });

        let traffic_for_stream = traffic_for_refresh.clone();
        let graph = graph.clone();
        let down_label = down_label.clone();
        let up_label = up_label.clone();
        let session_label = session_label.clone();
        glib::spawn_future_local(async move {
            while let Ok(sample) = rx.recv().await {
                traffic_for_stream.push(sample.up, sample.down);
                down_label.set_label(&format!("{}/s", format_bytes(sample.down)));
                up_label.set_label(&format!("{}/s", format_bytes(sample.up)));
                session_label.set_label(&format_bytes(
                    traffic_for_stream.total_up.get() + traffic_for_stream.total_down.get(),
                ));
                graph.queue_draw();
            }
            traffic_for_stream.streaming.set(false);
        });
    });

    scroller.upcast()
}
