# Roadmap

Status: ✅ done · 🚧 in progress · ⬜ not started

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold: repo, docs, agent instructions | ✅ |
| M1 | Skeleton app: Slint window, tray, settings, single instance, `chronoclip save/show/quit` | ✅ |
| M2 | Capture pipeline (X11 + NVIDIA) | ✅ |
| M3 | Save replay: global hotkey, lossless remux to MP4 | ✅ |
| M4 | Audio: game audio via PipeWire, optional mic track | ✅ |
| M5 | Polish: save notification and chime, tray state, start with desktop | ✅ |
| M6 | Packaging: .deb (Ubuntu/Mint) and AppImage on GitHub Releases | ✅ |
| M7 | Modern dark UI: Battle.net-style tabs, Discord-like look, custom title bar | ✅ |
| M8 | Other games: record any game, not just WoW: game detection, window picker, per-game clips | 🚧 |
| M9 | Wayland + AMD/Intel: portal/PipeWire capture, GlobalShortcuts portal, VAAPI | ⬜ |

## M8: Other games

Goal: Chronoclip records whatever game you play, not just WoW: it finds the game by itself (asking once when unsure), names it properly, and saves its clips in a folder per game. WoW behaves exactly as before.

Decisions (maintainer, 2026-11-27):
- **How automatic:** a "looks like a game" score, plus a "Record this game?" question for unknown games; the answer is remembered.
- **Two games open:** the first one found is recorded until it closes. Choosing another window in the picker switches to it (user override). The recorder also stays with its window when settings change.
- **Clips:** a folder per game, e.g. `Elden Ring/Elden Ring_2026-11-27_21-04-33.mp4`.
- **Game changes:** the buffer is cleared, so a clip never mixes two games.

Test games: WoW (Faugus/Battle.net), Elden Ring, Satisfactory, Palworld, Path of Exile 2 (Steam/Proton).

Findings while planning (2026-11-27, on the maintainer's desktop):
- The **Battle.net launcher** has the same Proton signals as a game: WM_CLASS `steam_app_faugus…`, and `SteamAppId`/`GAMEID`/`WINEPREFIX` in its environment. Launchers must be skipped explicitly.
- `_NET_WM_BYPASS_COMPOSITOR`: Chrome, Slack, Discord and 1Password set `2` ("please compose me"). SDL games set `1`.
- GameMode runs but only knows games started with `gamemoderun`. NVIDIA's GPU process list marks nothing as special (Xorg, Cinnamon and Chrome all use the GPU), and it's NVIDIA-only.
- Steam `appmanifest_*.acf` files also list tools (Proton, Steam Linux Runtime, Steamworks Redistributables); filter them.

Detection only reads window properties and `/proc/<pid>/{comm,cmdline,environ}` (kernel metadata). Never `/proc/<pid>/maps` or `mem` (hard rule 1).

- [x] Core: a `Game` model (name, how it's matched: executable, Steam app id, window title) and a games list in the settings, with WoW built in; replaces the WoW-only `WindowMatcher` (tests for matching and settings validation)
  - `chronoclip-core::games` (`Game` with `executables` / `title_contains`, `WindowFacts`, `find_game`); `Settings::games` (`[[games]]`, invalid entries dropped on load) and `all_games()` = WoW first, then the user's. Steam app ids come with the launcher task. Game sound is matched by the game's executables and name instead of WoW's. `CHRONOCLIP_WINDOW` is now a title-only game
  - Test instance: a game added only in `settings.toml` (`games = [{ name = "Test Game", title_contains = "cc-game-a" }]`) was found (`found window game=Test Game`), its sound recorded by process id, and a 12 s clip saved (H.264 + AAC). The Settings page keeps the games list when it saves
- [x] Window picker: "Choose a window…" on Home lists open windows (title, executable); picking one adds it to your games and records it
  - Home: "Game not found? Choose window" (while waiting) / "Choose window"; Games tab: "Add a game window…". Dialog over the window (`app/ui/picker.slint`), Esc or a click outside closes it. Lists `_NET_CLIENT_LIST` without desktop/dock windows, untitled ones and Chronoclip itself; shows the program or "Recorded as <game>"
  - A picked window becomes a game matched by its executable (`.exe` under Wine/Proton, else the program file); generic runners (Wine, Python, Java, …) are matched by title instead. Already covered (e.g. picking WoW) → "already in your games"
  - Test instance (2026-11-27): picked a test window from the picker → `window picked`, saved as `[[games]]`, recorded at once (`found window game=cc-game-b`)
  - Maintainer: works (2026-11-27), but a window that can't be recorded stayed in the list. Fixed: a pick is a trial; if recording it fails before it records (`RecorderStatus::Failed`), it's removed again and Home shows why. Windows under 64×64 aren't listed. Test instance: a 1×1 "Notes" window (NVENC refused it) was removed again with the reason on Home; a working test window was kept (`picked game records fine`)
- [x] Game names from launchers: Steam (`steam_app_<id>` / `SteamAppId` → `appmanifest`, tools filtered out), Faugus; the window title or executable name as fallback
  - `chronoclip-core::launchers` (`Launchers::load` / `name_for`): Steam libraries from native, `~/.steam` and Flatpak installs plus `libraryfolders.vdf`; Faugus `games.json` counts only when the window runs the entry's own `.exe` in its prefix (WoW runs in the Battle.net entry's prefix with its environment). The capture backend reads only `SteamAppId`, `SteamGameId`, `STEAM_COMPAT_APP_ID` and `WINEPREFIX` from `/proc/<pid>/environ`
  - Maintainer's computer (2026-11-27): Steam ids of the test games → ELDEN RING, Satisfactory, Palworld, Path of Exile 2; Proton Experimental filtered; the Battle.net window → "Battle.net" via Faugus. Test instance: a window with `SteamAppId=1245620` shows `ffplay · ELDEN RING` in the picker and was saved and recorded as "ELDEN RING"
- [x] Game names from Lutris and Heroic (implemented from their source code; neither is installed on the maintainer's computer)
  - Lutris sets `GAME_NAME` for every game it starts (`game.py` `configure_game`): no need to read `pga.db` (SQLite). A game started from a store launcher entry inherits that entry's `GAME_NAME` (Diablo IV from "Battle.net"), so names of store launchers are ignored
  - Heroic sets `HEROIC_APP_NAME` (`launcher.ts` `setupWrapperEnvVars`); titles come from `store_cache/*_library.json` and `sideload_apps/library.json` (native `~/.config/heroic` and Flatpak), any object with `app_name` + `title` (shapes from Heroic's backup tests)
  - Test instance (2026-11-27): test windows with `GAME_NAME=Hades` and with `HEROIC_APP_NAME` + a GOG library file showed `ffplay · Hades` and `ffplay · Unreal Tournament 2004` in the picker. Not verified with real Lutris/Heroic installs
- [x] Built-in games catalog as a data file in the repo (`crates/chronoclip-core/data/games.toml`, one `[[game]]` per game: name, executables, Steam app ids, optional title), compiled into the binary (`include_str!`), not a file users edit in a release (they add their own games in the app). World of Warcraft moves from code into it. A unit test validates every entry (name, at least one way to match, no executable or Steam id used twice) so pull requests get checked by CI; a short "Add a game" section in CONTRIBUTING/README says how to add one
  - `Game` gets `steam_app_ids` (matched against the window's Steam app id), which is more reliable for the catalog than executable names
  - Done: `crates/chronoclip-core/data/games.toml` (`catalog()`, strict parsing: a misspelled key fails), WoW moved there; `Game::steam_app_ids` matched against the window's Steam id (also stored for picked Steam games); priority: your games, then the catalog. Tests `catalog_entries_are_valid_and_unique`, `catalog_rejects_mistakes`. CONTRIBUTING: "Add a game to the catalog". Games tab: "Your games" (Remove) and "Built in (N)"
  - First entries with sources from the maintainer's installs: World of Warcraft, ELDEN RING, Palworld, Path of Exile 2, Satisfactory (Steam ids from the manifests, executables from the install folders)
  - Test instance (2026-11-27): a window with `SteamAppId=1245620` was recorded as "ELDEN RING" without picking it
- [x] Fill the catalog with many common games (popular Steam/Proton and native Linux titles, plus the maintainer's: Elden Ring, Satisfactory, Palworld, Path of Exile 2, …), each with a Steam app id and/or executable taken from a checkable source (Steam store, PCGamingWiki), not guessed
- [x] Installed games found automatically: every installed Steam game (`appmanifest_*.acf` in all libraries, matched by Steam app id; Proton/runtimes/redistributables filtered) and every installed Battle.net game (Blizzard games found in the Battle.net Wine prefixes of Faugus, Lutris, Heroic, Bottles and Steam non-Steam shortcuts, matched by the catalog's executables). Read at startup and when the Games tab opens, not stored in the settings, so uninstalled games disappear by themselves. Shown on the Games tab as "Installed on this computer (N)"; one that isn't a game (Wallpaper Engine, Blender) can be switched off ("never record")
  - `chronoclip-core::installed::find_installed`: Steam games (the catalog entry if there is one, else matched by app id) and catalog games whose executable is in a Wine prefix (Faugus, Lutris `prefix:`, Heroic `winePrefix`, Bottles, Steam `compatdata`, `~/.wine`, `~/Games/*`; `Program Files` up to 4 levels). Catalog additions from the maintainer's Battle.net prefix: Heroes of the Storm, StarCraft II, Warcraft III (Diablo IV only partly installed: no `.exe`, left out). Catalog test also checks the file stays sorted
  - Settings: `never_record` ("Don't record" on installed/built-in games, "Record again"; picking such a window switches it on again). Games tab: Your games · Installed on this computer (N) · Not recorded (N) · Other built-in games (N); the Games tab scans again when opened
  - The recorder's games list is shared (`Controller::games`) instead of part of its settings: changing it never restarts the recorder (no lost buffer); a session ends when its game leaves the list
  - Maintainer's computer (2026-11-27): 19 games found in 38 ms (release build): all 15 Steam games, Heroes of the Storm, StarCraft II, Warcraft III and WoW in `~/Faugus/battlenet`. Test instance: a window with Factorio's Steam id (installed, not in the catalog) was recorded as "Factorio" by itself; "Don't record" stopped it without a restart (`no longer in the games list`), "Record again" recorded it again
- [x] Search for long lists: a simple text filter on the Games tab (and in the window picker) that matches any part of the name, executables, title rule or Steam id, case-insensitive; the built-in catalog is shown in its own section ("Built in (N)"), your games first
  - `games::matches_query` (every word must appear in a field, case-insensitive) over `Game::search_terms` (name, executables, title rule, Steam ids) plus where it was found; Rust filters, Slint only passes the text. `SearchField` (Lucide `search` icon, clear button, Esc clears; in the picker a second Esc closes it). The picker focuses its search field when it opens
  - Test instance (2026-11-27): "faugus" on the Games tab shows the 4 Battle.net games; "Don't record" on the 2nd filtered row switched off the right game (StarCraft II); "zzz" shows "No games match"; in the picker "pick" left one window and Record picked it; Esc cleared, then closed the picker
  - 176 games (2026-11-27): Steam's most played games chart (`ISteamChartsService/GetMostPlayedGames`, 100 ids) plus ~80 popular titles, every id checked with `store.steampowered.com/api/appdetails` (name and type "game"; two ids that didn't check out dropped); software (Wallpaper Engine, OBS Studio, overlays, a mod) left out; ™ ® © removed from names. Steam entries have only the app id (enough under Steam/Proton). The 8 entries from the maintainer's installs keep their executables
  - Not covered: the non-Steam versions of these games (Battle.net, Epic, Xbox app), which need checkable executable names; games whose anti-cheat blocks Linux are listed anyway (harmless)
- [x] Game score: signals for (Wine/Proton/Steam launched, bypass-compositor `1`, fullscreen or monitor-sized, GameMode) and against (known launchers and helpers, browsers, Electron apps); unit tests on recorded window lists, including the Battle.net launcher above
  - `chronoclip-core::detect` (`game_score`, `looks_like_game`, threshold 2): +2 Windows program (`.exe`, `WINEPREFIX`, Steam ids), +2 `_NET_WM_BYPASS_COMPOSITOR = 1`, +1 fullscreen; 0 for known non-games (launchers, browsers, chat apps, terminals, video players) and windows under 640×360. The capture backend now collects the compositor hint, fullscreen state and size per window. GameMode left out: it only knows games started with `gamemoderun`
  - Maintainer's desktop (2026-11-27): every open window scores 0 (Battle.net launcher, Steam, Chrome, Discord, Slack, 1Password, terminal, file manager). Unit tests with those and with Proton, SDL and Wine-prefix games
- [x] "Record this game?" notification for a likely but unknown game (Always / Not this one), remembered by executable
  - While no game is recorded the recorder publishes likely games that are in no list (`Shared::candidates`); the controller asks about the first one not on the "Not recorded" list: a card on Home (*Record* / *Not this one*) and one notification per window with the same buttons (Cinnamon hides notifications over fullscreen windows, so Home is the reliable place). *Record* works like the picker (added, preferred, trial); *Not this one* adds it to "Not recorded" (undo on the Games tab)
  - Test instance (2026-11-27): an SDL window (renamed ffplay with `SDL_VIDEO_X11_WMCLASS=testgame`, `SDL_VIDEO_X11_NET_WM_BYPASS_COMPOSITOR=1`) → "Record cc-unknown?" on Home; *Not this one* → `never_record`, not asked again after the window was restarted; in a fresh instance *Record* → added and recorded (`picked game records fine`). glxgears is never asked about: it sets no `_NET_WM_PID`, so nothing is known about its process
  - Maintainer tried it in an isolated test instance: "I like it" (2026-11-27)
- [x] Per-game clips: folder and file name after the game (safe file names); status texts, tray, "nothing recorded" message name the game
  - `clips::safe_file_name` (no `/ \ : * ? " < > |` or control characters, for exFAT/NTFS drives too; no leading/trailing dots; 80 characters), `game_folder`, `clip_file_name(game, time)`; the replay buffer remembers its game (`ReplayBuffer::set_game`). Status, tray and the last clip card already name the game
  - Test instance (2026-11-27): a window with Hollow Knight: Silksong's Steam id → `clips/Hollow Knight Silksong/Hollow Knight Silksong_2026-11-27_09-51-51.mp4` (8 s, video + audio); Home shows "Hollow Knight: Silksong"
- [x] Switching games: when the recorded game closes, look again; a different game clears the buffer first
  - Done so far: the recorder keeps a shared "current window" (`Controller::current_window`), prefers it in `find_window` and sets it to what it records, so a second game doesn't take over and a settings change doesn't switch games. Picking a window points it there and restarts the recorder (buffer cleared). Test instance (2026-11-27), games [B, A]: A started first → A; B started → still A; picked B → B; settings changed (game removed) → still B; B closed → back to A. Picking a window one of your games already matches records it without adding a duplicate. Clip folders per game: done with the per-game clips task
- [ ] Games tab (planned as a Settings card) listing your games (World of Warcraft built in, games you added or that were detected), with **Remove** for one added by mistake, and the "never record" list (built with the window picker, so a wrong pick can be undone right away)
  - Done: a **Games tab** (Home · Games · Settings · About; moved out of Settings after the maintainer noted the list easily grows past 10 games) lists World of Warcraft ("Built in") and your games with how they're recognized, **Remove** and "Add a game window…". Checked with 12 games: scrolls, Remove hits the right entry. Test instance: Remove dropped the game from `settings.toml` and recording stopped ("Nothing recorded yet"). The "never record" list now exists (Not recorded section); the "Record this game?" question will add to it
- [x] Texts: "for World of Warcraft" → "for your games" in README, About, desktop file, metainfo, CLI help, autostart file
  - Also the .deb description, the Settings sound text, `app/Cargo.toml`, INSTALL.md and skills; About and README disclaimers name other publishers too. WoW stays as the reference setup and "made for WoW first". Existing autostart entries get the new text by themselves (`autostart::refresh` rewrites ours when it differs). `desktop-file-validate` and `appstreamcli validate --pedantic` pass
- [x] Docs: README, architecture.md (detection), AGENTS.md (project description), CHANGELOG
  - README: intro, using it, features (finding games, "Record this game?", Games tab, a folder per game), Games tab in Configuration, screenshots retaken (readme-screenshot skill; Games tab in the title bar). architecture.md: "Finding the game" and "Decisions made in M8". AGENTS.md: project description, layout (core's game modules, `data/games.toml`). CONTRIBUTING: "Add a game to the catalog"
- [x] Verified: fmt, clippy, tests; test instance with test windows (known game, unknown game → question, launcher ignored, switch clears buffer)
  - 2026-11-27: fmt and clippy clean; 101 unit tests and the 10 desktop tests (`--ignored`: GLX/NVENC capture, audio, XTest hotkey, window and installed-games listing; without the one that needs WoW running) pass
  - Test instance, one run: no game running → nothing asked about although the real Battle.net, Steam, Chrome and Discord windows were open; a window with Elden Ring's Steam id → recorded as "ELDEN RING" with game sound, clip in `ELDEN RING/`; an unknown SDL game started meanwhile → no switch, no question; ELDEN RING closed → "Record My Test Game?" → *Record* → recording switched, the old recording dropped, clip in `My Test Game/`
- [ ] Home: the last clip card becomes a list of the clips saved this session, newest first, at most 10, each with Play and Show in folder. When it doesn't fit, only the list scrolls: status, replay buffer and Save replay above it always stay visible
- [ ] AppImage menu entry: Cinnamon's window list shows a generic cog for a window without a menu entry (it ignores the window's own icon), which is what an AppImage has unless AppImageLauncher or Gear Lever integrates it. When run as an AppImage (`$APPIMAGE`) without an installed entry, Chronoclip offers to add its own: `~/.local/share/applications/io.github.etse.chronoclip.desktop` (`Exec` = the AppImage, `StartupWMClass=chronoclip`) and the icons in `~/.local/share/icons/hicolor`, updated when the AppImage moves (like the autostart entry), removable again; a switch in Settings
  - The .deb shows the right icon (maintainer installed the current build, 2026-11-27): only uninstalled runs (`cargo run`, a plain AppImage) get the cog
- [x] Verified in-game: WoW as before; Elden Ring, Satisfactory, Palworld and Path of Exile 2 detected with the right names, clips in their own folders, launchers (Battle.net, Steam) never recorded
  - Maintainer, 2026-11-27, 12-step checklist: all pass (waiting without questions with Battle.net/Steam/Discord open; Games tab with 19 installed games and search; WoW recorded with game sound into `World of Warcraft/`; Elden Ring started meanwhile didn't switch; Choose window switched, clip only Elden Ring; back to WoW after quitting it; Satisfactory, Palworld, Path of Exile 2 recorded by themselves with sound into their folders; Don't record / Record again)
  - Found two bugs, both fixed: every change on the Settings tab crashed the app ("RefCell already borrowed", from the Games-tab work in M8: a `match` kept a borrow of the settings alive while saving them; in a game the window froze); the window had no icon in the taskbar (Slint's `icon` doesn't reach the window with the software renderer; now set through winit from the PNG; probably since M1). Verified with a test instance: 6 setting changes in a row, `_NET_WM_ICON` 256×256 after start and after start in the tray

Out of scope: recording several games at once, per-game settings (fps, sound), Wayland (M9), an online game database (no network calls, hard rule 8).

## M7: Modern dark UI

Goal: a window that looks like the apps gamers use every day (Battle.net launcher, Discord): dark, clean, one accent colour, big clear status and a big **Save replay** button, while the Rust side and the recording stay as they are.

Decisions (maintainer, 2026-11-26): **top tabs** like the Battle.net launcher (Home · Settings · About), **Discord blurple** accent (`#5865F2`, red stays for the record dot), extras: hotkey recorder, last clip card, pulsing record dot, custom title bar. Dark only.

Approach: own Slint components instead of the stock `fluent` widgets (they can't be restyled this far). The window keeps getting its data through the same properties and callbacks; new data (buffer fill, last clip) comes from Rust like today. Theme + Home page first, then a screenshot for the maintainer's feedback before the rest.

- [x] Theme (`app/ui/theme.slint`): dark palette, accent, radii, spacing, type scale; bundled **Inter** font (OFL, license file shipped); dark system title bar hint where the system one is used
- [x] Component kit (`app/ui/components/`): buttons (primary/secondary/ghost), toggle, slider, dropdown (own popup), text field, card, tab bar, progress bar, status badge, icon (**Lucide** SVGs, ISC); hover/pressed/focus states, keyboard focus visible
  - `app/ui/components.slint` (button, icon button, keycap, card, progress bar, tab) and `controls.slint` (toggle, slider, segmented control, dropdown, text field, hotkey field). Fixed: the focus scopes swallowed the first click (`focus-on-click: false`)
- [x] Custom title bar: frameless window with our own dark bar (logo, tabs, minimize/close), drag to move, double-click to maximize, resize edges; setting to fall back to the system title bar if the window manager misbehaves
  - Slint's built-in `WindowMoveArea`, `maximized`/`minimized`; edge resizing via winit `drag_resize_window` (`unstable-winit-030`). Fixed after the maintainer's test: the window manager keeps the mouse release, so the edge handle stayed pressed and swallowed the next click; a release is now sent right after the resize starts. Fallback toggle verified (Cinnamon's title bar appears at once)
- [x] Home page: status card (state, game, resolution, fps, sound), buffer bar (filled/length, size), big **Save replay** button with the hotkey, last clip card (length, size, time; **Play**, **Show in folder**), save progress/result
  - Maintainer: "I like the look on the home page" (2026-11-26)
- [x] Settings page: the existing settings in cards (Recording, Sound, Saving, Folders, Startup), same auto-save behaviour, folder warnings and errors in the new style
- [x] Hotkey recorder: click the field, press the combination (Esc cancels); same validation as today
  - Verified with XTest: Ctrl+Shift+F10 recorded, saved and registered; Tab gives a clear error
- [x] About page: version, links, licenses; keeps Slint's "Made with Slint" attribution (license requirement)
  - Slint badge kept small (132 px). Maintainer on Settings/About: "I like it, this is good enough for a first version. Looks more modern!" (2026-11-27)
- [x] Pulsing record dot, animated only while the window is visible (no redraws while hidden or minimized)
  - About +1.2 % of one core while the window is visible; nothing when hidden
- [x ] Checks: readable at 100–200 % scaling, keyboard navigation (Tab, Enter, Space, arrows), no CPU while hidden; README screenshots retaken (readme-screenshot skill); licenses (Inter, Lucide) in the packages' license files and INSTALL/README if needed
  - Done: Inter and Lucide license texts in `THIRD-PARTY-LICENSES.txt` and the .deb copyright file; README screenshots (Home + Settings) and text, AppStream screenshots. Keyboard pass (2026-11-27): tabs and **Save replay** were not reachable and the dropdown list was mouse-only; fixed (focusable tabs and Save button, Up/Down in the dropdown, focus ring only for keyboard focus). Order Home: tabs → Save replay → Play → Show in folder; Settings: every control in reading order, the page scrolls to the focused one. Keys verified against settings.toml: slider ←/→, segmented ←/→, dropdown ↓, toggles Space, hotkey field Enter/Esc. 200 % (`SLINT_SCALE_FACTOR=2`, 1840×1280) and the 720×540 minimum: no clipping on any page
- [x ] Verified in-game: the maintainer uses the new window while playing (status, save, settings)
  - Maintainer: resize bug "verified … fixed. Did my manual test. Seems ok" (2026-11-27)

Out of scope: styling the tray *menu* (drawn by the desktop), clip library (dropped in M5), light theme.

## M6: Packaging

Goal: Ubuntu and Linux Mint users install Chronoclip from a **.deb**, everyone else runs the **AppImage**; both are downloaded from GitHub Releases and need no FFmpeg or build tools. First release: **v0.1.0**.

Both packages carry the same **LGPL FFmpeg** (built by us, shared libraries): the distros' FFmpeg differs in version between releases (Ubuntu 24.04 has 6.1, 26.04 newer), and Ubuntu's is a GPL build (`--enable-gpl --enable-libx264`) that we may not ship (AGENTS rule 7). With our own FFmpeg in `/usr/lib/chronoclip/`, **one .deb works on Ubuntu 22.04 / 24.04 / 26.04 and Mint 21 / 22 / 23**. X11, OpenGL, CUDA/NVENC (NVIDIA driver) and PulseAudio still come from the host.

- [x] Groundwork: app-menu entry (`chronoclip.desktop`), hicolor icons (16–512 px), AppStream metainfo, `chronoclip --version`; notifications and autostart use the installed icon
- [x] License files for binaries: third-party Rust licenses (`cargo-about`), FFmpeg's LGPL text, its exact source and build configuration, Slint attribution (already in About)
  - Rust crates: `packaging/licenses/generate.sh` (incl. the Slint royalty-free text that cargo-about 0.9 can't include). FFmpeg: `build.sh` writes `FFMPEG-LICENSE.txt` (source URL, SHA-256, configure flags) and the LGPL text; the release workflow must also attach the FFmpeg source tarball, as that file promises
- [x] LGPL FFmpeg build script (`packaging/ffmpeg/`): a fixed FFmpeg release, shared libs, no GPL/nonfree parts, only what Chronoclip uses (`h264_nvenc` + CUDA via `nv-codec-headers`, `aac`, mpegts/mp4 muxing and demuxing); the app is compiled against it
- [x] Build environment: everything built in an Ubuntu 22.04 container (oldest glibc we support), in CI; locally only with the maintainer's OK (Docker needs sudo)
  - `.github/workflows/package.yml`: Ubuntu 22.04 container on GitHub Actions (Rust, FFmpeg and cargo-about cached); result `Depends: libc6 (>= 2.35)`, i.e. 22.04's glibc
- [x] .deb: `packaging/deb/build.sh` (plain `dpkg-deb`; cargo-deb not needed), binary in `/usr/bin`, FFmpeg in `/usr/lib/chronoclip/` (rpath), desktop entry, icons, metainfo, licenses; depends on the X11/GL/PulseAudio/fontconfig/xkbcommon runtime packages, recommends the NVIDIA driver; installs and removes cleanly with apt
  - 8.5 MB. CI installs it on Ubuntu 22.04, 24.04 and 26.04, runs it (bundled FFmpeg loaded, all runtime libraries and `xdg-open` present, menu entry, icon, metainfo) and removes it without leftovers (2026-11-26). Ubuntu's minimized container images skip `/usr/share/doc`, so license files are checked in the package. Install on the maintainer's Mint machine and in-game test: see Verified
- [x] AppImage: same binary and FFmpeg plus the other non-system libraries, static AppImage runtime (no `libfuse2` needed on Ubuntu 24.04+); autostart points at the `.AppImage` file (`$APPIMAGE`), not the temporary mount
  - `packaging/appimage/build.sh` (appimagetool 1.9.1, type2-runtime 20251108, both pinned by SHA-256): 12 MB; only FFmpeg is bundled (the binary links just glibc, libgcc and fontconfig). On Mint: records and saves H.264 + AAC from its mount with the bundled FFmpeg; the autostart entry is repointed from the mount to `$APPIMAGE`. CI runs it on Ubuntu 22.04, Debian 12, Fedora, Arch and openSUSE
- [x] GitHub Releases: pushing a tag `v*` builds the .deb and the AppImage in CI and attaches them with SHA-256 checksums and release notes
  - `.github/workflows/release.yml` + `packaging/check-release.sh` + `CHANGELOG.md` + the `release` skill. Also attaches the FFmpeg source tarball. Dry run by hand (`gh workflow run release.yml -f version=0.1.0`) publishes nothing. Dry run skipped (maintainer). **v0.1.0 published 2026-11-26**: https://github.com/etse/chronoclip/releases/tag/v0.1.0 (all 11 jobs green; downloads match SHA256SUMS)
- [x] Docs: INSTALL.md user section (.deb and AppImage, NVIDIA requirement), README install section, architecture.md (packaging decisions), AGENTS.md (how to cut a release)
- [x] Verified: the .deb installs, records and saves on the maintainer's Mint machine (in-game) and removes cleanly; the AppImage records and saves on Mint; both start in Ubuntu 22.04 and 26.04 containers and the AppImage in a Fedora container in CI
  - .deb on the maintainer's Mint machine: "it works" (2026-11-26). AppImage (released v0.1.0) on Mint: "install is verified, its ok" (2026-11-26). AppImage runs on Ubuntu 22.04, Debian 12, Fedora, Arch and openSUSE in CI

Out of scope: Flatpak and AUR (dropped: the sandbox fights window capture, process detection and the global hotkey; Flathub would be the reason to revisit), AMD/Intel and Wayland (M9).

## M5: Polish

Goal: you notice every save, even in-game; you can see at a glance whether Chronoclip is recording; it starts by itself at login.

- [x] Save chime: a short chime generated in code (no sound file), played through the sound server (`libpulse`, already loaded) when a clip is saved; a different short tone on a failed save. Setting "Play a sound when saving", **on** by default
- [x] Desktop notification (`org.freedesktop.Notifications` via `notify-rust`, no system dependency): "Clip saved · 10:00 · 1.4 GB" with **Play** and **Show in folder** actions; save errors notify too. Cinnamon hides notifications over fullscreen windows by default, so the chime is the in-game feedback
- [x] Tray icon shows state: red while recording, grey while waiting for WoW, warning icon on errors; tooltip unchanged
- [x] Start with desktop: settings switch that writes/removes `~/.config/autostart/chronoclip.desktop` (`Exec=<binary> --hidden`); default off; reflects the file's actual state
- [x] WoW detection: already in place since M2/M4 (waits for `Wow.exe`, starts and restarts by itself); verified with Retail (`_forever_`); Classic not tested
- [x] Docs: README (features, settings), architecture.md (notifications, chime), INSTALL.md only if dependencies change
- [x] Verified in-game: chime audible in WoW (borderless) on save; notification after alt-tab; tray icon colors; autostart after a re-login
  - Maintainer: "it seems good" (2026-11-26)

Out of scope: clip library (dropped for now), trimming or editing clips, uploading or sharing, visual redesign (M7).

## M4: Audio

Goal: clips have the game's sound in sync with the video (track 1, plays everywhere), and optionally the microphone as a separate track 2. Audio is captured with PipeWire, encoded to AAC and written into the same 2 s segments as the video, so saving stays a remux.

- [x] Decide the PipeWire binding (spike): `pipewire` crate (needs `libpipewire-0.3-dev` + clang to build) vs. loading `libpipewire-0.3` at runtime vs. FFmpeg's `pulse` input via pipewire-pulse. Criteria: per-app capture, latency/timestamps, build and runtime dependencies (AGENTS rule 9). Record the choice in architecture.md (an ADR if it adds a build dependency). → PulseAudio API (`libpulse.so.0` at runtime, served by pipewire-pulse): per-app monitor streams, no build dependency
- [x] `chronoclip-capture`: `AudioSource` with two modes: **game only** captures the WoW (Wine/Proton) output stream, found by its process like the window; **all sound** captures the default output's monitor (game + Discord, music, …). Game only falls back to all sound when no WoW stream exists. Follows the stream/device when it is recreated (device change, game restart)
- [x] `chronoclip-capture`: microphone source (default input), only while the setting is on
- [x] `chronoclip-core`: audio timeline logic (unit tested): map capture timestamps to the video clock, insert silence for gaps (game silent, stream recreated) so the track stays continuous and in sync
- [x] `chronoclip-encode`: AAC encoder (FFmpeg's built-in `aac`, LGPL), 48 kHz stereo, ~160 kbit/s; segment writer muxes video + 1 or 2 audio streams into the MPEG-TS segments, cut on video keyframes
- [x] Recorder: audio thread(s) → bounded channel → writer, sharing one clock with video; audio starts and ends with each video session; a capture failure (no PipeWire, stream gone) logs, keeps the track going with silence and retries, instead of stopping
- [x] Remux: copy all streams; rebase every stream with the same offset at segment joins so audio stays in sync; mark track 1 as default ("Game") and track 2 as "Microphone"; still works for video-only segments
- [x] Settings/UI: audio setting "Game only" (default) / "All sound" / "Off", and the existing "Record microphone" (default off); changes apply with a recorder restart; audio status (capturing WoW / all sound / unavailable) in the window
- [x] Docs: architecture.md (audio path, decisions, measurements), INSTALL.md (PipeWire runtime/dev packages), README (features, configuration)
- [x] A clip only contains the current game: when the game is started again (its window reappears, or a new process), the previous game's recording is deleted (after a running save). Not when the game closes, so its end can still be saved afterwards
- [x] Clips never overlap: a save deletes the saved segments, so the next clip starts where the previous one ended (requested in-game; verified with three saves in a row: 16 s, 10 s, 2 s)
- [x] Settings save automatically (no Save button): lists and switches at once, the slider on release, text fields on Enter or leaving the field; unchanged values restart nothing (requested in-game: "All sound" was picked but never saved)
- [x] Verified with a test tone: A/V offset < 1 frame at the start and end of a 45 min clip and across a window resize; clip plays with sound in mpv, Firefox and Discord; CPU cost of audio measured
  - In-game finding: WoW (Proton via Faugus) is a native PipeWire client; through pipewire-pulse its stream has only `application.name = "World of Warcraft"`, so "game only" first fell back to all sound. Fixed by also matching the application name / window title; verified against running WoW (`finds_running_wow_stream`)
  - Done: 40 s and 10 min clips (no drift, offset within ffplay's own sync), window closed/reopened mid-clip, *All sound* + mic tracks, mpv playback, CPU (+1.2 % of one core); see architecture.md. Clips play with sound in Firefox and Discord (maintainer).
  - Long run (2026-11-26): 46.5 min recording with a resize at 22.5 min. As designed, the clip starts at the resize, so it was 24 min: median offset 0.0 ms (−30 to +10 ms, ±1 frame), no drift (first minute −3.4 ms, last −0.7 ms), no gaps, in sync right after the resize. Accepted instead of a separate 45 min run; the test sink runs on the system clock, so device drift was covered by the 10 min headset run
- [x] Verified in-game: "Game only" has WoW sound without Discord/music; "All sound" includes Discord; mic track with own voice when enabled
  - "Game only" records WoW sound and leaves out Discord and YouTube; "All sound" includes browser audio; the mic track (track 2) has finger snaps as clear peaks over a quiet background (maintainer, 2026-11-20). Players, browsers and Discord play only track 1

Out of scope: volume controls, mixing the mic into the game track, picking individual apps or devices (later, with M8), noise suppression.

## M3: Save replay

Goal: pressing the hotkey (default `Ctrl+F9`), clicking Save in the window or tray, or running `chronoclip save` writes the last N minutes as an MP4 to the output folder within seconds, by stream copy, while recording continues.

- [x] `chronoclip-core`: pick the segments for a save: the newest ones covering the buffer length, starting at the oldest segment that still exists (unit tested)
- [x] `chronoclip-core`: clip file naming (`Chronoclip_YYYY-MM-DD_HH-MM-SS.mp4`, no overwrites on collision) and free-space check against the snapshot size (unit tested)
- [x] Segment pinning: segments in a running save are not deleted by pruning until the save finishes; pruning catches up afterwards so disk use stays bounded
- [x] Include the moment of the press: the save waits for the in-progress segment to finish (≤ 2 s) before taking the snapshot
- [x] `chronoclip-encode`: remux MPEG-TS segments → one MP4 (stream copy, rebased continuous timestamps, `+faststart`), written to a temp file and renamed when complete
- [x] SaveService in `app/`: runs on its own thread, never blocks capture or the UI; one save at a time (a second request while saving is rejected with a message)
- [x] Controller: `save_replay` (UI button, tray, IPC) triggers the save; `chronoclip save` returns the clip path or the error
- [x] Global hotkey on X11 (XGrabKey, ignoring NumLock/CapsLock state) behind the `GlobalHotkey` backend switch (`app/src/hotkey.rs`, X11 now, portal later); re-registered when the setting changes; a grab failure (key taken by another app) is shown in the UI
- [x] Feedback: last saved clip and save errors in the UI status and tray tooltip; "Open folder" for the output folder (desktop notifications come in M5)
- [x] Docs: architecture.md (save path, decisions), INSTALL.md if dependencies change, README usage
- [x] Verified: glxgears save at 5 and 45 min buffers (45 min: 7.3 GB clip in 6.0 s); duration and frame count match, every join frame-exact, seeks and decodes cleanly in FFmpeg; save time measured (see architecture.md)
- [x] Verified in-game: hotkey works while WoW (borderless) has focus; clip looks good; game stayed smooth in light play (heavy content like raids not tested yet)

Out of scope: audio (M4), notifications and clip library (M5), Wayland GlobalShortcuts portal (M9; `chronoclip save` is the fallback there). Trimming or editing clips is not planned.

## M2: Capture pipeline (X11 + NVIDIA)

Goal: while the app runs, the WoW window is continuously encoded with NVENC into ~2 s MPEG-TS segments in the buffer folder, and segments older than the buffer length are deleted.

- [x] `chronoclip-core`: segment index with retention-based pruning (unit tested)
- [x] `chronoclip-encode`: NVENC H.264 encoder (CUDA frames), keyframe-aligned GOP, segment writer
- [x] `chronoclip-capture`: find the WoW window (Proton `Wow.exe`), XComposite + GLX texture-from-pixmap capture
- [x] GPU path: GL blit → CUDA–GL interop → CUDA frame, no CPU readback
- [x] Recorder: capture/encode thread → bounded channel → writer thread; frame pacing at the configured FPS
- [x] App: start/stop recorder, restart on settings change, status in UI and tray tooltip
- [x] Buffer folder cleaned on start and exit
- [x] Verified: glxgears at 2560×1368 @ 60 fps, segments decode and join into an MP4, overhead measured (see architecture.md)
- [x] Verified in-game: WoW (DX12, borderless windowed, 2560×1440 @ 60) via Faugus: game stays smooth, ~3 frames skipped, segments play in mpv. Uses copy mode (see architecture.md).

Out of scope: saving clips (M3), audio (M4), VAAPI/software encoders (M9). A window resize restarts the recorder at the new size.

## Later milestones

Details are added when a milestone starts. See the [README roadmap](../README.md#roadmap) for one-line summaries.
