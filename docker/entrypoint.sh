#!/bin/sh
# Container entrypoint: dispatch between direct-TUI mode and web-serve mode.
#
#   <no args> / <lanthorn args>   exec lanthorn directly (needs `docker run -it`)
#   serve [lanthorn args...]      exec ttyd wrapping lanthorn — each browser
#                                 connection gets its own lanthorn process
#
# serve-mode knobs (environment):
#   LANTHORN_WEB_PORT         port ttyd listens on (default 7681)
#   LANTHORN_WEB_CREDENTIAL   basic-auth as user:pass (default: no auth —
#                             do not expose an unauthenticated port publicly)
#   LANTHORN_WEB_AUDIO        on (default) or off: sound in the browser, via
#                             lanthorn-audio-relay on its own port
#   LANTHORN_WEB_AUDIO_PORT   that port (default 7682)
#   LANTHORN_WEB_IMAGES       sixel (default) or halfblocks: how pictures are
#                             sent to the browser. ttyd's xterm.js can render
#                             sixel, so covers and v6 art show as real images;
#                             lanthorn's auto-detection cannot see that.
set -eu

# A FIFO drained at real time, for any session with no browser listening.
# ALSA writing to /dev/null has no clock and spins a core; this is the clock.
start_sink() {
    export LANTHORN_AUDIO_DIR="${LANTHORN_AUDIO_DIR:-/tmp/lanthorn-audio}"
    mkdir -p "$LANTHORN_AUDIO_DIR"
    lanthorn-audio-relay sink "$LANTHORN_AUDIO_DIR/null.pcm" &
    tries=0
    while [ ! -p "$LANTHORN_AUDIO_DIR/null.pcm" ] && [ "$tries" -lt 20 ]; do
        sleep 0.1
        tries=$((tries+1))
    done
}

# ttyd's own page with docker/web-audio.js in its <head>, told which port the
# relay listens on. Generated per start so the port can be an env knob.
build_index() {
    awk -v f=/usr/local/share/lanthorn/web-audio.js -v port="$1" '
        BEGIN { while ((getline l < f) > 0) js = js l "\n" }
        {
            i = index($0, "</head>")
            if (i && !done) {
                print substr($0, 1, i - 1) "<script>window.LANTHORN_WEB_AUDIO_PORT=" port ";\n" js "</script>" substr($0, i)
                done = 1
            } else {
                print
            }
        }' /usr/local/share/lanthorn/ttyd-index.html
}

if [ "${1:-}" = "serve" ]; then
    shift
    # No story args after `serve` means the picker on the library mount.
    [ "$#" -gt 0 ] || set -- /stories

    # Each connection runs through the session wrapper, which strips the
    # page's audio argument and points ALSA at the session's FIFO. Pictures are
    # sent as sixel unless configured otherwise: xterm.js renders it once the
    # page addon is on (-t enableSixel below), and lanthorn's auto-detection
    # cannot learn that from xterm.js.
    images="${LANTHORN_WEB_IMAGES:-sixel}"
    set -- /usr/local/bin/lanthorn-serve-session lanthorn --image-protocol "$images" "$@"
    start_sink
    if [ "${LANTHORN_WEB_AUDIO:-on}" != "off" ]; then
        audio_port="${LANTHORN_WEB_AUDIO_PORT:-7682}"
        LANTHORN_WEB_AUDIO_BIND="0.0.0.0:$audio_port" lanthorn-audio-relay &
        build_index "$audio_port" > /tmp/lanthorn-index.html
        # --url-arg lets the page pass the session id; --index serves the
        # page that does so.
        set -- --url-arg --index /tmp/lanthorn-index.html "$@"
    fi
    if [ -n "${LANTHORN_WEB_CREDENTIAL:-}" ]; then
        set -- --credential "$LANTHORN_WEB_CREDENTIAL" "$@"
    fi
    # --writable: ttyd >= 1.7 is read-only by default, which would make the
    # game unplayable. disableLeaveAlert spares players a confirm-on-close
    # dialog; titleFixed names the browser tab.
    exec ttyd --writable \
        --port "${LANTHORN_WEB_PORT:-7681}" \
        -t titleFixed=lanthorn \
        -t disableLeaveAlert=true \
        -t enableSixel=true \
        "$@"
fi

# Direct mode: the same paced sink, so a terminal session does not spin a core
# on a sound card that is not there.
if [ -z "${LANTHORN_AUDIO_OUT:-}" ]; then
    start_sink
    export LANTHORN_AUDIO_OUT="$LANTHORN_AUDIO_DIR/null.pcm"
fi
exec lanthorn "$@"
