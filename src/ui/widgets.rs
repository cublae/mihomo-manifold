//! Small builders shared by the pages.

use adw::prelude::*;

use crate::config::Target;

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button
}

/// Primary actions carry a label as well as an icon. Some third-party icon
/// themes ship symbolic SVGs that GTK4 refuses to draw (anything wrapped in a
/// `<g>` element is dropped), which would leave a bare icon button invisible.
pub fn action_button(icon: &str, label: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(label)
        .build();
    let button = gtk::Button::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button
}

pub fn string_list<S: AsRef<str>>(items: &[S]) -> gtk::StringList {
    let list = gtk::StringList::new(&[]);
    for item in items {
        list.append(item.as_ref());
    }
    list
}

/// A vertical box that pages fill with `adw::PreferencesGroup`s, wrapped in a
/// clamp so the content stays readable on wide monitors.
pub fn page_container() -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .tightening_threshold(600)
        .child(&content)
        .build();

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();

    (scroller, content)
}

pub fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub fn dim_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .build();
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label
}

/// A dropdown over the routing targets (DIRECT, REJECT, generated groups).
pub fn target_dropdown(targets: &[Target], current: &Target) -> gtk::DropDown {
    let labels: Vec<String> = targets.iter().map(Target::label).collect();
    let dropdown = gtk::DropDown::builder()
        .model(&string_list(&labels))
        .valign(gtk::Align::Center)
        .build();
    if let Some(index) = targets.iter().position(|t| t == current) {
        dropdown.set_selected(index as u32);
    }
    dropdown
}

pub fn selected_target(targets: &[Target], dropdown: &gtk::DropDown) -> Target {
    targets
        .get(dropdown.selected() as usize)
        .cloned()
        .unwrap_or(Target::Direct)
}

/// A dialog with a header bar, a cancel and a confirm button.
pub fn form_dialog(
    title: &str,
    confirm_label: &str,
    content: &impl IsA<gtk::Widget>,
) -> (adw::Dialog, gtk::Button) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(520)
        .content_height(560)
        .build();

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);

    let cancel = gtk::Button::with_label("Cancel");
    let confirm = gtk::Button::with_label(confirm_label);
    confirm.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&confirm);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(content)
        .build();
    toolbar.set_content(Some(&scroller));
    dialog.set_child(Some(&toolbar));

    let closer = dialog.clone();
    cancel.connect_clicked(move |_| {
        closer.close();
    });

    (dialog, confirm)
}

pub fn copy_to_clipboard(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.as_ref().clipboard().set_text(text);
}

/// `1789000000` → `2026-09-05`; 0 means the panel reported no expiry.
pub fn format_expiry(unix_seconds: i64) -> String {
    if unix_seconds <= 0 {
        return "no expiry".to_string();
    }
    match chrono::DateTime::from_timestamp(unix_seconds, 0) {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "unknown".to_string(),
    }
}

pub fn format_timestamp(unix_seconds: i64) -> String {
    match chrono::DateTime::from_timestamp(unix_seconds, 0) {
        Some(dt) => {
            let local: chrono::DateTime<chrono::Local> = dt.into();
            local.format("%Y-%m-%d %H:%M").to_string()
        }
        None => "never".to_string(),
    }
}

/// Multi-line text entry used for raw rules and custom headers.
pub fn text_area(initial: &str, monospace: bool) -> (gtk::ScrolledWindow, gtk::TextView) {
    let view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::None)
        .monospace(monospace)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    view.buffer().set_text(initial);

    let scroller = gtk::ScrolledWindow::builder()
        .height_request(140)
        .child(&view)
        .build();
    scroller.add_css_class("card");
    (scroller, view)
}

pub fn text_of(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}
