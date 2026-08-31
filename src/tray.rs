//! StatusNotifierItem, so closing the window leaves the tunnel up.
//!
//! The item runs on the tokio runtime and talks to the GTK thread over channels:
//! menu callbacks arrive from D-Bus on another thread and must not touch widgets
//! themselves. If no status-notifier host is running — niri has none of its own,
//! it comes from a bar — spawning fails and the caller keeps the plain
//! quit-on-close behaviour rather than hiding the window where nobody can reach it.

use ksni::TrayMethods;

use crate::runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ToggleCore,
    Quit,
}

struct ManifoldTray {
    running: bool,
    commands: async_channel::Sender<TrayCommand>,
}

impl ManifoldTray {
    fn send(&self, command: TrayCommand) {
        // Dropping a click is better than blocking the D-Bus thread.
        let _ = self.commands.try_send(command);
    }
}

impl ksni::Tray for ManifoldTray {
    fn id(&self) -> String {
        "mihomo-manifold".into()
    }

    fn title(&self) -> String {
        "MihomoManifold".into()
    }

    fn icon_name(&self) -> String {
        if self.running {
            "network-vpn-symbolic".into()
        } else {
            "network-offline-symbolic".into()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "MihomoManifold".into(),
            description: if self.running {
                "Core is running".into()
            } else {
                "Core is stopped".into()
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open MihomoManifold".into(),
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::Show)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if self.running {
                    "Stop the core".into()
                } else {
                    "Start the core".into()
                },
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::ToggleCore)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The GTK side of the tray.
pub struct Tray {
    /// What the user clicked.
    pub commands: async_channel::Receiver<TrayCommand>,
    /// Core state to display; send on every change.
    pub status: async_channel::Sender<bool>,
    /// Yields once: whether a status-notifier host accepted the item.
    pub started: async_channel::Receiver<bool>,
}

pub fn spawn(running: bool) -> Tray {
    let (command_tx, command_rx) = async_channel::bounded(8);
    let (status_tx, status_rx) = async_channel::bounded(8);
    let (started_tx, started_rx) = async_channel::bounded(1);

    runtime::runtime().spawn(async move {
        let tray = ManifoldTray {
            running,
            commands: command_tx,
        };
        match tray.spawn().await {
            Ok(handle) => {
                let _ = started_tx.send(true).await;
                // Own the handle here: it is not Clone, and this is the only
                // place that needs it.
                while let Ok(running) = status_rx.recv().await {
                    if handle.update(|tray| tray.running = running).await.is_none() {
                        break;
                    }
                }
            }
            Err(err) => {
                eprintln!("mihomo-manifold: no tray ({err}); the window will quit on close");
                let _ = started_tx.send(false).await;
            }
        }
    });

    Tray {
        commands: command_rx,
        status: status_tx,
        started: started_rx,
    }
}
