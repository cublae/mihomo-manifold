//! Shared application state and the actions the pages trigger. GTK is single
//! threaded, so this is plain `Rc`/`RefCell`; anything blocking is handed to the
//! tokio runtime in `runtime.rs`.

use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::api::ClashApi;
use crate::config::AppConfig;
use crate::corectl::{self as core, CoreStatus};
use crate::{paths, runtime, subscription, sysproxy, template};

type Listener = Rc<dyn Fn(&Rc<AppState>)>;

pub struct AppState {
    pub config: RefCell<AppConfig>,
    pub status: RefCell<CoreStatus>,
    pub core_version: RefCell<Option<String>>,
    listeners: RefCell<Vec<Listener>>,
    toaster: RefCell<Option<adw::ToastOverlay>>,
    /// Set while a page rebuilds itself, so widget signals do not write back.
    refreshing: Cell<bool>,
}

impl AppState {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            config: RefCell::new(AppConfig::load()),
            status: RefCell::new(CoreStatus::Stopped),
            core_version: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            toaster: RefCell::new(None),
            refreshing: Cell::new(false),
        })
    }

    pub fn attach_toaster(&self, overlay: &adw::ToastOverlay) {
        *self.toaster.borrow_mut() = Some(overlay.clone());
    }

    pub fn toast(&self, message: &str) {
        if let Some(overlay) = self.toaster.borrow().as_ref() {
            overlay.add_toast(adw::Toast::builder().title(message).timeout(4).build());
        } else {
            eprintln!("mihomo-manifold: {message}");
        }
    }

    /// Pages call this to rebuild themselves whenever the config or core changes.
    pub fn subscribe(self: &Rc<Self>, listener: impl Fn(&Rc<AppState>) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    pub fn notify(self: &Rc<Self>) {
        let listeners = self.listeners.borrow().clone();
        let was_refreshing = self.refreshing.replace(true);
        for listener in listeners {
            listener(self);
        }
        self.refreshing.set(was_refreshing);
    }

    /// True while widgets are being repopulated programmatically.
    pub fn is_refreshing(&self) -> bool {
        self.refreshing.get()
    }

    pub fn save(self: &Rc<Self>) {
        if let Err(err) = self.config.borrow().save() {
            self.toast(&format!("Could not save settings: {err}"));
        }
    }

    /// Persist and tell every page to redraw.
    pub fn commit(self: &Rc<Self>) {
        self.save();
        self.notify();
    }

    pub fn api(&self) -> Option<ClashApi> {
        let cfg = self.config.borrow();
        ClashApi::new(&cfg.core.controller_url(), &cfg.core.secret).ok()
    }

    pub fn is_running(&self) -> bool {
        matches!(
            *self.status.borrow(),
            CoreStatus::Running | CoreStatus::Adopted
        )
    }
}

// ---------------------------------------------------------------- actions

/// Nodes for the active subscription, read from the cached profile.
pub fn active_proxies(state: &Rc<AppState>) -> Result<Vec<serde_yaml::Value>, String> {
    let cfg = state.config.borrow();
    let Some(sub) = cfg.active() else {
        return Err("Add a subscription first.".to_string());
    };
    match subscription::load_cached(sub) {
        Some(proxies) if !proxies.is_empty() => Ok(proxies),
        _ => Err(format!(
            "No downloaded profile for \"{}\" yet — update it first.",
            sub.name
        )),
    }
}

pub fn render_config(state: &Rc<AppState>) -> Result<String, String> {
    let proxies = active_proxies(state)?;
    let cfg = state.config.borrow();
    template::generate(&cfg, &proxies).map_err(|e| e.to_string())
}

/// Start the core, or hot-reload it if it is already up.
pub fn apply(state: &Rc<AppState>) {
    let yaml = match render_config(state) {
        Ok(yaml) => yaml,
        Err(err) => {
            state.toast(&err);
            return;
        }
    };

    if state.is_running() {
        if let Err(err) = paths::write_private(&paths::generated_config(), &yaml) {
            state.toast(&format!("Could not write the config: {err}"));
            return;
        }
        let Some(api) = state.api() else { return };
        let path = paths::generated_config().to_string_lossy().into_owned();
        let state = state.clone();
        runtime::spawn(
            async move { api.reload(&path).await.map_err(|e| e.to_string()) },
            move |result| {
                match result {
                    Ok(()) => {
                        state.toast("Configuration reloaded");
                        // The pages read proxies from the core, so they have to
                        // query it again. refresh_status alone would not do it:
                        // it only redraws when the status itself changed, and a
                        // reload leaves the core exactly as running as it was.
                        state.notify();
                    }
                    Err(err) => state.toast(&format!("Reload failed: {err}")),
                }
                // The mode may have just changed from tunnel to proxy-only.
                sync_system_proxy(&state);
                refresh_status(&state);
            },
        );
        return;
    }

    let cfg_snapshot = state.config.borrow().clone();
    if cfg_snapshot.core.tun_enabled {
        if let Some(warning) = core::tun_readiness(&cfg_snapshot.core.resolve_binary()).warning() {
            state.toast(&warning);
        }
    }

    if let Err(err) = core::start(&cfg_snapshot, &yaml) {
        *state.status.borrow_mut() = CoreStatus::Failed(err.to_string());
        state.toast(&format!("{err}"));
        state.notify();
        return;
    }

    // Give the core a moment to bind the controller, then confirm it is alive.
    let state = state.clone();
    let api = state.api();
    runtime::spawn(
        async move {
            let Some(api) = api else {
                return Err("controller unreachable".to_string());
            };
            for _ in 0..40 {
                if let Ok(version) = api.version().await {
                    return Ok(version);
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            Err("the core did not answer on the controller port".to_string())
        },
        move |result| match result {
            Ok(version) => {
                *state.status.borrow_mut() = CoreStatus::Running;
                *state.core_version.borrow_mut() = Some(version);
                sync_system_proxy(&state);
                state.toast("Core started");
                state.notify();
            }
            Err(err) => {
                let tail = core::tail_log(12);
                core::stop();
                *state.status.borrow_mut() = CoreStatus::Failed(err.clone());
                state.toast(&format!("{err}. Check the Logs page."));
                if !tail.is_empty() {
                    eprintln!("mihomo-manifold: core log tail:\n{tail}");
                }
                state.notify();
            }
        },
    );
}

/// Point the desktop at the core in proxy-only mode, and take it back when the
/// core is not the one serving it any more. Only ever touches settings that
/// point at us, so a proxy the user configured themselves is left alone.
pub fn sync_system_proxy(state: &Rc<AppState>) {
    let (wanted, host, port) = {
        let cfg = state.config.borrow();
        (
            cfg.core.set_system_proxy && !cfg.core.tun_enabled && state.is_running(),
            "127.0.0.1".to_string(),
            cfg.core.mixed_port,
        )
    };

    if wanted {
        if !sysproxy::set(&host, port) {
            state.toast("This desktop has no proxy settings to write (gsettings schema missing).");
        }
    } else if sysproxy::points_at(&host, port) {
        sysproxy::clear();
    }
}

pub fn stop(state: &Rc<AppState>) {
    core::stop();
    *state.status.borrow_mut() = CoreStatus::Stopped;
    // Do this before notifying: leaving the desktop pointed at a port that no
    // longer answers takes the whole session offline.
    sync_system_proxy(state);
    *state.core_version.borrow_mut() = None;
    state.notify();
}

/// Probe the controller; also picks up a core started outside the GUI.
pub fn refresh_status(state: &Rc<AppState>) {
    let Some(api) = state.api() else { return };
    let child_alive = core::is_child_alive();
    let state = state.clone();
    runtime::spawn(async move { api.version().await.ok() }, move |version| {
        let previous = state.status.borrow().clone();
        let next = match (version.is_some(), child_alive) {
            (true, true) => CoreStatus::Running,
            (true, false) => CoreStatus::Adopted,
            (false, _) => match previous {
                CoreStatus::Failed(ref err) => CoreStatus::Failed(err.clone()),
                _ => CoreStatus::Stopped,
            },
        };
        *state.core_version.borrow_mut() = version;
        if next != previous {
            *state.status.borrow_mut() = next;
            state.notify();
        } else {
            *state.status.borrow_mut() = next;
        }
    });
}

/// Download one subscription and remember what the panel reported.
pub fn update_subscription(state: &Rc<AppState>, id: &str, then_apply: bool) {
    let (sub, hwid) = {
        let cfg = state.config.borrow();
        let Some(sub) = cfg.subscription(id) else {
            return;
        };
        (sub.clone(), cfg.hwid.clone())
    };

    state.toast(&format!("Updating \"{}\"…", sub.name));
    let state = state.clone();
    let id = id.to_string();
    runtime::spawn(
        async move { subscription::fetch(sub, hwid).await },
        move |result| {
            {
                let mut cfg = state.config.borrow_mut();
                let Some(entry) = cfg.subscription_mut(&id) else {
                    return;
                };
                match &result {
                    Ok(fetched) => {
                        entry.last_error = None;
                        entry.last_updated = Some(chrono::Utc::now().timestamp());
                        entry.node_count = fetched.proxies.len();
                        entry.user_info = fetched.user_info;
                        if let Some(title) = &fetched.title {
                            if entry.name.trim().is_empty() || entry.name == "New subscription" {
                                entry.name = title.clone();
                            }
                        }
                    }
                    Err(err) => entry.last_error = Some(err.to_string()),
                }
                if cfg.active_subscription.is_none() {
                    cfg.active_subscription = Some(id.clone());
                }
            }
            match result {
                Ok(fetched) => {
                    state.toast(&format!("{} nodes downloaded", fetched.proxies.len()));
                    state.commit();
                    // Downloading only refreshes the file on disk. The core is
                    // still serving the nodes it was started with, so without a
                    // reload the new ones never reach the Nodes page.
                    let feeds_the_core = {
                        let cfg = state.config.borrow();
                        cfg.active().is_some_and(|active| active.id == id)
                    };
                    if then_apply || (feeds_the_core && state.is_running()) {
                        apply(&state);
                    }
                }
                Err(subscription::FetchError::DeviceLimit(message)) => {
                    state.commit();
                    show_device_limit(&state, &message);
                }
                Err(err) => {
                    state.toast(&format!("Update failed: {err}"));
                    state.commit();
                }
            }
        },
    );
}

/// The panel refused the device: show which HWID was sent and how to change it.
fn show_device_limit(state: &Rc<AppState>, message: &str) {
    let hwid = state.config.borrow().hwid.value();
    let dialog = adw::AlertDialog::builder()
        .heading("Device slot rejected")
        .body(format!(
            "{message}\n\nThis machine identifies itself as:\n{hwid}\n\nFree a slot in the panel, or set a different HWID in Settings."
        ))
        .build();
    dialog.add_response("close", "Close");
    dialog.add_response("settings", "Open Settings");
    dialog.set_response_appearance("settings", adw::ResponseAppearance::Suggested);

    let state_for_response = state.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "settings" {
            state_for_response.toast("Settings → Device identity");
        }
    });

    if let Some(overlay) = state.toaster.borrow().as_ref() {
        if let Some(root) = overlay.root().and_downcast::<gtk::Window>() {
            dialog.present(Some(&root));
            return;
        }
    }
    state.toast(message);
}
