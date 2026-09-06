#!/bin/sh
# What dtach actually starts inside a DETACHED lanthorn session (SQ-1323).
#
#   lanthorn-session-run <pid-file> <command> [args...]
#
# Run only when a session is CREATED — `dtach -A` executes nothing when it finds
# an existing socket to attach to — so everything here happens once per game,
# not once per connection.
#
# Two jobs, and both exist because dtach hands the program a terminal that is
# not yet a terminal:
#
#   1. RECORD THE PID. dtach daemonises its master and gives the caller no
#      handle on the session, so the reaper in docker/entrypoint.sh would have
#      nothing to end. `$$` here survives the `exec` below, so the file names
#      the GAME — which means an expired session is ended with a SIGTERM the app
#      answers by saving and exiting, not by being cut off mid-turn.
#
#   2. WAIT FOR A WINDOW SIZE. dtach's `init_pty` creates the session's pty with
#      no winsize at all — "We don't have to set the window size here, because
#      the attacher will send it in a packet" (master.c) — so for the first few
#      milliseconds `stty size` is `0 0`. A full-screen TUI that measures the
#      terminal in that window lays its whole first frame out on nothing. The
#      attacher's packet arrives almost at once, so this is a handful of 50 ms
#      naps in practice; the ceiling is there so a session can never fail to
#      start because a size never came.
set -eu

pid_file="$1"
shift
printf '%s\n' "$$" > "$pid_file"

i=0
while [ "$i" -lt 100 ]; do
    cols="$(stty size 2>/dev/null || printf '0 0')"
    cols="${cols##* }"
    case "$cols" in
        ''|*[!0-9]*) cols=0 ;;
    esac
    if [ "$cols" -gt 0 ]; then
        break
    fi
    sleep 0.05
    i=$((i+1))
done

exec "$@"
