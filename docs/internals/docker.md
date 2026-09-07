# Docker: lanthorn as a server

> For players, the short version is in [the guide](../guide/play-in-a-browser.md).

lanthorn is a terminal app, which means it containerizes and serves cleanly:
everything it draws — the TUI, the live map, even Kitty-protocol graphics — is
bytes over a pty. One image supports two ways of running it.

```sh
docker build -t lanthorn .
```

The multi-stage build compiles the workspace with the repo's pinned toolchain
(`rust-toolchain.toml`) inside a Rust builder image and ships a small
Debian-slim runtime carrying all four release binaries (`lanthorn`,
`zvm-cli`, `gvm-cli`, `scott-cli`). No Rust toolchain is needed on the host.

## Mode 1: play in your own terminal

```sh
docker run -it --rm \
  -v ~/if-games:/stories \
  -v lanthorn-data:/data \
  lanthorn
```

This is full-fidelity lanthorn: your terminal talks to the app through the
container's pty, so everything that works locally works here — including the
Kitty graphics protocol for v6 artwork, if your terminal speaks it. With no
arguments the story picker opens on `/stories`; any lanthorn arguments work in
place of the default (`docker run -it --rm ... lanthorn /stories/zork1.z5`,
`... lanthorn --help`).

Two host-terminal facts pass through automatically: `docker run -it` conveys
your terminal size and resizes, and lanthorn's capability probes (Kitty
graphics, colours) travel over the pty like any other escape sequences. If
your terminal's `TERM` is something unusual, forward it: `-e TERM=$TERM`.

Kitty's shared-memory art transfer needs the container and your terminal
sharing one IPC namespace, so add `--ipc=host` if you want it; without it
lanthorn's probe simply gets no answer and falls back to compressed transfer,
which still draws v6 artwork correctly, just over the wire instead of through
shared memory.

## Mode 2: serve it to browsers

```sh
docker run -d --name lanthorn \
  -p 7681:7681 \
  -v ~/if-games:/stories \
  -v lanthorn-data:/data \
  lanthorn serve
```

Then open <http://localhost:7681>. The container runs
[ttyd](https://github.com/tsl0922/ttyd) (a pinned static release binary,
fetched at image-build time), which serves an xterm.js terminal in the browser and gives each visitor their
own lanthorn session — several people can play at once, sharing the `/stories`
library and the `/data` save directory. A session **outlives the websocket that
started it**: see [the session model](#the-session-model) below.

Arguments after `serve` go to lanthorn (`... lanthorn serve /stories/zork1.z5`
pins every connection to one story); with none, each connection gets the
picker on `/stories`.

Serve-mode knobs, as environment variables:

| variable | meaning | default |
|---|---|---|
| `LANTHORN_WEB_PORT` | port ttyd listens on | `7681` |
| `LANTHORN_WEB_CREDENTIAL` | HTTP basic auth, `user:pass` | unset (no auth) |
| `LANTHORN_WEB_AUDIO` | `on` or `off`: the game's sound, played in the browser | `on` |
| `LANTHORN_WEB_AUDIO_PORT` | the port that sound is served on | `7682` |
| `LANTHORN_WEB_IMAGES` | `sixel` or `halfblocks`: how pictures are sent to the browser | `sixel` |
| `LANTHORN_WEB_FONT` | a CSS font-family name to prefer over the page's own embedded font (which still loads as a fallback) | unset |
| `LANTHORN_WEB_FONT_SIZE` | the terminal's font size in the page | `16` |
| `LANTHORN_WEB_TOUCH` | `on` or `off`: turn touch drags into scroll and mouse-drag reports (see below) | `on` |
| `LANTHORN_WEB_DETACH` | `on` or `off`: keep a game running when its connection drops (see below) | `on` |
| `LANTHORN_WEB_SESSION_TTL` | seconds an abandoned game is kept before it is ended | `21600` (6 h) |
| `LANTHORN_WEB_SESSION_DIR` | where detached sessions keep their sockets and stamps | `/tmp/lanthorn-sessions` |
| `LANTHORN_WEB_AUTOSAVE` | `on` or `off`: pass `--auto-save on` to every served game | `on` |
| `LANTHORN_WEB_GRAB_ZONE` | `1`-`6` or `off`: how many cells wide the draggable pane boundaries are (see below) | `4` |

**Do not expose an unauthenticated port beyond localhost** — a lanthorn
session includes a story picker that can browse and download into `/stories`,
and any writable terminal is an interactive program running on your machine.
For anything public, set `LANTHORN_WEB_CREDENTIAL` and terminate TLS in front
of it with your usual reverse proxy (Caddy, nginx, Traefik); ttyd itself
speaks plain HTTP/WebSocket here.

`docker-compose.yml` at the repo root is a ready-made example of this mode:
`mkdir -p stories && docker compose up -d`.

### The touch grab zone

`grab_zone_cells` (SQ-1327) says how wide a target the draggable pane
boundaries present — the story/map splitter and the dock panels' top edges.
lanthorn's own default is **2**, which is drawn for a mouse pointer; on a tablet
a finger cannot reliably land on so narrow a strip, and the browser image is
where the tablets are. Serve mode therefore seeds **4**, adjustable with
`LANTHORN_WEB_GRAB_ZONE` (1-6, or `off` to leave the setting entirely alone).

This is the one serve knob that writes into a file the player owns, because
`grab_zone_cells` has no command-line flag to state it with for one run. The
rules that make that safe, all checked by `docker/test-entrypoint.sh`:

- **Only where the player has not answered.** An uncommented `grab_zone_cells =`
  anywhere in `$HOME/.lanthorn/config.toml` stops the seed dead. A *commented*
  one — which is what lanthorn's own seeded template carries — does not, because
  that is documentation rather than a decision.
- **Inserted above the first `[table]`, never appended.** The template has real
  section headers, and a top-level key written after one silently becomes a key
  *inside* that section.
- **A value outside 1-6 is refused rather than written.** `grab_zone_cells =
  banana` is not merely a bad setting: it makes `config.toml` unparseable, after
  which `write_config_at` refuses to save *any* setting until somebody edits the
  file by hand.
- **A config.toml that does not exist yet is created holding just this line.**
  That costs nothing, because `config_template::top_up` appends every documented
  setting an existing file has never held, commented, on the next launch — so
  the player still ends up with the full annotated catalogue, with this one key
  already answered.

Once seeded the key is the player's: the settings screen writes over it like any
other, and the container never touches it again.

### The session model

A browser connection is a fragile thing — a closed tab, a sleeping tablet, a
train going into a tunnel — and ttyd's answer to a dropped websocket is to
signal the process it started. Two mechanisms, added by SQ-1323, mean that no
longer costs the player anything.

**Every served game saves after every turn.** The entrypoint passes
`--auto-save on`, so the resume archive in `/data` is rewritten as each turn
completes and again on the way out. This is a flag on the command line rather
than a line written into `/data/.lanthorn/config.toml`, deliberately: the
container's cadence must never silently become the player's own setting, and a
desktop lanthorn still ships with `auto_save = false`. `LANTHORN_WEB_AUTOSAVE=off`
turns it off; an explicit `--auto-save off` after `serve` wins over both.

**And the game outlives the connection.** Each visitor's page mints a session id
(`docker/web-session.js`), keeps it in `localStorage` for its origin, and passes
it back on the command line through ttyd's `--url-arg` as
`--web-session=<id>` — the same channel the browser-audio FIFO id already used,
now shared rather than duplicated. `docker/serve-session.sh` runs the game as
`dtach -A /tmp/lanthorn-sessions/<id>.sock … lanthorn …`, so a reconnect with
that id **attaches to the running game** instead of starting a new one.

Why that survives the hang-up, precisely. ttyd's `LWS_CALLBACK_CLOSED`
(`src/protocol.c:373-379`) calls `pty_kill(pss->process, server->sig_code)`;
`pty_kill` is `uv_kill(-process->pid, sig)` (`src/pty.c:158-164`), a signal at
the process *group*, and `sig_code` defaults to `SIGHUP` (`src/server.c:169`).
That group contains the wrapper and the dtach *client* — the client's handler
prints `[detached]` and exits (`attach.c:202`, `:90`) — but not the dtach
*master*, which `setsid()`s into its own session (`master.c:462`) and sets
`SIGHUP` to `SIG_IGN` besides (`master.c:483`). With no client attached the
master reads the pty and discards it (`master.c:337`), so a detached game
neither blocks on a full pipe nor spins.

The reaper. A background loop in `docker/entrypoint.sh` sweeps every five
minutes and ends any session nobody has been in for `LANTHORN_WEB_SESSION_TTL`.
"Nobody has been in" is a heartbeat, not an event: `serve-session.sh` writes
`<id>.seen` every 30 seconds for as long as a browser is attached, so
`stale_sessions()` is pure arithmetic over a directory and a clock — which is
exactly what lets `docker/test-entrypoint.sh` check the TTL boundary, a corrupt
stamp, an empty stamp and an empty directory without a container. Ending a
session is a **SIGTERM at the game** (whose pid `docker/session-run.sh`
recorded, because dtach daemonises its master and hands back no handle), so
lanthorn runs its own termination path — restore the terminal, write the resume
archive — and the player who comes back after the TTL still resumes from the
save. Only the *live* session is gone.

Two supporting details that are not obvious from the outside:

- `docker/session-run.sh` waits for the attacher's window-size packet before
  starting lanthorn. dtach's `init_pty` creates the session's pty with no
  winsize at all ("we don't have to set the window size here, because the
  attacher will send it in a packet"), and a full-screen TUI that measures the
  terminal in that window lays its first frame out on nothing.
- `init: true` in `docker-compose.yml` gives the container a real init as pid 1.
  Without it ttyd is pid 1, and ttyd does not `wait()` for children it did not
  spawn — so every reaped session's daemonised master would linger as a zombie.

**dtach, not abduco.** abduco is the smaller, tidier program and would have been
the first choice; it is simply **not packaged for Debian trixie**
(`packages.debian.org/trixie/abduco` is a 404 — 0.6-1 exists in forky and sid
only), which the runtime image is. dtach 0.9-7 is in trixie, is one binary over
libc, and has the attach-or-create semantics this needs. The flags are
`-r winch` (a reattach repaints via `SIGWINCH`; dtach's default `ctrl_l` would
type a key *at the parser*), `-E` (no detach character — `^\` belongs to the
game) and `-z` (so does `^Z`).

#### The sound follows the session

**A detached game keeps its sound** (SQ-1328). It did not at first: the audio
FIFO was created when the page's audio socket opened and unlinked when it
closed, which made it a property of the *connection*, and a game that outlived
its connection would have had ALSA bound to a pipe nobody reads. So a detachable
session played into the paced sink and was silent in the browser — the one trade
SQ-1323 asked for.

The FIFO now belongs to the **session**. `crates/audio-relay` keeps a per-id
session — the FIFO, a reader thread that is always reading it, and whichever
websocket is attached at the moment — so a dropped tab merely *detaches*: the
thread goes on draining at real time and discarding, the game is never held up,
and the next connection with the same id attaches to the same FIFO, gets a fresh
header frame, and hears the game where it is now. `docker/web-audio.js` is the
other half, reopening the audio socket with the same id after a drop (backing
off 0.5 s to 8 s) and emptying its queue on each header frame, since whatever
was buffered belongs to a connection that is over.

Three details are worth knowing before touching any of it:

- **The relay can never see its game exit.** It holds the pipe's write end
  itself — which is what stops a quiet moment between two sounds reading as EOF
  — so ending a session is somebody else's word, and the word is the FIFO's
  *absence*. `reap_stale_sessions` unlinks `<id>.pcm` beside the stamp and the
  pid file, and the drain loop, polling anyway, notices within half a second.
  That is deliberately a **condition it polls** rather than an event it is sent:
  a `DELETE /audio/<id>` that went astray would leak a reader thread and an open
  pipe for the life of the container, where a missed sweep merely waits for the
  next one.
- **Sends are gated on the socket being writable**, and a chunk that would have
  to wait is dropped. A blocking send into a full socket buffer would stop the
  thread reading the FIFO, the pipe would fill, and the game's audio thread
  would block on `write` — the wedge this design exists to avoid, arriving by
  the front door. Ten seconds of an unwritable socket and the peer counts as
  gone, so a tab suspended with the laptop lid does not hold the session's
  sound.
- **Whether sessions detach is settled once, in the entrypoint**, which does the
  `command -v dtach` check and exports its conclusion. Read separately at each
  end, a container without dtach would have the wrapper running one game per
  websocket while the relay kept every FIFO alive — and a session nothing ever
  reaps holds a reader thread until the container stops.

`LANTHORN_WEB_DETACH=off` still means one game per websocket, and there the FIFO
dies with the socket exactly as it always did.

One wrinkle players may notice: up to a pipeful (64 KB, about a third of a
second) of the audio played while nobody was attached is still in the FIFO when
a tab comes back, so a reconnect can open on the tail of a sound that has
already finished. The page's own one-second cap keeps that bounded.

### Fetching the library's metadata on the server

The picker's `r` fetches titles, blurbs, ratings and cover art from IFDB into
`/data`. On a server you want that done once, up front, for the whole library,
which is what `--fetch` is for. With the compose file above:

```sh
docker compose run --rm lanthorn /stories --fetch missing
```

It walks `/stories` (sub-folders included), prints one line per story, and
writes the sidecars into the shared `/data` volume, so the next browser session
opens the picker with the metadata already there. `--fetch all` refetches
what is cached; run `missing` again after adding games.

For stories IFDB does not know by IFID, or has no cover for, a curated TSV
(see `--import-metadata` in the interface docs) is applied the same way, with
the file bind-mounted in:

```sh
docker compose run --rm -v "$PWD/curated.tsv:/curated.tsv:ro" lanthorn /stories --import-metadata /curated.tsv
```

### What the browser mode can and cannot show

xterm.js does not implement the Kitty graphics protocol, but ttyd's build of
it includes the image addon, which renders **sixel**. The entrypoint turns
that addon on (`-t enableSixel=true`) and starts each session with
`--image-protocol sixel`, so cover art in the picker and graphical v6 stories
show in the browser as real pictures instead of half-block cells. Sixel is a
256-colour format per image, so a cover is close to the original but not
photographic; `LANTHORN_WEB_IMAGES=halfblocks` restores the cell fallback.
Text games, the automap, mouse support and the full TUI are the same either
way. For Kitty fidelity, use mode 1 (or SSH to the host and run mode 1 there;
Kitty graphics work over SSH).

Sixel has no image ids, so an inline transcript picture cannot be re-placed by
reference the way Kitty does — scrolling it past its anchor cell would mean
re-sending the whole payload every scroll step over the browser's WebSocket.
lanthorn instead draws such an image as a plain background-filled footprint
while the transcript is still moving, and re-sends the full picture once the
scroll settles, so a scroll session costs one payload per image rather than
one per step (SQ-1198).

### Touch, on tablets and phones

xterm.js wires `touchstart`/`touchmove` only to its own scrollback viewport
(`browser/Viewport.ts`), which is a no-op on lanthorn's alternate screen — a
drag on a touchscreen would otherwise reach nothing. `docker/web-touch.js`
(injected by `build_index` unless `LANTHORN_WEB_TOUCH=off`) turns a touch
drag into the mouse events xterm.js already knows how to forward, classified
once per touch contact:

| fingers | direction | becomes |
|---|---|---|
| 1 | vertical (or still) | synthetic wheel events (as before SQ-1324) |
| 1 | horizontal | synthetic mouse drag (down/move/up) |
| 2 | either | synthetic mouse drag (down/move/up) |
| 1 | none (a tap) | nothing — tap-to-focus and the on-screen keyboard still work |

One-finger horizontal and two-finger touches were dead before this (xterm's
own viewport only scrolls vertically), so claiming them for a drag costs
nothing and leaves one-finger vertical scroll untouched. The drag path relies
on xterm.js's `Terminal.bindMouse()` (`browser/Terminal.ts`) attaching an
"always on" `mousedown` listener to the same `.xterm` element the wheel
synthesis dispatches on, and on `MouseService.getMouseReportCoords` reading
only `event.clientX`/`clientY` — so a synthetic `MouseEvent` with real
coordinates is indistinguishable from a native one. It only works because
lanthorn's own `EnableMouseCapture` (crossterm's `?1000h?1002h?1003h?1006h`)
already asked the terminal for button-motion tracking; see
`docker/web-touch.js`'s header comment for the full chain and
`docker/web-touch.test.js` (`node docker/web-touch.test.js`, no CI wiring —
see the file for why) for the gesture classifier's own tests.

### The page's own font

xterm.js otherwise renders in whatever monospace font the visitor's browser
falls back to, which decides whether lanthorn's Nerd Font icons and the map's
Legacy Computing half-diagonal corner glyphs (U+1FBA0–U+1FBA3) show up at all.
So the image fetches and embeds one: **IosevkaTerm Nerd Font Mono**, the one
Nerd Font that carries those diagonals. The Dockerfile's `font-fetch` stage
downloads a pinned Nerd Fonts release asset (SHA-256 verified before
anything unpacks it), keeps only the Regular and Bold static weights, and
converts both to woff2 (the `woff2` Debian package's `woff2_compress`) —
a few MB each rather than ~14 MB of raw TTF. `docker/entrypoint.sh`'s
`build_index` inlines both faces as `data:font/woff2;base64,…` `@font-face`
rules into the page it hands ttyd, and passes `-t fontFamily=` /
`-t fontSize=` accordingly. `LANTHORN_WEB_FONT` names a family to prefer
instead (the embedded face stays in the stack as a fallback), and
`LANTHORN_WEB_FONT_SIZE` overrides the size (default 16). Iosevka itself is
OFL-1.1 licensed and the Nerd Fonts patch is MIT; both are redistributable,
and the licence text the release ships travels into the image alongside the
fonts, at `/usr/local/share/lanthorn/fonts/LICENSE.md`.

### Sound in the browser

The container has no sound device, and in mode 1 the game is silent, as on any
Linux host without one. Mode 2 plays it in the browser, through a second
channel beside the terminal, because a pty carries no audio:

- ALSA's `default` device in the image is the `file` plugin (`docker/asound.conf`),
  which writes what a process plays, as 16-bit 44.1 kHz stereo, to the path in
  `LANTHORN_AUDIO_OUT`, or to `/dev/null` when that is unset. lanthorn itself is
  unchanged: it opens the default device as always.
- `lanthorn-audio-relay` (a fourth binary in the image, `crates/audio-relay`)
  listens on port **7682**. A browser connecting to `ws://host:7682/audio/<id>`
  gets a FIFO created for that id, a JSON frame naming the format, then the
  raw PCM as it is played. The FIFO belongs to the *session*, not to that
  socket: see [the sound follows the session](#the-sound-follows-the-session).
- ttyd serves its own page with two small scripts added. `docker/web-session.js`
  owns the session id, and `docker/web-audio.js` opens `…/audio/<id>` with it
  (and reopens it after a drop); the id reaches the terminal through ttyd's
  `?arg=`, where the per-connection wrapper (`docker/serve-session.sh`) strips
  it and points ALSA at that session's FIFO before starting lanthorn. Playback
  starts on the first key or click, which is the gesture browsers require
  before they will play anything.

So publish **both** ports (`-p 7681:7681 -p 7682:7682`; the compose file does).
Behind a reverse proxy, the page connects to the same hostname on port 7682
with `ws` or `wss` to match the page, so terminate TLS for that port too.
`LANTHORN_WEB_AUDIO=off` restores the silent, single-port setup. To check a
deployment from a shell, `lanthorn-audio-relay client ws://host:7682/audio/abcdefgh12345678 10`
connects as the page would and reports what arrives.

## The two volumes

| mount | contents |
|---|---|
| `/stories` | the game library; the picker opens here. The repo ships no stories (commercial games are gitignored), so this is yours to fill — or use the picker's built-in IFDB search (`/`) to download freely available ones into it. |
| `/data` | the container user's `$HOME`. Saves, `config.toml` / `style.toml`, and `.lanthorn` map archives live in `/data/.lanthorn`. Name it a volume and saves persist across image upgrades. |

The container runs as an unprivileged user (`lanthorn`, uid 1000). If you
bind-mount host directories and see permission errors, either `chown` them to
uid 1000 or run with `--user "$(id -u):$(id -g)"` (then `$HOME` is still
`/data`, so keep that mount writable by your uid).

If you'd rather bind-mount a host directory for `/data` than use a named
volume, mount it at `/data/.lanthorn`, not `/data` — `$HOME` is `/data`, and
that's where saves, configs, and archives actually live underneath it. Don't
point it at your native `~/.lanthorn`: `config.toml`'s story paths and
recent-stories list are container paths (`/stories/...`) that mean nothing to
a native lanthorn on the host, and vice versa, so the two installs would
fight over one config. Use a dedicated host directory (e.g.
`~/lanthorn-docker`) if you want a bind mount at all — the named volume is
still the simpler default (upgrade-safe, no uid fuss); back it up with
`docker run --rm -v lanthorn-data:/data -v "$PWD":/backup debian tar czf
/backup/lanthorn-data.tgz -C /data .`.

## Publishing

`.github/workflows/docker.yml` builds the image on every version tag and
pushes it to GitHub Container Registry as
`ghcr.io/<owner>/lanthorn:<version>` / `:latest` (pre-release tags skip
`latest`), so "serve it up" can be one line with no checkout at all:

```sh
docker run -d -p 7681:7681 -v ~/if-games:/stories -v lanthorn-data:/data \
  ghcr.io/sharkusk/lanthorn:latest serve
```

The published image is `linux/amd64`; on Apple-Silicon Docker Desktop it runs
under Rosetta, or build natively from a checkout with the one `docker build`
line at the top of this page.
