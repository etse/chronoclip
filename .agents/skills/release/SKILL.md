---
name: release
description: Cut a Chronoclip release (vX.Y.Z on GitHub Releases with the .deb, AppImage, FFmpeg source and checksums), or rehearse one with a dry run. Use when the maintainer asks for a release, a version bump, or release notes.
---

# Releasing Chronoclip

A pushed tag `vX.Y.Z` runs `.github/workflows/release.yml`:

1. `packaging/check-release.sh X.Y.Z`: Cargo.toml, the AppStream metainfo and CHANGELOG.md must
   agree on the version and date; it writes the release notes from the CHANGELOG section.
2. `package.yml` (reused): builds the .deb and AppImage in Ubuntu 22.04, installs the .deb on
   Ubuntu 22.04/24.04/26.04, runs the AppImage on Ubuntu, Debian, Fedora, Arch and openSUSE.
3. Adds `ffmpeg-<ver>.tar.xz` (the LGPL source that `FFMPEG-LICENSE.txt` promises) and
   `SHA256SUMS`, then `gh release create`.

Tagging publishes to everyone: **always ask the maintainer before pushing a tag.**

## Steps

1. **Version** (semver; while 0.x: minor for features, patch for fixes). Set
   `[workspace.package] version` in `Cargo.toml`, then `cargo build` so `Cargo.lock` follows.
2. **CHANGELOG.md**: move the entries from `## [Unreleased]` into a new
   `## [X.Y.Z] - YYYY-MM-DD` section (written for users: what they get, not how). Leave an
   empty `## [Unreleased]` above it.
3. **Metainfo** (`packaging/linux/io.github.etse.chronoclip.metainfo.xml`): add
   `<release version="X.Y.Z" date="YYYY-MM-DD">` with a one-paragraph description as the
   *first* entry of `<releases>`; validate with `appstreamcli validate --no-net --pedantic`.
4. `packaging/check-release.sh X.Y.Z` must print OK.
5. Commit (`chore: release X.Y.Z`) and push when the maintainer says so; wait until CI and
   Package are green (`gh run list --commit $(git rev-parse HEAD)`).
6. **Dry run** (recommended for anything that changed packaging or the workflow):
   `gh workflow run release.yml -f version=X.Y.Z`, then
   `gh run watch $(gh run list --workflow Release -L1 --json databaseId -q '.[0].databaseId')`.
   Inspect with `gh run download <run-id> -n release-dry-run -D /tmp/rel` (notes, files,
   checksums). Nothing is published.
7. With the maintainer's go: `git tag -a vX.Y.Z -m "Chronoclip X.Y.Z" && git push origin vX.Y.Z`.
   Watch the Release run; afterwards `gh release view vX.Y.Z` must list the .deb, the AppImage,
   `ffmpeg-*.tar.xz` and `SHA256SUMS`.
8. Mark the roadmap/milestone task (milestone skill) and tell the maintainer the release URL.

## If something goes wrong

- The check job fails: fix the version/date/CHANGELOG mismatch it names, commit, and either
  move the tag (only if the release was **not** published: `git tag -d vX.Y.Z; git push --delete
  origin vX.Y.Z`, then tag again) or release X.Y.(Z+1).
- Never change or re-tag a published release: users and checksums depend on it. Publish a new
  patch version instead.
- Logs: `gh run view <run-id> --log-failed` (gh is logged in on the maintainer's machine).
