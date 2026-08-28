//! Live core log, streamed from the controller's `/logs` endpoint.

use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use crate::api::LogLine;
use crate::runtime;
use crate::state::AppState;
use crate::ui::widgets;

const CAPACITY: usize = 2000;
const LEVELS: [&str; 5] = ["error", "warning", "info", "debug", "silent"];

struct LogState {
    lines: RefCell<VecDeque<LogLine>>,
    filter: RefCell<String>,
    streaming: Cell<bool>,
    paused: Cell<bool>,
}

impl LogState {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            lines: RefCell::new(VecDeque::with_capacity(CAPACITY)),
            filter: RefCell::new(String::new()),
            streaming: Cell::new(false),
            paused: Cell::new(false),
        })
    }

    fn push(&self, line: LogLine) {
        let mut lines = self.lines.borrow_mut();
        if lines.len() == CAPACITY {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn matches(&self, line: &LogLine) -> bool {
        let filter = self.filter.borrow();
        filter.is_empty()
            || line.payload.to_lowercase().contains(filter.as_str())
            || line.level.to_lowercase().contains(filter.as_str())
    }

    fn render(&self) -> String {
        self.lines
            .borrow()
            .iter()
            .filter(|line| self.matches(line))
            .map(|line| format!("[{}] {}", line.level, line.payload))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn scroll_to_end(view: &gtk::TextView) {
    let buffer = view.buffer();
    let mut end = buffer.end_iter();
    view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
}

pub fn page(state: &Rc<AppState>) -> gtk::Widget {
    let logs = LogState::new();

    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let level = gtk::DropDown::builder()
        .model(&widgets::string_list(&LEVELS))
        .selected(2)
        .tooltip_text("Level requested from the core")
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter")
        .hexpand(true)
        .build();

    let pause = gtk::ToggleButton::builder()
        .icon_name("media-playback-pause-symbolic")
        .tooltip_text("Pause")
        .build();

    let clear = widgets::icon_button("edit-clear-all-symbolic", "Clear");

    toolbar.append(&level);
    toolbar.append(&search);
    toolbar.append(&pause);
    toolbar.append(&clear);
    container.append(&toolbar);

    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&view)
        .build();
    scroller.add_css_class("card");
    container.append(&scroller);

    let hint = widgets::dim_label(
        "The core writes its own log to $XDG_STATE_HOME/mihomo-manifold/core.log as well.",
    );
    container.append(&hint);

    // ---- filtering ----
    let filter_logs = logs.clone();
    let filter_view = view.clone();
    search.connect_search_changed(move |entry| {
        *filter_logs.filter.borrow_mut() = entry.text().to_lowercase();
        filter_view.buffer().set_text(&filter_logs.render());
        scroll_to_end(&filter_view);
    });

    let clear_logs = logs.clone();
    let clear_view = view.clone();
    clear.connect_clicked(move |_| {
        clear_logs.lines.borrow_mut().clear();
        clear_view.buffer().set_text("");
    });

    let pause_logs = logs.clone();
    pause.connect_toggled(move |button| pause_logs.paused.set(button.is_active()));

    // Changing the level means asking the core for a different stream.
    let level_state = state.clone();
    let level_logs = logs.clone();
    level.connect_selected_notify(move |_| {
        level_logs.streaming.set(false);
        level_state.notify();
    });

    // ---- streaming ----
    let stream_logs = logs.clone();
    let stream_view = view.clone();
    let stream_level = level.clone();
    state.subscribe(move |state| {
        if !state.is_running() || stream_logs.streaming.get() {
            return;
        }
        let Some(api) = state.api() else { return };
        stream_logs.streaming.set(true);

        let level_name = LEVELS[stream_level.selected().min(4) as usize].to_string();
        let (tx, rx) = async_channel::bounded::<LogLine>(256);
        runtime::runtime().spawn(async move { api.logs_stream(&level_name, tx).await });

        let logs = stream_logs.clone();
        let view = stream_view.clone();
        glib::spawn_future_local(async move {
            while let Ok(line) = rx.recv().await {
                if logs.paused.get() {
                    continue;
                }
                let visible = logs.matches(&line);
                logs.push(line);
                if visible {
                    let buffer = view.buffer();
                    let mut end = buffer.end_iter();
                    let text = {
                        let lines = logs.lines.borrow();
                        let last = lines.back().unwrap();
                        format!("[{}] {}\n", last.level, last.payload)
                    };
                    buffer.insert(&mut end, &text);
                    scroll_to_end(&view);
                }
            }
            logs.streaming.set(false);
        });
    });

    container.upcast()
}
