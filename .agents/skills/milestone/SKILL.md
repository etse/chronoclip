---
name: milestone
description: Plan, track and close Chronoclip milestones in docs/roadmap.md: writing a milestone's plan with the maintainer, ticking tasks with evidence, keeping README/architecture.md in sync, in-game verification, renumbering milestones, and commits. Use when starting or finishing a milestone, after finishing a roadmap task, or when the roadmap needs changing.
---

# Milestones

`docs/roadmap.md` is the source of truth for progress. Every new session starts with a summary
of it (AGENTS.md, "Session start"), so it must always be accurate.

## Roadmap format

- Top table: `| M<n> | <one-line scope> | ✅ / 🚧 / ⬜ |`. Exactly one milestone is 🚧 at a time.
- One section per started milestone, newest first, right below the table:

```markdown
## M<n>: <Name>

Goal: <what the maintainer gets, in one or two sentences>.

<Context that explains the decisions, if any.>

- [ ] <task: concrete, testable; name crates/files where useful>
- [x] <finished task>
  - <evidence or finding, e.g. measurements, in-game result with date, maintainer's words>
- [ ] Docs: README, architecture.md, INSTALL.md (if dependencies change), AGENTS.md (if commands/layout change)
- [ ] Verified: <automated checks>
- [ ] Verified in-game: <what the maintainer checks in WoW or other games>

Out of scope: <what is explicitly not part of it, and where it goes instead>.
```

- The README has a matching one-line roadmap list (`N. **M<n>, name** ✅ …`); keep it in sync.

## Starting a milestone

1. Check what already exists (code, docs, earlier milestones) before proposing tasks.
2. Propose a plan to the maintainer: a table of tasks with what they get and the size, your
   recommendation, and the open decisions as questions (multiple choice where possible).
3. Write the agreed plan into the roadmap (section + 🚧 in the table + README line), and
   ask before committing.

## While working

- Tick a task (`- [x]`) only when it is done **and** verified; add a sub-bullet with the
  evidence (numbers, test names, what was checked). Leave tasks that only need the
  maintainer's in-game check unticked and say so.
- A finding that changes the plan (e.g. "WoW's audio stream has no process id") goes in as a
  sub-bullet where it happened, then into `docs/architecture.md`.
- Every new dependency goes into INSTALL.md in the same change (AGENTS.md rule 9).

## Closing a milestone

1. All tasks ticked, including the in-game verification by the maintainer (quote them and
   date it, e.g. `Maintainer: "it seems good" (2026-11-26)`). If they accept a smaller test
   than planned, record what was done and why it was accepted.
2. `docs/architecture.md`: a "Decisions made in M<n>" section and measurements.
3. README: features moved from "Planned" to "Works today", configuration table, screenshot
   if the UI changed (readme-screenshot skill).
4. Table and README line ✅. Ask before committing.

## Renumbering or reordering milestones

References to milestone numbers are spread over docs **and code** (log messages, error
texts shown to users, comments). Map old → new, then find every reference:

```sh
git grep -n -i -E "\bM[0-9]\b|milestone [0-9]" -- . ':!Cargo.lock'
```

Update all of them, including "Out of scope" notes of finished milestones, and rebuild.

## Commits

Only commit when the maintainer asks. Conventional Commits (`feat:`, `fix:`, `docs:` …),
small and focused; roadmap updates are `docs:` commits. Check `git status --short` is empty
afterwards and report the hash.
