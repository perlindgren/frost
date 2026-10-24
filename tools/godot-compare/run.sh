#!/usr/bin/env bash
# Drive tools/godot-compare, the Godot half of the lighting comparison in
# docs/benchmarking.md §18.
#
# Two reasons this exists as a script rather than a command line. Godot writes
# its state to ~/Library/Application Support/Godot and aborts — not complains,
# aborts — when that is unwritable, so HOME is pointed at a scratch directory
# and the engine never learns the difference. And the harness's own flags have
# to come after `--`, or Godot parses them as its own.
#
#   tools/godot-compare/run.sh --fullscreen             # the full ladder, ~3 min
#   tools/godot-compare/run.sh --fullscreen --quick     # twenty seconds
#   GODOT_BIN=/path/to/Godot tools/godot-compare/run.sh --fullscreen
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

godot="${GODOT_BIN:-}"
if [[ -z "$godot" ]]; then
  for candidate in "/Applications/Godot.app/Contents/MacOS/Godot" /Applications/Godot*.app/Contents/MacOS/Godot; do
    if [[ -x "$candidate" ]]; then
      godot="$candidate"
      break
    fi
  done
fi
if [[ -z "$godot" ]]; then
  echo "godot-compare: no Godot in /Applications — set GODOT_BIN to the binary" >&2
  exit 1
fi

# /tmp and not $TMPDIR: the per-user temp directory macOS hands a shell is not
# always writable from here, and Godot only needs somewhere to keep a shader
# cache.
scratch="/tmp/godot-compare-home"
mkdir -p "$scratch"
echo "godot-compare: engine $godot"
exec env HOME="$scratch" "$godot" --path "$here" -- "$@"
