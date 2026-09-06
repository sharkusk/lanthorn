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
#   LANTHORN_WEB_TOUCH        on (default) or off: turn touch drags on the
#                             served page into the mouse reports lanthorn
#                             already understands, classified once per
#                             contact — one finger vertical becomes
#                             wheel-scroll (the transcript and map scroll on a
#                             tablet), one finger HORIZONTAL or ANY two-finger
#                             drag becomes a synthetic mouse drag (pan and
#                             resize), and a tap is left alone so
#                             tap-to-focus and the on-screen keyboard still
#                             work. xterm.js's own touch handling only
#                             scrolls its own viewport, a no-op on lanthorn's
#                             alternate screen.
#   LANTHORN_WEB_DETACH       on (default) or off: keep a game running when
#                             its browser connection drops, so reconnecting
#                             resumes it mid-sentence instead of starting
#                             over. Needs the page's session id (see
#                             docker/web-session.js) and `dtach`; without
#                             either it quietly behaves as `off`. A
#                             detachable session plays into the paced sink
#                             rather than the browser — see the audio note in
#                             docker/serve-session.sh for why the two cannot
#                             both be had.
#   LANTHORN_WEB_SESSION_TTL  seconds a detached game with nobody attached is
#                             kept before it is ended (default 21600, six
#                             hours). Ending it is a SIGTERM, so the app
#                             writes its resume state on the way out and the
#                             next visit picks up from there.
#   LANTHORN_WEB_SESSION_DIR  where detached sessions keep their sockets and
#                             stamps (default /tmp/lanthorn-sessions)
#   LANTHORN_WEB_AUTOSAVE     on (default) or off: pass `--auto-save on` to
#                             every served game, so the resume state is
#                             written after each turn. A pty that can vanish
#                             under a game wants this; a desktop lanthorn
#                             still defaults to off.
#   LANTHORN_WEB_GRAB_ZONE    how many cells wide the draggable pane
#                             boundaries are (1-6, default 4, or `off` to
#                             leave the setting alone). A fingertip is not a
#                             mouse pointer, and lanthorn's own default of 2
#                             is drawn for a pointer. Unlike the knobs above
#                             this has no command-line flag, so it is SEEDED
#                             into $HOME/.lanthorn/config.toml — once, and
#                             never over a value the player has already set.
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

# Detached sessions whose last-seen stamp is older than `ttl` (SQ-1323).
#
# A pure function of the directory's contents and the clock — no processes, no
# `abduco`-style listing to parse — which is what lets docker/test-entrypoint.sh
# feed it fixture stamps and check the arithmetic. `docker/serve-session.sh`
# writes `<id>.seen` every 30 seconds for as long as a browser is attached, so
# "the stamp is old" is exactly "nobody has been here for a while", including
# for a wrapper that died without running its trap.
stale_sessions() {
    _dir="$1"
    _now="$2"
    _ttl="$3"
    for _s in "$_dir"/*.seen; do
        [ -f "$_s" ] || continue
        _id="${_s##*/}"
        _id="${_id%.seen}"
        _seen="$(cat "$_s" 2>/dev/null || printf '')"
        # A stamp we cannot read as a number is a stamp from a broken write, and
        # the safe reading of that is "long ago" — an unreapable session would
        # hold a game and its memory for the life of the container.
        case "$_seen" in
            ''|*[!0-9]*) _seen=0 ;;
        esac
        if [ "$((_now - _seen))" -ge "$_ttl" ]; then
            printf '%s\n' "$_id"
        fi
    done
}

# End the sessions `stale_sessions` names, and forget them.
#
# SIGTERM at the GAME (whose pid docker/session-run.sh recorded), not SIGKILL at
# dtach: lanthorn answers a termination signal by restoring the terminal and
# writing its resume state, so an expired session becomes a save the player can
# come back to rather than an hour thrown away. dtach's master sees the child go
# and unlinks its own socket (`atexit(unlink_socket)`).
#
# The socket check is the guard against a recycled pid: a session whose master is
# already gone has no socket, and its stale pid file must not be allowed to name
# somebody else's process.
reap_stale_sessions() {
    _dir="$1"
    stale_sessions "$1" "$2" "$3" | while IFS= read -r _id; do
        _pid="$(cat "$_dir/$_id.pid" 2>/dev/null || printf '')"
        case "$_pid" in
            ''|*[!0-9]*) _pid="" ;;
        esac
        if [ -n "$_pid" ] && [ -S "$_dir/$_id.sock" ]; then
            kill -TERM "$_pid" 2>/dev/null || true
        fi
        rm -f "$_dir/$_id.seen" "$_dir/$_id.pid"
    done
}

# Whether `file` already SETS `key` — an uncommented `key = …` at the start of a
# line, which is the only form actually in force. A commented `# key = 2` in
# lanthorn's own seeded template is documentation, not a decision, and must not
# stop the container stating a default beside it.
config_sets_key() {
    [ -f "$1" ] || return 1
    grep -qE "^[[:space:]]*$2[[:space:]]*=" "$1"
}

# Put `key = value` into `file` unless the player has already said something
# about it (SQ-1327). For a setting the container wants a different DEFAULT for
# and that has no command-line flag to say it with.
#
# Inserted before the file's first `[table]` header, never appended: lanthorn's
# seeded config.toml carries real section headers, and a top-level key written
# after one would silently become a key IN that section.
#
# A file that does not exist yet is created holding just this line. That costs
# the player nothing — `config_template::top_up` appends every documented
# setting an existing file has never held, commented, on the next launch — so
# they still end up with the full annotated catalogue, with this one key already
# answered.
seed_config_key() {
    _file="$1"
    _key="$2"
    _value="$3"
    if config_sets_key "$_file" "$_key"; then
        return 0
    fi
    mkdir -p "$(dirname "$_file")" 2>/dev/null || return 0
    if [ ! -f "$_file" ]; then
        printf '%s = %s\n' "$_key" "$_value" > "$_file" 2>/dev/null || true
        return 0
    fi
    _tmp="$_file.lanthorn-seed"
    awk -v line="$_key = $_value" '
        !seeded && /^[[:space:]]*\[/ { print line; print ""; seeded = 1 }
        { print }
        END { if (!seeded) print line }
    ' "$_file" > "$_tmp" 2>/dev/null && mv "$_tmp" "$_file" 2>/dev/null
    rm -f "$_tmp" 2>/dev/null || true
}

# ttyd's own page, with the served IosevkaTerm Nerd Font Mono faces always
# inlined into <head> (so icons and the map's diagonals render regardless of
# the visitor's own font), docker/web-session.js always added and always FIRST
# (it owns the session id both of the others depend on, and it may reload the
# page before either of them has run), docker/web-audio.js added only when a
# browser audio port is live ($1 — empty means audio is off), docker/web-touch.js
# added unless LANTHORN_WEB_TOUCH=off, and docker/web-font.js always added
# ($2/$3 — the same fontFamily/fontSize string ttyd's own `-t` options get,
# see the dispatch below). web-font.js re-measures xterm.js's character cell
# once the embedded font is actually ready, so the page doesn't cold-open
# measured against the fallback monospace and spaced out (SQ-1263). Generated
# per start so the audio port, the font, and the touch setting stay in step
# with the current environment.
build_index() {
    audio_port="$1"
    term_font_family="$2"
    term_font_size="$3"
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
        -v tf="$LANTHORN_SHARE_DIR/web-touch.js" -v touch_on="$touch_on" \
        -v sf="$LANTHORN_SHARE_DIR/web-session.js" \
        -v ff="$LANTHORN_SHARE_DIR/web-font.js" -v tfont="$term_font_family" -v tsize="$term_font_size" '
        BEGIN {
            if (port != "") { while ((getline l < f) > 0) js = js l "\n" }
            if (touch_on != "") { while ((getline l < tf) > 0) tjs = tjs l "\n" }
            while ((getline l < sf) > 0) sjs = sjs l "\n"
            while ((getline l < ff) > 0) fjs = fjs l "\n"
        }
        {
            i = index($0, "</head>")
            if (i && !done) {
                head_insert = "\n<script>\n" sjs "</script>"
                if (port != "") {
                    head_insert = head_insert "\n<script>window.LANTHORN_WEB_AUDIO_PORT=" port ";\n" js "</script>"
                }
                if (touch_on != "") {
                    head_insert = head_insert "\n<script>\n" tjs "</script>"
                }
                head_insert = head_insert "\n<script>window.LANTHORN_WEB_FONT=" "\047" tfont "\047" ";window.LANTHORN_WEB_FONT_SIZE=" tsize ";\n" fjs "</script>"
                # The marker always lands on a line of its own — substr($0,1,i-1)
                # would otherwise run straight into head_insert on the same
                # line, which the head/tail splice below would then drop whole.
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

    # Each connection runs through the session wrapper, which strips the page's
    # session argument, points ALSA at the right place, and — unless
    # LANTHORN_WEB_DETACH=off — hands the connection to the dtach session named
    # after that id. Pictures are sent as sixel unless configured otherwise:
    # xterm.js renders it once the page addon is on (-t enableSixel below), and
    # lanthorn's auto-detection cannot learn that from xterm.js.
    images="${LANTHORN_WEB_IMAGES:-sixel}"

    # A pty here can disappear under a running game at any moment — a closed
    # tab, a sleeping tablet, a roaming Wi-Fi hop — so the served build saves
    # the resume state every turn, where a desktop lanthorn leaves that off
    # (SQ-1323). It is a FLAG rather than a line written into /data's
    # config.toml, so it can never silently become the player's own setting.
    #
    # Ahead of "$@" — the arguments the operator wrote after `serve` — for the
    # same reason `--image-protocol` is: ours is the default, theirs is the
    # instruction, and clap's last occurrence wins.
    if [ "${LANTHORN_WEB_AUTOSAVE:-on}" != "off" ]; then
        set -- lanthorn --image-protocol "$images" --auto-save on "$@"
    else
        set -- lanthorn --image-protocol "$images" "$@"
    fi
    set -- /usr/local/bin/lanthorn-serve-session "$@"

    # A fingertip is not a mouse pointer, and lanthorn's `grab_zone_cells`
    # default of 2 is drawn for a pointer — on a tablet the splitter between the
    # story and the map is a target a finger cannot reliably land on (SQ-1327).
    # There is no flag for it, so it is seeded into the data home's config.toml,
    # and only where the player has not already answered. A value that is not a
    # number in range is ignored rather than written: an invalid `config.toml` is
    # not merely a bad setting — `write_config_at` then refuses to save settings
    # at all until somebody fixes the file by hand.
    grab_zone="${LANTHORN_WEB_GRAB_ZONE:-4}"
    case "$grab_zone" in
        [1-6]) seed_config_key "${HOME:-/data}/.lanthorn/config.toml" grab_zone_cells "$grab_zone" ;;
        off|'') : ;;
        *) echo "lanthorn: ignoring LANTHORN_WEB_GRAB_ZONE=$grab_zone (want 1-6, or off)" >&2 ;;
    esac

    start_sink

    # Detached sessions: where they live, and the loop that ends the abandoned
    # ones. Started before ttyd so it is already sweeping when the first
    # connection arrives, and it survives the `exec` below as ttyd's own child.
    detach_on=""
    if [ "${LANTHORN_WEB_DETACH:-on}" != "off" ]; then
        detach_on="1"
        session_dir="${LANTHORN_WEB_SESSION_DIR:-/tmp/lanthorn-sessions}"
        export LANTHORN_WEB_SESSION_DIR="$session_dir"
        mkdir -p "$session_dir"
        session_ttl="${LANTHORN_WEB_SESSION_TTL:-21600}"
        (
            while :; do
                sleep "${LANTHORN_WEB_SESSION_SWEEP:-300}"
                reap_stale_sessions "$session_dir" "$(date +%s)" "$session_ttl"
            done
        ) &
        # Named so it can be found: `exec ttyd` below replaces this shell, which
        # leaves the sweeper with no parent to ask about it. An operator who
        # wants the sweeping stopped — and docker/test-entrypoint.sh, which
        # starts a real dispatch and must not leave one behind — has this.
        printf '%s\n' "$!" > "$session_dir/reaper.pid"
    fi

    audio_port=""
    if [ "${LANTHORN_WEB_AUDIO:-on}" != "off" ]; then
        audio_port="${LANTHORN_WEB_AUDIO_PORT:-7682}"
        LANTHORN_WEB_AUDIO_BIND="0.0.0.0:$audio_port" lanthorn-audio-relay &
    fi

    # The embedded face stays second in the stack, so an override that lacks
    # the map's diagonals or the Nerd Font icons still falls back to it
    # instead of to the browser's own default monospace. Computed here,
    # before build_index, so the page's window.LANTHORN_WEB_FONT (used to
    # re-measure once that face is ready, see web-font.js) and ttyd's own
    # `-t fontFamily=` below name the exact same string.
    font_family="IosevkaTerm Nerd Font Mono"
    if [ -n "${LANTHORN_WEB_FONT:-}" ]; then
        font_family="${LANTHORN_WEB_FONT}, IosevkaTerm Nerd Font Mono"
    fi
    font_size="${LANTHORN_WEB_FONT_SIZE:-16}"

    # build_index always inlines the served font and the session script; it adds
    # the audio script only when audio_port is non-empty. --index serves the
    # generated page either way; --url-arg is what lets the page pass its
    # session id back on the command line, and BOTH features need it — the audio
    # socket to name its FIFO, and a detachable session to find the game it
    # belongs to. Without either, ttyd need not accept arguments from a URL at
    # all, and does not.
    build_index "$audio_port" "$font_family" "$font_size" > /tmp/lanthorn-index.html
    set -- --index /tmp/lanthorn-index.html "$@"
    if [ -n "$audio_port" ] || [ -n "$detach_on" ]; then
        set -- --url-arg "$@"
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
        -t "fontFamily=$font_family" \
        -t "fontSize=$font_size" \
        "$@"
fi

# Direct mode: the same paced sink, so a terminal session does not spin a core
# on a sound card that is not there.
if [ -z "${LANTHORN_AUDIO_OUT:-}" ]; then
    start_sink
    export LANTHORN_AUDIO_OUT="$LANTHORN_AUDIO_DIR/null.pcm"
fi
exec lanthorn "$@"
