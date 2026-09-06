#!/bin/sh
# One browser connection. ttyd runs this with the configured lanthorn command
# line plus whatever the page appended through `?arg=` (ttyd --url-arg). The
# page appends `--web-session=<id>` (docker/web-session.js, which keeps the id
# in localStorage so it survives a reload), and that one string decides two
# things: which FIFO the browser's audio arrives on, and — since SQ-1323 —
# which RUNNING GAME a reconnect belongs to.
#
# Two ways to run the game, chosen by LANTHORN_WEB_DETACH:
#
#   on (default)  the game runs inside a dtach session named after the id, so
#                 it OUTLIVES the websocket. ttyd's hang-up ends this wrapper
#                 and the dtach client; the master ignores SIGHUP and has
#                 setsid()'d out of the process group ttyd signals, so the game
#                 plays on, and the next connection with the same id attaches to
#                 it mid-sentence.
#   off           exactly what happened before: one lanthorn per websocket, and
#                 a dropped connection ends the game (which still auto-saves —
#                 see --auto-save in docker/entrypoint.sh).
#
# AUDIO, AND THE TRADE. Browser sound arrives over a FIFO the relay creates when
# the audio socket opens and UNLINKS when it closes (crates/audio-relay), so it
# is a property of the CONNECTION. A detached game outlives its connection, and
# its ALSA output was bound to that FIFO at the first sound and cannot be
# re-pointed afterwards; writing into a pipe with no reader is how an audio
# thread wedges. So a detachable session plays into the entrypoint's paced sink
# instead: silent in the browser, and never stuck. `LANTHORN_WEB_DETACH=off`
# takes the other side of the trade — sound, but a dropped connection ends the
# game.
#
# No FIFO and no detach (a page without the script, audio switched off, a
# blocked socket) means LANTHORN_AUDIO_OUT points at the paced sink and the
# session is silent: as before.
set -eu

# --- functions (docker/test-entrypoint.sh sources everything above the marker
# --- at the bottom of this block, so keep them free of side effects) --- #

# The id names a FILE at three ends — the audio FIFO, the dtach socket, the
# session's stamp — so it is validated before it is ever spliced into a path.
# The rule is the relay's: 8 to 64 characters of [A-Za-z0-9_-] and nothing else.
# Prints the id when it is usable and nothing when it is not, so a caller can
# write `id="$(valid_session_id "$candidate")"` and test for empty.
valid_session_id() {
    case "${1:-}" in
        ''|*[!A-Za-z0-9_-]*) return 0 ;;
    esac
    if [ "${#1}" -lt 8 ] || [ "${#1}" -gt 64 ]; then
        return 0
    fi
    printf '%s' "$1"
}

# Where a session's dtach socket lives, given the session directory and a
# candidate id. Prints nothing for an id the rule above rejects — which is what
# makes "no id" and "a bad id" the same case at every call site.
session_socket() {
    _id="$(valid_session_id "${2:-}")"
    [ -n "$_id" ] || return 0
    printf '%s/%s.sock' "$1" "$_id"
}

# --- end of function definitions; the wrapper's own work begins below --- #

session=""
n=$#
i=0
while [ "$i" -lt "$n" ]; do
    a="$1"
    shift
    i=$((i+1))
    case "$a" in
        --web-session=*) session="${a#--web-session=}" ;;
        *) set -- "$@" "$a" ;;
    esac
done
session="$(valid_session_id "$session")"

audio_dir="${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}"
session_dir="${LANTHORN_WEB_SESSION_DIR:-/tmp/lanthorn-sessions}"

detach=""
if [ "${LANTHORN_WEB_DETACH:-on}" != "off" ] && [ -n "$session" ] && command -v dtach >/dev/null 2>&1; then
    detach="1"
fi

if [ -z "$detach" ] && [ -n "$session" ]; then
    fifo="$audio_dir/$session.pcm"
    # The page opens the audio socket before the terminal one, but the two
    # handshakes race; give the relay up to two seconds to create the FIFO.
    tries=0
    while [ ! -p "$fifo" ] && [ "$tries" -lt 20 ]; do
        sleep 0.1
        tries=$((tries+1))
    done
    if [ -p "$fifo" ]; then
        export LANTHORN_AUDIO_OUT="$fifo"
    fi
fi

# No session FIFO: write to the paced sink the entrypoint runs, never to
# /dev/null, which has no clock and lets the audio thread spin a core.
if [ -z "${LANTHORN_AUDIO_OUT:-}" ] && [ -p "$audio_dir/null.pcm" ]; then
    export LANTHORN_AUDIO_OUT="$audio_dir/null.pcm"
fi

if [ -z "$detach" ]; then
    exec "$@"
fi

mkdir -p "$session_dir"
sock="$(session_socket "$session_dir" "$session")"
seen="$session_dir/$session.seen"
pid_file="$session_dir/$session.pid"

# The heartbeat is what tells the reaper this session still has somebody in it.
# Stamping only at attach and detach would have read a player eight hours into
# Trinity as an abandoned session; stamping on a timer means the reaper's whole
# rule is "the stamp is older than the TTL", and a wrapper that dies without
# running its trap simply stops beating and ages out correctly.
printf '%s\n' "$(date +%s)" > "$seen"
(
    while :; do
        sleep "${LANTHORN_WEB_SESSION_BEAT:-30}"
        printf '%s\n' "$(date +%s)" > "$seen" 2>/dev/null || exit 0
    done
) &
beat=$!
# ttyd signals the whole process group on a websocket close (uv_kill(-pid,
# SIGHUP), src/pty.c), so this shell gets the hang-up too — and the EXIT trap is
# how the session's last-seen stamp ends up honest rather than up to 30 seconds
# stale at the exact moment it starts to matter.
trap 'printf "%s\n" "$(date +%s)" > "$seen" 2>/dev/null || true; kill "$beat" 2>/dev/null || true' EXIT HUP INT TERM

# -A: attach to the session if the game is already running, else start it.
# -r winch: a reattach asks for a repaint with SIGWINCH. dtach's default is
#     ctrl_l, which types a key AT THE GAME — the parser would see it.
# -E: no detach character. ^\ belongs to the game, and the way out of a
#     lanthorn session is quitting it, not orphaning it.
# -z: the suspend key likewise goes to the game rather than to dtach.
# lanthorn-session-run records the game's pid for the reaper and holds it until
# the attacher has sent a real window size (see docker/session-run.sh).
dtach -A "$sock" -E -z -r winch \
    /usr/local/bin/lanthorn-session-run "$pid_file" "$@"
