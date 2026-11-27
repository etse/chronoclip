//! Window list for the picker (needs an X11 session).

#[test]
#[ignore = "needs X11"]
fn lists_open_windows() -> Result<(), Box<dyn std::error::Error>> {
    let windows = chronoclip_capture::list_windows()?;
    for w in &windows {
        let game = chronoclip_core::Game::from_window(&w.facts);
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
