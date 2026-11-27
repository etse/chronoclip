//! Is an unknown window a game? Used to ask "Record this game?" for windows no games list
//! knows. Only what the capture backend already collects ([`WindowFacts`]) is looked at.
//!
//! For a game: started through Wine/Proton/Steam, asks the desktop not to composite it
//! (`_NET_WM_BYPASS_COMPOSITOR = 1`, SDL's default), fullscreen. Against: programs that are
//! known not to be games (launchers, browsers, chat apps, terminals, video players) and small
//! windows. Measured on the maintainer's desktop (2026-11-27): Chrome, Slack, Discord and
//! 1Password set bypass `2`; the Battle.net launcher looks like a Proton game in every other
//! way, so it must be on the list.

use crate::games::WindowFacts;

/// Smaller windows are no games (dialogs, launchers' small windows, tools).
const MIN_SIZE: (u32, u32) = (640, 360);

/// Points a window needs to be asked about.
const THRESHOLD: u32 = 2;

/// Programs (executable or WM_CLASS, lower case, without `.exe`) that are not games.
const NOT_GAMES: &[&str] = &[
    // Game launchers and stores
    "battle.net",
    "battle.net launcher",
    "agent",
    "steam",
    "steamwebhelper",
    "epicgameslauncher",
    "eadesktop",
    "eabackgroundservice",
    "origin",
    "ubisoftconnect",
    "upc",
    "uplay",
    "galaxyclient",
    "riotclientservices",
    "riotclientux",
    "rockstarlauncher",
    "launcher",
    "lutris",
    "heroic",
    "faugus-launcher",
    "bottles",
    "itch",
    "explorer",
    "winecfg",
    "regedit",
    // Browsers, chat and other apps
    "chrome",
    "google-chrome",
    "chromium",
    "firefox",
    "brave",
    "vivaldi",
    "opera",
    "discord",
    "vesktop",
    "slack",
    "teams",
    "zoom",
    "telegram-desktop",
    "signal-desktop",
    "spotify",
    "obs",
    "code",
    "electron",
    "1password",
    // Terminals and desktop
    "ghostty",
    "gnome-terminal-server",
    "kitty",
    "alacritty",
    "konsole",
    "xterm",
    "wezterm-gui",
    "nemo",
    "nautilus",
    "dolphin",
    "thunar",
    "cinnamon",
    "chronoclip",
    // Video players (some ask for no compositing in fullscreen)
    "mpv",
    "vlc",
    "celluloid",
    "totem",
    "smplayer",
    "ffplay",
];

/// True if the program behind `window` is known not to be a game.
pub fn is_known_non_game(window: &WindowFacts) -> bool {
    let normalize = |name: &str| {
        let lower = name.trim().to_lowercase();
        lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
    };
    window
        .executable()
        .iter()
        .chain(window.class.iter())
        .map(|n| normalize(n))
        .any(|n| NOT_GAMES.contains(&n.as_str()))
}

/// How much `window` looks like a game (0 = not at all). See the module docs.
pub fn game_score(window: &WindowFacts) -> u32 {
    if is_known_non_game(window) || window.size.0 < MIN_SIZE.0 || window.size.1 < MIN_SIZE.1 {
        return 0;
    }
    let windows_program = window
        .process_names
        .iter()
        .any(|n| n.to_ascii_lowercase().ends_with(".exe"))
        || [
            "WINEPREFIX",
            "STEAM_COMPAT_APP_ID",
            "SteamAppId",
            "SteamGameId",
        ]
        .iter()
        .any(|k| window.env(k).is_some());
    let steam_game = crate::launchers::steam_app_id(window).is_some();
    let mut score = 0;
    if windows_program || steam_game {
        score += 2;
    }
    if window.bypass_compositor == Some(1) {
        score += 2;
    }
    if window.fullscreen {
        score += 1;
    }
    score
}

/// True if Chronoclip should ask whether to record `window`.
pub fn looks_like_game(window: &WindowFacts) -> bool {
    game_score(window) >= THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(class: &str, process: &[&str]) -> WindowFacts {
        WindowFacts {
            title: "window".into(),
            class: vec![class.into(), class.into()],
            process_names: process.iter().map(|&s| s.into()).collect(),
            size: (1920, 1080),
            ..WindowFacts::default()
        }
    }

    #[test]
    fn proton_and_sdl_games_look_like_games() {
        // A Proton game: Steam's class and a Windows executable.
        let proton = window("steam_app_999999", &["Game.exe", r"Z:\games\Game.exe"]);
        assert!(looks_like_game(&proton));
        // A native SDL game: asks for no compositing.
        let sdl = WindowFacts {
            bypass_compositor: Some(1),
            ..window("supergame", &["supergame", "/opt/supergame/supergame"])
        };
        assert!(looks_like_game(&sdl));
        // A Wine program from a prefix (Lutris, Bottles).
        let wine = WindowFacts {
            env: vec![("WINEPREFIX".into(), "/home/me/Games/x".into())],
            ..window("game.exe", &["Game.exe"])
        };
        assert!(looks_like_game(&wine));
    }

    #[test]
    fn apps_and_launchers_are_not_games() {
        // Measured on the maintainer's desktop.
        let battlenet = WindowFacts {
            env: vec![
                ("SteamAppId".into(), "faugus5dd8f7d6".into()),
                ("WINEPREFIX".into(), "/home/me/Faugus/battlenet/pfx/".into()),
            ],
            ..window("steam_app_faugus5dd8f7d6", &["Battle.net.exe"])
        };
        assert!(!looks_like_game(&battlenet));
        let chrome = WindowFacts {
            bypass_compositor: Some(2),
            fullscreen: true,
            ..window(
                "google-chrome",
                &["chrome", "/opt/google/chrome/chrome --type"],
            )
        };
        assert!(!looks_like_game(&chrome));
        assert!(!looks_like_game(&window("steam", &["steamwebhelper"])));
        assert!(!looks_like_game(&window("ghostty", &["ghostty"])));
        let mpv = WindowFacts {
            bypass_compositor: Some(1),
            fullscreen: true,
            ..window("mpv", &["mpv", "/usr/bin/mpv"])
        };
        assert!(!looks_like_game(&mpv));
        // A plain native window, even fullscreen: not enough.
        let editor = WindowFacts {
            fullscreen: true,
            ..window("gimp", &["gimp-2.10"])
        };
        assert!(!looks_like_game(&editor));
    }

    #[test]
    fn small_windows_are_not_games() {
        let small = WindowFacts {
            size: (400, 300),
            ..window("steam_app_999999", &["Game.exe"])
        };
        assert_eq!(game_score(&small), 0);
    }
}
