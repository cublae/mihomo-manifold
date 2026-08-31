//! Desktop proxy settings, for the proxy-only mode where nothing captures
//! traffic on its own and applications have to be pointed at the port.
//!
//! Everything here is best-effort: the schema belongs to gsettings-desktop-schemas
//! and a session may not have it, in which case the whole feature quietly does
//! nothing rather than taking the app down with it — looking a schema up that is
//! not installed aborts the process.

use gtk::gio;
use gtk::prelude::*;

const SCHEMA: &str = "org.gnome.system.proxy";
const HTTP: &str = "org.gnome.system.proxy.http";
const HTTPS: &str = "org.gnome.system.proxy.https";
const SOCKS: &str = "org.gnome.system.proxy.socks";

/// `None` when the schema is not installed in this session.
fn settings(schema: &str) -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup(schema, true)?;
    Some(gio::Settings::new(schema))
}

pub fn is_available() -> bool {
    settings(SCHEMA).is_some()
}

/// Point the desktop at the core's mixed port. Returns false when the desktop
/// has no proxy settings to write.
pub fn set(host: &str, port: u16) -> bool {
    let Some(root) = settings(SCHEMA) else {
        return false;
    };

    for schema in [HTTP, HTTPS, SOCKS] {
        let Some(child) = settings(schema) else {
            continue;
        };
        let _ = child.set_string("host", host);
        let _ = child.set_int("port", port as i32);
    }
    // The mixed port speaks HTTP and SOCKS, so one address covers all three.
    let _ = root.set_string("mode", "manual");
    gio::Settings::sync();
    true
}

/// Hand the desktop back to its default resolution.
pub fn clear() -> bool {
    let Some(root) = settings(SCHEMA) else {
        return false;
    };
    let _ = root.set_string("mode", "none");
    gio::Settings::sync();
    true
}

/// Whether the desktop currently points at this host and port.
pub fn points_at(host: &str, port: u16) -> bool {
    let Some(root) = settings(SCHEMA) else {
        return false;
    };
    if root.string("mode") != "manual" {
        return false;
    }
    settings(HTTP)
        .map(|http| http.string("host") == host && http.int("port") == port as i32)
        .unwrap_or(false)
}
