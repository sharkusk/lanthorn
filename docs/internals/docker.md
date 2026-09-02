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
fetched at image-build time), which serves an xterm.js terminal in the browser and spawns **one lanthorn process
per connection** — several people can play at once, each in their own session,
sharing the `/stories` library and the `/data` save directory. A game saved in
one session restores in the next.

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

**Do not expose an unauthenticated port beyond localhost** — a lanthorn
session includes a story picker that can browse and download into `/stories`,
and any writable terminal is an interactive program running on your machine.
For anything public, set `LANTHORN_WEB_CREDENTIAL` and terminate TLS in front
of it with your usual reverse proxy (Caddy, nginx, Traefik); ttyd itself
speaks plain HTTP/WebSocket here.

`docker-compose.yml` at the repo root is a ready-made example of this mode:
`mkdir -p stories && docker compose up -d`.

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
  raw PCM as it is played.
- ttyd serves its own page with a small script added (`docker/web-audio.js`).
  The script mints a session id, opens the audio socket, and passes the id to
  the terminal through ttyd's `?arg=`; a per-connection wrapper
  (`docker/serve-session.sh`) strips that argument and points ALSA at the
  session's FIFO before starting lanthorn. Playback starts on the first key or
  click, which is the gesture browsers require before they will play anything.

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
