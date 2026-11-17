//! Handles commands from the UI, the tray and the IPC socket.
//!
//! Lives on any thread; touches the UI only through `upgrade_in_event_loop`.

use chronoclip_core::ipc::{Request, Response};

use crate::ui::{self, MainWindow};

pub struct Controller {
    window: slint::Weak<MainWindow>,
}

impl Controller {
    pub fn new(window: slint::Weak<MainWindow>) -> Self {
        Self { window }
    }

    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Ok("pong".into()),
            Request::Show => {
                self.show_window();
                Response::Ok(String::new())
            }
            Request::Save => self.save_replay(),
            Request::Quit => {
                self.quit();
                Response::Ok("quitting".into())
            }
        }
    }

    pub fn show_window(&self) {
        let result = self.window.upgrade_in_event_loop(|window| {
            if let Err(e) = slint::ComponentHandle::show(&window) {
                tracing::warn!("could not show window: {e}");
            }
        });
        if let Err(e) = result {
            tracing::warn!("event loop is gone: {e}");
        }
    }

    /// Save the replay buffer. Recording arrives in milestone 2, so this reports that for now.
    pub fn save_replay(&self) -> Response {
        let msg = "Nothing to save yet: recording is not implemented (milestone 2).";
        tracing::info!("save requested: {msg}");
        let _ = self
            .window
            .upgrade_in_event_loop(move |window| ui::set_message(&window, msg, true));
        Response::Error(msg.into())
    }

    pub fn quit(&self) {
        if let Err(e) = slint::quit_event_loop() {
            tracing::warn!("could not quit event loop: {e}");
        }
    }
}
