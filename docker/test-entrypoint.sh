#!/bin/sh
# Regression test for docker/entrypoint.sh's build_index(): the served page
# always carries both @font-face rules (IosevkaTerm Nerd Font Mono, Regular
# and Bold — see the Dockerfile's font-fetch stage, SQ-1256), the audio
# script is added only when a browser audio port is given, the touch-scroll
# script (SQ-1262) is added unless LANTHORN_WEB_TOUCH=off, the font-ready
# script (SQ-1263) is always added and carries the same fontFamily/fontSize
# ttyd's own `-t` options get, and — either way — everything from ttyd's
# original page, both before and after </head>, survives the splice
# unchanged.
#
# Since SQ-1323 it also covers the browser-session machinery: that
# docker/web-session.js is always injected and always ahead of the audio script
# that reads the id it mints; that the id rule which turns that id into a FIFO,
# a socket and a stamp rejects everything it should; that the reaper's
# stale-session arithmetic holds against stamps written by hand; and that the
# real serve dispatch actually passes `--url-arg` and `--auto-save on` on to
# ttyd, which is the only place a correctly-resolved knob can still be dropped.
#
# SQ-1327 adds the touch grab zone the container seeds into the player's own
# config.toml, which is the one knob here that writes to a file somebody else
# owns — so every insertion point, every refusal and every idempotence case is
# checked.
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
cp "$repo_root/docker/web-session.js" "$fixture_dir/web-session.js"
cp "$repo_root/docker/web-audio.js" "$fixture_dir/web-audio.js"
cp "$repo_root/docker/web-touch.js" "$fixture_dir/web-touch.js"
cp "$repo_root/docker/web-font.js" "$fixture_dir/web-font.js"

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

# Same trick for the per-connection wrapper's id/socket derivation (SQ-1323):
# those are pure functions above their own marker, and the half below it runs
# dtach.
wrapper_marker="$(grep -n '^# --- end of function definitions; the wrapper.s own work begins below --- #$' "$repo_root/docker/serve-session.sh" | head -1 | cut -d: -f1)"
if [ -z "$wrapper_marker" ]; then
    echo "docker/test-entrypoint.sh: marker comment not found in serve-session.sh" >&2
    exit 1
fi
wrapper_funcs="$fixture_dir/wrapper_functions.sh"
head -n "$wrapper_marker" "$repo_root/docker/serve-session.sh" > "$wrapper_funcs"
# shellcheck source=/dev/null
. "$wrapper_funcs"
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

fixture_font_family="Fixture Font, IosevkaTerm Nerd Font Mono"
fixture_font_size="16"

# --- audio off ---
out1="$fixture_dir/out_noaudio.html"
build_index "" "$fixture_font_family" "$fixture_font_size" > "$out1"

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

grep -q 'function onTouchMove' "$out1"
check "no-audio: touch-scroll script is inlined even when audio is off" "$?"

grep -q "window.LANTHORN_WEB_FONT='$fixture_font_family';window.LANTHORN_WEB_FONT_SIZE=$fixture_font_size;" "$out1"
check "no-audio: injected window.LANTHORN_WEB_FONT/SIZE match what the page uses" "$?"

grep -q 'function remeasure' "$out1"
check "no-audio: font-ready script is inlined even when audio is off" "$?"

# --- audio on ---
out2="$fixture_dir/out_audio.html"
build_index "7682" "$fixture_font_family" "$fixture_font_size" > "$out2"

grep -q 'window.LANTHORN_WEB_AUDIO_PORT=7682;' "$out2"
check "audio: script carries the given port" "$?"

grep -q 'crates/audio-relay/src/lib.rs' "$out2"
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

grep -q 'function onTouchMove' "$out2"
check "audio: touch-scroll script is inlined alongside the audio script" "$?"

grep -q "window.LANTHORN_WEB_FONT='$fixture_font_family';window.LANTHORN_WEB_FONT_SIZE=$fixture_font_size;" "$out2"
check "audio: injected window.LANTHORN_WEB_FONT/SIZE match what the page uses" "$?"

grep -q 'function remeasure' "$out2"
check "audio: font-ready script is inlined alongside the audio script" "$?"

# --- touch off ---
out3="$fixture_dir/out_notouch.html"
LANTHORN_WEB_TOUCH=off
export LANTHORN_WEB_TOUCH
build_index "" "$fixture_font_family" "$fixture_font_size" > "$out3"
unset LANTHORN_WEB_TOUCH

grep -q 'function onTouchMove' "$out3"
[ "$?" != "0" ]
check "LANTHORN_WEB_TOUCH=off: touch-scroll script is not injected" "$?"

n="$(grep -c '@font-face' "$out3")"
[ "$n" = "2" ]
check "LANTHORN_WEB_TOUCH=off: still exactly two @font-face rules (got $n)" "$?"

grep -q '<div id="terminal"></div>' "$out3"
check "LANTHORN_WEB_TOUCH=off: body content after </head> survives" "$?"

grep -q 'function remeasure' "$out3"
check "LANTHORN_WEB_TOUCH=off: font-ready script is still injected" "$?"

grep -q "window.LANTHORN_WEB_FONT='$fixture_font_family';window.LANTHORN_WEB_FONT_SIZE=$fixture_font_size;" "$out3"
check "LANTHORN_WEB_TOUCH=off: injected window.LANTHORN_WEB_FONT/SIZE still match" "$?"

# --- the session id (SQ-1323) ---
#
# The page's session id is what makes a reconnect find the game it left, and it
# names a file at three ends — the audio FIFO, the dtach socket, the reaper's
# stamp. Everything below is the arithmetic that has to hold before any of that
# is safe.

grep -q 'lanthorn.session' "$out1"
check "session: web-session.js is injected even when audio is off" "$?"

grep -q 'lanthorn.session' "$out2"
check "session: web-session.js is injected when audio is on too" "$?"

# It has to run BEFORE web-audio.js: it owns the id that file reads, and it may
# reload the page before the audio socket is ever opened.
sess_line="$(grep -n 'lanthorn.session' "$out2" | head -1 | cut -d: -f1)"
audio_line="$(grep -n 'LANTHORN_WEB_AUDIO_PORT=7682' "$out2" | head -1 | cut -d: -f1)"
[ -n "$sess_line" ] && [ -n "$audio_line" ] && [ "$sess_line" -lt "$audio_line" ]
check "session: the session script is spliced ahead of the audio script" "$?"

sess_head_line="$(grep -n '</head>' "$out2" | head -1 | cut -d: -f1)"
[ -n "$sess_line" ] && [ -n "$sess_head_line" ] && [ "$sess_line" -lt "$sess_head_line" ]
check "session: the session script lands before </head>" "$?"

grep -q 'lanthorn.session' "$out3"
check "LANTHORN_WEB_TOUCH=off: the session script is still injected" "$?"

# valid_session_id: the rule the relay applies, restated where a path is built.
[ "$(valid_session_id 'abcdefgh12345678')" = "abcdefgh12345678" ]
check "id: a plain 16-character id is accepted" "$?"

[ "$(valid_session_id 'a-b_c-d_e')" = "a-b_c-d_e" ]
check "id: dashes and underscores are accepted" "$?"

[ -z "$(valid_session_id 'short')" ]
check "id: fewer than 8 characters is rejected" "$?"

[ -z "$(valid_session_id "$(printf 'x%.0s' 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64 65)")" ]
check "id: more than 64 characters is rejected" "$?"

[ -z "$(valid_session_id '../../etc/passwd')" ]
check "id: a path traversal is rejected before it can name a socket" "$?"

[ -z "$(valid_session_id 'has space here')" ]
check "id: a space is rejected" "$?"

[ -z "$(valid_session_id '')" ]
check "id: the empty string is rejected" "$?"

# session_socket: the derivation itself, which is what `dtach -A` is handed.
[ "$(session_socket /tmp/lanthorn-sessions 'abcdefgh12345678')" = "/tmp/lanthorn-sessions/abcdefgh12345678.sock" ]
check "socket: a good id becomes <dir>/<id>.sock" "$?"

[ -z "$(session_socket /tmp/lanthorn-sessions '../../etc/passwd')" ]
check "socket: a rejected id yields no path at all, so there is nothing to open" "$?"

[ -z "$(session_socket /tmp/lanthorn-sessions '')" ]
check "socket: no id yields no path" "$?"

# --- the reaper's arithmetic (SQ-1323) ---
#
# `stale_sessions` is a pure function of a directory and a clock precisely so it
# can be checked here, against stamps written by hand, with no dtach, no
# container and no waiting six hours.

sess_dir="$fixture_dir/sessions"
mkdir -p "$sess_dir"
now=1000000
ttl=21600                                  # the shipped six hours

printf '%s\n' "$now" > "$sess_dir/freshsession01.seen"
printf '%s\n' "$((now - ttl + 60))" > "$sess_dir/nearlystale01.seen"
printf '%s\n' "$((now - ttl))" > "$sess_dir/exactlyold01.seen"
printf '%s\n' "$((now - 999999))" > "$sess_dir/ancientone01.seen"
printf 'not-a-number\n' > "$sess_dir/corruptstamp1.seen"
: > "$sess_dir/emptystamp01.seen"
# Not a stamp at all: the socket and pid files live in the same directory and
# must not be mistaken for sessions.
: > "$sess_dir/freshsession01.pid"
: > "$sess_dir/freshsession01.sock"

stale="$(stale_sessions "$sess_dir" "$now" "$ttl" | sort | tr '\n' ' ')"
expected="ancientone01 corruptstamp1 emptystamp01 exactlyold01 "
[ "$stale" = "$expected" ]
check "reaper: names exactly the stale sessions (got '$stale', want '$expected')" "$?"

printf '%s' "$stale" | grep -q 'freshsession01'
[ "$?" != "0" ]
check "reaper: a session beating right now is never named" "$?"

printf '%s' "$stale" | grep -q 'nearlystale01'
[ "$?" != "0" ]
check "reaper: a session one minute short of the TTL is left alone" "$?"

printf '%s' "$stale" | grep -q 'exactlyold01'
check "reaper: a session exactly at the TTL is reaped (the boundary is inclusive)" "$?"

printf '%s' "$stale" | grep -q 'corruptstamp1'
check "reaper: an unreadable stamp counts as long ago, not as forever-fresh" "$?"

printf '%s' "$stale" | grep -q 'emptystamp01'
check "reaper: an empty stamp counts as long ago too" "$?"

n="$(stale_sessions "$sess_dir" "$now" "$ttl" | grep -c 'sock\|pid')"
[ "$n" = "0" ]
check "reaper: sockets and pid files in the same directory are not sessions (got $n)" "$?"

# An empty directory must produce nothing rather than the unmatched glob itself
# — `for s in dir/*.seen` iterates the literal pattern when nothing matches, and
# a reaper that then tried to kill a session called `*` would be a very bad day.
empty_dir="$fixture_dir/sessions-empty"
mkdir -p "$empty_dir"
n="$(stale_sessions "$empty_dir" "$now" "$ttl" | wc -l | tr -d ' ')"
[ "$n" = "0" ]
check "reaper: an empty session directory names nothing, not the unmatched glob (got $n)" "$?"

n="$(stale_sessions "$fixture_dir/no-such-dir" "$now" "$ttl" | wc -l | tr -d ' ')"
[ "$n" = "0" ]
check "reaper: a session directory that does not exist names nothing (got $n)" "$?"

# --- the touch grab zone seeded into config.toml (SQ-1327) ---
#
# `grab_zone_cells` has no command-line flag, so the container states its
# different default by writing the key into the player's own config.toml. That
# makes every one of these a check on somebody's file: a wrong insertion point
# silently moves a top-level key into a [section], and a value that is not a
# number makes the whole file unparseable — after which lanthorn refuses to save
# any setting at all until it is fixed by hand.

cfg_dir="$fixture_dir/cfghome"
mkdir -p "$cfg_dir"

# A file shaped like lanthorn's seeded template: commented defaults, then real
# section headers.
template_cfg() {
    cat > "$1" <<'CFG'
# lanthorn config
# volume = 30
# grab_zone_cells = 2

[map]
# icons = "nerd"
CFG
}

# 1. A fresh install: no file at all.
rm -f "$cfg_dir/fresh.toml"
seed_config_key "$cfg_dir/fresh.toml" grab_zone_cells 4
[ "$(cat "$cfg_dir/fresh.toml")" = "grab_zone_cells = 4" ]
check "grab zone: an absent config.toml is created holding just the key" "$?"

# 2. The template: the key exists only as a COMMENT, which is documentation, not
#    a decision — so the container may still state its default.
template_cfg "$cfg_dir/template.toml"
seed_config_key "$cfg_dir/template.toml" grab_zone_cells 4
grep -qx 'grab_zone_cells = 4' "$cfg_dir/template.toml"
check "grab zone: a commented default does not count as the player having chosen" "$?"

# …and it must land BEFORE the first [table], or it becomes a key inside it.
seed_line="$(grep -n '^grab_zone_cells = 4$' "$cfg_dir/template.toml" | head -1 | cut -d: -f1)"
table_line="$(grep -n '^\[map\]$' "$cfg_dir/template.toml" | head -1 | cut -d: -f1)"
[ -n "$seed_line" ] && [ -n "$table_line" ] && [ "$seed_line" -lt "$table_line" ]
check "grab zone: the key is inserted above the first [table], not appended into it" "$?"

grep -q '^# volume = 30$' "$cfg_dir/template.toml"
check "grab zone: the rest of the player's file survives untouched" "$?"

grep -q '^# icons = "nerd"$' "$cfg_dir/template.toml"
check "grab zone: content after the first [table] survives too" "$?"

[ ! -f "$cfg_dir/template.toml.lanthorn-seed" ]
check "grab zone: no temp file is left beside the config" "$?"

# 3. The player has already answered. Never clobber it.
printf 'grab_zone_cells = 6\n\n[map]\n' > "$cfg_dir/mine.toml"
seed_config_key "$cfg_dir/mine.toml" grab_zone_cells 4
grep -qx 'grab_zone_cells = 6' "$cfg_dir/mine.toml"
check "grab zone: a value the player set is left alone" "$?"

n="$(grep -c 'grab_zone_cells' "$cfg_dir/mine.toml")"
[ "$n" = "1" ]
check "grab zone: ...and no second copy of the key is added (got $n)" "$?"

# 4. Running twice must be the same as running once — the container restarts.
template_cfg "$cfg_dir/twice.toml"
seed_config_key "$cfg_dir/twice.toml" grab_zone_cells 4
seed_config_key "$cfg_dir/twice.toml" grab_zone_cells 4
n="$(grep -c '^grab_zone_cells' "$cfg_dir/twice.toml")"
[ "$n" = "1" ]
check "grab zone: seeding is idempotent across restarts (got $n)" "$?"

# 5. Leading whitespace still counts as set — TOML allows it.
printf '  grab_zone_cells = 3\n' > "$cfg_dir/indented.toml"
seed_config_key "$cfg_dir/indented.toml" grab_zone_cells 4
n="$(grep -c 'grab_zone_cells' "$cfg_dir/indented.toml")"
[ "$n" = "1" ]
check "grab zone: an indented key counts as set (got $n)" "$?"

# 6. A file with no [table] at all: appended is correct there.
printf 'volume = 30\n' > "$cfg_dir/flat.toml"
seed_config_key "$cfg_dir/flat.toml" grab_zone_cells 4
grep -qx 'grab_zone_cells = 4' "$cfg_dir/flat.toml"
check "grab zone: a config with no sections gets the key appended" "$?"

grep -qx 'volume = 30' "$cfg_dir/flat.toml"
check "grab zone: ...without disturbing what was there" "$?"

# --- the serve dispatch itself (SQ-1323) ---
#
# Everything above tests functions in isolation; this runs the real dispatch
# with stub `ttyd`, `lanthorn-audio-relay` and `dtach` on PATH and reads back
# the exact ttyd command line it built. It is the only check that can catch a
# knob that resolves correctly and is then never passed on — a missing
# `--url-arg` (the page's session id never reaches the wrapper, so nothing
# detaches and nothing has sound) or a missing `--auto-save on` (the container
# stops saving per turn, which is SQ-1323 all over again) both look exactly like
# a working image until somebody's connection drops.

stub_dir="$fixture_dir/bin"
mkdir -p "$stub_dir"

cat > "$stub_dir/ttyd" <<'STUB'
#!/bin/sh
# The entrypoint execs this last; record the whole command line and stop.
printf '%s\n' "$@" > "$TTYD_ARGS_OUT"
STUB

cat > "$stub_dir/lanthorn-audio-relay" <<'STUB'
#!/bin/sh
# `sink <path>` is the only form the entrypoint waits on: it polls for the FIFO
# to appear, so making one is the whole of what this has to do.
if [ "${1:-}" = "sink" ]; then
    mkfifo "$2" 2>/dev/null || true
fi
STUB

# dtach only has to EXIST for the wrapper to choose the detaching path; nothing
# here ever runs a session.
printf '#!/bin/sh\nexit 0\n' > "$stub_dir/dtach"
chmod +x "$stub_dir/ttyd" "$stub_dir/lanthorn-audio-relay" "$stub_dir/dtach"

TTYD_ARGS_OUT="$fixture_dir/ttyd_args.txt"
export TTYD_ARGS_OUT

# Run one dispatch with the current environment and leave its ttyd command line
# in $TTYD_ARGS_OUT, one argument per line. The sweeper the entrypoint starts is
# killed afterwards — `exec ttyd` orphans it, and a test must not leave a loop
# running on the developer's machine.
run_dispatch() {
    _sess="$fixture_dir/sessions-live"
    rm -rf "$_sess" "$fixture_dir/audio"
    mkdir -p "$_sess" "$fixture_dir/audio"
    rm -f "$TTYD_ARGS_OUT"
    (
        PATH="$stub_dir:$PATH"
        export PATH
        LANTHORN_AUDIO_DIR="$fixture_dir/audio"
        export LANTHORN_AUDIO_DIR
        LANTHORN_WEB_SESSION_DIR="$_sess"
        export LANTHORN_WEB_SESSION_DIR
        sh "$repo_root/docker/entrypoint.sh" serve /stories
    ) >/dev/null 2>&1
    if [ -f "$_sess/reaper.pid" ]; then
        kill "$(cat "$_sess/reaper.pid")" 2>/dev/null || true
    fi
}

# $1 = the argument to look for, exactly.
ttyd_has() {
    grep -qxF -- "$1" "$TTYD_ARGS_OUT"
}

run_dispatch

ttyd_has "--url-arg"
check "dispatch: ttyd is told to accept the page's session argument" "$?"

ttyd_has "--auto-save"
check "dispatch: the served game is launched with --auto-save" "$?"

ttyd_has "on"
check "dispatch: ...and the value is on" "$?"

ttyd_has "/usr/local/bin/lanthorn-serve-session"
check "dispatch: the game still runs through the per-connection wrapper" "$?"

ttyd_has "--writable"
check "dispatch: ttyd is still writable, or the game is unplayable" "$?"

[ -f "$fixture_dir/sessions-live/reaper.pid" ]
check "dispatch: the session sweeper is started and names itself" "$?"

# --- LANTHORN_WEB_AUTOSAVE=off ---
LANTHORN_WEB_AUTOSAVE=off
export LANTHORN_WEB_AUTOSAVE
run_dispatch
unset LANTHORN_WEB_AUTOSAVE

ttyd_has "--auto-save"
[ "$?" != "0" ]
check "LANTHORN_WEB_AUTOSAVE=off: the flag is not passed" "$?"

ttyd_has "--url-arg"
check "LANTHORN_WEB_AUTOSAVE=off: the session argument is still accepted" "$?"

# --- LANTHORN_WEB_DETACH=off with audio off: nothing wants a URL argument ---
LANTHORN_WEB_DETACH=off
LANTHORN_WEB_AUDIO=off
export LANTHORN_WEB_DETACH LANTHORN_WEB_AUDIO
run_dispatch

ttyd_has "--url-arg"
[ "$?" != "0" ]
check "detach+audio off: ttyd takes no arguments from a URL at all" "$?"

[ -f "$fixture_dir/sessions-live/reaper.pid" ]
[ "$?" != "0" ]
check "LANTHORN_WEB_DETACH=off: no session sweeper is started" "$?"

# --- LANTHORN_WEB_DETACH=on with audio off: the id is still needed ---
LANTHORN_WEB_DETACH=on
export LANTHORN_WEB_DETACH
run_dispatch
unset LANTHORN_WEB_DETACH LANTHORN_WEB_AUDIO

ttyd_has "--url-arg"
check "detach on, audio off: the session id is still passed through" "$?"

# --- the grab zone through a real dispatch (SQ-1327) ---
#
# run_dispatch points HOME at the fixture, so the seeded config.toml lands
# where the test can read it rather than in the developer's own home.
grab_home="$fixture_dir/grabhome"

run_grab_dispatch() {
    rm -rf "$grab_home"
    mkdir -p "$grab_home"
    ( HOME="$grab_home"; export HOME; run_dispatch )
}

run_grab_dispatch
grep -qx 'grab_zone_cells = 4' "$grab_home/.lanthorn/config.toml"
check "dispatch: serve mode seeds the wider touch grab zone by default" "$?"

LANTHORN_WEB_GRAB_ZONE=6
export LANTHORN_WEB_GRAB_ZONE
run_grab_dispatch
grep -qx 'grab_zone_cells = 6' "$grab_home/.lanthorn/config.toml"
check "LANTHORN_WEB_GRAB_ZONE=6: the given width is what is written" "$?"

LANTHORN_WEB_GRAB_ZONE=off
run_grab_dispatch
[ ! -f "$grab_home/.lanthorn/config.toml" ]
check "LANTHORN_WEB_GRAB_ZONE=off: nothing is written to the player's config" "$?"

# Anything that is not 1-6 must be REFUSED, not written: `grab_zone_cells =
# banana` is invalid TOML, and lanthorn then declines to save any setting at all
# until somebody edits the file by hand.
LANTHORN_WEB_GRAB_ZONE=banana
run_grab_dispatch
[ ! -f "$grab_home/.lanthorn/config.toml" ]
check "LANTHORN_WEB_GRAB_ZONE=banana: a bad value is ignored, never written" "$?"

LANTHORN_WEB_GRAB_ZONE=99
run_grab_dispatch
[ ! -f "$grab_home/.lanthorn/config.toml" ]
check "LANTHORN_WEB_GRAB_ZONE=99: out of range is ignored too" "$?"
unset LANTHORN_WEB_GRAB_ZONE

if [ "$fail" != "0" ]; then
    echo "docker/test-entrypoint.sh: FAILED" >&2
    exit 1
fi
echo "docker/test-entrypoint.sh: all checks passed"
