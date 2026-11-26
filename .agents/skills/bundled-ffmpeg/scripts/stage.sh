#!/usr/bin/env bash
# Lay out an installed-style copy (usr/bin + usr/lib/chronoclip) built against the bundled FFmpeg.
# Thin wrapper around packaging/stage.sh, which the package builds use too.
exec "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)/packaging/stage.sh" "$@"
