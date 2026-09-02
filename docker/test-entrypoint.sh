#!/bin/sh
# Regression test for docker/entrypoint.sh's build_index(): the served page
# always carries both @font-face rules (IosevkaTerm Nerd Font Mono, Regular
# and Bold — see the Dockerfile's font-fetch stage, SQ-1256), the audio
# script is added only when a browser audio port is given, and — either way —
# everything from ttyd's original page, both before and after </head>,
# survives the splice unchanged.
#
# Self-contained: builds a fixture directory instead of touching the image's
# real /usr/local/share/lanthorn (LANTHORN_SHARE_DIR overrides it), so this
# needs no Docker build. Run it with:
#   sh docker/test-entrypoint.sh
#
# Runs under GNU coreutils (the debian:trixie-slim runtime image) and under
# macOS's BSD tools alike — entrypoint.sh avoids GNU-only flags for that.
set -u

here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
repo_root="$(CDPATH= cd -- "$here/.." && pwd)"

fixture_dir="$(mktemp -d)"
trap 'rm -rf "$fixture_dir"' EXIT

mkdir -p "$fixture_dir/fonts"
cp "$repo_root/docker/web-audio.js" "$fixture_dir/web-audio.js"

cat > "$fixture_dir/ttyd-index.html" <<'HTML'
<!DOCTYPE html><html><head><meta charset="utf-8"><title>ttyd</title></head><body><div id="terminal"></div></body></html>
HTML

# Fixture "fonts" — not real font bytes (those are fetched and SHA-256-
# verified at image build time; see the Dockerfile's font-fetch stage). Only
# their exact byte-for-byte round trip through the data: URI matters here, so
# any deterministic binary content proves it, including bytes that are not
# valid UTF-8 — real woff2 output isn't either.
printf 'REGULAR-WOFF2-FIXTURE-\000\001\002\376\377-BYTES' > "$fixture_dir/fonts/IosevkaTermNerdFontMono-Regular.woff2"
printf 'BOLD-WOFF2-FIXTURE-\000\001\002\376\377-BYTES' > "$fixture_dir/fonts/IosevkaTermNerdFontMono-Bold.woff2"

LANTHORN_SHARE_DIR="$fixture_dir"
export LANTHORN_SHARE_DIR

# Pull in start_sink()/build_index() (and the LANTHORN_SHARE_DIR default)
# without running entrypoint.sh's own dispatch logic at the bottom.
marker_line="$(grep -n '^# --- end of function definitions; dispatch begins below --- #$' "$repo_root/docker/entrypoint.sh" | head -1 | cut -d: -f1)"
if [ -z "$marker_line" ]; then
    echo "docker/test-entrypoint.sh: marker comment not found in entrypoint.sh" >&2
    exit 1
fi
funcs="$fixture_dir/entrypoint_functions.sh"
head -n "$marker_line" "$repo_root/docker/entrypoint.sh" > "$funcs"
# shellcheck source=/dev/null
. "$funcs"
# entrypoint.sh's own `set -eu` just carried into this shell via the source
# above — drop the -e half so a `grep -q` that legitimately finds nothing
# doesn't abort this test script before it gets to report that as a pass.
set +e

fail=0
check() {
    if [ "$2" != "0" ]; then
        echo "FAIL: $1"
        fail=1
    else
        echo "PASS: $1"
    fi
}

# $1 = html file, $2 = weight (400 or 700); prints the raw base64 payload of
# that face's @font-face src, or nothing if the weight isn't present.
extract_b64() {
    awk -F 'base64,' -v w="font-weight:$2;" '
        index($0, w) { n = split($2, a, ")"); print a[1]; exit }
    ' "$1"
}

# --- audio off ---
out1="$fixture_dir/out_noaudio.html"
build_index "" > "$out1"

n="$(grep -c '@font-face' "$out1")"
[ "$n" = "2" ]
check "no-audio: exactly two @font-face rules (got $n)" "$?"

grep -q 'LANTHORN_WEB_AUDIO_PORT' "$out1"
[ "$?" != "0" ]
check "no-audio: no audio script injected" "$?"

grep -q '<title>ttyd</title>' "$out1"
check "no-audio: head content before the splice survives" "$?"

grep -q '<div id="terminal"></div>' "$out1"
check "no-audio: body content after </head> survives" "$?"

extract_b64 "$out1" 400 | base64 -d > "$fixture_dir/decoded_regular_1.bin"
cmp -s "$fixture_dir/decoded_regular_1.bin" "$fixture_dir/fonts/IosevkaTermNerdFontMono-Regular.woff2"
check "no-audio: Regular (400) data: URI decodes to exact original bytes" "$?"

extract_b64 "$out1" 700 | base64 -d > "$fixture_dir/decoded_bold_1.bin"
cmp -s "$fixture_dir/decoded_bold_1.bin" "$fixture_dir/fonts/IosevkaTermNerdFontMono-Bold.woff2"
check "no-audio: Bold (700) data: URI decodes to exact original bytes" "$?"

# --- audio on ---
out2="$fixture_dir/out_audio.html"
build_index "7682" > "$out2"

grep -q 'window.LANTHORN_WEB_AUDIO_PORT=7682;' "$out2"
check "audio: script carries the given port" "$?"

grep -q 'TAG = "--web-audio="' "$out2"
check "audio: web-audio.js content is actually inlined, not just referenced" "$?"

n="$(grep -c '@font-face' "$out2")"
[ "$n" = "2" ]
check "audio: still exactly two @font-face rules (got $n)" "$?"

grep -q '<title>ttyd</title>' "$out2"
check "audio: head content before the splice survives" "$?"

grep -q '<div id="terminal"></div>' "$out2"
check "audio: body content after </head> survives" "$?"

extract_b64 "$out2" 400 | base64 -d > "$fixture_dir/decoded_regular_2.bin"
cmp -s "$fixture_dir/decoded_regular_2.bin" "$fixture_dir/fonts/IosevkaTermNerdFontMono-Regular.woff2"
check "audio: Regular (400) data: URI still decodes to exact original bytes" "$?"

# <script> must land before </head>, and before the font <style> is fine
# either order, but it must not land after </head>.
script_line="$(grep -n '<script>' "$out2" | head -1 | cut -d: -f1)"
head_line="$(grep -n '</head>' "$out2" | head -1 | cut -d: -f1)"
[ -n "$script_line" ] && [ -n "$head_line" ] && [ "$script_line" -lt "$head_line" ]
check "audio: <script> lands before </head>" "$?"

if [ "$fail" != "0" ]; then
    echo "docker/test-entrypoint.sh: FAILED" >&2
    exit 1
fi
echo "docker/test-entrypoint.sh: all checks passed"
