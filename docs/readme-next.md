# README, staged

**What this is.** README prose for features that are in `main` but **not in a
released build**. The README describes the *released* build — someone reads it
having just downloaded the latest release, and a README that describes `main`
tells them to type flags their binary rejects. So the prose is written when the
work lands, parked here, and applied when a version is cut.

**What this is not.** A copy of the README. Only deltas live here, so this file
cannot drift out of sync with anything — it is not a second source of truth
about the shipped build, and there is nothing here to keep updated. A staged
*whole* README would become exactly that, and rot.

**How to use it.**

- **Landing a user-visible feature?** Write the README prose now, in README
  voice, and add an entry below. The changelog entry goes in `CHANGELOG.md`
  under `## Unreleased`; that is a different genre for a different reader —
  the changelog tells an existing user what changed, the README tells a
  newcomer what lanthorn does.
- **Every entry names its destination**: which section, and what it replaces or
  follows. Applying this file at release should be mechanical. An entry that
  says only "mention the new flag" is a re-reading exercise for whoever cuts the
  release, and half of those get missed.
- **Cutting a release?** Apply every entry, then empty this file back to this
  header. Draining it is a release task, like removing a "coming next release"
  caveat.

Entries are grouped by README section, in the order those sections appear.

---

## Quick start

**Replaces** the flag list in the paragraph ending *"`lanthorn --help` has the
flags; the ones people reach for are …"*.

> `lanthorn --help` has the flags; the ones people reach for are `--sound off`,
> `--images off` and `--image-protocol`.

**Follows** the same paragraph — a URL is a launching shape, so it belongs
beside the directory and the disk image:

> ```bash
> lanthorn https://ifarchive.org/if-archive/games/zcode/curses.z5
> ```
>
> A web address works anywhere a path does. lanthorn fetches it, opens it like
> any other file — story, Blorb, disk image, zip — and then offers to keep it in
> your library so the next launch finds it without fetching again.

---

## Try these first

**Add** to the **In the story picker** table, after the `/` row:

> | **Shift+U** | Downloads a story straight into your library from a web
> address you paste. |


**Add** to the same table, after the **Shift+U** line:

> | **Ctrl+F** | Filters your library as you type: title, author, filename or folder. |
> | **Enter** on a folder | A library sorted into folders is listed folder by folder; Enter opens one and **Backspace** returns up. |

---

## What it does

**Replaces** the **Graphical v6, drawn properly** bullet:

> - **Graphical v6, drawn properly** — *Zork Zero*'s illustrated frame at an
>   authentic 640×400, set in the typeface the original interpreter used, read
>   off the media rather than bundled. Three ways to draw it: **hybrid** puts
>   text in real terminal cells and art in real pixels, **raster** paints the
>   whole pane as one image in the game's own face, and **extended** keeps
>   raster's face while growing the story downward instead of letterboxing it —
>   a tall terminal gets more rows to read, with the side art tiled out of its
>   own artwork at the artist's spacing. `/set-v6-render` cycles them.
>   → [v6 graphics](docs/features/v6-graphics.md)

The mode is also `v6_render = "extended"` in `config.toml` and `--v6-render
extended` on the command line, but the README is not the place to enumerate all
three spellings — the features doc is.

**Follows** the *"A real terminal UI"* bullet, as the last bullet in the list —
it is a thing lanthorn does for the player rather than a surface it draws:

> - **A light held up while you play** — Lanthorn's Guiding Light offers the
>   words this story's parser knows, the noun you were reaching for, and a
>   caution before a move that cannot be taken back. When it suggests a word it
>   has already tried it, silently, in a throwaway copy of your own game — so it
>   recommends what works where you are standing instead of listing what the
>   dictionary holds. It says so once, then marks every later line with one glyph
>   in the margin — never in the story's own voice, and never a spoiler.
>   `--guidance off`, `/set-guidance`, or the settings screen turns it off.
>   → [customization](docs/features/customization.md)

> - **It asks about your font once, and sets every icon from the answer** —
>   lanthorn writes characters; the font is the terminal's, and nothing can ask
>   it whether it has a glyph. So on a first launch it shows two rows and asks
>   which one draws properly, then writes the answer into `style.toml` as preset
>   names you can still edit. `/run-font-check` asks again when you change fonts.
>   → [customization](docs/features/customization.md)

---

## Play the original disks

**Add** to the media table:

> | Commodore 1541, GCR bitstream | `.g64` | Commodore 128 (7) |

A `.g64` is the raw bitstream a 1541's head reads rather than decoded sectors —
the format archives use when a disk's protection lives in how the bits are laid
down. lanthorn decodes it to sectors and plays it, and the protection is not a
problem because Infocom's lived in the loader, which lanthorn never runs.

**Replaces** whatever the README says about zips (currently in the section on
what a zip carries):

> **A zip is opened like a volume.** What is inside is identified by its
> *contents*, not its name, so a zip carries anything lanthorn runs — every
> Z-machine version including graphical v6, Glulx, Scott Adams, Blorb
> containers — and a Blorb or a hints file packed beside the story is found and
> used. **A zip holding two games lists both**, one row each, exactly as a
> compilation disc does: pick one in the browser or name it with
> `--story <name>`, and each keeps its own saves under its own name inside the
> archive. A zip holding one game still opens straight into it.
>
> **And a downloaded zip of release floppies** is offered to your library: say
> yes and the whole release is unpacked where the picker will find it and
> launched; say no and lanthorn tells you why rather than failing obscurely.
> Only the disk images come out of the archive — never a readme, a cover or
> anything else that happened to be in it.

---

## Playing aids

**Follows** the paragraph introducing Lanthorn's Guiding Light — the pane's own
switches are the visible half of the same idea:

> The story pane's frame carries a few clickable switches, each showing what
> state it is in — the command band and the Guiding Light along the bottom, the
> map and its return probe at the right, and on a graphical v6 story the render
> mode and the pixel lock up on the top border. Each is drawn twice over: a
> different glyph for each state, and lit when it is on, so you can read them at
> a glance without relying on colour. Hover one for a line saying what a click
> does and which command does the same, because a click *is* that command.
>
> If you told the font check you have a patched font, they are proper icons — a
> map, a docked panel, a lamp, a padlock. If you did not, they are plain shapes
> that say the same thing.
>
> What you switch there is remembered for **that story**, not for every story: a
> map you hid, a light you put out, a render mode you preferred. The settings
> screen still sets the default new games inherit.

**Add** after that, as its own paragraph — the two aids that answer "what is
there a word for?":

> Press **F4** and every word already on screen that the story knows lights up
> for a few seconds, over its own prose, without moving a line of it. It answers
> the oldest frustration in the genre: a room description names a dozen nouns and
> two of them are implemented. *Mini-Zork* opens on a `field` the story has never
> heard of, and that word stays dark. The claim it makes is the dictionary's and
> it says so each time — these are words this story knows, which is not a promise
> that the thing is within reach.
>
> The command band's **WHAT** column carries the same idea as a list. Under what
> is actually here, dimmed, are the nouns the story has *printed* this session —
> the things a room describes rather than the ones it contains. *Arthur* says of
> the torque that "imbedded in one of the knobs is a sliver of crystal", and the
> crystal is a real object with a real use; that block is where it turns up.
> Newest first, and it accumulates, so a noun named forty turns ago is still one
> click away.

**Add** to the **A map that draws itself** bullet under *What it does*, or as its
own sentence there:

> - …and it can go and look. Switch on the **return probe** and, after a move
>   that leaves the map with no way back, lanthorn plays a silent throwaway copy
>   of your game to find one — trying the reverse first, then the ways a passage
>   might curve. What it learns is drawn like anything you walked yourself; what
>   it fails to find is left as the honest gap it is, because one-way passages
>   are real and the map should not pretend otherwise. Off by default, per story,
>   and it never shows you a room you have not been to.

---

## Terminal support

**Replaces** *"or turn images off with `--no-images`"*:

> or turn images off with `--images off`.

---

## Configuration

**Follows** the paragraph beginning *"lanthorn reads
`~/.lanthorn/config.toml`…"*:

> An **exported transcript** is not quite what is on screen: lanthorn's own
> guidance is marked in the margin while you play, and written out with the word
> `Lanthorn:` in front of it, because a file has no margin and no colour.

---

## Anywhere the old flag spellings appear

The whole `--no-x` surface is gone across all four front-ends. **Grep the README
for `--no-` before cutting the release** — the two occurrences known today are
listed above, but any written between now and then need the same treatment.

| was | is |
|---|---|
| `--no-sound` | `--sound on\|off` |
| `--no-images` | `--images on\|off` |
| `--no-accel` | `--accel on\|off` |
| `--no-game-colours` | `--game-colours on\|off` |
| `--no-aux` | `--aux on\|off` |
| `--no-timed-input` | `--timed-input on\|off` |
| `--no-more` / `--no-page` | `--pager on\|off` |
| `--system-colours` | `--colour machine` |
| `--no-status` | removed — `--story-only` was already its name |

Worth a line somewhere if there is a natural home: `--colour
terminal|theme|machine` is new, and picks which of the three sources the story's
default page and ink come from. It selects a *regime*, not merely a first
preference: `--colour machine` gets a bare story file the machine's own screen
(what `--system-colours` did), and `--colour terminal` or `--colour theme` gets a
release floppy the plain one — your colours, resolved through the standard
table, exactly as the same story looks opened as a file. The artwork is the
disk's either way.
