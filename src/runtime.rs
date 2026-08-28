//! Bridge between the GTK main loop and tokio. Network and process work runs on
//! a background runtime; results come back over a channel that is drained inside
//! the GLib main context, so widgets are only ever touched from the UI thread.

use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("manifold-io")
            .build()
            .expect("building the tokio runtime")
    })
}

/// Run `task` off the UI thread and hand its result to `on_done` back on it.
pub fn spawn<F, T, C>(task: F, on_done: C)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    runtime().spawn(async move {
        let value = task.await;
        let _ = tx.send(value).await;
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            on_done(value);
        }
    });
}
