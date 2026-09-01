#!/bin/sh
# One browser connection. ttyd runs this with the configured lanthorn command
# line plus whatever the page appended through `?arg=` (ttyd --url-arg). The
# page appends `--web-audio=<id>` once it has opened ws://…/audio/<id>, which
# is when lanthorn-audio-relay creates the session's FIFO. Strip that argument,
# point ALSA at the FIFO when it is there, and exec lanthorn with the rest.
#
# No FIFO (a page without the script, audio switched off, a blocked socket)
# means LANTHORN_AUDIO_OUT stays unset and ALSA writes to /dev/null: silent,
# as before.
set -eu
session=""
n=$#
i=0
while [ "$i" -lt "$n" ]; do
  a="$1"
  shift
  i=$((i+1))
  case "$a" in
    --web-audio=*) session="${a#--web-audio=}" ;;
    *) set -- "$@" "$a" ;;
  esac
done
# The id names a file: the same rule the relay applies (8 to 64 of [A-Za-z0-9_-]).
case "$session" in
  *[!A-Za-z0-9_-]*) session="" ;;
esac
if [ "${#session}" -lt 8 ] || [ "${#session}" -gt 64 ]; then
  session=""
fi
if [ -n "$session" ]; then
  fifo="${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}/$session.pcm"
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
if [ -z "${LANTHORN_AUDIO_OUT:-}" ] && [ -p "${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}/null.pcm" ]; then
  export LANTHORN_AUDIO_OUT="${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}/null.pcm"
fi
exec "$@"
