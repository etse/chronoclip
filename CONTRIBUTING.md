# Contributing to Chronoclip

Thanks for your interest! Chronoclip is early, so design discussion is just as valuable as code.

## Ground rules

- Read [AGENTS.md](AGENTS.md). Its hard rules and conventions apply to humans too.
- **Never** add code that injects into, hooks, or reads memory from the game process.
- Bigger changes (new dependency, new platform backend, architecture change) should start as an issue or an ADR in [`docs/adr/`](docs/adr/).

## Workflow

1. Fork and create a branch: `feat/short-description` or `fix/short-description`.
2. Make focused changes with tests where practical.
3. Run format, lint, and tests (see the *Commands* section in AGENTS.md).
4. Use [Conventional Commits](https://www.conventionalcommits.org/) for commit messages.
5. Open a pull request and fill in the template.

## Add a game to the catalog

Chronoclip recognizes the games in [`crates/chronoclip-core/data/games.toml`](crates/chronoclip-core/data/games.toml)
by itself. Missing one? A pull request that adds it is very welcome, and you don't need to know Rust:

1. Add a `[[game]]` entry, sorted by name:

   ```toml
   [[game]]
   # Steam store page 1245620; Game/eldenring.exe in the Steam install.
   name = "ELDEN RING"
   steam_app_ids = [1245620]
   executables = ["eldenring.exe"]
   ```

   - `name`: the store's spelling. Clip folders are named after it.
   - `steam_app_ids`: the number in the game's Steam store link (`store.steampowered.com/app/<id>`).
     Best choice for Steam games.
   - `executables`: the file name of the program that opens the game window, as found in the
     install folder. Not a launcher or a small starter program that only starts the real one.
   - `title_contains`: only if nothing else works.
   - A comment saying where the id or file name comes from (your install, the Steam store,
     PCGamingWiki). Please don't guess: a wrong name means the game is never found.
2. Run `cargo test -p chronoclip-core catalog` (CI runs it too). It checks that every entry can
   match something and that no executable or Steam id is used by two games.

The catalog is compiled into the app. People who use Chronoclip add their own games in the app
(Games tab → *Add a game window…*), not in this file.

## Writing an ADR

Copy the format of [`docs/adr/0001-tech-stack.md`](docs/adr/0001-tech-stack.md), number it sequentially, and set the status to *Proposed*. It becomes *Accepted* when merged.

## Using AI coding agents

Agent instructions live in `AGENTS.md` (read by opencode, Codex, and others). `CLAUDE.md` imports it for Claude Code. Keep agent instructions in `AGENTS.md` only. You are responsible for reviewing and testing any agent-generated code you submit.
