//! Chronoclip: replay buffer for World of Warcraft on Linux.
//!
//! Without a subcommand this starts the tray app (or shows the window of an already running
//! instance). Subcommands talk to the running instance over a Unix socket.

mod controller;
mod ipc;
mod tray;
mod ui;

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use chronoclip_core::Paths;
use chronoclip_core::ipc::{Request, Response};
use chronoclip_core::settings::Settings;
use clap::{Parser, Subcommand};
use slint::ComponentHandle;

use crate::controller::Controller;

#[derive(Parser)]
#[command(
    name = "chronoclip",
    version,
    about = "Replay buffer for World of Warcraft on Linux"
)]
struct Cli {
    /// Start in the tray without showing the window (for autostart).
    #[arg(long)]
    hidden: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone, Copy)]
enum Command {
    /// Save the last N minutes in the running instance.
    Save,
    /// Show the window of the running instance.
    Show,
    /// Quit the running instance.
    Quit,
}

impl From<Command> for Request {
    fn from(cmd: Command) -> Self {
        match cmd {
            Command::Save => Request::Save,
            Command::Show => Request::Show,
            Command::Quit => Request::Quit,
        }
    }
}

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let result = match cli.command {
        Some(cmd) => run_client(cmd.into()),
        None => run_app(cli.hidden),
    };
    result.unwrap_or_else(|e| {
        eprintln!("chronoclip: {e:#}");
        ExitCode::FAILURE
    })
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Send one request to the running instance and print its response.
fn run_client(request: Request) -> anyhow::Result<ExitCode> {
    let paths = Paths::detect()?;
    match ipc::send(&paths.socket, request) {
        Ok(Response::Ok(msg)) => {
            if !msg.is_empty() {
                println!("{msg}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Response::Error(msg)) => {
            eprintln!("chronoclip: {msg}");
            Ok(ExitCode::FAILURE)
        }
        Err(e) if ipc::is_not_running(&e) => {
            eprintln!("chronoclip: Chronoclip is not running. Start it with `chronoclip`.");
            Ok(ExitCode::FAILURE)
        }
        Err(e) => Err(e).context("could not talk to the running instance"),
    }
}

/// Start the tray app.
fn run_app(hidden: bool) -> anyhow::Result<ExitCode> {
    let paths = Paths::detect()?;

    let listener = match ipc::bind(&paths.socket).context("could not create IPC socket")? {
        ipc::Bound::Listener(listener) => listener,
        ipc::Bound::AlreadyRunning => {
            tracing::info!("Chronoclip is already running, showing its window");
            ipc::send(&paths.socket, Request::Show)?;
            return Ok(ExitCode::SUCCESS);
        }
    };

    let settings = Settings::load(&paths.config_file).unwrap_or_else(|e| {
        tracing::warn!("{e:#}; using default settings");
        Settings::default()
    });

    let window = ui::MainWindow::new().context("could not create the window")?;
    let controller = Arc::new(Controller::new(window.as_weak()));
    ui::init(&window, paths.clone(), &settings, Arc::clone(&controller));

    // Keep the tray alive until the event loop ends; dropping it removes the icon.
    let _tray = tray::create(Arc::clone(&controller));
    ipc::serve(listener, Arc::clone(&controller)).context("could not start IPC thread")?;

    if !hidden {
        window.show()?;
    }
    tracing::info!(config = %paths.config_file.display(), "Chronoclip started");
    slint::run_event_loop_until_quit()?;

    ipc::cleanup(&paths.socket);
    tracing::info!("Chronoclip stopped");
    Ok(ExitCode::SUCCESS)
}
