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
# AUDIO. Browser sound arrives over a FIFO named after the same session id, and
# since SQ-1328 that FIFO belongs to the SESSION rather than to the websocket:
# the relay creates it on the page's first audio socket and goes on reading it —
# draining at real time and discarding — while nobody is attached, so a detached
# game is never writing into a pipe nobody reads. Both modes therefore point
# LANTHORN_AUDIO_OUT at the session's own FIFO, and a game that drops its
# connection keeps its sound for whoever comes back to it.
#
# The wait below is only ever paid ONCE per game. A game's ALSA path is fixed
# when it opens the device and nothing here can re-point it afterwards, so a
# REATTACH — a dtach socket already exists for this id — has nothing to wait for
# and does not.
#
# No FIFO (a page without the script, audio switched off, a blocked socket)
# means LANTHORN_AUDIO_OUT points at the entrypoint's paced sink and the session
# is silent: as before, and never spinning a core on /dev/null.
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

# A player name, by lanthorn's own rule (crates/app/src/data_roots.rs,
# `validate_player_name`): 1 to 29 characters of [A-Za-z0-9._-], no leading `.`.
# Prints the name when it is usable and nothing when it is not.
valid_player_name() {
    case "${1:-}" in
        ''|.*|*[!A-Za-z0-9._-]*) return 0 ;;
    esac
    if [ "${#1}" -gt 29 ]; then
        return 0
    fi
    printf '%s' "$1"
}

# Where a player's sessions live under the session directory (SQ-1318): one
# directory per player, `_default` for the no-player case. A session is keyed by
# (player, id), so a pasted URL carrying somebody else's id starts a fresh game
# in the pasting player's own directory instead of attaching to theirs. Prints
# nothing for a name the rule above rejects. (`_default` is also a legal player
# name, but proxy mode, the only way a name arrives here, refuses a missing one,
# so the two never share a container.)
session_player_dir() {
    if [ -z "${2:-}" ]; then
        printf '%s/_default' "$1"
        return 0
    fi
    _p="$(valid_player_name "$2")"
    [ -n "$_p" ] || return 0
    printf '%s/%s' "$1" "$_p"
}

# Where a session's dtach socket lives, given the session directory, a
# candidate id and (optionally) the player. Prints nothing for an id or player
# the rules reject — which is what makes "no id" and "a bad id" the same case at
# every call site — and for a path too long for a unix socket (sun_path holds 104
# bytes on macOS and 108 on Linux, NUL included; the page's 16-character ids never get near
# it, a hand-written 64-character id under a long player name can).
session_socket() {
    _id="$(valid_session_id "${2:-}")"
    [ -n "$_id" ] || return 0
    _pd="$(session_player_dir "$1" "${3:-}")"
    [ -n "$_pd" ] || return 0
    _path="$_pd/$_id.sock"
    if [ "${#_path}" -gt 103 ]; then
        return 0
    fi
    printf '%s' "$_path"
}

# --- end of function definitions; the wrapper's own work begins below --- #

# PROXY MODE (SQ-1318): LANTHORN_WEB_AUTH_HEADER is set, so ttyd trusts that
# header from the reverse proxy in front of it and exports its value as
# TTYD_USER. The name becomes the player. Lanthorn authenticates nobody; the
# proxy did. Outside proxy mode TTYD_USER is never read — without the header
# switch ttyd never sets it, and a stray one in the environment is not a login.
proxy=""
if [ -n "${LANTHORN_WEB_AUTH_HEADER:-}" ]; then
    proxy="1"
fi

session=""
n=$#
i=0
skip=""
while [ "$i" -lt "$n" ]; do
    a="$1"
    shift
    i=$((i+1))
    if [ -n "$skip" ]; then
        skip=""
        continue
    fi
    case "$a" in
        --web-session=*) session="${a#--web-session=}" ;;
        # In proxy mode the player is the header's alone. `--player` outranks
        # LANTHORN_PLAYER inside lanthorn, so a `?arg=--player=amy` in the URL
        # would otherwise pick anybody's saves: drop it, and the value of the
        # two-word form with it.
        --player) if [ -n "$proxy" ]; then skip="1"; else set -- "$@" "$a"; fi ;;
        --player=*) if [ -z "$proxy" ]; then set -- "$@" "$a"; fi ;;
        *) set -- "$@" "$a" ;;
    esac
done
session="$(valid_session_id "$session")"

player=""
if [ -n "$proxy" ]; then
    player="$(valid_player_name "${TTYD_USER:-}")"
    if [ -z "$player" ]; then
        printf 'lanthorn: refusing this session: the authenticated user name %s is not a valid player name (1-29 characters of letters, digits, ".", "_" and "-", not starting with ".").\n' "'${TTYD_USER:-}'" >&2
        exit 1
    fi
    LANTHORN_PLAYER="$player"
    export LANTHORN_PLAYER
fi

audio_dir="${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}"
session_dir="${LANTHORN_WEB_SESSION_DIR:-/tmp/lanthorn-sessions}"

detach=""
if [ "${LANTHORN_WEB_DETACH:-on}" != "off" ] && [ -n "$session" ] && command -v dtach >/dev/null 2>&1; then
    detach="1"
fi

sock=""
if [ -n "$detach" ]; then
    sock="$(session_socket "$session_dir" "$session" "$player")"
    # An id or player whose socket path would not fit: play without detaching
    # rather than hand dtach a path it cannot bind.
    if [ -z "$sock" ]; then
        detach=""
    fi
fi

# Is this connection joining a game that is already running? Then its ALSA path
# was settled when that game started, and the wait below would buy nothing.
attaching=""
if [ -n "$sock" ] && [ -S "$sock" ]; then
    attaching="1"
fi

if [ -n "$session" ] && [ "${LANTHORN_WEB_AUDIO:-on}" != "off" ]; then
    # Keyed like the socket, <audio dir>/<player|_default>/<id>.pcm, which is
    # where the relay (docker/entrypoint.sh starts it with the same header
    # setting) creates it.
    fifo="$audio_dir/${player:-_default}/$session.pcm"
    if [ -z "$attaching" ]; then
        # The page opens the audio socket before the terminal one, but the two
        # handshakes race; give the relay up to two seconds to create the FIFO.
        tries=0
        while [ ! -p "$fifo" ] && [ "$tries" -lt 20 ]; do
            sleep 0.1
            tries=$((tries+1))
        done
    fi
    # Named even on a reattach, where it costs nothing and covers the one case
    # `-S` reads wrong: a socket left behind by a master that died without
    # unlinking it, which `dtach -A` answers by starting a NEW game.
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

pdir="$(session_player_dir "$session_dir" "$player")"
mkdir -p "$pdir"
seen="$pdir/$session.seen"
pid_file="$pdir/$session.pid"

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
