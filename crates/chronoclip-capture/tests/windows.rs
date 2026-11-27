//! Window list for the picker (needs an X11 session).

#[test]
#[ignore = "needs X11"]
fn lists_open_windows() -> Result<(), Box<dyn std::error::Error>> {
    let windows = chronoclip_capture::list_windows()?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let data_home = chronoclip_core::paths::data_dirs(&home).remove(0);
    let launchers =
        chronoclip_core::launchers::Launchers::load(&home, &data_home, &home.join(".config"));
    for w in &windows {
        let game = chronoclip_core::Game::from_window(&w.facts, launchers.name_for(&w.facts));
        eprintln!(
            "{:?} {:?} → {:?} [{}]",
            w.facts.title,
            w.facts.class,
            game.name,
            game.describe()
        );
    }
    assert!(!windows.is_empty(), "no windows found");
    assert!(windows.iter().all(|w| !w.facts.title.trim().is_empty()));
    Ok(())
}

/// Steam names from this computer's real manifests (needs Steam with games installed).
#[test]
#[ignore = "needs Steam with installed games"]
fn steam_names_on_this_computer() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let data_home = chronoclip_core::paths::data_dirs(&home).remove(0);
    let launchers =
        chronoclip_core::launchers::Launchers::load(&home, &data_home, &home.join(".config"));
    for id in [1_245_620, 526_870, 1_623_730, 2_694_490, 1_493_710] {
        eprintln!("{id}: {:?}", launchers.steam_name(id));
    }
    assert!(
        launchers.steam_name(1_245_620).is_some(),
        "ELDEN RING not found"
    );
}

/// Installed games on this computer (Steam, Wine prefixes), and how long the scan takes.
#[test]
#[ignore = "lists the games installed on this computer"]
fn installed_games_on_this_computer() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let data_home = chronoclip_core::paths::data_dirs(&home).remove(0);
    let config_home = home.join(".config");
    let started = std::time::Instant::now();
    let launchers = chronoclip_core::launchers::Launchers::load(&home, &data_home, &config_home);
    let found =
        chronoclip_core::installed::find_installed(&launchers, &home, &data_home, &config_home);
    eprintln!("{} games in {:?}", found.len(), started.elapsed());
    for f in &found {
        eprintln!("  {} [{}] ({})", f.game.name, f.game.describe(), f.source);
    }
}
