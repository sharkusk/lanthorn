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
#   LANTHORN_WEB_TOUCH        on (default) or off: convert a vertical touch
#                             drag on the served page into wheel-scroll
#                             reports lanthorn already understands, so the
#                             transcript and map scroll on a tablet or phone
#                             (xterm.js's own touch handling only scrolls its
#                             own viewport, a no-op on lanthorn's alternate
#                             screen)
#   LANTHORN_WEB_IMAGES       sixel (default) or halfblocks: how pictures are
#                             sent to the browser. ttyd's xterm.js can render
#                             sixel, so covers and v6 art show as real images;
#                             lanthorn's auto-detection cannot see that.
#   LANTHORN_WEB_FONT         a CSS font-family name to prefer over the page's
#                             own embedded IosevkaTerm Nerd Font Mono (which
#                             still loads as a fallback, so icons and the
#                             map's diagonals keep drawing even if the
#                             override doesn't cover them)
#   LANTHORN_WEB_FONT_SIZE    the terminal's font size in the page (default 16)
set -eu

# Where the image's fetched-at-build-time assets (ttyd's page, the audio
# script, the embedded font faces) live. Overridable so docker/test-entrypoint.sh
# can exercise build_index() against a fixture directory instead of the real
# image layout.
LANTHORN_SHARE_DIR="${LANTHORN_SHARE_DIR:-/usr/local/share/lanthorn}"

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

# ttyd's own page, with the served IosevkaTerm Nerd Font Mono faces always
# inlined into <head> (so icons and the map's diagonals render regardless of
# the visitor's own font), docker/web-audio.js added only when a browser
# audio port is live ($1 — empty means audio is off), and docker/web-touch.js
# added unless LANTHORN_WEB_TOUCH=off. Generated per start so the audio port,
# the font, and the touch setting stay in step with the current environment.
build_index() {
    audio_port="$1"
    src="$LANTHORN_SHARE_DIR/ttyd-index.html"
    fonts_dir="$LANTHORN_SHARE_DIR/fonts"
    family="IosevkaTerm Nerd Font Mono"
    touch_on=""
    if [ "${LANTHORN_WEB_TOUCH:-on}" != "off" ]; then
        touch_on="1"
    fi

    # `base64 | tr -d` rather than GNU's `-w0`, so the same script runs under
    # macOS's BSD coreutils for `docker/test-entrypoint.sh`.
    # Each embedded face runs to a few MB of base64 — too large to trust to
    # awk's own field/line handling, and (passed as a -v argument) too large
    # for some platforms' command-line length limit — so the <style> block is
    # built on its own here and spliced in below with grep/head/tail/cat
    # rather than through awk.
    css_tmp="$(mktemp)"
    {
        printf '<style>\n'
        printf "@font-face{font-family:'%s';font-weight:400;font-style:normal;font-display:swap;src:url(data:font/woff2;base64," "$family"
        base64 < "$fonts_dir/IosevkaTermNerdFontMono-Regular.woff2" | tr -d '\n'
        printf ") format('woff2');}\n"
        printf "@font-face{font-family:'%s';font-weight:700;font-style:normal;font-display:swap;src:url(data:font/woff2;base64," "$family"
        base64 < "$fonts_dir/IosevkaTermNerdFontMono-Bold.woff2" | tr -d '\n'
        printf ") format('woff2');}\n"
        printf '</style>\n'
    } > "$css_tmp"

    merged_tmp="$(mktemp)"
    awk -v f="$LANTHORN_SHARE_DIR/web-audio.js" -v port="$audio_port" \
        -v tf="$LANTHORN_SHARE_DIR/web-touch.js" -v touch_on="$touch_on" '
        BEGIN {
            if (port != "") { while ((getline l < f) > 0) js = js l "\n" }
            if (touch_on != "") { while ((getline l < tf) > 0) tjs = tjs l "\n" }
        }
        {
            i = index($0, "</head>")
            if (i && !done) {
                head_insert = ""
                if (port != "") {
                    head_insert = head_insert "\n<script>window.LANTHORN_WEB_AUDIO_PORT=" port ";\n" js "</script>"
                }
                if (touch_on != "") {
                    head_insert = head_insert "\n<script>\n" tjs "</script>"
                }
                # The marker always lands on a line of its own — with neither
                # script, head_insert is empty and substr($0,1,i-1)
                # would otherwise run straight into it on the same line,
                # which the head/tail splice below would then drop whole.
                print substr($0, 1, i - 1) head_insert "\n@@LANTHORN_FONT_CSS@@\n" substr($0, i)
                done = 1
            } else {
                print
            }
        }' "$src" > "$merged_tmp"

    mark_line="$(grep -n '@@LANTHORN_FONT_CSS@@' "$merged_tmp" | head -1 | cut -d: -f1)"
    head -n "$((mark_line - 1))" "$merged_tmp"
    cat "$css_tmp"
    tail -n "+$((mark_line + 1))" "$merged_tmp"

    rm -f "$css_tmp" "$merged_tmp"
}
# --- end of function definitions; dispatch begins below --- #
# (docker/test-entrypoint.sh cuts the file at the line above to source
# start_sink/build_index without running the dispatch itself — keep it a
# single, unindented line if you touch the functions above.)

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

    audio_port=""
    if [ "${LANTHORN_WEB_AUDIO:-on}" != "off" ]; then
        audio_port="${LANTHORN_WEB_AUDIO_PORT:-7682}"
        LANTHORN_WEB_AUDIO_BIND="0.0.0.0:$audio_port" lanthorn-audio-relay &
    fi
    # build_index always inlines the served font; it adds the audio script
    # only when audio_port is non-empty. --index serves the generated page
    # either way; --url-arg (letting the page pass its session id back) is
    # only needed when that page opens the audio socket.
    build_index "$audio_port" > /tmp/lanthorn-index.html
    set -- --index /tmp/lanthorn-index.html "$@"
    if [ -n "$audio_port" ]; then
        set -- --url-arg "$@"
    fi
    if [ -n "${LANTHORN_WEB_CREDENTIAL:-}" ]; then
        set -- --credential "$LANTHORN_WEB_CREDENTIAL" "$@"
    fi

    # The embedded face stays second in the stack, so an override that lacks
    # the map's diagonals or the Nerd Font icons still falls back to it
    # instead of to the browser's own default monospace.
    font_family="IosevkaTerm Nerd Font Mono"
    if [ -n "${LANTHORN_WEB_FONT:-}" ]; then
        font_family="${LANTHORN_WEB_FONT}, IosevkaTerm Nerd Font Mono"
    fi
    # --writable: ttyd >= 1.7 is read-only by default, which would make the
    # game unplayable. disableLeaveAlert spares players a confirm-on-close
    # dialog; titleFixed names the browser tab.
    exec ttyd --writable \
        --port "${LANTHORN_WEB_PORT:-7681}" \
        -t titleFixed=lanthorn \
        -t disableLeaveAlert=true \
        -t enableSixel=true \
        -t "fontFamily=$font_family" \
        -t "fontSize=${LANTHORN_WEB_FONT_SIZE:-16}" \
        "$@"
fi

# Direct mode: the same paced sink, so a terminal session does not spin a core
# on a sound card that is not there.
if [ -z "${LANTHORN_AUDIO_OUT:-}" ]; then
    start_sink
    export LANTHORN_AUDIO_OUT="$LANTHORN_AUDIO_DIR/null.pcm"
fi
exec lanthorn "$@"
