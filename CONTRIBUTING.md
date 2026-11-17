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

## Writing an ADR

Copy the format of [`docs/adr/0001-tech-stack.md`](docs/adr/0001-tech-stack.md), number it sequentially, and set the status to *Proposed*. It becomes *Accepted* when merged.

## Using AI coding agents

Agent instructions live in `AGENTS.md` (read by opencode, Codex, and others). `CLAUDE.md` imports it for Claude Code. Keep agent instructions in `AGENTS.md` only. You are responsible for reviewing and testing any agent-generated code you submit.
