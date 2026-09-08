#!/usr/bin/env bash
# Regenerate the save-format interop golden files and run the live interop suite.
# Developer-run (NOT CI). See docs/superpowers/specs/2026-07-08-save-interop-testing-design.md
#
# Requires: dfrotz  (brew install frotz)
#
# Z-machine only. Glulx interop is deferred to SQ-0229 (homebrew glulxe is curses-only
# and not headless-scriptable; a sound fixture needs Inform 6 + library).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

command -v dfrotz >/dev/null || { echo "dfrotz not found — run: brew install frotz" >&2; exit 1; }

# Point P: open mailbox -> take leaflet -> north  (North of House, leaflet carried).
STORY="crates/zvm/tests/fixtures/minizork.z3"
GOLD="crates/zvm/tests/fixtures/interop/minizork-at-P.qzl"
mkdir -p "$(dirname "$GOLD")"
rm -f "$GOLD"
printf 'open mailbox\ntake leaflet\nnorth\nsave\n%s\nquit\ny\n' "$GOLD" | dfrotz -m "$STORY" >/dev/null
[ -s "$GOLD" ] || { echo "FAILED to write $GOLD" >&2; exit 1; }
echo "wrote $GOLD ($(wc -c < "$GOLD") bytes)"

# Point P (SQ-1421): east -> take scarf  (Servant's Room, scarf carried).
# Leading blank line dismisses Curses' own "[Please press SPACE to begin.]"
# splash screen -- without it, dfrotz's dumb interface (which reads a whole
# LINE even to satisfy a single-key prompt) consumes "east" as that keypress
# and discards the rest of the line, desyncing every command after it.
STORY2="crates/zvm/tests/fixtures/curses.z5"
GOLD2="crates/zvm/tests/fixtures/interop/curses-at-P.qzl"
rm -f "$GOLD2"
printf '\neast\ntake scarf\nsave\n%s\nquit\ny\n' "$GOLD2" | dfrotz -m "$STORY2" >/dev/null
[ -s "$GOLD2" ] || { echo "FAILED to write $GOLD2" >&2; exit 1; }
echo "wrote $GOLD2 ($(wc -c < "$GOLD2") bytes)"

# Run the live interop tests, which need dfrotz at test time (they skip
# vacuously without it — SQ-1421 replaced the earlier `#[ignore]` pair with a
# runtime `dfrotz` resolution, so no `-- --ignored` flag is needed any more).
echo "running live interop tests (cargo test -p lanthorn-zvm --test save_interop) ..."
cargo test -p lanthorn-zvm --test save_interop
