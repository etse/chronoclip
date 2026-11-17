//! System tray icon, using Slint's built-in `SystemTrayIcon` (StatusNotifierItem on Linux).

use std::sync::Arc;

use crate::controller::Controller;
use crate::ui::Tray;

/// Create the tray icon. Must be called on the main thread. If no tray host is available the
/// error is logged; the app stays usable through `chronoclip show`.
pub fn create(controller: Arc<Controller>) -> Option<Tray> {
    let tray = match Tray::new() {
        Ok(tray) => tray,
        Err(e) => {
            tracing::warn!("tray icon unavailable: {e}");
            return None;
        }
    };
    tray.on_open({
        let controller = Arc::clone(&controller);
        move || controller.show_window()
    });
    tray.on_save_replay({
        let controller = Arc::clone(&controller);
        move || {
            controller.save_replay();
        }
    });
    tray.on_quit(move || controller.quit());
    Some(tray)
}
