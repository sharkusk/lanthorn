# The Commodore 64 Mysterious Adventures: measured data layout

## What this document is

> **Implemented (SQ-1414, 2026-09-09).** `crates/scott/src/c64.rs` reads all
> eleven of these releases into a `scott::Database`, and decodes their §8.2
> pictures; `crates/scott/tests/c64_specimens.rs` re-runs the oracle
> comparison this document describes. The specification has since absorbed
> every correction listed below, so a reader wanting the normative form should
> go to `scott-dialects-spec.md` §4.5, §4.6, §5.3, §6.2, §6.4, §9.3 and §10.4;
> this document remains the record of how those facts were measured.
>
> Three things the implementation found that are not in either document yet.
> **§4.2's dictionary reader is wrong for this family** — a `*` occupies one of
> the cell's own bytes rather than an extra one, and *Waxworks* carries cells
> that are nothing but spaces, which §4.2's space escape mis-aligns; the
> correct reading is plain (word length + 1)-byte NUL-padded cells keeping only
> §4.2's leading-NUL escape, and the six byte-identical titles come out exactly
> right under it and wrong under §4.2's. (This document's own "four-byte
> NUL-padded dictionary cells" is the same slip: the cells are five bytes at
> word length four.) **§5.3's Mysterious Commodore 64 dictionary repair is
> unnecessary here** — `ANY` and the six direction words are already noun cells
> 0-6 in all eleven, and applying it would truncate *The Time Machine*'s stored
> `NORTH` and `SOUTH`. And **the driver's pointer block has a seventh address**,
> the dictionary, at `$4917`/`$491C`, which agrees with §4.1's signature in all
> eleven; the spec's §6.2 now records it, this document's table does not.

Findings from SQ-1455, an investigation into how the eleven Commodore 64
*Mysterious Adventures* releases store their game tables, and specifically into
which of the three options
[`scott-dialects-spec.md`](scott-dialects-spec.md) §7.2 offers ("implement the
emulator and all twenty-one recognisers; support only the releases whose
catalogued pass count is zero; or require already-unpacked images as input")
applies to them.

**The answer is the second, and emphatically so. Nothing is crunched.** All
eleven game files are raw, uncompressed 6502 memory images in which every table
the spec describes sits in the clear, and the addresses of five of those tables
are readable as immediate operands at a fixed offset inside the file. This is
the *easiest* member of the whole memory-image family to load, not a hard one.

Everything below was measured against the specimens named in §"Specimens", using
the public 1541 disk-image layout, the published 6502 opcode encodings, and the
freely redistributable `.dat` conversions as an oracle. **No GPL interpreter
source was read for this work**, in keeping with
[`clean-room.md`](clean-room.md); the only secondary source consulted was
`scott-dialects-spec.md` itself, which is this repository's own clean-room
artefact.

## Specimens

**They are installed on disk — look there first.** Every Scott Adams dialect
specimen now lives under the gitignored `stories/scott-dialects/` tree, so a
later lane need not re-fetch anything:

| directory | holds | from |
|---|---|---|
| `stories/scott-dialects/c64/` | `mystadv.zip`, both D64 images, and `prg/<image>/*.prg` — the thirteen program files extracted for this work | `/if-archive/scott-adams/games/c64/mystadv.zip` |
| `stories/scott-dialects/mysterious-dat/` | the eleven reference-format conversions used here as the oracle | `/if-archive/scott-adams/games/scottfree/mysterious.tar.gz` |
| `stories/scott-dialects/spectrum/` | the twenty ZX snapshots of spec §10.3, four of them negative controls | `/if-archive/games/spectrum/mystsoft.zip` |
| `stories/scott-dialects/ti99/` | the twelve tokenised TI-99/4A releases of spec §10.2 | `/if-archive/scott-adams/games/ti99/scott_adams_ti99_games.zip` |

Each directory carries a `README.txt` naming its IF Archive path and the sha256
of every file in it. `stories/` is gitignored, so none of this is committed and
none of it may be staged.

The primary specimen for this investigation is
`/if-archive/scott-adams/games/c64/mystadv.zip`, 187,015 bytes, sha256
`08907754669907ab651bb5f0caf0e974d3d938563385e582297065f7cb943bcd` — the same
file §10.4 of the spec pins, re-fetched and re-hashed for this work. It holds
`MYSTADV1.D64` and `MYSTADV2.D64`, both 174,848-byte 35-track images with no
error map, whose digests likewise match §10.4.

The oracle is the eleven reference-format conversions from
`/if-archive/scott-adams/games/scottfree/mysterious.tar.gz`, dated 1994-10-05,
whose digests match §10.3's table.

## The container step is trivial

Each image carries an ordinary CBM DOS directory at track 18 sector 1 and one
closed program file (type `$C2`) per game, plus a `BOOT` program. Extracting
them needs only the sector geometry and block chaining the spec already
documents in §7.2; there is no copy protection, no non-standard interleave and
no bad-sector trick anywhere in either image.

`BOOT` loads at `$0801` and is a plainly tokenised BASIC menu — it prints
"MYSTERIOUS ADVENTURES … VERSIONS BY BRIAN HOWARTH, (C)1981-4" and `LOAD`s the
title the player picks. It contains no game data. §6.5's rule that **a loader
must be told which game is wanted** is confirmed: nothing in the container
distinguishes one game file from the others except its name.

| disk | game files |
|---|---|
| MYSTADV1.D64 | `BATON`, `TIME MACHINE`, `ARROW I`, `ARROW II`, `PULSAR 7`, `CIRCUS` |
| MYSTADV2.D64 | `EXPERIMENT`, `WIZARD OF AKYRZ`, `PERSEUS`, `INDIANS`, `WAXWORKS` |

## Per-title findings

Every game file loads at `$4000` and begins `4C E3 48` — a 6502 `JMP $48E3`,
the interpreter's own entry vector, followed by a long run of zero bytes that is
genuinely empty workspace. The zero run is what made this look like it might be
a cruncher stub; it is not. There is no decompressor: the byte at file offset
*n* + 2 is simply the byte at address `$4000` + *n*.

| PRG | load | bytes | sha256 (first 12) | 16-bit sum | raw tables? | `AUTO\0GO\0` at |
|---|---|---|---|---|---|---|
| `BATON` | `$4000` | 27,555 | `7dd995cff54e` | `$01FF` | **yes** | `$685F` |
| `TIME MACHINE` | `$4000` | 27,622 | `4297dd1e11d3` | `$BBDD` | **yes** | `$680F` |
| `ARROW I` | `$4000` | 30,534 | `b447f1205d2e` | `$AFE0` | **yes** | `$675F` |
| `ARROW II` | `$4000` | 34,054 | `76c14159832e` | `$1A8C` | **yes** | `$68FF` |
| `PULSAR 7` | `$4000` | 26,162 | `b8c93d0e45db` | `$1058` | **yes** | `$69E1` |
| `CIRCUS` | `$4000` | 24,400 | `e57513c61102` | `$1194` | **yes** | `$684F` |
| `EXPERIMENT` | `$4000` | 33,742 | `c13a8cee97de` | `$DB00` | **yes** | `$67C1` |
| `WIZARD OF AKYRZ` | `$4000` | 29,071 | `64bdf4ba53e0` | `$A7FE` | **yes** | `$6A6F` |
| `PERSEUS` | `$4000` | 27,785 | `f72fa51dc1f9` | `$E116` | **yes** | `$6851` |
| `INDIANS` | `$4000` | 28,174 | `faf18b1edee8` | `$0AD7` | **yes** | `$680E` |
| `WAXWORKS` | `$4000` | 28,644 | `adb73a99dc4b` | `$25CC` | **yes** | `$69D1` |

*Scheme characterisation:* **none.** Every file answers §4.1's plain
`41 55 54 4F 00 47 4F 00` signature at the offset shown, exactly once, with
back-off 0, in the raw bytes. Room descriptions, messages and item names are
plain ASCII, NUL-terminated, readable with `strings`. Neither the §5.1
compressed action table nor the §5.2 compressed text scheme appears anywhere,
which confirms §6.3's claim for this platform. No 6502 execution is needed at
any stage.

### The clinching test: the tables are byte-identical to the published conversions

For six of the eleven titles, the *entire* action table, room-description block,
room-connection block, message block, item-description block and item-location
table appear in the PRG as one contiguous byte string identical to the same
tables built from the title's `.dat` conversion — no re-encoding, no
transformation, byte for byte:

| title | actions | room descs | connections | messages | item descs | item locs |
|---|---|---|---|---|---|---|
| `BATON` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `ARROW I` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `ARROW II` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `EXPERIMENT` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `PERSEUS` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `WAXWORKS` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |

`CIRCUS` matches in actions, connections and item locations but is a different
text revision (twenty-four of its messages and eight of its item names differ
from the conversion). `TIME MACHINE`, `PULSAR 7`, `WIZARD OF AKYRZ` and
`INDIANS` have different header counts from their conversions and are simply
different releases of the same games — the same situation §6.1 records for the
ZX *Golden Baton*. In every one of those five the tables are still raw and still
decode cleanly; they just describe a different edition.

## The layout, measured

### One shared driver, eleven data payloads

The eleven files are the *same interpreter binary* with different data appended.
Comparing any two byte by byte, the first difference falls at `$4330`, `$43B6`
or `$48E7` — all inside the driver, all at per-title constants. The entire
region `$4000`–`$432F` is identical in all eleven.

### The system-message block is at a fixed address

`$4406` in every title, and the 790 bytes there hash identically across all
eleven. It is 44 CR-or-NUL-terminated strings; §4.4's terminator rules apply
unchanged, and §4.3's "read the first string there; if it is not `NORTH`, back
up one byte" search is satisfied at the address itself with no back-up needed.
Strings 0–5 are `NORTH`, `SOUTH`, `EAST`, `WEST`, `UP`, `DOWN`, confirming
§4.4's rule that Commodore 64 English releases have no separate direction-word
table and take those six from the head of the block.

### The header is at a fixed address, and word 0 is a `JMP` operand

`4C 19 4D` — `JMP $4D19` — sits at `$5DD6` in all eleven, and the header's first
count begins immediately after it at **`$5DD9`**. This explains a small mystery
in §4.5: the "word 0 is unused" of the early header shape is not a field at all
here, it is the two-byte operand of the instruction in front of the header. It
is constant (`$4D19`) across all eleven and is a serviceable recognition
signature in its own right.

Header field order follows §4.5 exactly, and the spec's four-way split of the
series is confirmed:

| title | shape | header bytes | items | acts | words | rooms | carry | start | treas | wlen | lamp | msgs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `BATON` | Mysterious C64 | 24 | 48 | 166 | 78 | 31 | 6 | 1 | 0 | 4 | 200 | 99 |
| `TIME MACHINE` | Mysterious C64 | 24 | 62 | 161 | 85 | 44 | 6 | 43 | 0 | 4 | 200 | 73 |
| `ARROW I` | Mysterious C64 | 24 | 64 | 150 | 90 | 52 | 5 | 1 | 0 | 4 | 32767 | 82 |
| `ARROW II` | Arrow 2 C64 | 24 | 90 | 176 | 82 | 65 | 9 | 1 | 0 | 4 | 400 | 87 |
| `PULSAR 7` | early | 26 | 88 | **195** | 145 | 45 | 6 | 1 | 0 | 4 | 200 | 74 |
| `CIRCUS` | Mysterious C64 | 24 | 65 | 165 | 97 | 36 | 6 | 1 | 0 | 4 | 150 | 72 |
| `EXPERIMENT` | early | 26 | 65 | 156 | 79 | 59 | 6 | 10 | 10 | 4 | 200 | 65 |
| `WIZARD OF AKYRZ` | Mysterious C64 | 24 | 49 | 199 | 85 | 40 | 6 | 1 | 4 | 4 | 65535 | 99 |
| `PERSEUS` | early | 26 | 59 | 165 | 130 | 40 | 6 | 1 | 0 | 4 | 200 | 96 |
| `INDIANS` | Ten Little Indians | 23 | 73 | 161 | 82 | 63 | 5 | 61 | 0 | 4 | **500** | 67 |
| `WAXWORKS` | early | 26 | 57 | 189 | 105 | 41 | 6 | 1 | 0 | 4 | 250 | 91 |

The "header bytes" column is new information: the header's *length* differs by
shape, and the action table begins immediately after it. The plain early shape
spends 26 bytes (eleven count words plus one filler word); the byte-packed
Mysterious shape spends 24; *Ten Little Indians* spends 23, which is an odd
number because its packing is genuinely at byte granularity and not, as §4.5
renders it, a word grid read half at a time (see "Corrections", below).

### The addresses are in the file — they need no catalogue

`$4000`'s `JMP` lands at `$48E3`. Three bytes in — past one `JSR` — a run of
twelve `LDA #imm` / `STA <zp>` pairs begins at **`$48E6`** and plants five table
addresses and one end marker into zero page, low byte then high byte, as literal
operands. The order and the zero-page slots are identical in all eleven:

| zero page | holds |
|---|---|
| `$32`/`$33` | room descriptions |
| `$2C`/`$2D` | room connections |
| `$34`/`$35` | messages |
| `$36`/`$37` | item descriptions |
| `$2E`/`$2F` | item locations |
| `$30`/`$31` | first byte after the item-location table |

The immediate operands are at addresses `$48E7`, `$48EB`, `$48EF`, … — every
fourth byte from `$48E7` to `$4913` — and pairing them recovers, exactly:

| title | actions | dictionary | rooms | conns | msgs | items | locs | loc end | pictures |
|---|---|---|---|---|---|---|---|---|---|
| `BATON` | `$5DEF` | `$685F` | `$6B76` | `$6DC5` | `$6E85` | `$75A1` | `$7881` | `$78B2` | `$78EF` |
| `TIME MACHINE` | `$5DEF` | `$680F` | `$6B6B` | `$6DFF` | `$6F0D` | `$74B9` | `$77CC` | `$780B` | `$7870` |
| `ARROW I` | `$5DEF` | `$675F` | `$6AC0` | `$6DF7` | `$6F35` | `$7515` | `$783A` | `$787B` | `$78E0` |
| `ARROW II` | `$5DEF` | `$68FF` | `$6C2E` | `$6FC4` | `$7150` | `$7701` | `$7BEA` | `$7C45` | `$7CAA` |
| `PULSAR 7` | `$5DF1` | `$69E1` | `$6EB9` | `$7101` | `$7215` | `$76C9` | `$7B34` | `$7B8D` | `$7BF2` |
| `CIRCUS` | `$5DEF` | `$684F` | `$6C19` | `$6E6B` | `$6F49` | `$7515` | `$786B` | `$78AD` | `$7912` |
| `EXPERIMENT` | `$5DF1` | `$67C1` | `$6A69` | `$6E1F` | `$6F87` | `$742F` | `$77CD` | `$780F` | `$7874` |
| `WIZARD OF AKYRZ` | `$5DEF` | `$6A6F` | `$6D6C` | `$6F14` | `$700A` | `$7851` | `$7B5D` | `$7B8F` | `$7BCC` |
| `PERSEUS` | `$5DF1` | `$6851` | `$6C7A` | `$701F` | `$7115` | `$7947` | `$7D07` | `$7D43` | `$7D8F` |
| `INDIANS` | `$5DEE` | `$680E` | `$6AED` | `$6F75` | `$70F5` | `$7607` | `$7995` | `$79DF` | `$7A44` |
| `WAXWORKS` | `$5DF1` | `$69D1` | `$6DAA` | `$710E` | `$720A` | `$7A50` | `$7EB8` | `$7EF2` | `$7F2F` |

The action and dictionary columns are not from the pointer block — the
dictionary is §4.1's signature hit, and the action table's start is the
dictionary address minus (action count + 1) × 16. That arithmetic lands on the
byte immediately after the header in **ten of eleven** titles, which is a free
cross-check on both the header shape and the action count.

### The tables chain contiguously, and the whole file is accounted for

Reading forward from the pointer addresses with §4.4's plain encodings — sixteen
byte action records, four-byte NUL-padded dictionary cells with `*` synonym
markers, NUL-terminated strings, six unsigned exit bytes per room, one location
byte per item — every table ends exactly where the next begins, in all eleven:

```
$4000  driver (shared)          $48E6  pointer block
$4406  system messages (44)     $5DD6  JMP $4D19
$5DD9  header                   →  actions  →  dictionary
       →  room descriptions  →  room connections  →  messages
       →  item descriptions  →  item locations
       →  61–113 zero bytes  →  picture data  →  EOF
```

That order is §4.3's **later** family, minus the room-image, item-flag and
item-image lists, which are absent (there is no room for them: the action table
starts immediately after the header). §6.1's claim that the Commodore 64
releases are not early-family is confirmed.

The picture data is a straight §8.2 opcode stream: for every one of the eleven
titles, walking it from the first `$FF` after the zero gap yields **exactly the
room count** of images and consumes the file to its final byte. Every byte of
every game file is thereby accounted for, which is about as strong a
confirmation as a layout claim can get.

## What a loader would have to know that it cannot read

Very little — which is the surprise. Against §4.6's list of roughly seventeen
per-release numbers, this family needs:

| fact | status here |
|---|---|
| load address | **in the file** (PRG bytes 0–1; always `$4000`) |
| dictionary address | **derivable** — §4.1 signature, back-off 0 |
| header address | **fixed** at `$5DD9`; the `JMP $4D19` at `$5DD6` marks it |
| item/action/word/room counts, carry, start, treasure, word length, lamp, messages | **in the header** |
| action table address | **derivable** — dictionary − (actions + 1) × 16 |
| room-description, room-connection, message, item-description, item-location addresses | **in the file**, at `$48E6` |
| picture-data address | **derivable** — first `$FF` at or after the address in `$30`/`$31` |
| picture count | **derivable** — equals the room count |
| system-message address | **fixed** at `$4406` |
| direction words | **derivable** — first six system messages |
| **header field order** | **must be tabulated** (one of four; see the table above) |
| **verb-cell / noun-cell counts** | **must be tabulated**, as §4.6 says — but see below |

The dictionary's cell count is the only genuinely undetermined quantity, and its
practical consequence is far smaller than §4.6 implies. Because the room
descriptions have their own pointer, a loader never needs the dictionary's
*end*; it needs only how many cells to split into verbs and nouns. Getting the
split wrong costs vocabulary, not layout. And the ambiguity at the boundary is
harmless in a second way: where the last dictionary cell is not NUL-terminated
it runs straight into room 0's text, so a few junk characters can prefix room
0's description — and room 0 is the copyright slot, which is never displayed.

**So a loader for these eleven needs exactly two tabulated numbers per title:
which of the four header shapes it uses, and its verb/noun cell split.**
Everything else is in the bytes.

## Two per-release repairs, in the §5.3 mould

- **`PULSAR 7`.** The header's action count is **195**, but only 191 records
  (action count 190) fit between the end of the header and the dictionary, and
  191 records validate perfectly — every vocabulary word below 150 × 150, every
  condition code in 0–19, and the table ending exactly on the dictionary's first
  byte — while 196 records overrun into the dictionary by 80 bytes. **The stored
  count is wrong and must be replaced by 190**, precisely the shape of §5.3's
  "the header's action count (243) is wrong and must be replaced by 236" for the
  German Commodore 64 *Gremlins*.
- **`TIME MACHINE`.** Its item-description block holds exactly **62**
  NUL-terminated strings between the item-description pointer and the
  item-location pointer, where the header's item count of 62 implies 63 records.
  The item-location table is a full 63 bytes. Item 62 therefore has a location
  but no stored description; a loader must supply an empty one rather than read
  a 63rd string, which would run into the location table.

## Corrections the spec should take

1. **§4.5 / §6.2 — word 0 of the early header is a `JMP` operand, not a field.**
   Worth saying, because it explains why the value is constant, and because
   `4C 19 4D` at `$5DD6` is a usable recognition signature for the whole family.

2. **§4.5 — the *Ten Little Indians* header is byte-packed, not word-packed, and
   the stated lamp rule recovers the wrong number.** §4.5 says "lamp turns =
   high byte of word 7". Read on a word grid, word 7 is `$F400` and its high byte
   is 244. Read as the bytes actually are — four count words, then four single
   bytes (max carried 5, start room 61, treasure count 0, word length 4), then a
   filler byte, then two ordinary little-endian words — the lamp is `F4 01` =
   **500**, which is exactly what the published conversion says, and the message
   count is `43 00` = 67, which the existing rule also yields. The existing rule
   is recovering the *low byte of the correct value* and calling it the answer.
   The byte-level reading is the one to specify.

3. **§6.1 — "their pictures are a separate Commodore 64 bitmap set" is wrong.**
   All eleven use the §8.2 Family B vector format, the same as the ZX releases:
   `$C0` move, `$C1` fill, `$FF` end, opcode-as-vertical line draw. The walk
   yields room-count images and ends on the last byte of the file for every
   title. Nothing in these files is a §8.3 bitmap.

4. **§6.4 — the Commodore 64 releases are first-person, not second-person.** The
   790-byte system-message block at `$4406` is byte-identical in all eleven and
   reads `I'm in a `, `I am carrying:`, `I'm not carrying it!`,
   `I don't see it here`, `That is beyond my power`, `I fell and broke my neck!`,
   `I'm carrying too much!`, `I'm DEAD!!`, `I can't go in that direction`. Not
   one of `You are in a`, `You can also see`, `You haven't got it` or
   `You are carrying` occurs anywhere in any of the eleven files. §6.4's table of
   Mysterious second-person wording may well be right for the ZX releases; it is
   demonstrably not a property of the series, and the section should say which
   platform it describes. Relatedly, §6.4 lists the visible-objects heading among
   three strings "not in the file at all" — `Things I can see:` **is** in the
   file, at `$442C`.

5. **§7.2 — add that this family needs no cruncher.** The section's "three honest
   options" framing is right, but a reader currently has no way to know that the
   largest single group of Commodore 64 releases on the IF Archive falls in the
   zero-pass subset. Saying so turns "support only the zero-pass releases" from
   an austere-sounding compromise into a route that covers eleven titles.

6. **§4.6 — record that pointer blocks exist.** The general claim that "a memory
   image contains no directory, no pointer word and no self-describing
   structure" holds as a statement about the *format*, but it invites an
   implementer to assume that a catalogue is the only route. For this family the
   driver's own initialisation code is a de facto pointer table at a fixed
   offset, and looking for one is worth doing before tabulating seventeen numbers
   for any new release.

7. **§10.4 — the two signature offsets quoted are sector-relative and only
   coincidentally meaningful.** §10.4 gives file offsets `$4739` and `$4B29`
   inside the raw D64s. After extraction the useful figures are the per-title
   dictionary addresses in the address table above. Worth replacing, since a
   reader could mistake the D64 offsets for something a loader would use.

## Recommendation

**Option two of §7.2 — support the zero-pass releases — with no emulator and no
cruncher recognisers.** For the Commodore 64 Mysterious Adventures this is not a
reduced-scope compromise; it is complete coverage of all eleven titles on both
disks. The work is:

1. A D64 reader (sector geometry, directory walk, block chaining) — already
   specified in §7.2 and already modelled by `crates/blorb/src/d64.rs`, which
   reads the same geometry for Infocom releases.
2. Strip the two-byte load address; the remainder is the memory image at
   `$4000`.
3. Find §4.1's `AUTO\0GO\0`; read the header at `$5DD9` with one of four field
   orders; read the five table addresses at `$48E6`; decode §4.4's plain
   encodings.
4. Apply the two repairs above and §5.3's existing Mysterious C64 dictionary
   repairs.

Per the hard rules, none of that belongs in `scott`, which takes no
dependencies and should keep receiving a decoded database; the D64 and PRG steps
belong in a host.

The one thing worth stating plainly for whoever picks this up: **the "no
recognisable cruncher signature" observation in SQ-1452 was correct and its
worrying interpretation was wrong.** There was no signature because there is no
cruncher. The `JMP` plus zero run at `$4000` is an entry vector and empty
workspace, and the entry vector points at an initialisation routine that hands
you the table addresses.
