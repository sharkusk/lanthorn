# Play in a browser

For anyone who doesn't have — or doesn't want — a fancy terminal, and would
rather just open a URL. lanthorn's Docker image can serve itself to any
browser on your network.

## One command

```sh
mkdir -p stories && docker compose up -d
```

`docker-compose.yml` at the repo root is a ready-made deployment: drop your
game files into the new `stories/` folder, run that command, and open
<http://localhost:7681>. You'll see a real terminal running lanthorn's story
picker, delivered to the page by [ttyd](https://github.com/tsl0922/ttyd)
serving an xterm.js terminal — everyone who opens it gets their own game, so
several people can play at once, sharing the same story library and the same
saves.

Without compose, the same thing is one `docker run`:

```sh
docker run -d --name lanthorn \
  -p 7681:7681 -p 7682:7682 \
  -v ~/if-games:/stories \
  -v lanthorn-data:/data \
  lanthorn serve
```

## What the browser can show

xterm.js doesn't speak the kitty graphics protocol, but the browser terminal
does render **sixel** — so cover art in the picker and graphical v6 stories
show up in the browser as real pictures, not half-block cells. sixel is a
256-colour format per image, so a cover is close to the original but not
photographic. Text games, the automap, mouse support and the full TUI look
and work the same as anywhere else. `LANTHORN_WEB_IMAGES=halfblocks` falls
back to the cell renderer if you'd rather have that; for full kitty graphics
fidelity, SSH to the host and run lanthorn there directly instead — see
[command line](command-line.md). The page brings its own font, so icons and
map diagonals draw correctly on any machine.

## If your connection drops

It doesn't cost you the game. Close the tab, walk out of Wi-Fi range, let a
tablet fall asleep for an hour — come back to the same address and you are
back in the same room, mid-sentence, with your transcript and your map exactly
where you left them. Your browser quietly remembers which game is yours, so
there is nothing to click and nothing to restore — and mouse clicks, map
dragging and touch scrolling all keep working on the reattached tab too,
with nothing for you to re-enable.

That holds for six hours of being away. After that the game is put down for
you — but not lost: lanthorn saves after every single turn here, so the next
time you open the page it picks your progress straight back up, one turn at
most behind where you stopped. The same is true of anything more violent than
a dropped connection, a restarted server included.

Six hours is a default, not a rule: `LANTHORN_WEB_SESSION_TTL` (in seconds)
sets how long an untouched game is kept, and `LANTHORN_WEB_DETACH=off` turns
the whole thing off, for a server that would rather every visit start fresh.

## Playing on a tablet or phone

Dragging one finger up or down the transcript or the story picker's list
scrolls it, same as a mouse wheel. Drag one finger sideways, or drag with two
fingers in any direction, to pan the map or resize a pane's splitter — a
plain tap still just taps, so the on-screen keyboard still comes up when you
need it. To resize the dividers between panes, drag them with two fingers,
since a one-finger drag that starts moving up or down is read as a scroll.
`LANTHORN_WEB_TOUCH=off` turns all of this off if you'd rather the
browser handle touch its own way.

The edges you drag are made wider here than on a desktop, because a fingertip
is not a mouse pointer: the splitter between the story and the map, and the
top edges of the inventory and room panels, are four cells deep instead of
two. `LANTHORN_WEB_GRAB_ZONE` sets that (anything from 1 to 6), and whatever
you set in lanthorn's own settings screen wins over it from then on.

## Sound in the browser

Sound plays too, over a second connection alongside the terminal — a terminal
session carries no audio of its own, so the container relays what lanthorn
plays to a small audio server, and the served page opens that connection and
starts playback on your first key press or click, which is the gesture
browsers require before they'll play anything. That's why the compose file
publishes two ports: 7681 for the terminal, 7682 for sound.
`LANTHORN_WEB_AUDIO=off` turns this off and drops back to a silent,
single-port setup. Full detail, including what to do behind a reverse proxy,
lives in [sound](sound.md) and [remote sound](../internals/remote-sound.md).

**And the sound survives a reconnect too.** Close the tab in the middle of a
storm and the game keeps playing it; come back and you hear where the game is
now, not where you left it. There is nothing to switch on and nothing to click
— the page reopens the sound connection by itself, the same way it finds your
game again.

## The library and its metadata

Two volumes matter: `/stories` is your game library — nothing ships in the
image, so this is yours to fill, or use the picker's built-in IFDB search
(`/`) to download freely available games straight into it. `/data` holds
saves, `config.toml`/`style.toml`, and the map archives, so name it a
persistent volume and everything survives an image upgrade.

The compose file's default (`lanthorn-data`, a named Docker volume) is the
easiest way to keep that: Docker manages it for you and it just works across
upgrades. If you'd rather see your saves as ordinary files on your machine,
mount a folder of your own there instead — but mount it at
`/data/.lanthorn`, not `/data` itself, and pick a dedicated folder like
`~/lanthorn-docker` rather than your host's own `~/.lanthorn`, since a
native lanthorn install and the container don't understand each other's
story paths. `docker-compose.yml` has that alternative ready to uncomment.

On a shared server you want the library's titles, blurbs, ratings and cover
art fetched once, up front, rather than per visitor. Run the fetch on the
server itself:

```sh
docker compose run --rm lanthorn /stories --fetch missing
```

That walks the whole library (subfolders included) and writes the metadata
into the shared `/data` volume, so every browser session opens the picker
with it already there. Run `--fetch missing` again after adding games, or
`--fetch all` to refetch everything. For titles IFDB doesn't know, bind-mount
a curated TSV and apply it the same way:

```sh
docker compose run --rm -v "$PWD/curated.tsv:/curated.tsv:ro" \
  lanthorn /stories --import-metadata /curated.tsv
```

## Sharing it

To let others on your network in, point them at your machine's address
instead of `localhost`. **Do not expose an unauthenticated port beyond your
own network** — a lanthorn session includes a story picker that can browse
and download into `/stories`, and any writable terminal is an interactive
program running on your machine. Set `LANTHORN_WEB_CREDENTIAL` for HTTP
basic auth, and put a reverse proxy (Caddy, nginx, Traefik) in front for TLS
before exposing it any further than that.

## Publishing

A pre-built image is published to GitHub Container Registry on every release,
so serving lanthorn up doesn't need a checkout at all:

```sh
docker run -d -p 7681:7681 -p 7682:7682 \
  -v ~/if-games:/stories -v lanthorn-data:/data \
  ghcr.io/sharkusk/lanthorn:latest serve
```

## Going deeper

- [Docker](../internals/docker.md) — both modes, every environment variable, and the audio relay's plumbing
- [Command line](command-line.md) — mode 1, a portable lanthorn in your own terminal instead of a browser
- [Sound](sound.md) — how audio reaches a session, browser or not
