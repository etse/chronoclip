//! Glue between the Slint UI and Rust: fills the form from settings and reads it back.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use chronoclip_capture::{OpenWindow, list_windows};
use chronoclip_core::disk::{
    estimate_buffer_bytes, format_gb, nominal_bitrate_mbps, tmpfs_warning,
};
use chronoclip_core::games::{catalog, find_game, matches_query};
use chronoclip_core::installed::InstalledGame;
use chronoclip_core::launchers::Launchers;
use chronoclip_core::paths::{config_home, data_home};
use chronoclip_core::settings::{
    AudioMode, DEFAULT_FPS, EncoderPreference, SUPPORTED_FPS, Settings,
};
use chronoclip_core::{Game, Paths, autostart};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::controller::Controller;

/// Code generated from `ui/*.slint`. It uses `unwrap()` internally, which our lints forbid.
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod generated {
    slint::include_modules!();
}
pub use generated::{GameRow, MainWindow, RecState, ResizeEdge, Tray, TrayState, WindowRow};

fn encoder_label(encoder: EncoderPreference) -> &'static str {
    match encoder {
        EncoderPreference::Auto => "Auto",
        EncoderPreference::Nvenc => "NVENC (NVIDIA)",
        EncoderPreference::Vaapi => "VAAPI (AMD / Intel)",
        EncoderPreference::Software => "Software (slow)",
    }
}

fn encoder_from_label(label: &str) -> Option<EncoderPreference> {
    EncoderPreference::ALL
        .into_iter()
        .find(|e| encoder_label(*e) == label)
}

fn audio_label(audio: AudioMode) -> &'static str {
    match audio {
        AudioMode::Game => "Game only",
        AudioMode::All => "All sound",
        AudioMode::Off => "Off",
    }
}

fn audio_from_label(label: &str) -> Option<AudioMode> {
    AudioMode::ALL
        .into_iter()
        .find(|a| audio_label(*a) == label)
}

fn string_model(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}

pub fn init(window: &MainWindow, paths: Paths, settings: &Settings, controller: Arc<Controller>) {
    let paths = Rc::new(paths);
    let save_controller = Arc::clone(&controller);
    let clip_controller = Arc::clone(&controller);
    let trial_controller = Arc::clone(&controller);
    let picker_controller = Arc::clone(&controller);
    let games_controller = Arc::clone(&controller);

    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_status_text("Starting…".into());
    window.set_fps_options(string_model(SUPPORTED_FPS.iter().map(u32::to_string)));
    window.set_encoder_options(string_model(
        EncoderPreference::ALL
            .iter()
            .map(|e| encoder_label(*e).to_owned()),
    ));
    window.set_audio_options(string_model(
        AudioMode::ALL.iter().map(|a| audio_label(*a).to_owned()),
    ));
    show_settings(window, settings, &paths);
    show_games(window, settings, &controller.installed());

    window.on_estimate_disk(|minutes, fps| {
        let fps = fps.parse().unwrap_or(DEFAULT_FPS);
        let minutes = u32::try_from(minutes).unwrap_or(0);
        let bytes = estimate_buffer_bytes(minutes, nominal_bitrate_mbps(fps));
        format!("About {} on disk at 1080p{fps}.", format_gb(bytes)).into()
    });

    window.on_buffer_dir_warning({
        let paths = Rc::clone(&paths);
        move |dir| {
            let dir = paths.expand_home(dir.trim());
            tmpfs_warning(&dir, paths.runtime_dir.as_deref())
                .unwrap_or_default()
                .into()
        }
    });

    // Settings are saved and applied as soon as a field changes (text fields when editing
    // is done). `current` is what is saved, so unchanged fields don't restart anything.
    let current = Rc::new(RefCell::new(settings.clone()));
    // Save, show and apply `settings` if they differ from `current`; `done` is the message.
    let commit = {
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        let current = Rc::clone(&current);
        move |settings: Settings, done: &str| {
            let Some(window) = weak.upgrade() else { return };
            if settings == *current.borrow() {
                // Nothing changed; clear an error from an earlier invalid entry.
                if window.get_message_is_error() {
                    set_message(&window, "", false);
                }
                return;
            }
            if let Err(e) = settings.save(&paths.config_file) {
                return set_message(&window, &capitalize(&e.to_string()), true);
            }
            show_settings(&window, &settings, &paths);
            set_message(&window, done, false);
            tracing::info!(?settings, "settings saved");
            *current.borrow_mut() = settings.clone();
            controller.apply_settings(&settings);
            show_games(&window, &settings, &controller.installed());
        }
    };
    let commit = Rc::new(commit);
    let apply = {
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        let current = Rc::clone(&current);
        let commit = Rc::clone(&commit);
        move |done: &str| {
            let Some(window) = weak.upgrade() else { return };
            // Games aren't part of the form; keep them (also on "Reset to defaults").
            match read_settings(&window, &paths, &current.borrow()) {
                Ok(settings) => commit(settings, done),
                Err(e) => set_message(&window, &e, true),
            }
        }
    };
    let apply = Rc::new(apply);

    // Window picker: the open windows as listed when it was opened (or refreshed).
    // `picker_all`: every listed window with its row; `picker_windows`: the ones shown
    // (filtered by the search text), whose index the Record button passes.
    let picker_all: Rc<RefCell<Vec<(OpenWindow, WindowRow)>>> = Rc::default();
    let picker_windows: Rc<RefCell<Vec<OpenWindow>>> = Rc::default();
    let filter_picker = {
        let weak = window.as_weak();
        let picker_all = Rc::clone(&picker_all);
        let picker_windows = Rc::clone(&picker_windows);
        move || {
            let Some(window) = weak.upgrade() else { return };
            let query = window.get_picker_query();
            let (windows, rows): (Vec<OpenWindow>, Vec<WindowRow>) = picker_all
                .borrow()
                .iter()
                .filter(|(w, row)| {
                    let mut terms = vec![row.title.to_string(), row.detail.to_string()];
                    terms.extend(w.facts.class.iter().cloned());
                    terms.extend(w.facts.executable());
                    matches_query(&query, &terms)
                })
                .cloned()
                .unzip();
            *picker_windows.borrow_mut() = windows;
            window.set_open_windows(ModelRc::new(VecModel::from(rows)));
        }
    };
    let filter_picker = Rc::new(filter_picker);
    window.on_search_windows({
        let filter_picker = Rc::clone(&filter_picker);
        move |_| filter_picker()
    });
    window.on_choose_window({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        let picker_all = Rc::clone(&picker_all);
        let filter_picker = Rc::clone(&filter_picker);
        let current = Rc::clone(&current);
        move || {
            let Some(window) = weak.upgrade() else { return };
            let windows = match list_windows() {
                Ok(windows) => windows,
                Err(e) => {
                    tracing::warn!("cannot list windows: {e}");
                    Vec::new()
                }
            };
            let own_pid = std::process::id();
            let windows: Vec<OpenWindow> = windows
                .into_iter()
                .filter(|w| w.pid != Some(own_pid))
                .collect();
            let installed = installed_games(&picker_controller);
            let games = current.borrow().all_games(&installed);
            let recording = picker_controller.current_window();
            let launchers = load_launchers(&paths);
            let rows: Vec<WindowRow> = windows
                .iter()
                .map(|w| WindowRow {
                    title: w.facts.title.trim().into(),
                    detail: match find_game(&games, &w.facts) {
                        Some(game) if recording == Some(w.id) => {
                            format!("Recording now · {}", game.name)
                        }
                        Some(game) if catalog().contains(game) => {
                            format!("Built in: {}", game.name)
                        }
                        Some(game) => format!("In your games: {}", game.name),
                        None => {
                            let program = w
                                .facts
                                .executable()
                                .or_else(|| w.facts.class.last().cloned())
                                .unwrap_or_default();
                            // The launcher's name, when it says more than the title.
                            match launchers.name_for(&w.facts) {
                                Some(name) if name != w.facts.title.trim() => {
                                    format!("{program} · {name}")
                                }
                                _ => program,
                            }
                        }
                    }
                    .into(),
                })
                .collect();
            *picker_all.borrow_mut() = windows.into_iter().zip(rows).collect();
            // Opening the picker starts a new search; Refresh keeps the text.
            if !window.get_picker_open() {
                window.set_picker_query("".into());
            }
            filter_picker();
            window.set_picker_open(true);
        }
    });
    window.on_pick_window({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        let current = Rc::clone(&current);
        let commit = Rc::clone(&commit);
        move |index| {
            let Some(window) = weak.upgrade() else { return };
            let picked = usize::try_from(index)
                .ok()
                .and_then(|i| picker_windows.borrow().get(i).cloned());
            let Some(picked) = picked else { return };
            let mut settings = current.borrow().clone();
            let installed = installed_games(&trial_controller);
            // A window one of your games already matches is just recorded, not added again.
            // (One you switched off isn't in that list: picking it switches it on again.)
            let known =
                find_game(&settings.all_games(&installed), &picked.facts).map(|g| g.name.clone());
            let name = known.unwrap_or_else(|| {
                let launcher_name = load_launchers(&paths).name_for(&picked.facts);
                settings.add_game(Game::from_window(&picked.facts, launcher_name), &installed)
            });
            tracing::info!(game = %name, title = %picked.facts.title, "window picked");
            window.set_picker_open(false);
            // The picked window wins over the game being recorded (user override).
            let switch = trial_controller.prefer_window(picked.id);
            if settings == *current.borrow() {
                if switch {
                    trial_controller.restart_recorder();
                }
                set_message(&window, &format!("Recording {name}."), false);
            } else {
                // Kept only if it can be recorded (see on_game_failed).
                trial_controller.try_game(&name);
                commit(settings, &format!("Added {name} to your games."));
            }
        }
    });
    window.on_game_failed({
        let weak = window.as_weak();
        let current = Rc::clone(&current);
        let commit = Rc::clone(&commit);
        move |name, error| {
            let Some(window) = weak.upgrade() else { return };
            let mut settings = current.borrow().clone();
            settings.games.retain(|g| g.name != name.as_str());
            tracing::info!(game = %name, "picked game can't be recorded: removed again");
            commit(settings, "");
            set_message(
                &window,
                &format!("Can't record {name}, so it wasn't added to your games: {error}"),
                true,
            );
        }
    });
    // Buttons on the Games tab: section 0 yours ("Remove"), 1 installed and 3 built in
    // ("Don't record"), 2 not recorded ("Record again").
    window.on_search_games({
        let weak = window.as_weak();
        let current = Rc::clone(&current);
        let controller = Arc::clone(&games_controller);
        move |_| {
            let Some(window) = weak.upgrade() else { return };
            show_games(&window, &current.borrow(), &controller.installed());
        }
    });
    window.on_game_action({
        let weak = window.as_weak();
        let current = Rc::clone(&current);
        let commit = Rc::clone(&commit);
        let controller = Arc::clone(&games_controller);
        move |section, index| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            let mut settings = current.borrow().clone();
            // The rows as shown, i.e. filtered by the search text.
            let sections = game_sections(
                &settings,
                &controller.installed(),
                &window.get_games_query(),
            );
            let Some(row) = usize::try_from(section)
                .ok()
                .and_then(|s| sections.get(s))
                .and_then(|rows| rows.get(index))
            else {
                return;
            };
            let game = row.game.clone();
            let done = match section {
                0 => {
                    settings.games.retain(|g| *g != game);
                    format!("Removed {}.", game.name)
                }
                2 => {
                    settings.never_record.retain(|g| *g != game);
                    format!("{} will be recorded again.", game.name)
                }
                _ => {
                    settings.never_record.push(game.clone());
                    format!("{} won't be recorded.", game.name)
                }
            };
            tracing::info!(game = %game.name, section, "games list changed");
            commit(settings, &done);
        }
    });
    // Opening the Games tab looks for newly installed (or uninstalled) games.
    window.on_tab_opened({
        let weak = window.as_weak();
        let current = Rc::clone(&current);
        let controller = Arc::clone(&games_controller);
        move |tab| {
            let Some(window) = weak.upgrade() else { return };
            if tab == GAMES_TAB {
                let installed = controller.refresh_installed();
                show_games(&window, &current.borrow(), &installed);
            }
        }
    });

    window.on_apply({
        let apply = Rc::clone(&apply);
        move || apply("Settings saved.")
    });

    // "Start with the desktop" is the autostart file itself, not a setting.
    let exe = autostart::program_path(std::env::current_exe().ok(), std::env::var_os("APPIMAGE"));
    if let Some(exe) = &exe {
        match autostart::refresh(&paths.autostart_file, exe) {
            Ok(true) => tracing::info!("autostart entry now starts {}", exe.display()),
            Ok(false) => {}
            Err(e) => tracing::warn!("could not update the autostart entry: {e}"),
        }
    }
    window.set_autostart(autostart::is_enabled(&paths.autostart_file));
    window.on_autostart_toggled({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move |on| {
            let Some(window) = weak.upgrade() else { return };
            let result = match (&exe, on) {
                (Some(exe), true) => autostart::enable(&paths.autostart_file, exe),
                (None, true) => Err(std::io::Error::other(
                    "cannot find the Chronoclip program file",
                )),
                (_, false) => autostart::disable(&paths.autostart_file),
            };
            match result {
                Ok(()) => {
                    tracing::info!(on, file = %paths.autostart_file.display(), "autostart changed");
                    let text = if on {
                        "Chronoclip will start in the tray when you log in."
                    } else {
                        "Chronoclip will no longer start when you log in."
                    };
                    set_message(&window, text, false);
                }
                Err(e) => {
                    set_message(&window, &format!("Could not change autostart: {e}"), true);
                }
            }
            window.set_autostart(autostart::is_enabled(&paths.autostart_file));
        }
    });

    window.on_reset_defaults({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move || {
            let Some(window) = weak.upgrade() else { return };
            show_settings(&window, &Settings::default(), &paths);
            apply("Defaults restored and saved.");
        }
    });

    window.on_open_folder({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move |dir| {
            let dir = paths.expand_home(dir.trim());
            if let Err(e) = open_folder(&dir)
                && let Some(window) = weak.upgrade()
            {
                set_message(
                    &window,
                    &format!("Could not open {}: {e}", dir.display()),
                    true,
                );
            }
        }
    });

    window.on_save_replay(move || {
        save_controller.save_replay();
    });

    window.on_play_last_clip({
        let controller = Arc::clone(&clip_controller);
        move || controller.play_last_clip()
    });
    window.on_show_last_clip(move || clip_controller.show_last_clip());

    window.on_open_url(|url| crate::notify::open_url(&url));

    // Hotkey recorder: name the pressed key, validate like a typed hotkey, then save it.
    window.on_hotkey_captured({
        let weak = window.as_weak();
        move |text, ctrl, alt, shift, meta| {
            let Some(window) = weak.upgrade() else { return };
            match hotkey_from_keys(&text, ctrl, alt, shift, meta) {
                Ok(hotkey) => {
                    window.set_hotkey(hotkey.into());
                    window.set_hotkey_error(SharedString::new());
                    window.invoke_apply();
                }
                Err(message) => window.set_hotkey_error(message.into()),
            }
        }
    });

    // Our title bar's close button: like the window manager's, hide to the tray.
    window.on_close_window({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade()
                && let Err(e) = window.hide()
            {
                tracing::warn!("could not hide window: {e}");
            }
        }
    });

    window.on_start_resize({
        let weak = window.as_weak();
        move |edge| {
            if let Some(window) = weak.upgrade() {
                start_resize(&window, edge);
            }
        }
    });
}

/// Turn a key press from the hotkey recorder into a hotkey like `Ctrl+Shift+F9`.
fn hotkey_from_keys(
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Result<String, String> {
    use slint::platform::Key;
    const F_KEYS: [Key; 24] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::F16,
        Key::F17,
        Key::F18,
        Key::F19,
        Key::F20,
        Key::F21,
        Key::F22,
        Key::F23,
        Key::F24,
    ];
    let key = if let Some(n) = F_KEYS.iter().position(|k| SharedString::from(*k) == text) {
        format!("F{}", n + 1)
    } else {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase().to_string(),
            _ => {
                return Err(
                    "That key can't be a hotkey. Use F1–F24, or a letter or digit together with Ctrl, Alt or Super."
                        .into(),
                );
            }
        }
    };
    let mut combo = String::new();
    for (held, name) in [
        (ctrl, "Ctrl"),
        (alt, "Alt"),
        (shift, "Shift"),
        (meta, "Super"),
    ] {
        if held {
            combo.push_str(name);
            combo.push('+');
        }
    }
    combo.push_str(&key);
    combo
        .parse::<chronoclip_core::Hotkey>()
        .map(|h| h.to_string())
        .map_err(|e| capitalize(&format!("{e}.")))
}

/// Let the window manager resize the frameless window from `edge` (it follows the mouse).
fn start_resize(window: &MainWindow, edge: ResizeEdge) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::window::ResizeDirection;
    let direction = match edge {
        ResizeEdge::North => ResizeDirection::North,
        ResizeEdge::South => ResizeDirection::South,
        ResizeEdge::East => ResizeDirection::East,
        ResizeEdge::West => ResizeDirection::West,
        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
    };
    let result = window
        .window()
        .with_winit_window(|w| w.drag_resize_window(direction));
    if let Some(Err(e)) = result {
        tracing::debug!("window manager refused to resize: {e}");
    }
    // The window manager now owns the mouse until the resize ends, so the button release never
    // reaches us: the edge's TouchArea would stay "pressed", keep the resize cursor and swallow the
    // next click. Release it ourselves, after the current press event is done (Slint sets the
    // mouse grab when this handler returns), and let the next mouse move set the cursor again.
    let weak = window.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        use slint::platform::{PointerEventButton, WindowEvent};
        let Some(window) = weak.upgrade() else { return };
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: slint::LogicalPosition::new(0.0, 0.0),
                button: PointerEventButton::Left,
            });
        window.window().dispatch_event(WindowEvent::PointerExited);
    });
}

/// The installed games the launchers know (read when the picker needs them).
fn load_launchers(paths: &Paths) -> Launchers {
    Launchers::load(
        &paths.home_dir,
        &data_home(&paths.home_dir),
        &config_home(&paths.home_dir),
    )
}

pub fn set_message(window: &MainWindow, text: &str, is_error: bool) {
    window.set_message(text.into());
    window.set_message_is_error(is_error);
}

fn show_settings(window: &MainWindow, settings: &Settings, paths: &Paths) {
    window.set_hotkey(settings.hotkey.to_string().into());
    window.set_buffer_minutes(settings.buffer_minutes as f32);
    window.set_fps(settings.fps.to_string().into());
    window.set_encoder(encoder_label(settings.encoder).into());
    window.set_audio(audio_label(settings.audio).into());
    window.set_microphone(settings.microphone);
    window.set_save_sound(settings.save_sound);
    window.set_custom_title_bar(settings.custom_title_bar);
    window.set_buffer_dir(paths.contract_home(&settings.buffer_dir(paths)).into());
    window.set_output_dir(paths.contract_home(&settings.output_dir(paths)).into());
}

/// Index of the Games tab in the title bar (Home · Games · Settings · About).
const GAMES_TAB: i32 = 1;

/// One row on the Games tab.
struct GameEntry {
    game: Game,
    detail: String,
}

/// The Games tab's sections: yours, installed, not recorded, other built-in games.
fn game_sections(
    settings: &Settings,
    installed: &[InstalledGame],
    query: &str,
) -> [Vec<GameEntry>; 4] {
    let entry = |game: &Game, extra: Option<&str>| GameEntry {
        game: game.clone(),
        detail: match extra {
            Some(extra) => format!("{} · {extra}", game.describe()),
            None => game.describe(),
        },
    };
    let yours = |game: &Game| {
        settings
            .games
            .iter()
            .any(|g| g.name.eq_ignore_ascii_case(&game.name) || g.covers(game))
    };
    // Search: name, executables, title rule, Steam ids and where it was found.
    let found = |game: &Game, extra: Option<&str>| {
        let mut terms = game.search_terms();
        terms.extend(extra.map(str::to_owned));
        matches_query(query, &terms)
    };
    let mine = settings
        .games
        .iter()
        .filter(|g| found(g, None))
        .map(|g| entry(g, None))
        .collect();
    let installed_rows: Vec<GameEntry> = installed
        .iter()
        .filter(|i| !yours(&i.game) && !settings.is_never_recorded(&i.game))
        .filter(|i| found(&i.game, Some(&i.source)))
        .map(|i| entry(&i.game, Some(&i.source)))
        .collect();
    let ignored = settings
        .never_record
        .iter()
        .filter(|g| found(g, None))
        .map(|g| entry(g, None))
        .collect();
    let builtin = catalog()
        .iter()
        .filter(|g| {
            !yours(g)
                && !settings.is_never_recorded(g)
                && !installed.iter().any(|i| i.game.name == g.name)
                && found(g, None)
        })
        .map(|g| entry(g, None))
        .collect();
    [mine, installed_rows, ignored, builtin]
}

/// Fill the Games tab (filtered by its search text).
fn show_games(window: &MainWindow, settings: &Settings, installed: &[InstalledGame]) {
    let query = window.get_games_query();
    let [mine, installed, ignored, builtin] = game_sections(settings, installed, &query);
    let rows = |entries: Vec<GameEntry>, action: &str, known: bool| -> ModelRc<GameRow> {
        let rows: Vec<GameRow> = entries
            .into_iter()
            .map(|e| GameRow {
                name: e.game.name.into(),
                detail: e.detail.into(),
                action: action.into(),
                known,
            })
            .collect();
        ModelRc::new(VecModel::from(rows))
    };
    window.set_games(rows(mine, "Remove", false));
    window.set_installed_games(rows(installed, "Don't record", true));
    window.set_ignored_games(rows(ignored, "Record again", false));
    window.set_builtin_games(rows(builtin, "Don't record", true));
}

/// The installed games' entries (from the controller's last scan).
fn installed_games(controller: &Controller) -> Vec<Game> {
    controller.installed().into_iter().map(|i| i.game).collect()
}

fn read_settings(window: &MainWindow, paths: &Paths, base: &Settings) -> Result<Settings, String> {
    let hotkey = window
        .get_hotkey()
        .trim()
        .parse()
        .map_err(|e| format!("Hotkey: {e}."))?;
    let fps = window
        .get_fps()
        .parse()
        .map_err(|_| format!("Invalid frame rate `{}`.", window.get_fps()))?;
    let encoder = encoder_from_label(&window.get_encoder())
        .ok_or_else(|| format!("Unknown encoder `{}`.", window.get_encoder()))?;
    let audio = audio_from_label(&window.get_audio())
        .ok_or_else(|| format!("Unknown sound setting `{}`.", window.get_audio()))?;

    let settings = Settings {
        hotkey,
        buffer_minutes: window.get_buffer_minutes().round() as u32,
        fps,
        encoder,
        audio,
        microphone: window.get_microphone(),
        save_sound: window.get_save_sound(),
        custom_title_bar: window.get_custom_title_bar(),
        buffer_dir: dir_setting(&window.get_buffer_dir(), &paths.default_buffer_dir, paths),
        output_dir: dir_setting(&window.get_output_dir(), &paths.default_output_dir, paths),
        games: base.games.clone(),
        never_record: base.never_record.clone(),
    };
    settings
        .validate()
        .map_err(|e| capitalize(&e.to_string()))?;
    Ok(settings)
}

/// Empty or equal to the default means "use the default" (stored as `None`).
fn dir_setting(text: &str, default: &Path, paths: &Paths) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let path = paths.expand_home(text);
    (path != default).then_some(path)
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn open_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Reap the child in the background so it doesn't linger as a zombie.
    let mut child = Command::new("xdg-open").arg(dir).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
