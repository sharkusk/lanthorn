# The Scott Adams game-file dialects

## What this document is

This is an independent functional description of the file formats used by
Scott Adams / Adventure International adventure games and their relatives, and
of the runtime behaviours that distinguish them. It is written for anyone
implementing a loader or an interpreter, in any language. It carries this
repository's BSD-3-Clause licence.

It describes formats **as observed** — in named specimen files that anyone can
fetch and measure, and in the behaviour of named existing implementations, some
of which are GPL-licensed. It reproduces no code from any of them, and no
pseudo-code standing in for code. Every statement below is either a fact about
a byte layout, a procedure a conforming reader must carry out, or a behaviour
an interpreter must produce. Where a fact is irreducibly per-game, it appears
as a data table with a column saying how the entry could be re-derived from the
specimen bytes without the table.

The document was produced under the clean-room protocol described in
[`clean-room.md`](clean-room.md): the author read the sources listed below; the
implementer reads only this document and the specimens.

**Terms used throughout.** A *database* is the complete static description of
one game — its rooms, items, vocabulary, messages and script. The *reference
text format* (extension `.dat`, sometimes called the "ScottFree format" or the
"TRS-80 format") is the plain-ASCII interchange encoding of a database,
summarised in §2; it is the common target every dialect in this document
decodes to. A *dialect* is any other encoding of the same database. A *memory
image* is a byte array that is a copy of a home computer's RAM, in which a
game's tables sit at the addresses the original program used. A *container* is
whatever file format a memory image or a database arrives wrapped in — a
snapshot, a disk image, a cartridge dump.

## Sources

| Source | Licence | What was read | Version read |
|---|---|---|---|
| Gargoyle, `terps/scott/**` (a much-extended descendant of ScottFree) — <https://github.com/garglk/garglk> | GPL-2.0-or-later | Detection, the TI-99/4A loader and interpreter, the memory-image loaders, the compressed action and text schemes, the picture decoders, the per-game facts table | commit `9597add4091e5aaf6ebc31399b049158e12ca565` (2026-09-06), release 2026.1 |
| ScottFree 1.14 by Alan Cox — the `Definition` file, the accompanying notes, and the interpreter source, distributed as `/if-archive/scott-adams/interpreters/scottfree/ScottFree.tar.gz` | GPL-2.0-or-later | The reference text format and the condition/action opcode meanings summarised in §2; the string and auto-noun edge cases of §2.5 and §2.6, on which the `Definition` file is silent | archive file dated 1995-04-27, sha256 `a033a7e11c7f20e392df8aac1122c06cc10a4cad85df8c644c9bdbbdbf22d8b6` |
| Gargoyle, `terps/scott/ai_uk/decompressz80.*` — a reduced copy of the ZX Spectrum snapshot handling from libspectrum / Fuse, © Philip Kendall, Darren Salt, Fredrick Meunier | GPL-2.0-or-later | ZX Spectrum `.z80` snapshot structure | same commit as above |
| Gargoyle, `terps/c64diskimage/**` — the disk-image library by Per Olofsson, <https://paradroid.automac.se/diskimage/> | BSD-3-Clause | Commodore 1541 disk image geometry and directory structure | same commit as above |
| Gargoyle, `terps/unp64/**` — derived from Exomizer, © Magnus Lind, with modifications by iAN CooG and Petter Sjölund | zlib | What generic Commodore 64 depacking requires | same commit as above |
| Gargoyle, `terps/scott/saga/ciderpress.*` and `woz2nib.*` — based on CiderPress (<https://github.com/fadden/ciderpress>), fragments of a2tools by Terry Kyriacopoulos and Paul Schlyter, and `woz2dsk.pl` by LEE Sau Dan | see upstream projects | Apple II nibble decoding and WOZ conversion | same commit as above |

Two notes on the first row. The whole of that project's `terps/scott` tree is
GPL-2.0-or-later, including the files that carry no licence header of their own
(`ai_uk/sagadraw.c` among them), which inherit the aggregate's terms; there is
no permissively-licensed subdirectory inside it. The permissive components in
the table are separate directories — the disk-image library and the depacking
code — and are listed with their own terms.

A third note, on when each part was read. The table's first row was read again
at the same commit on 2026-09-09, the TI-99/4A loader and interpreter only, to
settle the three questions Appendix A records; §3.1, §3.4, §3.7, §3.8, §9.1 and
§11's TI-99/4A paragraph carry the results. Every count and offset those
sections quote was measured on the §10.2 specimens rather than taken from any
implementation.

A fourth note. **The same first row was read a third time, at the same commit,
on 2026-09-09**, for two things: the plain dictionary reading, to reconcile it
with the three findings the Commodore 64 *Mysterious Adventures* implementer
reported (§4.2, §5.3 and §8.2 now carry the results); and the **US S.A.G.A.
binary database**, which §11 previously refused by name and which §12 now
specifies. Every field width, offset, table order and worked example in §12 was
measured on the §12.13 specimens and checked against the reference-format twin
of the same game; nothing in it is a count or an offset taken on trust. Two
facts §12 states about the reference's behaviour rather than about the format
are labelled as such — the version/adventure dispatch pairs of §12.2 and the
runtime behaviours of §12.11 — and the reference was consulted for those and
for nothing else in that section.

**A fifth note. The first row was read a fourth time, at the same commit, on
2026-09-09**, for one thing only: the meaning of **command codes 89 and 90**,
which §12's releases use and their reference-format conversions do not, and which
§2.4's "codes 0 through 89" did not cover. What was taken from it is the identity
of command 90 — the picture it draws and the wait that follows — and the fact
that the reference implements neither code for any release of §12 while
consuming an operand for both. **The operand counts themselves were not taken
from it**; they are measured in §12.8 against the reference-format twins, they
contradict the reference for command 89, and §12.8 says so and shows the record
that settles it. The picture-file names in that section were read out of the
specimens' own disk catalogues.

**One non-GPL secondary source, added 2026-09-09.**
[`scott-c64-layout-findings.md`](scott-c64-layout-findings.md), this
repository's own BSD-3-Clause investigation of the eleven Commodore 64
*Mysterious Adventures* releases (SQ-1455, 2026-09-09), which read no GPL
interpreter source at all — only this document, the public 1541 disk-image
layout, the published 6502 opcode encodings, and the specimens themselves, with
the freely redistributable `.dat` conversions as an oracle. §4.5, §4.6, §5.3,
§6.1, §6.2, §6.4, §7.2 and §10.4 carry its results. **Every fact it reports was
re-measured on the §10.4 specimens before being written here**, including the
header address and its `JMP` marker, the pointer block's contents and order, the
byte-packed *Ten Little Indians* lamp value, the system-message block, and the
two per-release repairs of §5.3 — so those sections rest on the specimens, not
on the findings document. Where they contradict an existing GPL implementation's
behaviour, §6.1, §6.4 and §5.3 say which is right for these specimens and what
the implementation does instead, without quoting it.

The §10.5, §10.6 and §10.7 specimens were fetched and catalogued on the same
date from atarimania.com, the Asimov Apple II archive mirror and
myabandonware.com respectively; the identifications there are measurements on
those files against §4.1, §4.5, §7.3, §7.4, §8.4 and §8.5, and no implementation
was consulted for any of them.

Several of the container formats below are also documented publicly and
independently of any of the above, and an implementer should prefer the public
document where one exists: the ZX Spectrum `.z80` snapshot format (World of
Spectrum's `z80format.txt`), the Commodore `.d64` image layout (the
`d64.txt`/`G64` notes in the VICE documentation set), the Atari `.atr` header
(the "Atari Disk Image" note by Nick Kennedy), Apple II DOS 3.3 6-and-2 GCR
encoding (Apple's own *DOS Programmer's Manual*, chapter 3, and Beneath Apple
DOS), and the Apple II WOZ format (<https://applesaucefdc.com/woz/reference2/>).
Where this document and a public specification disagree, the public
specification is more likely to be right about the container and this document
is more likely to be right about which parts of it the game data actually uses.

## Contents

- §1 — Scope, and how the dialects relate
- §2 — The ScottFree `.dat` text format, as the common target; §2.5 and §2.6
  are the **`.dat` text format details** — the item auto-noun split and the
  quoted-string syntax — on which the `Definition` document is silent
- §3 — TI-99/4A tokenised releases
- §4 — Memory-image releases: locating the tables
- §5 — Compressed action tables and compressed text
- §6 — Mysterious Adventures native releases
- §7 — Containers and decompression, per platform
- §8 — SAGA picture data
- §9 — Runtime behaviour that varies by dialect
- §10 — Specimens
- §11 — Refusal cases
- §12 — The US S.A.G.A. binary database: the column-major format the American
  Atari 8-bit, Apple II and Commodore 64 disk releases carry
- Appendix A — How lanthorn uses this document

---

## 1. Scope, and how the dialects relate

One database, many encodings. Every game in this document is the same kind of
artefact: a table of rooms with six exits each, a table of items with a start
location each, a flat verb/noun dictionary, a pool of messages, and a script of
condition/command lines keyed by verb and noun. What differs between dialects
is only how those tables are written down, where they are found, and a handful
of runtime conventions.

The dialects, in the order this document specifies them:

| Dialect | What the file is | How the tables are found |
|---|---|---|
| Reference text format (`.dat`) | ASCII decimal numbers and quoted strings | Sequentially: a twelve-number header, then tables in fixed order |
| TI-99/4A tokenised | A memory image based at address `0x0380` | A signature scan finds a fixed anchor; a 34-byte header at a fixed distance from it holds eleven table pointers |
| Adventure International memory image | A memory image of a Commodore 64, ZX Spectrum, Atari 8-bit or Apple II | A dictionary signature scan, then a header of counts near it, then tables at offsets that are partly derivable and partly per-release |
| Compressed action table | The same, with the script bit-packed | As above |
| Compressed text | The same, with strings bit-packed | As above |
| Mysterious Adventures | The same, with a different header shape and a different system-message set | As above |

The TI-99/4A dialect is the only one of these that is fully self-describing.
Every other memory-image dialect needs some per-release knowledge, and §4 says
how much of it can be recovered from the specimen.

Graphics (§8) are orthogonal: a memory-image release may or may not carry
picture data, and the picture data has its own format per platform.

**A note on offsets that runs through the whole document.** Every table-layout
rule below is stated against a *decompressed memory image* — the byte array a
platform's RAM held while the game was running. It is never stated against an
offset into a distributed file. Distributed files are compressed (§7), and the
compression is not order-preserving in any useful sense: a run-length scheme
passes literal text through unchanged, so a dictionary signature is often still
*findable* in the raw file, but the offset at which it is found bears no fixed
relation to the address it will have after decompression. Decompress first,
then locate.

---

## 2. The ScottFree `.dat` text format, as the common target — with format details

This section is a summary, not a full specification — the format is documented
in the `Definition` file distributed with ScottFree, and that document is the
primary reference for it. It is restated here because every dialect below is
specified by its differences from this format, and because §2.5 and §2.6
document two points on which the `Definition` file is silent.

### 2.1 Lexical structure

The file is a stream of two token kinds separated by arbitrary whitespace:
decimal integers (optionally signed) and double-quoted strings. There are no
comments and no section markers; a reader knows what to expect next purely from
position and from the counts in the header.

### 2.2 Header

Twelve integers. The first is unused. The remaining eleven are, in order:
number of items, number of actions, number of words, number of rooms, maximum
items carried, starting room, number of treasures, significant word length,
light-source duration in turns, number of messages, treasure room.

**Every "number of" value is the highest valid index, not a cardinality.** A
declared item count of 65 means items 0 through 65 exist — sixty-six of them.
This convention holds for every count in every dialect in this document.

### 2.3 Tables, in file order

1. **Actions**, one line of eight integers each, count + 1 of them.
   - integer 0 is `verb x 150 + noun`;
   - integers 1 through 5 are conditions, each `code + value x 20`;
   - integer 6 is `command1 x 150 + command2`;
   - integer 7 is `command3 x 150 + command4`.

   A verb of 0 marks the line as automatic rather than player-triggered, and
   the noun field then holds a percentage chance instead of a noun.
2. **Dictionary**, count + 1 pairs of quoted strings, a verb and a noun each
   time. The two lists are separate vocabularies that happen to be interleaved
   in the file; the shorter is padded with empty entries to the same length. A
   word whose first character is `*` is a synonym of the nearest preceding word
   that is not so marked, and matching it yields that earlier word's index.
   Matching is case-insensitive and compares only the first *word-length*
   characters.
3. **Rooms**, count + 1 records of six integers then a quoted string. The six
   are exits to north, south, east, west, up and down, in that order; 0 means
   no exit. A description whose first character is `*` is to be printed
   literally, without the interpreter's "I'm in a" prefix; the asterisk itself
   is not printed.
4. **Messages**, count + 1 quoted strings.
5. **Items**, count + 1 records of a quoted string then an integer. A text
   whose first character is `*` marks a treasure, and the asterisk *is*
   printed. A trailing `/WORD/` names the noun by which the item can be taken
   and dropped; see §2.5. The integer is the item's starting room; 0 means not
   in play, and 255 means carried.
6. **Action comments**, count + 1 quoted strings, one per action line. Purely
   documentary; a reader may discard them. Databases converted from tape
   editions often omit this table entirely.
7. **Trailer**, three integers: a version, an adventure number, and a magic
   number. Optional in practice.

### 2.4 Condition and command codes

The twenty condition codes and the command codes 0 through 89 are enumerated in
the `Definition` file and are not restated here; the dialect sections below
name a code only where a dialect's meaning differs from it. Two conventions are
worth stating because later sections depend on them: condition code 0 is not a
test at all but a way of smuggling an operand into the line for a command to
consume, and command codes 1-51 and 102 upward print a message rather than
performing an action.

### 2.5 Item auto-noun: the trailing `/WORD/` marker

An item's text may carry a suffix naming the single dictionary noun by which
the player can take or drop that item directly. The rule is exact, and three of
its edges are commonly got wrong.

**The rule.** Let *T* be the item's text as read from the file.

1. Find the position of the **first** `/` in *T*. If there is none, the item
   has no auto-noun and its displayed text is *T* unchanged.
2. Let *R* be the remainder of *T* from that slash to the end of the string.
   If *R* is **exactly** `//` or **exactly** `/*`, the item has no auto-noun
   **and its displayed text is *T* unchanged** — the trailing `//` or `/*`
   stays in what the player sees.
3. Otherwise the item's displayed text is *T* truncated at that first slash,
   with the slash itself removed. The auto-noun begins at the character after
   that slash and runs to the **next** `/` if there is one, or to the end of
   the string if there is not.

No case conversion is applied when the marker is read. No whitespace is
trimmed, before or after the slash. Comparison against a typed word is
case-insensitive and truncated to the header's word length, so an auto-noun
longer than the word length has its excess characters ignored, and one shorter
than the word length must match the typed word over its whole length.

**Why "first slash", with a distinguishing example.** A text of the form
`Luger/LUGER/GUN/` occurs in real databases. Splitting at the first slash gives
displayed text `Luger` and auto-noun `LUGER`. Splitting at the *last* slash
gives displayed text `Luger/LUGER/GUN` and an empty auto-noun — the item then
shows a name with slashes in it and cannot be taken by name at all. The first
reading is correct.

**Why "exactly `//`", with a distinguishing example.** The check in step 2 is
against the whole remainder, not against a prefix. A text of `lamp//` has
remainder `//` and so keeps its literal trailing slashes and gets no auto-noun.
A text of `lamp//X` has remainder `//X`, which is not exactly `//`, so the rule
falls through to step 3: the displayed text becomes `lamp`, and the auto-noun
runs from after the first slash to the next slash — a span of zero characters,
so the auto-noun is the empty string. An empty auto-noun is present but can
never match a typed word, so the item behaves as though it had none, while
still having lost its suffix from the display. A prefix test would instead
treat `lamp//X` as "no auto-noun" and leave `//X` on screen.

**Missing closing slash.** `combination torture chamber/rec room` is a real
item text from a public specimen: a first slash with no second one. The
displayed text becomes `combination torture chamber` and the auto-noun becomes
`rec room` — a two-word auto-noun that can never match a single typed word, but
which does silently remove two words from what the player sees. This is the
format behaving as specified, not a defect to repair.

**Two more edges.** The `*` treasure marker is examined on the text *before*
the auto-noun split, so `*Pot of RUBIES*/RUB/` is a treasure whose displayed
text is `*Pot of RUBIES*` and whose auto-noun is `RUB`. And a word length of 0
in the header would make every comparison vacuously true; no real database has
one, and a conforming loader should treat 0 as "compare the whole word".

### 2.6 Quoted string syntax

**Delimiters.** A string token begins at the next non-whitespace byte, which
must be `"`. Any other byte there is a malformed file. Whitespace before the
opening quote — spaces, tabs, carriage returns, newlines, vertical tabs, form
feeds — is skipped and discarded.

**Termination.** The string ends at the next `"` that is *not* immediately
followed by another `"`. The byte after that closing quote is not consumed and
is available to the next token. End of file before that point is a malformed
file.

**Escaped quote.** Two consecutive `"` bytes inside a string are an escape for
one literal `"` character. Both bytes are consumed and one `"` is produced.

**Backtick.** A backtick byte (0x60) anywhere inside a string is produced as a
`"` character. This is how the format writes quotation marks in game text, and
it is common — every one of the twelve original games uses it. An item text
reading `Rusty axe (Magic word BUNYON on it)/AXE/` with backticks around the
magic word displays as `Rusty axe (Magic word "BUNYON" on it)`.

**Order of the two rules matters.** The doubled-quote test is applied to a byte
*before* the backtick substitution, and the backtick substitution never
produces a delimiter. So a backtick can never close a string, and a `"`
produced from a backtick can never pair with a following literal `"` to form an
escape. Consider four bytes: backtick, `"`, `"`, `X`. The backtick yields one
`"` character; the two literal quotes are then an escape yielding a second `"`
character; `X` follows; the string so far is `""X`. Under the opposite order —
substituting backticks first — the three quote characters would read as one
escaped quote followed by an opening delimiter, and the string would end in the
wrong place.

**Newlines.** A string may span any number of lines. A newline byte inside a
string is part of the string and is preserved. Real databases rely on this:
message text is routinely written across two or three source lines and is
expected to reflow, so an interpreter typically treats an embedded newline as
whitespace at display time rather than as a hard break — but that is a
presentation decision, and the *loader* must preserve the byte.

**Carriage returns.** The format predates any concern about line endings. A
database authored on a system using CR+LF contains a CR byte before each
embedded newline, and a strict reading preserves it. A loader that intends to
produce identical strings from CR+LF and LF encodings of the same database must
discard CR bytes inside strings, and should document that it does so; this is
the one place where a conforming loader may reasonably differ from a strict
byte-preserving reading.

**Non-ASCII bytes.** The format has no character-set declaration, and bytes
above 0x7F occur in some databases. A loader targeting a Unicode string type
must choose an interpretation — treating the file as Latin-1, or substituting a
placeholder character — and must not fail the load over it.

---

## 3. TI-99/4A tokenised releases

The Texas Instruments TI-99/4A releases of the twelve original Adventure
International games are the best-behaved dialect in this document: the file is
a raw memory image, every table is reached through a pointer in a header, and
nothing anywhere is per-release. A complete loader for this dialect needs no
game-specific knowledge at all.

### 3.1 Detection

The signature is ten bytes:

```
30 30 30 30 00 30 30 00 28 28
```

— ASCII `0 0 0 0`, NUL, `0 0`, NUL, `( (`. It is **searched for anywhere in the
file**; the first occurrence wins. It is not at a fixed file offset, because
the distributed files carry container prefixes of differing sizes.

The signature is not part of any game table. It falls inside a fixed-size
character-pattern block that every release of this dialect carries between its
title screen and its data header, and its value is a run of glyph bitmaps. What
makes it usable is that the **distance from the signature to the data header is
constant**: the header begins 0x317 bytes after the first byte of the
signature.

Define the **baseline** *B* = (offset of the signature) − 0x589. Then:

| item | file offset | length |
|---|---|---|
| title screen | *B* + 0x80 | 960 bytes |
| character-pattern block containing the signature | *B* + 0x440 | 0x460 bytes |
| **data header** | *B* + 0x8A0 | 34 bytes |

and a **stored address** *A* — the form every pointer in this dialect takes —
resolves to file offset *A* − 0x380 + *B*. Equivalently: the memory image is
based at address 0x0380, and *B* absorbs whatever container prefix the file
carries. A loader therefore does not need to recognise or strip any container.

The distributed TI-99/4A game files carry a 128-byte file-descriptor header
whose first ten bytes are the TI filename padded with spaces. In those files
the signature lands at 0x589 and *B* is 0, verified across all twelve original
titles.

**Validation before acceptance.** Detection is speculative, so a loader must
check before committing: *B* must not be negative; the header must lie wholly
within the file; and all eleven pointers in the header must resolve to offsets
within the file. A failure of any of these means "not this dialect", not "a
corrupt file" — another dialect's detector has yet to run.

**Validating a pointer validates a table's start, never its extent, and a read
that falls outside the file yields zero.** Nothing in this dialect states where
a table ends: a pointer table's length is implied by a header count and its
data's length by the next entry, and neither is checked against the file. So a
file can pass every test above and still be short of a table's last bytes. The
conforming behaviour is not to refuse it and not to read past the end, but to
treat any stored 16-bit value whose two bytes do not both lie within the file as
**0**, and to let each consumer take that 0 at face value — for a dispatch
entry (§3.7), zero already means "this verb has no records".

*Worked example, `adv07.fiad` (Mystery Fun House).* The file is 10,594 bytes,
0x2962, with *B* = 0. Its header gives a highest verb index of 83 and an
explicit-action pointer that resolves to file offset 0x28BC, so its dispatch
table is 84 words spanning 0x28BC through 0x2963 inclusive — **two bytes past
the end of the file**. All eleven header pointers resolve inside the file and
every other table fits; only this last entry does not. Entry 83 therefore reads
as 0 and verb 83 has no records. Nothing is lost by that: entries 0 through 82
are intact (42 of them are non-zero, the highest chain belonging to verb 81),
and verb 83's *dictionary* slot is unusable too — the difference between its
pointer and the next runs to thousands of characters, far past the 20-character
limit of §3.6, so the slot is empty and no typed word can ever select it. A
reader that instead refuses the file, or that reads two bytes of whatever
follows it in memory, is wrong in a way this specimen will show.

### 3.2 Container and endianness

The file is a raw memory image with no dialect-specific wrapper. **Every
multi-byte quantity in this dialect is big-endian**, with the most significant
byte first: the light-duration field, all eleven header pointers, and every
entry of every pointer table. There are no other multi-byte quantities;
everything else is a single byte.

### 3.3 Header

Thirty-four bytes at *B* + 0x8A0.

| offset | width | field |
|---|---|---|
| 0 | 1 | highest item index |
| 1 | 1 | highest verb index |
| 2 | 1 | highest noun index |
| 3 | 1 | the "red room" (the room a dead player is moved to), which is also the highest room index |
| 4 | 1 | maximum items carried |
| 5 | 1 | starting room |
| 6 | 1 | number of treasures — **present but not authoritative**; see below |
| 7 | 1 | significant word length |
| 8 | 2 | light-source duration in turns, big-endian |
| 10 | 1 | treasure room |
| 11 | 1 | unassigned; no meaning is known |
| 12 | 2 | pointer: object table — **declared but of unknown layout**; see §11 |
| 14 | 2 | pointer: initial item locations |
| 16 | 2 | pointer: noun-to-item link table |
| 18 | 2 | pointer: item description pointer table |
| 20 | 2 | pointer: message pointer table |
| 22 | 2 | pointer: room exit table |
| 24 | 2 | pointer: room description pointer table |
| 26 | 2 | pointer: noun pointer table |
| 28 | 2 | pointer: verb pointer table |
| 30 | 2 | pointer: explicit (verb-triggered) action dispatch table |
| 32 | 2 | pointer: implicit (automatic) action block |

Counts are highest-index values, exactly as in the reference text format.

**Differences from the reference text format's twelve numbers.** The leading
unused number is absent. There is no action count: actions are variable-length
records reached through per-verb chains and terminated in band, and nothing in
the format counts them. There is no combined word count: verbs and nouns have
separate counts, and a reader that needs the reference format's single count
must take the larger of the two. There is no message count: it is derived (see
§3.5). The room count is spelled as the dead-room number. The light duration is
sixteen bits here rather than an unbounded decimal. The treasure count is
present but should be ignored in favour of counting items whose description
begins with `*`, since the two are not guaranteed to agree. Against those
losses the header gains eleven table pointers, where the text format has none
and relies on tables following one another in a fixed order.

### 3.4 Table shapes

Two shapes exist, and everything is reached through a header pointer. Nothing
is at a fixed offset except the header and the title screen, and nothing is
scanned for except the detection signature.

**Flat byte arrays.** Located directly at the resolved pointer, one or more
bytes per entry, no header and no terminator; the extent is implied by a header
count.

- **Room exits** — six bytes per room for rooms 0 through the highest room
  index: north, south, east, west, up, down, in that order. 0 means no exit.
- **Initial item locations** — one byte per item, holding exactly the three-way
  value the reference format's per-item integer holds: a room number, **0
  meaning not in play**, and **255 meaning carried at the start of the game**.
  There are no other reserved values. Measured over the twelve §10.2 specimens,
  every byte in this table is 0, 255, or a room number no greater than the
  header's highest room index; 255 occurs in five of them, on exactly the items
  whose reference-format twins carry −1 or 255 — `adv03` item 48 (the bomb
  detector), `adv05` item 16 (Tent STAKE), `adv07` items 3 (Shoes), 25 (Watch)
  and 56 (chewing gum), `adv08` items 4 (Empty canteen) and 8 (Unlit
  flashlite), `adv10` item 17 (Watch).

  This byte is both the item's starting location and the value the "still in its
  initial room" and "has been moved" conditions (§3.7, opcodes 200 and 201)
  compare against for the rest of the game — and they compare against it
  literally, 255 included. An item that starts carried is therefore "still in
  its initial room" for exactly as long as the player keeps hold of it, and
  counts as "moved" the moment it is put down anywhere; picking it up again
  makes it unmoved once more. That is the same arithmetic the reference format
  performs on the same two values, so a database converted either way behaves
  identically.
- **Noun-to-item link table** — one byte per item, holding a *noun index*. A
  non-zero value names the noun by which that item may be taken and dropped;
  zero means the item has no such name. This is this dialect's spelling of the
  text format's trailing `/WORD/` marker: the word is a dictionary index held
  out of line, and the item's description text carries no marker at all. A
  reader must apply synonym folding (§3.6) when comparing a parsed noun against
  a link byte.

**Pointer tables.** Arrays of big-endian 16-bit stored addresses, used for room
descriptions, messages, item descriptions, verbs and nouns. Entry *i* gives the
start of item *i*'s data and **entry *i* + 1 gives its end**. There is no length
field and no terminator; every extent is the difference between two consecutive
pointers. Two consequences follow and both are load-bearing: each such table
carries one entry beyond its highest index, as an end sentinel; and the first
entry points at the first byte after the table itself, so the data a table
addresses immediately follows it.

### 3.5 Strings, and the derived message count

Within a string's extent the bytes form a sequence of **length-prefixed
chunks**: one byte giving a character count *L*, then exactly *L* bytes of
characters, then the next chunk. Chunks are consumed until the extent is
exhausted. The decoded string is the chunks' characters concatenated **with a
single space between consecutive chunks**. The separator is implied, never
stored; no space follows the last chunk. There is no terminator byte and no
compression.

A chunk length of 0, or greater than 100, marks the string as malformed. A
string that decodes to nothing should be replaced by a single-period
placeholder rather than an empty value, because the interpreter's description
and message paths treat an empty description as an error and a leading period
as "nothing to show".

Characters are 7-bit ASCII with four display substitutions specific to this
dialect: byte 0x40 renders as a copyright sign followed by a space (two output
characters from one stored byte); 0x7B renders as `ä`; 0x7D renders as `ü`;
0x0C renders as `ö`.

The text format's two leading-asterisk conventions both survive: an item whose
decoded description begins with `*` is a treasure and the asterisk is printed,
and a room whose decoded description begins with `*` is printed literally
without the "I'm in a" prefix.

**Message count.** It is derived, not stored. Resolve the message pointer
table's address, read its first entry, resolve that, and take the byte
difference divided by two: that is the number *N* of words in the table.
Because the last word is an end sentinel, the real messages are numbered 0
through *N* − 2. A reader should treat *N* − 2 as the highest real message index
and must never let a failed decode past the end propagate as a hard error, since
the message-printing opcode range is validated against whatever count the
reader derived.

**Item description count.** The same derivation is available from the item
description pointer table, but the header's highest item index is authoritative
where the two disagree.

### 3.6 Dictionary

Verbs and nouns live in two entirely separate pointer tables, located
independently through two header pointers. They are not interleaved.

Words are stored as **bare characters, with no terminator and no padding**;
each word's length is the difference between consecutive pointers. This is the
sharpest storage difference from the text format, where each dictionary word
occupies a fixed field. A word of 20 characters or more should be rejected,
leaving the slot empty. A zero-length word (two equal consecutive pointers) is
not expected and must be treated as an empty entry rather than allowed to stall
a reader.

The verb table must contain at least (highest verb index + 2) words and the
noun table likewise, because the last real word's length depends on the
following entry.

**Reconciling the two counts.** A reader producing a reference-format database
must pad the shorter dictionary up to the longer one's length with entries that
can never match — a single period is the conventional filler — so that both can
be indexed over the same range.

**Synonyms.** The text format's convention survives verbatim: a stored word
whose first character is `*` is a synonym of the nearest preceding word not so
marked, and matching it yields that earlier word's index. The asterisk is not
part of the compared text. Matching is case-insensitive over the first
*word-length* characters.

In a real specimen — the TI-99/4A release of Adventureland — the first twenty
verbs decode as `AUT GO *ENT *RUN *WAL *CLI JUM AT CHO *CUT GET *TAK *PIC *CAT
LIG *BUR *IGN INV DRO *REL`, and the first twenty nouns as `ANY NOR SOU EAS WES
UP DOW NET FIS AWA MIR AXE *AX WAT BOT *CON HOL INV SPI WIN`. Words are already
truncated to the header's word length (3, for this game) as stored.

### 3.7 Action encoding

**This is where the dialect departs from the reference format completely.**

A reference-format action is a fixed record of eight numbers with multiplicative
packing, five condition slots whatever the line needs, and command operands
smuggled through spare condition slots. This dialect keeps none of that.
Records are variable length and terminated in band. Conditions and commands
share one linear opcode stream and may be freely interleaved. Operands are
inline bytes immediately after their opcode; there is no operand-smuggling
mechanism. Nothing is multiplied by 150 or by 20 anywhere. There is a
branch-on-failure mechanism the reference format has no equivalent for. And the
verb is not stored in the record at all — it is expressed by which chain the
record belongs to.

**Nothing in this dialect is bit-packed.** Every opcode is a whole byte and
every operand is a whole byte; the classes are told apart by numeric range, not
by bit fields. The only multi-byte values in the entire dialect are the
big-endian pointers and the 16-bit light duration.

**Explicit (verb-triggered) records.** The explicit dispatch table is a pointer
table with exactly one entry per verb index, 0 through the highest verb index.
It has no sentinel and does not obey the "next entry is the end" rule; each
word stands alone. A word of zero means the verb has no records. A non-zero
word resolves to the head of that verb's chain. A verb index beyond the highest
verb index is reachable, because the effective word count is the larger of the
two dictionary counts, so a reader must bounds-check before indexing.

Each record is:

```
byte 0        noun index this record matches; 0 means "any noun"
byte 1        link L: the distance from byte 1 to the next record's byte 0,
              or 0 if no record follows this one in the chain
bytes 2...    opcode stream, delimited by its own end-of-record opcode
```

**Byte 1 is a link, not an extent, and reading it as an extent is the one
mistake this encoding invites.** The next record begins at the current record's
start plus 1 + *L*, so a non-zero link does happen to size this record as well
as locate the next one; **a link of 0 says only that no record follows**, and
says nothing whatever about how long this record's own opcode stream is. The
stream always begins at byte 2, and where it ends is decided in band and at run
time: at the end-of-record opcode 255, or earlier at the first condition that
does not hold (see "Success, failure and the handler stack" below).

So **a record whose link byte is zero is the last of its chain and is a real
record in every other respect** — eligible to match, and carrying a full opcode
stream that runs exactly like any other record's. Take the link for a length and
that last record appears to have an empty stream; an empty stream can never
reach opcode 255, so it can only fail, and every chain in every game then ends
in a record that always fails. The damage is worst where a chain has exactly one
record, which is how this dialect spells several of the verbs an implementer
might otherwise assume are built in: in Adventureland, `INVENTORY`, `QUIT`,
`STOP` and `SCORE` are each a single link-0 record, and under the wrong reading
all four answer "I can't do that yet." rather than doing what they say. §3.8
works that case through byte by byte.

A reader that stores each record as a slice, rather than as a pointer into the
block, therefore has to recover the last record's extent by walking its opcode
stream — by the operand counts tabulated below — to the 255 that ends it.
Measured over the twelve §10.2 specimens (1,870 explicit records reached through
twelve dispatch tables, 378 automatic records, 448 of them link-0), that walk
terminates on a 255 every time, and for every record with a non-zero link it
lands on exactly the byte the link predicts. The two ways of ending a record
agree wherever both are present; the link is redundant except as the address of
the next record.

**Implicit (automatic) records** have identical geometry, with byte 0
reinterpreted as a percentage chance, 0 to 100, that the record runs on any
given turn. They form one chain reached directly from the header pointer, with
the same "link zero ends the chain" rule and the same warning about it. **If the
very first byte of the implicit block is zero the game has no automatic actions
at all** and the block must not be walked — which makes a genuine leading
0%-chance record unrepresentable; the empty reading is the correct one.

**Opcode ranges.**

| leading byte | class | operand bytes |
|---|---|---|
| 0-182 | print the message with this number | 0 |
| 183-201 | condition test | 1, except two which take 0 |
| 202-211 | unassigned | unknown — see §11 |
| 212-254 | command | 0, 1 or 2 |
| 255 | end of record, and the record succeeds | 0 |

A byte in the message range that exceeds the derived message count + 1 is not a
message and must be treated as unrecognised.

**Conditions.** Each takes one operand byte unless noted. The right-hand column
gives the equivalent reference-format condition code, since the numeric orders
are unrelated.

| byte | the record continues only if | ref. code |
|---|---|---|
| 183 | item *p* is carried | 1 |
| 184 | item *p* is in the current room | 2 |
| 185 | item *p* is carried or in the current room | 3 |
| 186 | item *p* is not in the current room | 5 |
| 187 | item *p* is not carried | 6 |
| 188 | item *p* is neither carried nor in the current room | 12 |
| 189 | item *p* is in play (location is not 0) | 13 |
| 190 | item *p* is not in play | 14 |
| 191 | the player is in room *p* | 4 |
| 192 | the player is not in room *p* | 7 |
| 193 | bit flag *p* is set | 8 |
| 194 | bit flag *p* is clear | 9 |
| 195 | the player is carrying something — **no operand** | 10 |
| 196 | the player is carrying nothing — **no operand** | 11 |
| 197 | the current counter is at most *p* | 15 |
| 198 | the current counter is greater than *p* | 16 |
| 199 | the current counter equals *p* | 19 |
| 200 | item *p* is still in its initial room | 17 |
| 201 | item *p* has been moved from its initial room | 18 |

The two zero-operand conditions break any assumption that a condition always
carries an operand.

**Commands.**

| byte | effect | operands | ref. code |
|---|---|---|---|
| 212 | clear the main text window | 0 | 70 |
| 213 | unassigned | ? | — |
| 214 | turn automatic inventory listing on | 0 | none |
| 215 | turn automatic inventory listing off | 0 | none |
| 216, 217 | no effect | 0 | — |
| 218 | push a failure handler | 1 | none |
| 219 | take item *p*, respecting the carry limit; on refusal print the "carrying too much" message and fail the record | 1 | 52 |
| 220 | drop item *p* into the current room | 1 | 53 |
| 221 | move the player to room *p* and redescribe | 1 | 54 |
| 222 | remove item *p* from play | 1 | 55 / 59 |
| 223 | set darkness | 0 | 56 |
| 224 | clear darkness | 0 | 57 |
| 225 | set bit flag *p* | 1 | 58 |
| 226 | clear bit flag *p* | 1 | 60 |
| 227 | set bit flag 0 | 0 | 67 |
| 228 | clear bit flag 0 | 0 | 68 |
| 229 | kill the player | 0 | 61, but see §9.4 |
| 230 | put an item into a room | 2: **room first, then item** | 62, operands reversed |
| 231 | end the game immediately | 0 | 63 |
| 232 | print the score, ending the game if all treasures are stored | 0 | 65 |
| 233 | list the inventory | 0 | 66 |
| 234 | refill the light source: restore its duration to the header value, place it in the inventory, clear the light-out flag | 0 | 69 |
| 235 | save the game | 0 | 71 |
| 236 | exchange the locations of items *p1* and *p2* | 2 | 72 |
| 237 | take item *p*, ignoring the carry limit | 1 | 74 |
| 238 | move item *p1* to the location of item *p2* | 2 | 75 |
| 239 | no effect | 0 | 0 |
| 240 | redescribe the current room | 0 | 64 / 76 |
| 241 | no effect | 0 | — |
| 242 | **increment** the current counter | 0 | none |
| 243 | decrement the current counter, floored at 0 | 0 | 77 |
| 244 | print the current counter, then a space | 0 | 78 |
| 245 | set the current counter to *p* | 1 | 79 |
| 246 | add *p* to the current counter | 1 | 82 |
| 247 | subtract *p* from the current counter, floored at -1 | 1 | 83 |
| 248 | exchange the current room with the stored room | 0 | 80 |
| 249 | exchange the current room with saved-room slot *p*, and redescribe | 1 | 87 |
| 250 | exchange the current counter with counter slot *p* (slots 0-15; higher values clamp to 15) | 1 | 81 |
| 251 | print the noun the player typed | 0 | 84 |
| 252 | print the noun the player typed, then a newline | 0 | 85 |
| 253 | print a newline | 0 | 86 |
| 254 | pause for about one second | 0 | 88 |

Two operand-order facts are load-bearing. **Opcode 230 takes the room first and
the item second**, the reverse of the reference format's equivalent; opcodes 236
and 238 take the acting item first, matching it. There is no picture-drawing
opcode in this dialect.

**Success, failure and the handler stack.** A record's result is failure unless
it reaches opcode 255, which sets success. The first condition that does not
hold terminates the record with failure immediately — so commands earlier in
the stream have already taken effect and are not undone. Opcode 219 can also
terminate a record with failure after printing its message.

Opcode 218 pushes a **failure handler** whose single operand byte encodes a
resume position: *target* = (the position of the operand byte within the opcode
stream) + (the value of the operand byte). Positions are counted from the first
byte of the opcode stream, and an operand value of 0 therefore targets the
operand byte itself. Handlers stack, to a depth of 32; exceeding that is a
malformed record. When a record terminates in failure and the stack is not
empty, the most recently pushed handler is popped and execution resumes at its
target, the result still failure until some later opcode 255 sets it. Reaching
opcode 255 clears the whole stack, so a handler can only ever be entered by
failure, never by falling into it. The construct is an if/else.

**Record selection.** For a parsed verb and noun, the verb's chain is walked
from its head. A record matches if its noun byte equals the parsed noun or is 0.
A matching record runs; **if it succeeds the search ends**, and if it fails the
walk continues — a failed record does not end the search. Non-matching records
are skipped. Three outcomes must be distinguishable to the caller: succeeded;
ran to the end of the chain having matched at least one record; and ran to the
end having matched nothing (which includes a verb with a zero dispatch entry).
The two failure outcomes produce different messages (§9.4).

**Implicit execution.** Every record in the implicit chain is visited in order,
once per turn, and a percentage roll against its first byte decides whether its
opcode stream runs. Success or failure has no effect on whether later records
are visited: there is no early exit, no chaining, and no continuation opcode.
This differs fundamentally from the reference format, where automatic lines are
ordinary action lines with verb 0 and where a continuation command makes
subsequent zero-keyed lines part of the same logical action.

### 3.8 Worked example: decoding by hand

*Three examples. The bytes of the middle one, a general action record, are
constructed for this document; the string example and the link-0 action record
that closes the section are both read out of a real specimen.*

**A string, from a specimen.** In the TI-99/4A release of Adventureland the
room description pointer table resolves to file offset 0x1DB8, and its first
four entries are the big-endian words `21 7E`, `21 7E`, `21 8C`, `21 A8`.
Room 1's extent is therefore stored addresses 0x217E to 0x218C, which with *B*
= 0 resolve to file offsets 0x1DFE to 0x1E0C. Those fourteen bytes are:

```
06 64 69 73 6D 61 6C 06 73 77 61 6D 70 2E
```

Decoding: `06` is a chunk of six characters, `64 69 73 6D 61 6C` = `dismal`;
bytes remain before the end of the extent, so an implied space follows; `06` is
another chunk of six, `73 77 61 6D 70 2E` = `swamp.`; the extent is now
exhausted, so no trailing space. Room 1's description is `dismal swamp.`, and
because it does not begin with `*` it is presented after the "I'm in a" prefix.
Room 10 in the same game decodes to `*I'm on the shore of a lake.` and is
therefore printed literally.

**An action record, constructed.** Suppose detection found the signature at
file offset 0x789, so *B* = 0x789 − 0x589 = 0x200 and a stored address *A*
resolves to *A* − 0x180. Suppose the explicit-action pointer holds `20 00`,
resolving to file offset 0x1E80. The dispatch entry for verb 10 is the word at
0x1E80 + 10 x 2 = 0x1E94; suppose it holds `21 00`, resolving to 0x1F80. At
0x1F80:

```
07 0B B9 05 C1 03 ED 05 E1 01 0C FF
```

- `07` — this record matches noun 7 only; a 0 here would match any noun.
- `0B` — link 11, so the next record of verb 10's chain begins 12 bytes on, at
  0x1F8C, and this record occupies 0x1F80 through 0x1F8B. The link is non-zero,
  so this is not the last record of the chain.

The ten-byte opcode stream, with positions counted from its first byte:

| pos | bytes | meaning |
|---|---|---|
| 0 | `B9 05` | condition 185: item 5 must be carried or in the current room |
| 2 | `C1 03` | condition 193: bit flag 3 must be set |
| 4 | `ED 05` | command 237: take item 5, ignoring the carry limit |
| 6 | `E1 01` | command 225: set bit flag 1 |
| 8 | `0C` | 12 is in the message range: print message 12 |
| 9 | `FF` | end of record; the record succeeds |

Observably: typing the verb with index 10 and the noun with index 7, while item
5 is to hand and flag 3 is set, takes item 5, sets flag 1, prints message 12,
and ends the search. If item 5 is absent, nothing happens at all and the walk
moves on to the record at 0x1F8C.

**The same action in the reference format.** The text format cannot carry
inline operands, so both must be smuggled through condition slots in the order
the commands consume them:

| position | value | derivation |
|---|---|---|
| verb/noun | 1507 | 10 x 150 + 7 |
| condition 1 | 103 | code 3 (carried or here), value 5: 3 + 5 x 20 |
| condition 2 | 68 | code 8 (bit set), value 3: 8 + 3 x 20 |
| condition 3 | 5 | code 0 (parameter), value 5 — operand for the take |
| condition 4 | 1 | code 0 (parameter), value 1 — operand for the set-flag |
| condition 5 | 0 | unused, padded |
| commands 1,2 | 11158 | 74 x 150 + 58 |
| commands 3,4 | 1800 | 12 x 150 + 0 |

The whole line reads `1507 103 68 5 1 0 11158 1800`. The comparison makes the
three structural differences concrete: the tokenised record needs no padding
(ten opcode bytes against eight fixed numbers), needs no operand-smuggling
slots, and encodes the verb by which chain it is in rather than in a packed
number.

**A link-0 record, from a specimen: Adventureland's `INVENTORY`.** This is the
case §3.7 warns about, in the shortest form a real game contains. In
`adv01.fiad` the signature lands at 0x589, so *B* = 0 and the header is at
0x8A0; the header gives a highest verb index of 65 and a word length of 3, and
the explicit-action pointer resolves to file offset 0x2992. Verb 17 is `INV`.
Its dispatch entry is the word at 0x2992 + 17 x 2 = 0x29B4, which holds
`29 A2` and resolves to file offset 0x2622. The four bytes there are:

```
00 00 E9 FF
```

- `00` — matches any noun, so the command matches whether or not a noun follows.
- `00` — the link: no record follows, so this is the whole of verb 17's chain.
  It is **not** a statement that the stream is empty.
- `E9` — 233: list the inventory.
- `FF` — 255: end of record, and the record succeeds.

**What the player sees.** Typing `INVENTORY` (or `INV`, or any word beginning
`INV`) prints the carried-items line — `I am carrying : `, the items separated
by `, `, a closing period and a space, or `Nothing. ` if the player's hands are
empty — and nothing else, because the record succeeded and a success ends the
search with no acknowledgement of its own. Read the link byte as an extent and
the same four bytes decode to a record with no opcodes, which cannot reach 255,
so it fails; it is the chain's only record, so the chain reports "matched but
every match failed", and the player sees `I can't do that yet. ` — a wrong
answer that reads like a deliberate one, on a verb the game's own title screen
advertises.

Four more of Adventureland's verbs have this shape, and each would break the
same way: `QUI` (verb 26) is `00 00 E8 E7 FF` — print the score, then end the
game; `STO` (31), which is STOP rather than STORE, is `00 00 22 FF` — print
message 34, `To stop the game say QUIT.`; `SCO` (32) is `00 00 E8 FF` — print
the score; and `SAV` (34) is `41 00 44 EB FF`, which prints message 68, `OK.`,
and then saves. That last one is keyed to noun 65, `GAM`, rather than open, so
by the matching rule above it answers `SAVE GAME` while a bare `SAVE` matches
nothing in the chain — and `SAVE GAME` is exactly the form the title screen
advertises, alongside `HELP`, `QUIT`, `SCORE` and `TAKE INVENTORY`. **Not one
of these five verbs is built into the interpreter** (§9.1): they are ordinary
chains, and they work only because a link-0 record runs.

---

## 4. Memory-image releases: locating the tables

This section covers the Commodore 64, ZX Spectrum, Atari 8-bit and Apple II
releases of the Adventure International and Brian Howarth games. Getting from a
distributed file to a memory image is §7's problem; this section assumes you
have the memory image and asks where the tables are inside it.

### 4.1 Dictionary signatures

A memory image is recognised by finding one of nine fixed byte strings anywhere
in it. **The search is unanchored** and the first signature in the trial order
below that matches anywhere wins; longer, more specific keys must be tried
before shorter ones that could alias them. The match lands **inside the
dictionary**, on or near its first cell; a per-signature constant is subtracted
to reach the dictionary's first byte.

| # | hex | printable | back-off | dictionary layout implied |
|---|---|---|---|---|
| 1 | `41 55 54 4F 00 47 4F 00` | `AUTO\0GO\0` | 0 | four-letter cells, plain text |
| 2 | `41 55 54 00 47 4F 00` | `AUT\0GO\0` | 0 | three-letter cells, plain text |
| 3 | `47 4F 00 00 00 00 2A 43 52 4F 53 53 2A 52 55 4E 00` | `GO\0\0\0\0*CROSS*RUN\0` | 6 | five/six-byte cells, plain text |
| 4 | `61 55 54 4F 67 4F 00` | `aUTOgO\0` | 0 | four-letter cells, case-marked synonyms, **compressed strings** |
| 5 | `67 45 48 45 4E 53 54 45 49 47 45` | `gEHENSTEIGE` | 5 | German Commodore 64: five-letter cells, case-marked |
| 6 | `C7 45 48 45 4E 53 54 45 49 47 45` | `\xC7EHENSTEIGE` | 5 | German ZX: five-letter cells, high-bit-marked |
| 7 | `81 00 00 00 C9 52 00 00 41 4E 44 41 45 4E 54 52` | — | 0 | Spanish ZX: four-letter cells, high-bit-marked |
| 8 | `81 00 00 00 69 52 00 00 41 4E 44 41 45 4E 54 52` | — | 0 | Spanish Commodore 64: four-letter cells, case-marked |
| 9 | `41 55 54 4F 00 56 41 49 00 00 2A 45 4E 54 52` | `AUTO\0VAI\0\0*ENTR` | 0 | Italian: five-byte cells, `*` markers |

Signature 1 is the first two cells, `AUTO` and `GO`, NUL-padded to four bytes;
signature 2 the same two words in three-byte cells; signature 3 begins at the
*second* cell (`GO` padded to six) followed by two synonym cells, hence the
six-byte back-off; signatures 5 and 6 begin at the second cell (the German for
"to go") and back off five bytes over the first; signatures 7 and 8 begin at
the first cell, a one-byte placeholder for the automatic-action word.

**Why the compressed dialect's signature is mixed case.** In the plain-text
dictionaries a synonym carries an explicit `*` byte, as in the reference
format. The compressed-dialect dictionaries do not spend a byte on it: **the
case of a cell's first letter carries the synonym bit.** A lower-case first
letter means this cell is a head word — a reader upper-cases it and stores it
unmarked. Any other first byte (an upper-case letter) means this cell is a
synonym of the most recent head word, and a reader synthesises the `*` prefix.
A first byte of `.` or NUL is an empty cell. Every letter after the first is
stored upper-case regardless. Hence `aUTOgO`: `aUTO` is the head word AUTO,
`gO` is the head word GO, and neither is a synonym. The German and Spanish ZX
releases spell the same bit with **bit 7 of the first byte** instead, because
their character sets commit the high half of the byte range to accented
letters.

### 4.2 Dictionary cell reading

**The dictionary is a fixed grid of (word length + 1)-byte cells**, read as a
stream. A cell carries its word left-aligned and NUL-padded to the full cell
width. A `*` in the cell's **first byte** marks a synonym of the nearest
preceding canonical word, exactly as in the reference format, and **it occupies
one of the cell's own bytes**, so a synonym has one character less room than a
canonical word. A cell whose bytes are all NUL, or all spaces, is an empty cell
and consumes its full width like any other. Reading stops at any byte above 127;
that is a terminator condition, not an error.

*Worked example*, the first ten bytes of the Commodore 64 *Golden Baton*
dictionary at `$685F` (§10.4). The word length is 4, so cells are five bytes:

```
41 55 54 4F 00 | 47 4F 00 00 00 | 53 57 49 4D 00 | …
A  U  T  O     | G  O           | S  W  I  M
```

`AUTO` fills four bytes and takes one pad; `GO` fills two and takes three;
`SWIM` follows. Ten cells later the same block reads
`47 45 54 00 00 | 2A 54 41 4B 45` — `GET` with two pads, then the synonym
`*TAKE`, five characters in a five-byte cell of which the `*` is one. The ZX
Spectrum releases of the same eleven titles spell their dictionaries the same
way; *The Golden Baton*'s first sixty-four dictionary bytes are identical on the
two platforms.

**One alignment escape, and where it may be applied.** A NUL where a cell should
begin can be padding rather than an empty cell: skip it, and take the following
(word length + 1) bytes as the cell. **This is safe only where an empty cell is
spelled with spaces**, and on the Commodore 64 *Mysterious Adventures* releases
it is: measured across all eleven §10.4 specimens, **not one dictionary contains
an all-NUL cell**, while *The Golden Baton* has two all-space cells (verb cells
77 and 78) and *The Wizard of Akyrz* three (verb cells 22, 48 and 61). On the ZX
Spectrum releases of the same titles the opposite holds — all-NUL cells are
common, up to 27 in one dictionary — and **the escape must not be applied
there**, because it would eat the first byte of every one of them.

Measured on the eleven Commodore 64 specimens, bounded by the room-description
address §6.2's pointer block gives, the escape fires **exactly once in the whole
corpus**: at *The Golden Baton*'s final cell, where the block reads
`48 45 4C 4D 00 | 4F 46 46 00 00 | 00 | 43 41 53 54 00` — `HELM`, `OFF`, one
stray byte, and `CAST`. Its published conversion's last noun is `CAST` (§10.4),
and without the skip that cell is misread. With the skip, every one of the
eleven dictionaries ends **exactly** on the room-description address, with no
byte left over and none borrowed.

**Three escapes an existing implementation applies instead, and why they are
wrong here.** That implementation reads *word length* characters per cell rather
than a whole cell, and recovers the difference with escapes: the leading-NUL
skip above, a `*` anywhere restarting the character count so that the word after
it still gets the full width, and a space immediately followed by a non-space
being dropped with the character count rewound. On a NUL-padded grid this is
self-correcting and agrees with the grid reading cell for cell — but only while
every cell ends in at least one pad byte and no two blank cells are adjacent. It
loses alignment permanently on either of two shapes the specimens contain:

- **a cell whose word fills all (word length + 1) bytes**, leaving no pad. *The
  Time Machine* and *Arrow of Death* part 2 store `NORTH` and `SOUTH` as noun
  cells 1 and 2, five characters in a five-byte cell, and a four-character read
  leaves the reader standing on the `H`;
- **two adjacent all-space cells.** The space rewind recovers one space, not
  two, so *The Golden Baton*'s cells 77 and 78 shift the reader by a byte it
  never gets back.

Measured against the six titles whose tables are byte-identical to their
published conversions — *The Golden Baton*, *Arrow of Death* parts 1 and 2, *The
Feasibility Experiment*, *Perseus and Andromeda* and *Waxworks* — the grid
reading reproduces **every verb and noun cell of all six**. The escape reading
reproduces four of the six and fails *The Golden Baton* (78 of its 79 noun cells
wrong) and *Arrow of Death* part 2 (82 wrong). **Read the grid.**

**And the space rewind is unnecessary everywhere measured.** Across the eleven
Commodore 64 dictionaries, the eleven ZX Spectrum ones, and the fifteen US
databases of §12, a space followed by a non-space **never once** occurs at a
position where the rewind would fire. It is not a rule of any dialect this
document describes; do not implement it.

**Ordering differs from the reference format.** There, the dictionary is
alternating (verb, noun) pairs. **In a memory image it is two contiguous
blocks: every verb cell, then every noun cell.** A reader takes
(verb-cell count + noun-cell count + 1) cells in one pass and splits them. The
two block sizes are not derivable from the header's combined word count; see
§4.6. (The US binary database of §12 reverses the two blocks — **nouns first** —
and is located by scanning for the noun block's first cell rather than by a
signature; see §12.5.)

### 4.3 From a signature hit to the tables

**A memory image contains no directory, no pointer word and no self-describing
structure.** Location is one arithmetic identity plus a set of per-release
constants:

1. Find the signature; subtract its back-off; call the result the **found
   dictionary offset**.
2. For each candidate release whose implied dictionary layout matches, compute
   **baseline = found dictionary offset − that release's dictionary address**.
   The release's addresses are addresses in the original machine's memory
   image; the baseline absorbs whatever constant shift the particular dump or
   container introduced, which is what lets one set of relative addresses work
   across differently-trimmed copies of the same release.
3. Every other address for that release becomes an offset by adding the
   baseline.
4. At (header address + baseline), read **fifteen consecutive little-endian
   16-bit words**. There is no magic number and no length field; the header is a
   bare array of counts whose *field order varies by release* (§4.5).
5. **Validate.** The parsed item, action, word and room counts and the
   maximum-carried must equal the candidate's expected values, and must fall in
   the ranges items 10-500, actions 100-500, words 50-190, rooms 10-100. On any
   failure, try the next candidate. If none validates, the file is not a
   recognised memory image.

Each table's start is given per release either as an absolute address or as a
sentinel meaning **"immediately after the previous table read"**, so the
reading order fixes what "previous" means. It differs between two families:

- **Later releases** read: room-image list, item-flag list, item-image list,
  actions, dictionary, room descriptions, room connections, messages, item
  descriptions, item locations, picture data, system messages, direction words.
- **Early releases** — Pirate Adventure, Voodoo Castle, Strange Odyssey,
  Buckaroo Banzai, and all eleven Mysterious Adventures ZX releases — read:
  actions, room connections, item locations, dictionary, room descriptions,
  messages, item descriptions, picture data, system messages, direction words.
  Early releases have no room-image list, no item-flag list and no item-image
  list at all.

Two location rules are not per-release:

- **System messages on Commodore 64 English releases**: the release's address is
  a hint. Read the first string there; if it is not `NORTH`, back the start up
  by one byte and retry, repeating until it is. A conforming loader must
  reproduce this search rather than trust a fixed offset.
- **Action comments and the trailer do not exist.** The reference format's
  per-action comment strings, version number and adventure number have no
  memory-image equivalent. There is nothing to skip.

### 4.4 Uncompressed table encodings

All multi-byte values are **little-endian 16-bit**. All counts are highest-index
values, so a table of *N* things has *N* + 1 records, exactly as in the
reference format.

**Actions** — (action count + 1) records of exactly **eight 16-bit words, 16
bytes each**: the vocabulary word, five condition words, two command words. The
packing inside each word is **identical to the reference format** — verb x 150
+ noun; condition code + 20 x value; and each command word holding two commands
as quotient and remainder by 150. So the plain memory-image action table is the
reference action table with each decimal integer replaced by a little-endian
word and the whitespace removed.

**Room connections** — (room count + 1) records of **six unsigned bytes**:
north, south, east, west, up, down; 0 means no exit. Room-major. Unlike the
reference format, the exits and the room texts are two separate tables in
different places.

**Room descriptions** — (room count + 1) NUL-terminated byte strings, back to
back. A byte above 127 aborts the read. The **leading `*` meaning "print
literally"** is a literal byte at the start of the string, stripped at print
time, exactly as in the reference format.

**Messages** — (message count + 1) NUL-terminated strings, back to back.

**Item descriptions** — (item count + 1) NUL-terminated strings. In early
releases a byte above 126 also terminates a string and is discarded. Both
reference-format conventions are spelled identically: a literal leading `*`
marks a treasure, and the auto-get word is the text between the first `/` and
the next `/`, with the literal remainders `//` and `/*` meaning "no auto-get
word" — the rule of §2.5 applies unchanged.

**Item locations** — (item count + 1) **single unsigned bytes**, a separate
table rather than a number following each item's text. **255 means carried** and
**0 means out of play**, as in the reference format.

**Room-image list** (later releases only) — one byte per room. Bit 7 is a
per-room darkness flag in at least one release; the picture number is the low
seven bits; 255 means the room has no picture. Early releases with line-drawn
pictures have no such table and take a room's picture number as **the room
number minus one**.

**Item-flag list** (later releases only) — one byte per item. The low seven bits
hold the room in which this item's picture is drawn; the picture is drawn only
when the item is in that room and the player is there too.

**Item-image list** (later releases only) — one byte per item, the picture
number; 255 means none.

**System messages** — up to **45** strings, each terminated by NUL **or** by a
carriage return (0x0D, which is kept as the string's last byte). Zero-length
strings are skipped and do not consume an index. Early releases read only 40.
There is no count field; a reader simply takes strings until it has enough.

**Direction words** — **6** strings with the same terminator rules, in the order
north, south, east, west, up, down. Absent from Commodore 64 English releases,
which take their direction words from the head of the system-message block.

### 4.5 The eleven header shapes

All read the same fifteen little-endian words; they differ only in which index
carries which fact. Indices are zero-based from the header address.

- **Early** — items 1, actions 2, words 3, rooms 4, max carried 5, start room 6,
  treasure count 7, word length 8, lamp turns 9, messages 10, treasure room 11.
  Word 0 is unused. **This is the reference format's own field order.**
- **Late** — items 1, actions 2, words 3, rooms 4, max carried 5, word length 6,
  messages 7. Start room is forced to 1, treasure count and treasure room to 0,
  lamp turns to −1 ("never runs out").
- **US** — word length 0, words 1, actions 2, items 3, messages 4, rooms 5, max
  carried 6, start room 7, treasure count 8, lamp turns 9, treasure room = the
  **high byte** of word 10. This shape belongs to the binary database of §12,
  where the header's address is a fixed offset rather than a catalogued one.
- **Gremlins Commodore 64** — items 1, actions 2, rooms 3, words 5, max carried
  6, word length 7, start room 8; message count is the constant 98; lamp −1.
- **Robin of Sherwood Commodore 64** — items 1, actions 2, messages 3, rooms 4,
  max carried 5, words 6, word length 7; start room 1; lamp −1.
- **Supergran Commodore 64** — actions 1, words 2, items 3, rooms 4, messages 5,
  word length 6, max carried 8; start room 1; lamp −1.
- **Seas of Blood Commodore 64** — items 0, actions 1, messages 2, rooms 3, max
  carried 4, word length 6; word count is the constant 134; start room 1;
  lamp −1.
- **Mysterious Commodore 64** — items 1, actions 2, words 3, rooms 4, **max
  carried = low byte of word 5, start room = high byte of word 5**, treasure
  count 6, word length 7, lamp turns 8, messages 9.
- **Arrow of Death part 2 Commodore 64** — as Mysterious Commodore 64 except
  items 3, actions 1, words 2.
- **Ten Little Indians Commodore 64** — **this one is not a word grid at all,
  and reading it as one recovers a wrong lamp value.** Read it as bytes, from
  **the header address plus two** (i.e. from word 1, past the two bytes that in
  this family are a `JMP` operand — see below): four little-endian words (items,
  actions, words, rooms), then **four single bytes** (max carried, start room,
  treasure count, word length), then **one filler byte**, then two ordinary
  little-endian words (lamp turns, messages). Seventeen bytes of fields, an odd
  number of bytes into the header, which is the whole point: the packing is at
  byte granularity. *Worked example, on the specimen named in §10.4:* those
  seventeen bytes are `49 00 A1 00 52 00 3F 00 05 3D 00 04 00 F4 01 43 00`,
  giving items 73, actions 161, words 82, rooms 63, max carried 5, start room
  61, treasure count 0, word length 4, **lamp turns 500** and messages 67.
  Read on a word grid instead — which is what an earlier draft of this section
  described, following an existing implementation, as "lamp turns = high byte of
  word 7" — word 7 is `$F400` and its high byte is 244, which is the *low byte
  of the correct value* mistaken for the answer. 500 is what that title's
  published conversion says (§10.3), and 244 is not; the byte-level reading is
  the one to implement. (The same draft's "messages = high byte of word 8" does
  give the right answer, 67, by the identical mechanism — word 8 is `$4301` and
  its high byte `$43` is the *low* byte of the byte-level word `43 00`. It
  agrees only because 67 fits in a byte. 500 does not, which is why the lamp is
  where the rule shows itself, and why one field agreeing is no evidence for the
  other.)
- **None** — the release is refused.

**Word 0 of the early shape is not a field.** On every Commodore 64 Mysterious
Adventures release measured (§10.4) the two bytes at the header address are the
operand of a `JMP` instruction whose opcode byte immediately precedes the
header, and the counts begin two bytes in. The value is therefore constant
across the whole family rather than merely unused; see §6.2 for the addresses
and for what that buys a loader.

### 4.6 What must be tabulated, and what can be re-derived

The honest answer, per fact:

| fact | re-derivable from the bytes? |
|---|---|
| dictionary address | **Yes.** It *is* the signature hit, less the back-off. This one fact is what makes every other address usable. |
| dictionary layout (which of the nine) | **Yes.** The signature says which. |
| item, action, word and room counts; max carried; word length; message count | **Yes, from the header** — but only once the header's address *and* field order are known, so in practice they serve as an identification checksum rather than as inputs. |
| verb-cell count and noun-cell count | **No.** In many releases the verb count is the word count + 1, but not in general: one Gremlins release has 126 words and 115 verb cells; Seas of Blood has 134 words and 69 verb cells; the Commodore 64 Feasibility Experiment has 79 words and 56 verb cells; the Commodore 64 Perseus has 130 words and 82 noun cells. |
| header address | **Partly.** Nothing in the file marks it — but see the measurement below. |
| header field order (one of eleven) | **No.** |
| table reading order (early or later family) | **No.** It changes what the "follows immediately" sentinel means, so it cannot be guessed without trying both and checking every table lands plausibly. |
| action encoding (plain, count-prefixed, or US column-major — §12.8) | **No**, though a reader could disambiguate by checking that decoding (action count + 1) records lands exactly on the next known table. |
| room-description, room-connection, message, item-description, item-location, system-message and direction-word addresses | **No.** |
| room-image, item-flag and item-image list addresses | **No.** |
| character-set address, picture-data address, picture-address bias, picture count, palette identity, picture format version | **No.** |
| behavioural family, sub-type bits, title | **No** — these are the answer, not an input. |

**Measured: the header address is more recoverable than it looks, for the early
shape.** Scanning a decompressed 48K ZX memory image for any window of twelve
little-endian words satisfying *items* 10-500, *actions* 100-500, *words*
50-190, *rooms* 10-100, *max carried* 1-20, *start room* between 1 and the room
count, *word length* 3-5 and *message count* below 200 yields **exactly one
candidate** in each of two Mysterious Adventures specimens tested — Escape from
Pulsar 7 at address 0x6351 and The Golden Baton at 0x6349 — and none at all in
two releases that use the late header shape, which the constraints correctly
exclude. That is not a proof, but it does mean an implementer who wants to
support an untabulated *early-shape* release has a viable, testable starting
point: scan, take the unique candidate, and verify that the tables it implies
land where the reading order says they should.

**Everything else really must be tabulated.** A memory-image loader with no
release knowledge at all is not possible for this format: it carries no
directory, no lengths, no magic numbers and no version field. The only
self-describing structures in the whole family are the count byte of a
compressed action record (§5.1) and the length byte of a compressed string
record (§5.2). An implementer supporting a release must obtain roughly
seventeen numbers for that exact release; there is no derivation for most of
them.

**But look for a driver pointer block before you tabulate anything.** The
paragraph above is a statement about the *format*: nothing in the format
declares where a table is. It is not a statement about any particular
**program**, and a memory image contains a program as well as its data. Where
one interpreter binary was shipped with several different data payloads — which
is how a whole series is normally built — its initialisation code plants the
table addresses into fixed locations from literal operands, and those operands
sit at a **fixed offset inside every release of that series**. That is a de
facto pointer table: not self-describing, because a reader must still be told
where to look and in what order the addresses appear, but one constant per
*series* instead of a dozen per *release*.

The whole Commodore 64 Mysterious Adventures family is such a series, and §6.2
gives its offsets. The recognition procedure that finds one, stated generally:
take two releases believed to share a driver, compare them byte for byte, and
note where the identical prefix ends; the pointer block lies inside that prefix,
and every one of its operands is a byte that *differs* between the two. Then
check the candidate addresses against tables you can already locate by other
means — the dictionary from its §4.1 signature, and the end of the last table
from the start of the picture data — and confirm that reading forward from each
candidate with §4.4's encodings makes every table end exactly where the next
begins.

**What a loader still needs per title, when a pointer block is present.** Two
numbers, and no more:

1. **Which header field order (§4.5) the release uses.** Not derivable, but
   cheaply *checkable*: the action table begins immediately after the header,
   and its start is also the dictionary address minus (action count + 1) × 16.
   A wrong field order gives a wrong action count, and the two arithmetics then
   disagree. Trying each candidate shape and keeping the one where they agree is
   a sound identification procedure — with the caveat of §5.3 that a stored
   action count is occasionally simply wrong, in which case the disagreement is
   the *defect*, not the wrong shape, and the span between the header's end and
   the dictionary is the authority.
2. **The dictionary's verb-cell and noun-cell split.** Genuinely not derivable,
   as this section says above. Its consequence is smaller than it looks: where
   the room descriptions have their own pointer, a loader never needs the
   dictionary's *end*, only how to divide the cells it reads. Getting the split
   wrong costs vocabulary, not layout.

The catalogue an existing implementation carries is 65 records: 54 full
memory-image records and 11 cut-down records holding only header counts, used
to recognise a Mysterious Adventures database file by its header alone.

---

## 5. Compressed action tables and compressed text

These are **two independent axes**, and conflating them is the commonest
misreading of this family. Gremlins and Supergran have compressed action tables
with a plain four-letter dictionary and plain NUL-terminated strings. Only
Robin of Sherwood and Seas of Blood carry both, and they are the only two
releases whose dictionary signature is the mixed-case `aUTOgO\0`. In a
specimen check across twenty ZX Spectrum snapshots, the four
compressed-action releases split exactly that way: Gremlins and Supergran
answered the plain `AUTO\0GO\0` signature, Robin of Sherwood and Seas of Blood
answered `aUTOgO\0`.

### 5.1 The compressed action table

The name is historical: **this is length omission, not bit-packing of fields.**
Each field is still a whole little-endian 16-bit word with exactly the
reference format's arithmetic inside it; what is saved is the trailing words
that would be zero.

A record is variable length, 3 to 17 bytes:

| bytes | content |
|---|---|
| 0-1 | vocabulary word, little-endian: verb x 150 + noun |
| 2 | **count byte** |
| 3 … | *C* condition words, little-endian |
| … | *M* command words, little-endian |

The count byte's bit layout, bit 7 most significant:

```
 bit   7  6  5    4  3  2  1  0
      [   M   ]  [      C      ]
```

- **bits 0-4** are *C*, the number of condition words physically present, 0 to
  5. A value above 5 is a corrupt record.
- **bits 5-7** are *M*, the number of command words physically present, 0 to 2.
  A value above 2 is a corrupt record.

**Expanding to the reference format's eight integers:** emit the vocabulary
word; emit the *C* condition words followed by (5 − *C*) zeros; emit the *M*
command words followed by (2 − *M*) zeros. Field semantics are unchanged. The
table is (action count + 1) such records read consecutively; because record
length is self-describing there is nothing to seek and no index.

**Worked example.** The eleven bytes

```
DB 05 27 41 00 05 00 8E 2C 6C 07
```

decode as follows. The vocabulary word is `DB 05` = 0x05DB = 1499; 1499 ÷ 150 =
9 and 1499 mod 150 = 149, so verb 9, noun 149. The count byte `27` is binary
`00100111`: bits 0-4 are `00111` = 3 conditions, bits 5-7 are `001` = 1 command
word. Three condition words follow: `41 00` = 65, `05 00` = 5, `8E 2C` = 11406.
Condition 1 is code 65 mod 20 = 5 with value 65 ÷ 20 = 3 ("item 3 is not in the
room with the player"); condition 2 is code 5 with value 0; condition 3 is code
11406 mod 20 = 6 with value 570. One command word follows: `6C 07` = 1900;
1900 ÷ 150 = 12 and 1900 mod 150 = 100, so commands 12 and 100. The record
occupies eleven bytes and the next begins immediately after. As reference-format
integers the line reads `1499 65 5 11406 0 0 1900 0`.

**Which releases use it.** Thirteen memory-image releases: Savage Island parts I
and II (ZX and Commodore 64), all seven Gremlins releases (English ZX in two
variants, German ZX, English Commodore 64, German Commodore 64, Spanish ZX,
Spanish Commodore 64), Supergran (both), Robin of Sherwood (both) and Seas of
Blood (both). Every other memory-image release uses the fixed 16-byte record of
§4.4, except the UK Hulk releases (§11).

### 5.2 Compressed text

**The alphabet.** A fixed 32-entry table indexed by a five-bit symbol, the same
for every release that uses the scheme:

| symbol | 0 | 1-26 | 27 | 28 | 29 | 30 | 31 |
|---|---|---|---|---|---|---|---|
| meaning | space | `a` … `z` in order | `'` | **shift** | `,` | `.` | **end of string** |

There is no upper-case letter, no digit and no other punctuation. In particular
the scheme **cannot represent `*` or `/`**, which is why treasures cannot be
marked in a compressed release and why the auto-get separator is different.

**Record layout and index-to-position mapping.** Compressed strings are a
**chain of self-describing records** beginning at the table's start; there is no
index array. **Byte 0 of a record is its length byte**: the record's total
length in bytes, *including that byte*, is the low seven bits. **Bit 6 of the
same byte is also the leading-capital flag**: clear means the first letter the
record produces is capitalised, set means it is not. Bit 7 is unused. To reach
string *n*, start at the table address and advance, *n* times, by the low seven
bits of the byte currently under the cursor.

Because length and capital flag share bit 6, the two are not independent: a
record 64 bytes or longer necessarily reads as "do not capitalise". In practice
payloads never approach 64 bytes for these releases — a 63-byte record carries
99 characters — so the observed behaviour is that bit 6 set means "start lower
case" and records are short. A conforming reader should take the low seven bits
as the length *and* test bit 6 for the capital, and should refuse a chain whose
accumulated advance runs past the end of the image.

A consequence for table location: a compressed table's total size is not
knowable without walking it, so every table that follows one is given an
absolute address; the "follows immediately" sentinel is never used after a
compressed table.

**The decoding procedure.** Symbols are packed **eight to a group of five
bytes**, most significant bit first: read the five payload bytes as one 40-bit
big-endian quantity and take five-bit symbols from bit 35 downward, so symbol
*k* of a group occupies bits (39 − 5*k*) down to (35 − 5*k*). Groups follow one
another with no padding or alignment.

Carry one boolean, "capitalise the next letter", initialised true when bit 6 of
the length byte is clear. Then for each symbol in turn:

1. Look the symbol up in the alphabet.
2. If it is **shift** (28), set "capitalise" true and treat the produced
   character as a **space** — the shift both emits a space and arms the capital.
3. If it is a letter and "capitalise" is true, emit the upper-case letter and
   clear "capitalise"; otherwise emit the character as it stands. The
   apostrophe, comma, full stop and space never consume the armed capital.
4. If it is **end of string** (31), the string is complete; stop, and do not
   decode the rest of the group.
5. If the emitted character was a **full stop**, emit an additional space after
   it and set "capitalise" true. If it was a **comma**, emit an additional space
   after it and leave "capitalise" alone.
6. A string exceeding 255 produced characters is a corrupt record.

Note what is not in the stream: the space after `.` and `,`, and the capital
after `.`, are synthesised by the decoder rather than stored.

**Worked example.** Encode the eight symbols `a`(1), `t`(20), shift(28), `s`(19),
`e`(5), `a`(1), `.`(30), end(31). As a bit stream, most significant first:

```
00001 10100 11100 10011 00101 00001 11110 11111
```

Regrouped into bytes: `00001101 00111001 00110010 10000111 11011111`, that is
`0D 39 32 87 DF`. With a length byte of `06` — six bytes total, bit 6 clear so
the first letter is capitalised — the record is:

```
06 0D 39 32 87 DF
```

Decoding, with "capitalise" starting true:

| symbol | value | entry | action | output so far |
|---|---|---|---|---|
| 1 | 1 | `a` | capital armed: emit `A`, disarm | `A` |
| 2 | 20 | `t` | emit `t` | `At` |
| 3 | 28 | shift | arm capital, emit a space | `At ` |
| 4 | 19 | `s` | capital armed: emit `S`, disarm | `At S` |
| 5 | 5 | `e` | emit `e` | `At Se` |
| 6 | 1 | `a` | emit `a` | `At Sea` |
| 7 | 30 | `.` | emit `.`, synthesise a space, arm capital | `At Sea. ` |
| 8 | 31 | end | terminate | `At Sea. ` |

The result is `At Sea. `, eight characters including the trailing space, from
six bytes. Had the length byte been `46` instead of `06`, the same payload would
decode as `at Sea. ` — and the chain walk would advance 70 bytes rather than 6,
which is the shared-bit hazard above.

**Per-table adjustments after decoding.**

- **Room descriptions**: the first character of every decoded room description
  is forced to lower case, so the interpreter can print its "You are " prefix. A
  compressed release cannot express a literal description, since `*` is not in
  the alphabet.
- **Item descriptions**: the auto-get word is delimited by a **full stop**, not
  by `/`. The first full stop ends the printed name; the auto-get word is what
  follows it, skipping the synthesised space, and ends at the next full stop.
  Because the decoder capitalises after a full stop, the auto-get word's first
  letter arrives upper-case; a reader upper-cases the remaining
  (word length − 1) characters so the whole word matches the dictionary's
  upper-case cells. An item whose decoded text *begins* with a full stop is an
  empty slot with no auto-get word.
- **Messages** and battle text are used as decoded.

### 5.3 Per-release load-time repairs

Some releases store data that is not coherent as it stands and must be patched
at load time. These are facts about those releases, not rules of the format.

**Robin of Sherwood (both platforms).** Three tables are not where the ordinary
rules put them and one is deliberately incomplete. The room-image list is read
from its own absolute address and holds entries for rooms 0-10 and then, **with
no gap**, for rooms 74 onward: rooms 11-73 have no entry, so a reader consumes
eleven bytes, skips 63 room slots without consuming anything, and resumes. The
room descriptions likewise come from an absolute address and there are only
**33** compressed strings for **94** room slots: indices 0-10 are rooms 0-10,
rooms 11-71 all take the fixed literal text `in Sherwood Forest`, and index 11
onward are rooms 72-93. A 555-byte forest-image table is read from a third
absolute address as a variable-stride list: each entry is 5 bytes if its first
byte has bit 7 set, otherwise 11 bytes, and within an 11-byte entry a last byte
of 0xFF means the entry is really 10 bytes; in every byte, bit 7 is a flag and
the picture number is the low seven bits. Two display constants are overridden:
the exits separator becomes a single space and the message separator becomes
`". "`.

**Seas of Blood (both platforms).** Three supplementary tables live outside the
ordinary layout: a 124-byte enemy table terminated early by a 0xFF byte, 32
compressed battle-message strings read with the §5.2 chain walk, and 2010 bytes
of extra picture data. On the ZX release, item 125's text is unusable as stored
and must be replaced with `A loose plank`, auto-get word `PLAN`.

**Gremlins, German Commodore 64.** Five repairs: verb cell 0 set to `AUTO`; noun
cell 0 set to `ANY`; noun cell 28 set to the synonym `*Y.M.C`; the first
condition word of action 0 set to 1005; the vocabulary word of action 6 set to
100; item 99's picture set to "none"; and **the header's action count (243) is
wrong and must be replaced by 236**.

**Gremlins, German ZX.** Verb cell 0 set to `AUTO`, noun cell 0 to `ANY`, noun
cell 28 to `*Y.M.C`; message 90 is unusable as stored and must be replaced.

**Secret Mission (both platforms).** Item 3's picture number set to 23 and its
flag byte to 0x82 (bit 7 set, room 2); room 2's picture number set to 0.

**Savage Island part I (both platforms).** Item 20's picture number set to 13.
**Both parts start the player in room 30**, which is not what the header says.

**Savage Island part II, Commodore 64.** Room 30's picture number set to 20.

**Mysterious Adventures, all Commodore 64 releases — the direction-noun repair is
not needed, and doing it damages two releases.** An existing implementation
installs `ANY` into noun cell 0 and copies the first six system messages (the
direction words), truncated to the word length, into noun cells 1-6, for exactly
the releases whose catalogue record is marked both *Mysterious* and *Commodore
64* — this eleven and no others. **Measured on all eleven §10.4 specimens, those
seven cells are already there**, read straight out of the dictionary by §4.2's
grid rule: noun cells 0-6 are `ANY NORT SOUT EAST WEST UP DOWN` in nine of the
eleven, and `ANY NORTH SOUTH EAST WEST UP DOWN` in *The Time Machine* and *Arrow
of Death* part 2, whose releases spell the two long ones with five characters in
a five-byte cell. Overwriting them with system-message text truncated to the
word length would replace those two with `NORT` and `SOUT` — a repair that
destroys stored data.

The repair is needed only by a reader that has lost cell alignment before it
reaches noun cell 0, which is what §4.2's escape reading does on these
specimens: on *The Golden Baton* it mis-reads 78 of the 79 noun cells, and on
*Arrow of Death* part 2 it goes wrong from `NORTH` onward. **Read the grid and
drop the repair.**

What survives per release is the stray last cell, and only that: Time Machine
verb 86; Arrow of Death part 1 noun 82; Arrow of Death part 2 verb 80; Escape
from Pulsar 7 noun 102; Circus noun 96; Feasibility Experiment noun 80; Perseus
and Andromeda noun 82. On the specimens these are the cells that fall past the
end of the real vocabulary and read as fragments of the release's copyright
string. **The Golden Baton's entry loses its first half**: its final cell reads
`CAST` directly under §4.2's alignment escape, which fires there and nowhere
else in the corpus, so no substitution is called for; the word-count reduction
to 79 stands.

**Escape from Pulsar 7, Commodore 64.** **The header's action count (195) is
wrong and must be replaced by 190**, exactly as for the German Commodore 64
*Gremlins* above. Measured on the §10.4 specimen: the header ends at `$5DF1` and
the dictionary begins at `$69E1`, a span of 3,056 bytes, which is 191 sixteen-
byte records to the byte — and 191 records is what an action count of 190 means
(§4.4's counts are highest-index). The 191st record read at a count of 195 is
the dictionary's own first cells, `41 55 54 4F 00 47 4F 00 …`, and 196 records
overrun the dictionary by 80 bytes. All 191 records validate: every vocabulary
word is below 150 × 150.

**The Time Machine, Commodore 64.** Its item-description block holds **62**
NUL-terminated strings where the header's item count of 62 implies 63 records,
and its item-location table is a full 63 bytes. Item 62 therefore has a location
and no stored description, and a loader **must supply an empty description
rather than read a 63rd string**, which would run into the location table. The
control is every other title in the series: on the §10.4 specimens the item
block holds exactly (item count + 1) strings, and only this one does not.

**System-message remapping.** The stored system-message block is not in the
order an interpreter wants, and the permutation is per release family with no
marker in the file. The families are: early Adventure International (three
contiguous runs re-based at stored indices 2, 6 and 13); Claymorgue (runs at 6
and 2); Gremlins, Supergran and Savage Island ZX (runs at 2, 6 and 17 plus five
individually placed strings); Mysterious ZX (runs at 2, 15 and 31 plus three
literal overrides); and nine distinct explicit permutation lists for the
Commodore 64 releases, of between 23 and 42 entries each. Several of these also
override individual strings with literals the file does not contain at all —
the Commodore 64 Supergran releases must supply the two halves of an
"is a word I don't know" sentence, because the stored block splits it
differently.

---

## 6. Mysterious Adventures native releases

Brian Howarth's eleven-title series — *The Golden Baton*, *The Time Machine*,
*Arrow of Death* parts 1 and 2, *Escape from Pulsar 7*, *Circus*, *Feasibility
Experiment*, *The Wizard of Akyrz*, *Perseus and Andromeda*, *Ten Little
Indians*, *Waxworks* — is a dialect in its own right, though the difference is
in header shape, table order and runtime wording rather than in field
encodings. Of the 54 full memory-image releases an existing catalogue carries,
23 are Mysterious Adventures: eleven ZX Spectrum, eleven Commodore 64, and an
Italian ZX release of *Perseus and Andromeda*. Eleven further catalogue records
carry header counts only, to recognise the series' reference-format databases.

### 6.1 Telling a Mysterious release apart

**The dictionary signature does not distinguish it.** Ten of the eleven ZX
releases answer the ordinary plain four-letter signature `AUTO\0GO\0`, exactly
like Strange Odyssey or Spider-Man; the Italian *Perseus* answers the Italian
signature; every Commodore 64 release answers `AUTO\0GO\0` too. Word length is
4 throughout the series, 5 for the Italian release. Three other things identify
it:

1. **Header counts.** The seven counts — items, actions, words, rooms, maximum
   carried, word length, messages — matched exactly against a catalogue
   identify both the release and the series. This is the only route available
   for a reference-format database, which has no addresses to match.
2. **Table order.** All eleven ZX releases belong to the **early** family
   (§4.3), so their tables run actions, room connections, item locations,
   dictionary, room descriptions, messages, item descriptions, and they carry
   no room-image list, no item-flag list and no item-image list. Their pictures
   are the line-drawn vector format of §8, and a room's picture number is the
   room number minus one.
3. **Message count.** Every ZX release in the series stores exactly **82**
   messages, distinctive against the 75-99 typical of Adventure International
   releases.

The Commodore 64 releases are **not** early-family: they use the later table
order, minus the room-image, item-flag and item-image lists, which they do not
have (the action table starts immediately after the header, so there is no room
for them).

**Their pictures are §8.2's family B vector format, the same as the ZX
releases** — not a Commodore 64 bitmap set. Measured on all eleven §10.4
specimens: the picture block begins at the first `$FF` at or after the address
in the pointer block's sixth slot (§6.2), and walking it with §8.2's opcodes —
`$C0` move, `$C1` fill, `$FF` end, anything else a line whose opcode byte is the
vertical operand — yields **exactly the room count** of images and consumes the
file to its **final byte**, in every one of the eleven. Nothing in these files is
a §8.3 bitmap. §8.2's colour paragraph already anticipates this, in saying that
Commodore 64 releases use the Commodore 64 palette with remap table A; the
present section previously contradicted it, and §8.2 was right.

**Verified against specimens.** Decompressing the ZX snapshot of *Escape from
Pulsar 7* to a flat 48K image and scanning for a plausible early-shape header
(§4.6) yields exactly one candidate, at address 0x6351, whose fifteen words read
items 90, actions 220, words 145, rooms 45, maximum carried 6, start room 1,
treasures 0, word length 4, lamp turns 200, messages 75, treasure room 0 — the
reference format's field order exactly, and identical to the eleven numbers in
that title's published reference-format conversion. *The Golden Baton*'s
snapshot yields one candidate at 0x6349 reading items 48, actions 171, words
76, rooms 31, maximum carried 5, start room 1, treasures 0, word length 4, lamp
200, messages 99, treasure room 0 — which differs from that title's published
conversion (48 items but 166 actions, 78 words, 6 carried, 99 messages), and is
therefore evidence that the snapshot and the conversion are **different
releases of the same game**. See §10 on what that means for using one as an
oracle for the other.

### 6.2 The Mysterious Commodore 64 header, field by field

Against the ordinary early header, this shape **packs two facts into one word**
and drops one:

| fact | early header | Mysterious Commodore 64 |
|---|---|---|
| items | word 1 | word 1 |
| actions | word 2 | word 2 |
| words | word 3 | word 3 |
| rooms | word 4 | word 4 |
| maximum carried | word 5 | **low byte of word 5** |
| start room | word 6 | **high byte of word 5** |
| treasure count | word 7 | word 6 |
| word length | word 8 | word 7 |
| lamp turns | word 9 | word 8 |
| messages | word 10 | word 9 |
| treasure room | word 11 | forced to 0 |

Two releases deviate further, as §4.5 records: *Arrow of Death part 2* takes
items from word 3, actions from word 1 and words from word 2, and *Ten Little
Indians* is byte-packed throughout. Four Commodore 64 releases in the series —
*Escape from Pulsar 7*, *Feasibility Experiment*, *Perseus and Andromeda*,
*Waxworks* — use the plain early shape with no byte-packing.

**The header is at a fixed address, and it is marked.** In every one of the
eleven §10.4 specimens the header address is **`$5DD7`**, and the three bytes
`4C 19 4D` sit at `$5DD6` — a 6502 `JMP $4D19`. Word 0 of the header is that
instruction's operand, which is why §4.5's early shape calls it unused and why
its value never varies; the counts begin at **`$5DD9`**. The three-byte string
at `$5DD6` is a serviceable recognition signature for the whole family in its own
right.

The header's **length** varies with its shape, and the action table begins
immediately after it. Measured, from `$5DD6`'s operand (word 0) inclusive:

| shape | header bytes | action table begins at |
|---|---|---|
| early | 26 | `$5DF1` |
| Mysterious Commodore 64, and Arrow of Death part 2 | 24 | `$5DEF` |
| Ten Little Indians | 23 | `$5DEE` |

That start is independently confirmed by the arithmetic of §4.6 — dictionary
address minus (action count + 1) × 16 — in **ten of the eleven**; the eleventh
is *Escape from Pulsar 7*, whose stored action count is wrong (§5.3).

**The table addresses are in the file, at a fixed offset.** These eleven are one
interpreter binary with eleven different data payloads: the region `$4000` to
`$432F` is byte-identical in all of them, and the first byte that differs
anywhere is at `$4330`. The image begins with a `JMP` to `$48E3`; three bytes
past that entry point, at **`$48E6`**, a run of twelve load-immediate /
store-to-zero-page pairs plants **six sixteen-bit addresses**, low byte then high
byte, as literal operands. The operands are the bytes at **`$48E7`, `$48EB`,
`$48EF`, … every fourth byte through `$4913`**, and pairing them in order gives:

| pair | zero-page destination | address of |
|---|---|---|
| 1 | `$32`/`$33` | room descriptions |
| 2 | `$2C`/`$2D` | room connections |
| 3 | `$34`/`$35` | messages |
| 4 | `$36`/`$37` | item descriptions |
| 5 | `$2E`/`$2F` | item locations |
| 6 | `$30`/`$31` | the first byte after the item-location table |

A **seventh** address follows in the same routine, planted by two
load-immediate / store-to-absolute pairs rather than to zero page: its low byte
is at **`$4917`** and its high byte at **`$491C`**, and it is the **dictionary**.
That is a free and complete cross-check on §4.1: in all eleven specimens the
plain `AUTO\0GO\0` signature occurs exactly once and at exactly this address.

Reading forward from those addresses with §4.4's plain encodings, every table
ends exactly where the next begins, in all eleven — (room count + 1)
NUL-terminated room descriptions, (room count + 1) × 6 exit bytes,
(message count + 1) messages, (item count + 1) item descriptions and
(item count + 1) location bytes — with the single exception §5.3 records for
*The Time Machine*. The whole memory image is then accounted for:

```
$4000  entry vector, JMP $48E3      $48E6  pointer block (6 + 1 addresses)
$4406  system messages (44)         $5DD6  JMP $4D19
$5DD7  header                       →  actions  →  dictionary
       →  room descriptions  →  room connections  →  messages
       →  item descriptions  →  item locations
       →  a run of zero bytes (61, 76 or 101)  →  picture data  →  end of file
```

### 6.3 Table layout

Field encodings are **identical** to §4.4 in every respect: sixteen-byte plain
action records with the reference format's arithmetic, six-byte room-major exit
records, NUL-terminated strings, one location byte per item with 255 meaning
carried, `*` for treasure and for a literal room description, `/WORD/` for the
auto-get noun. **No Mysterious release uses the compressed action table and
none uses the compressed text scheme.** The differences are ordering and
absence, as §6.1 lists, plus the Commodore 64 dictionary repairs of §5.3.

### 6.4 System messages: wording, and which platform's wording

Mysterious releases ship their own driver messages, and an interpreter must
produce the series' wording. **Which wording, though, is a property of the
platform, not of the series** — see the Commodore 64 subsection below, which is
first-person throughout and contradicts the table here at almost every line.

Two strings are **not in the file at all** on the ZX releases and must be
supplied: the item separator is `" - "` and the message separator is a newline.
A third, the visible-objects heading — a newline, `Things I can see:`, another
newline — must be supplied on the ZX releases too, but **is** in the file on the
Commodore 64 releases.

The table below is the **ZX Spectrum** wording. There the series is
second-person where Adventure International is first-person, and an interpreter
should expose the choice as an option that a recognised Mysterious ZX release
forces on:

| situation | Adventure International default | Mysterious |
|---|---|---|
| room preamble | `I'm in a ` | `You are in a ` |
| additional objects | `\nI can also see: ` | `\nYou can also see: ` (overridden again, above) |
| inventory heading | `I'm carrying: \n` | `You are carrying:\n` |
| not carrying it | `I'm not carrying it. ` | `You haven't got it. ` |
| already have it | `I already have it. ` | `You have it. ` |
| not visible | `I don't see it here. ` | `You don't see it. ` |
| beyond power | `It is beyond my power to do that. ` | `It is beyond your power to do that. ` |
| refusal | `I can't do that yet. ` | `You can't do that yet. ` |
| bad direction | `I can't go in that direction. ` | `You can't go in that direction. ` |
| darkness | `I can't see. It is too dark!\n` | `You can't see. It is too dark!\n` |
| fatal fall | `\nI fell and broke my neck.` | `You fell down and broke your neck. ` |
| overload | `I've too much to carry. \n` | `You are carrying too much. \n` |
| death | `I'm dead. ` | `You're dead. ` |
| nothing to drop | `I have nothing to drop. ` | `You carry nothing. ` |
| lamp warning | `My light is growing dim. ` | `Your light is growing dim. ` |

Two further interpreter options are implied by the series and must be forced on
when one is recognised — **including for a reference-format database**, which
is exactly why a catalogue needs those eleven header-only records: the
"authentic light messages" option and the "prehistoric lamp" option. §9.2
specifies both.

**The Commodore 64 releases are first-person, and their block is in the file.**
Measured on all eleven §10.4 specimens: a 790-byte block at the fixed address
`$4406` is **byte-identical across every one of the eleven** and holds **44**
strings, each terminated by a NUL or a carriage return under §4.4's rules.
§4.3's "read the first string there; if it is not `NORTH`, back the start up by
one byte" search is satisfied at the address itself, with no back-up needed.
The block, in stored order, with the addresses it occupies in one of the
specimens:

| # | address | string | # | address | string |
|---|---|---|---|---|---|
| 0 | `$4406` | `NORTH` | 22 | `$451A` | `I am carrying:` |
| 1 | `$440C` | `SOUTH` | 23 | `$4529` | `I don't see it here` |
| 2 | `$4412` | `EAST` | 24 | `$453D` | `That is beyond my power` |
| 3 | `$4417` | `WEST` | 25 | `$4555` | `It's risky moving in the dark!` |
| 4 | `$441C` | `UP` | 26 | `$4574` | `Try a direction` |
| 5 | `$441F` | `DOWN` | 27 | `$4584` | `I fell and broke my neck!` |
| 6 | `$4424` | `EXITS: ` | 28 | `$459E` | `I'm carrying too much!` |
| 7 | `$442C` | `Things I can see:` | 29 | `$45B5` | `I'm DEAD!!` |
| 8 | `$443E` | `I'm in a ` | 30 | `$45C0` | `Want to play again ? ` |
| 9 | `$4448` | `I'ts too dark to see!` | 31 | `$45D6` | `Resume play on a saved game ? ` |
| 10 | `$445E` | `The Light ran out!` | 32 | `$45F5` | `I've stored ` |
| 11 | `$4471` | `Light runs out in ` | 33 | `$4602` | ` treasures` |
| 12 | `$4484` | ` turns` | 34 | `$460D` | `Out of 100 that rates ` |
| 13 | `$448B` | `XNSEWUD"` | 35 | `$4624` | `I can't do that just now!` |
| 14 | `$4494` | `" is a word I don't know..sorry!` | 36 | `$463E` | `I don't understand you` |
| 15 | `$44B5` | `I don't know what a "` | 37 | `$4655` | `Not a sausage!` |
| 16 | `$44CB` | `" is` | 38 | `$4664` | `The game is over, thanks for playing` |
| 17 | `$44D0` | `I can't go in that direction` | 39 | `$4689` | `WELL DONE! Mission completed!` |
| 18 | `$44ED` | `O.K.` | 40 | `$46A7` | `PRESS RECORD & PLAY THEN HIT RETURN` |
| 19 | `$44F2` | `---WHAT NOW ? ` | 41 | `$46CB` | `PRESS PLAY THEN HIT RETURN` |
| 20 | `$4501` | `Eh?` | 42 | `$46E6` | `ERROR ON TAPE WRITE!` |
| 21 | `$4505` | `I'm not carrying it!` | 43 | `$46FB` | `ERROR ON TAPE READ - REWIND TAPE` |

The misspelling in string 9 is the file's, not a transcription error. Strings
0-5 confirm §4.4's rule that Commodore 64 English releases keep no separate
direction-word table and take those six from the head of this block.

**None** of `You are in a`, `You can also see`, `You haven't got it` or
`You are carrying` occurs anywhere in any of the eleven files. The second-person
table above may well be right for the ZX releases; it is demonstrably not a
property of the series, and an interpreter must select the wording by the
platform it recognised, never by the series alone.

An existing implementation forces the second-person set for the whole series,
which produces the ZX wording on a Commodore 64 release; for these specimens
that is wrong, and the file's own block is right.

### 6.5 Platforms and containers

- **ZX Spectrum** — 48K snapshots, decompressed to a flat 48K image before
  anything else (§7.1). Eleven English releases plus the Italian *Perseus and
  Andromeda*.
- **Commodore 64** — disk and tape images (§7.2). **Two disk images each hold
  six and five games**: one carries Golden Baton, Time Machine, Arrow of Death
  1, Arrow of Death 2, Pulsar 7 and Circus as the named files `BATON`,
  `TIME MACHINE`, `ARROW I`, `ARROW II`, `PULSAR 7`, `CIRCUS`; the other
  carries Feasibility Experiment, Wizard of Akyrz, Perseus and Andromeda, Ten
  Little Indians and Waxworks as `EXPERIMENT`, `WIZARD OF AKYRZ`, `PERSEUS`,
  `INDIANS`, `WAXWORKS`. **A loader must be told which game is wanted**; there
  is nothing in the container that picks one. Single-title tape images exist
  for nine of the eleven.
- **Reference text format** — one file per title, recognised only by its header
  counts.

---

## 7. Containers and decompression, per platform

Everything in §§4-6 is stated against a **memory image**. This section is how
you get one.

The essential asymmetry: the ZX Spectrum, Atari 8-bit and Apple II containers
are identified **by content** — magic bytes at fixed offsets — while the
Commodore 64 path is identified **by exact file length plus a whole-file
checksum against a catalogue of known releases**. There is no content-based
Commodore 64 detection and no generic Commodore 64 path.

A practical ordering that works: try the reference text format; then the
TI-99/4A signature; then Commodore 64; then Atari; then Apple II; then ZX
Spectrum. Identification must be non-destructive — keep the original buffer
alive, because a later attempt may need it.

### 7.1 ZX Spectrum snapshots

**Identification.** There are no magic bytes. Read the little-endian word at
offset 0x06, the program counter. **Non-zero means version 1**: the header is
30 bytes and the memory data begins at offset 0x1E. **Zero means version 2 or
later**: read the little-endian word at offset 0x1E, the additional header
length. 23 means version 2; 54 or 55 means version 3; **any other value must be
refused**. The extended header begins at 0x20 and the memory blocks begin at
0x20 + that length — offset 0x37 for version 2, 0x56 or 0x57 for version 3.

The format is publicly documented; the description maintained alongside the
Fuse emulator project is the canonical reference and an implementer should work
from it. The fields this dialect actually needs are: **offset 0x06**, the
program counter (version discriminator); **offset 0x0C**, the flag byte, whose
**bit 5 means the version-1 memory data is compressed** and where a stored
value of 0xFF must be read as 0x01; **offset 0x1E**, the additional header
length; **offset 0x22**, the hardware mode; and **bit 7 of offset 0x25**, which
downgrades the machine one step (48K to 16K, 128K to +2, +3 to +2A).

Hardware mode values, which differ by version: in version 2, 0 = 48K,
1 = 48K + Interface 1, 2 = 48K + SamRam, 3 = 128K, 4 = 128K + Interface 1; in
version 3, 0 = 48K, 1 = 48K + Interface 1, 2 = 48K + SamRam, 3 = 48K + MGT,
4 = 128K, 5 = 128K + Interface 1, 6 = 128K + MGT. In both, 7 and 8 = +3,
9 = Pentagon, 10 = Scorpion, 12 = +2, 13 = +2A, 14 = TC2048, 15 = TC2068,
128 = TS2068. Any other value must be refused. The only consequence for this
purpose is whether the machine has 128K-style paging.

**Version 1 payload.** If bit 5 of offset 0x0C is clear, exactly 49,152 raw
bytes follow the 30-byte header, in address order. If it is set, the stream is
compressed and terminated by the four-byte marker `00 ED ED 00`, which is not
part of the data.

**Version 2 and 3 payload.** A sequence of blocks, each: a little-endian
compressed length (the sentinel **0xFFFF** meaning "the next 16,384 bytes are
stored uncompressed"), one page-number byte, then that many bytes of data.
Blocks run to end of file; **there is no end-of-data marker**. One pseudo-block
must be recognised and refused: a block header with length 0 and page 0 whose
first six bytes are `00 00 00 53 4C 54` marks appended level data of a
different format.

**The run-length procedure**, used for version-1 streams and for each
version-2/3 block:

1. If exactly one byte remains, emit it and stop.
2. Otherwise, if the current byte is `ED` **and** the next byte is also `ED`,
   consume both, then consume a **count** byte and a **value** byte, and emit
   the value byte *count* times. A count of zero emits nothing but still
   consumes the value byte.
3. Otherwise emit the current byte unchanged and advance by one.

Two consequences: a **single** `ED` followed by anything other than `ED` is a
literal, which is why a lone `ED` is never compressed; and because rule 1
pre-empts rule 2, a stream ending in a lone `ED` is a literal, not a truncated
run.

*Worked example.* Given the compressed bytes

```
41 42 ED ED 04 7F ED 43 ED ED 00 55 ED
```

`41` is not a run head, so emit `41`. `42` likewise. `ED ED` is a run: count
`04`, value `7F`, so emit `7F 7F 7F 7F`. The next `ED` is followed by `43`, not
another `ED`, so emit `ED` literally, then `43`. `ED ED` is a run with count
`00` and value `55`: emit nothing, but consume all four bytes. The final `ED` is
the last byte, so emit it. Output: `41 42 7F 7F 7F 7F ED 43 ED` — nine bytes
from thirteen.

A version-1 stream **must expand to exactly 49,152 bytes** and each
version-2/3 block **to exactly 16,384**; any other length is a corrupt file and
must be refused, not padded.

**Page number to memory address.** The page number is an index, not an address.
Pages outside 1-18 are invalid. Pages 1 and 2 are ROM images and are discarded;
page 11 is a Multiface ROM on every machine but a Scorpion and is likewise
discarded. On a machine **without** 128K paging, page 3 is a further ROM image
and is discarded, page 4 is renumbered to 5 and page 5 is renumbered to 3. Then
subtract 3 to get a RAM bank index; a bank appearing twice means a corrupt
file. For a **48K snapshot**, which is what every Scott Adams release is, the
net mapping is the only one an implementer strictly needs:

| page in file | address range |
|---|---|
| 8 | 0x4000 - 0x7FFF |
| 4 | 0x8000 - 0xBFFF |
| 5 | 0xC000 - 0xFFFF |

For a 128K snapshot, pages 3 through 10 are banks 0 through 7 and the
48K-visible view is bank 5 at 0x4000, bank 2 at 0x8000 and bank 0 at 0xC000 —
the power-on paging arrangement.

**Result.** A byte array of exactly **49,152 bytes whose first byte is address
0x4000**. Every ZX table address in this document is read against that base: a
table at address *N* is at array offset *N* − 0x4000.

*Verified.* Implementing exactly the above over twenty published ZX snapshots
places the dictionary signature at plausible RAM addresses in every Scott
Adams title present — for example 0x82E2 in Supergran, 0x873A in Golden Baton,
0x8DA8 in Robin of Sherwood, 0x9900 in Seas of Blood — and finds none in the
non-Scott titles in the same archive. The raw, undecompressed files also
contain the signature, at offsets that mean nothing, because the run-length
scheme passes literal text through; **this is the trap the "decompress first"
rule exists to prevent.**

**Snapshots in the register-dump format** (a fixed 27-byte header followed by
49,152 raw bytes for addresses 0x4000-0xFFFF, and a separate 128K variant) are
a different container and are not covered here. The layout is publicly
documented and trivial by comparison; an implementer should either specify it
independently or refuse such files by name, because reading one as a raw memory
image is wrong by 27 bytes.

**A second, game-specific layer.** Four ZX re-releases from one cracking group —
Pirate Adventure, Voodoo Castle, Strange Odyssey and Buckaroo Banzai — carry a
further **backwards** run-length encoding inside the decompressed image, applied
only when the ordinary signature scan of that image fails. Three little-endian
pointers are read from fixed image positions (0x1B42, 0x1B48 and 0x1B4B, each
less 0x4000) and two unpacking passes run backwards. The stream is read
backwards too: a marker byte with **bit 7 clear** is the high byte of a 16-bit
literal-run count whose low byte is the byte immediately before it, and that
many bytes are copied from successively lower source addresses to successively
lower destination addresses, the source pointer then moving two bytes down; a
marker with **bit 7 set** takes its low seven bits as a repeat count for the
two bytes immediately below the marker, emitted higher-addressed byte first,
the source pointer then moving three bytes down. The addresses involved are
properties of those four releases, and refusing them is a defensible choice.

### 7.2 Commodore 64 containers

**Identification is by catalogue, not by content.** A file is identified by its
exact byte length together with a 16-bit checksum — **the low sixteen bits of
the sum of every byte in the file, with wraparound; not a CRC** — matched
against a table of known releases. An unrecognised Commodore 64 file, however
valid, is rejected. The catalogue carries, per release: the game identity, the
exact length, the checksum, the container kind, a count of decompression
passes, optional decompression overrides, an optional name of a second file to
append, a signed offset controlling where the appended file lands, an optional
three-value copy instruction applied to the decompressed memory, and an
optional picture-data cut-off offset.

**Disk image geometry.** Only these lengths are accepted as disk images:
174,848 (35 tracks); 175,531 (35 tracks with an error map); 349,696 and 351,062
(70 tracks, with and without an error map); 819,200 and 822,400 (80 tracks).
40-track variants are rejected. In practice only the two 35-track lengths ever
appear.

A 35-track image is a flat concatenation of 256-byte sectors in ascending track
order, tracks numbered from **1**, sectors within a track from **0**, with no
header of any kind. Sectors per track: 21 for tracks 1-17, 19 for 18-24, 18 for
25-30, 17 for 31-35 — 683 sectors, 174,848 bytes. The linear block number is
(track − 1) x 21 for tracks 1-17; (track − 18) x 19 + 357 for 18-24;
(track − 25) x 18 + 490 for 25-30; (track − 31) x 17 + 598 for 31-35; add the
sector number and multiply by 256. *Worked example:* track 18 sector 0 is block
357, offset 91,392; track 19 sector 3 is block 379, offset 97,024; track 1
sector 0 is offset 0. The error map, when present, is 683 bytes at offset
174,848, one per block in the same order; it may be ignored.

**Directory.** The block availability map is at track 18 sector 0, but a reader
must **ignore its stored link and go directly to track 18 sector 1** — this is
what the drive firmware does. Directory sectors chain: bytes 0-1 are the track
and sector of the next directory sector, a track byte of **0** ending the
chain, and eight 32-byte entries follow at offsets 0, 32, 64, … (the first
entry's unused first two bytes are what the chain link occupies). Entry layout,
relative to the entry: +0 two unused bytes; **+2 file type**, 0x00 meaning
unused or deleted, high bit set meaning properly closed, 0xC2 being a closed
program file, which is what these games use; **+3 first data track**; **+4
first data sector**; **+5 sixteen filename bytes** in the machine's own
character set, padded with **0xA0**; +21 two bytes for relative-file side
sectors; +23 record length; +24 four unused; +28 two bytes used during an
overwrite; **+30 file size in blocks, little-endian**.

Name matching is against a 16-byte pattern padded with 0xA0: `*` in the pattern
matches everything from that position on, `?` matches any one character, and a
0xA0 in the *name* means the name ended and matches only a 0xA0 in the pattern.
A reader should **reject entries whose type byte is 0x00** and should bound the
directory walk — 64 sectors is a generous ceiling — since neither is guaranteed
by the data.

**File reading.** A file is a chain of 256-byte blocks. Byte 0 is the next
track and byte 1 the next sector. If byte 0 is **non-zero**, the block carries
a full **254 bytes** of payload at offsets 2-255. If byte 0 is **zero**, this
is the last block and **byte 1 is the number of used bytes plus one**, so the
payload is byte 1 − 1 bytes starting at offset 2; a byte 1 of 0 here is
malformed. A reader **must track visited track/sector pairs and abort on a
repeat**, or a corrupt image loops forever. The same firmware quirk applies: a
chain landing on track 18 sector 0 advances to track 18 sector 1 instead of
following the stored link. *Worked example:* a file starting at track 19 sector
3 whose first block begins `13 05` contributes 254 bytes and continues at track
19 sector 5 (offset 97,536); if that block begins `00 2A` it is the last and
contributes 0x2A − 1 = 41 bytes, for a 295-byte file.

**Tape images.** Offset 0x00 holds a 32-byte ASCII descriptor, conventionally
beginning with a name identifying the format; offset 0x20 the version; 0x22 the
maximum entry count; **0x24 the number of entries used**; 0x28 a 24-byte
container name. File records begin at **offset 0x40** and are 32 bytes: +0
entry type (0 free, 1 normal), +1 file type, **+2 start (load) address**, **+4
end address**, +6 unused, **+8 the file's data offset within the image** (four
bytes little-endian), +12 unused, +16 a sixteen-byte ASCII filename. If the
used-entry count is exactly **1**, the payload runs from the data offset to the
end of the file and the record's end address is not to be trusted; otherwise
the length is end minus start. The container is publicly documented and an
implementer should verify the descriptor even though a catalogue-based reader
need not.

**From container to program file.** From a disk image, either the **largest
file by block count** is taken (the usual case) or a file with a specific name
(the US variants and the compilation disks). From a tape image, a reader
synthesises the same shape by emitting the record's two start-address bytes and
then the payload. **The result is a program file: bytes 0-1 are the load
address, little-endian, and byte 2 corresponds to that address in memory.**
There is no fixed Commodore 64 base address — it is read from the file, every
time. Some releases require a second named file to be appended with **its own
two-byte load address stripped**, written at (length of the main file + a
signed offset from the catalogue); the offsets are small negative numbers, so
the appended data overlaps the main file's tail. Some require a block of the
decompressed memory to be copied from one address to another, and one requires
a 4 KiB window saved before that copy and restored elsewhere afterwards. All of
that is per-release data.

**Crunchers — the decisive fact.** **An implementer cannot support the
compressed Commodore 64 releases without a 6502 emulator.** There is no
per-packer decompression procedure to specify. Twenty-one packers are
recognised — Abuze Crunch, Byte Boiler, ECA Compacker, Trilogic Expert, Final
Super Compressor, a group of intro and loader stubs, Cruel Crunch, PuCrunch,
Master Compressor, Mr. Cross, Mr. Z, TCS Crunch, TBC Multicompactor, Time
Cruncher, XTC, CCS Packer, Megabyte Cruncher, Section 8 Packer, Caution, Action
Packer, and Exomizer — and recognition is by a **fixed byte pattern at a fixed
or near-fixed address**, almost always in the region around 0x0810-0x0830. A
match yields four numbers, several of them read out of the stub's own code at
known offsets from the match: the depacker's entry point, the address at which
it is considered finished, and the start and end of the unpacked region. **The
depacking itself is done by executing the stub**, instruction by instruction,
in a simulated Commodore 64 with plausible zero-page pointers, stack contents
and interrupt vectors, until the program counter reaches the return address or
an interrupt-return executes, with an iteration ceiling in the tens of
millions. Some releases need up to six successive passes, each re-running
recognition over the previous pass's output, and some need catalogued overrides
(a memory-filler byte and origin, a forced end address, a forced entry point)
because automatic discovery gets the answer wrong. The unpacked result is taken
as the range (start − 2) through end, with the two bytes at (start − 2)
overwritten with the start address, so the output is itself a program file.

Three honest options follow, and a specification should say so plainly:
implement the emulator and all twenty-one recognisers; support only the
releases whose catalogued pass count is **zero**, which is a real and
defensible subset including several disk releases; or require already-unpacked
images as input.

**The second option is larger than it sounds, and the largest single group of
Commodore 64 releases falls inside it.** All eleven Mysterious Adventures titles
on the two compilation disks of §6.5 need **no cruncher at all**. Measured on
the §10.4 specimens: each extracted program file loads at `$4000`, begins
`4C E3 48` (a `JMP` to its own entry vector) followed by a long run of genuinely
empty workspace, and every table §4 describes sits in it in the clear — plain
ASCII strings readable with a strings tool, plain sixteen-byte action records,
no §5.1 compressed action table and no §5.2 compressed text anywhere. The long
zero run is what makes these look as though they might carry a stub; they do
not. **The absence of a recognisable cruncher signature in such a file is not a
warning sign — it is the answer.** No 6502 execution is needed at any stage, and
the work reduces to the disk-image reader specified above, stripping the
two-byte load address, and §6.2's fixed addresses.

The Commodore 64 *Hulk* release of §10.7 is a second zero-pass case, found the
same way: its database file carries the fixed name `SHULK.DB` this section
already lists, is a plain uncrunched program file loading at `$8030`, and holds
the §4.1 plain signature at `$8308`.

**Which file inside a disk image.** Four policies, per release: the largest file
by block count; a fixed name (`SAGA.DB`, `SHULK.DB`, `PIRATE`, `SAGA1`,
`VOODOO CASTLE`, `VOODOO CASTLE 2`, `SAG1PIC`, `SAG3PIC`, `G1`,
`SAVAGEISLAND1+`, `SAVAGEISLAND2+`, `SAVAGE ISLAND P1`, `SAVAGE ISLAND P2`,
`SI1PC1`, `SI1PC2`, `SI2PIC`); a caller's choice among the names of a
compilation disk (§6.5); or, for pictures on US-variant disks, a name-pattern
scan taking every name whose first character is `R`, `B` or `S` followed by
three decimal digits. **There is no format-level rule that identifies the
database file on a Commodore 64 disk.**

### 7.3 Atari 8-bit disk images

**Identification** is an exact match on six bytes at offset 0: `96 02 80 16 80
00`. Read as three little-endian words these are the signature 0x0296, an image
size of 0x1680 (5,760) sixteen-byte paragraphs, and 128 bytes per sector. Only
the first is a true signature; pinning the other two accepts **only
single-density, 720-sector images** — a 92,160-byte data area and a
**92,176-byte file**. Every known Scott Adams release is exactly that size.

**Header**, sixteen bytes: 0x00 signature; 0x02 low word of the size in
paragraphs; 0x04 bytes per sector (128, 256 or rarely 512); 0x06 high byte of
the size in paragraphs; 0x07 disk flags, bit 0 conventionally write-protect;
0x08 first bad sector; 0x0A six reserved bytes. **The payload begins at offset
0x10.**

**Sector layout.** Sectors are numbered from 1, and for 128-byte sectors the
mapping is uniform: **file offset of sector *n* = 16 + (*n* − 1) x 128**. *Worked
example:* sector 5 is at offset 528; sector 360 at 45,968; sector 720 at
92,048, ending one byte short of the file's end. There is no compression
anywhere in this container.

**The short-first-three-sectors rule** applies only to 256-byte-per-sector
images and matters if an implementer widens support: sectors 1, 2 and 3 are
stored as 128 bytes each because the machine's boot loader always reads them in
single density, so sector *n* for *n* ≥ 4 is at 16 + 384 + (*n* − 4) x 256. A
minority of such images store all sectors at 256 bytes with no flag to say so;
the only discriminator is arithmetic — check whether the declared paragraph
count matches 3 x 128 + (sectors − 3) x 256 or sectors x 256 — and an image
satisfying neither must be refused.

**There is no directory walk.** For these releases the database is taken as a
byte range beginning at **file offset 0x04C1** and running to the end of the
file, with the game's header proper at **file offset 0x04F9** and the fifty
bytes in front taken only so the array has the same shape as a Commodore 64
program file. That offset lands at byte 49 of sector 10. **This is a property
of how these particular disks were mastered, not of the container**, and it
will not survive a differently-mastered disk; an implementer wanting generality
must walk the disk's own filesystem, which is publicly documented (a volume
table of contents in sector 360, a directory in sectors 361-368 of eight
sixteen-byte entries each, and a three-byte trailer in each data sector giving
the file number, the next sector and the byte count used).

**One splice is required.** The 128 bytes at file offsets 0xB390 through 0xB40F
are **sector 360**, the volume table of contents, and are not game data. Any
extracted range spanning them must have them excised and the following bytes
moved down. The arithmetic confirms both the header size and the sector stride:
16 + 359 x 128 = 45,968 = 0xB390.

**There is no memory base for this platform** — a reader works in disk-image
coordinates throughout, which is why every Atari picture offset in §8 is a file
offset.

**Companion disks.** These releases are two-sided, one side holding the
database and the other the pictures, and the second side is located by
**filename manipulation**, never by content: a catalogue of known filename pairs
matched on (size, whole-file checksum, exact filename, filename length), then a
heuristic that scans the given name backwards for the word `side` or `disk`
(case-insensitively), requires the next character to be a space or underscore,
and flips the character after that (`a` to `b`, `1` to `2`, `one` to `two`),
with fallbacks that strip or insert a bracketed suffix. This is filesystem
convention, not container format, and an implementer is free to substitute an
explicit "second file" argument.

**Refusals.** Any sector size but 128; any image size but 5,760 paragraphs; the
high size byte at 0x06 is never consulted, so images above 1 MiB must be
refused; header-less images of the same content are not recognised and should
be refused by name rather than guessed at by size; and a failed database parse
at 0x04C1 is a refusal, not a reason to search.

### 7.4 Apple II disk images

**Two containers are recognised.** A bit-preserving image is identified by
ASCII `W`, `O`, `Z` at offsets 0x00-0x02, then `1` or `2` at 0x03, then the four
fixed bytes `FF 0A 0D 0A` at 0x04-0x07; any mismatch is a hard error. Anything
else with at least **143,360 bytes** (35 x 16 x 256) is taken as a flat
sector image.

**Three important negatives**, each of which an implementer must name and
refuse rather than mishandle: a **raw nibble image** (232,960 bytes) is not
detected and would be read as sector data; **ProDOS sector ordering** is
indistinguishable from DOS 3.3 ordering by content, and a reader assuming DOS
3.3 will parse a ProDOS-ordered image structurally and yield wrong data; and
the **`2IMG` wrapper**, with its 64-byte header, is not recognised at all.

**Flat sector geometry.** 35 tracks numbered 0-34, 16 sectors numbered 0-15, 256
bytes each, no header: **offset = track x 4096 + sector x 256**. When reading
from a *nibble* stream the logical-to-physical skew must be applied — logical 0
through 15 map to physical 0, 13, 11, 9, 7, 5, 3, 1, 14, 12, 10, 8, 6, 4, 2,
15 — but when reading a flat DOS-ordered image the logical number **is** the
file index, which is the definition of that ordering.

**Volume table of contents** at track 17 sector 0: +0x01 track of the first
catalogue sector, +0x02 its sector, +0x03 the DOS release, +0x06 the volume
number, +0x27 the maximum track/sector pairs per list sector (normally 122),
+0x34 tracks per disk (expect 35), +0x35 sectors per track (expect 16), +0x36
bytes per sector, +0x38 onward a free-sector bitmap. A first-catalogue track of
35 or more, or sector of 16 or more, means this is not such a disk.

**Catalogue sectors** chain: +0x01 next track (0 ends the chain), +0x02 next
sector, then **seven 35-byte entries from +0x0B**. Entry layout: +0x00 track of
the file's first track/sector list, where **0x00 means never used and 0xFF
means deleted**; +0x01 its sector; +0x02 file type and lock flag, bit 7 meaning
locked and the low bits giving the type (0x00 text, 0x01 integer BASIC, 0x02
Applesoft, 0x04 binary, 0x08, 0x10, 0x20, 0x40 others); +0x03 a 30-byte
filename in high ASCII, space-padded; **+0x21 the file length in sectors**,
little-endian. A reader must stop on a self-referential link, on a next-track
of 35 or more or next-sector of 16 or more, and after 64 sectors.

**Filename normalisation**, applied per byte before comparison: if bit 7 is set
and the byte is 0xA0 or above, clear bit 7; if bit 7 is set and the byte is
below 0xA0, clear bit 7 and add 0x20; if bit 7 is clear, mask to the low six
bits, exclusive-OR with 0x20, then add 0x20. Then strip trailing spaces.
*Worked example:* stored `C4 C1 D4 C1 A0 A0` all have bit 7 set and are at or
above 0xA0, so they become `44 41 54 41 20 20` = `DATA` plus spaces, trimming to
`DATA`.

**Track/sector lists** chain the same way: +0x01 next track (0 ends), +0x02 next
sector, +0x05 the sector offset of the first entry (should be a multiple of
122), then **up to 122 track/sector pairs from +0x0C, track byte first**. A pair
of (0, 0) is a **sparse** sector: the file logically holds 256 zero bytes there
and no sector is read. A pair whose track is 35 or more, whose sector is 16 or
more, or whose track is 0 with a non-zero sector, is invalid. If another list
sector follows, keep all 122 pairs including trailing zeros, since they may be
sparse; if this is the last, keep up to the last non-zero pair. Bound the chain
at 32 list sectors. **A file's data is the concatenation of the sectors named
by its pairs, in order.**

**A catalogue entry's sector count includes the file's own list sectors**, so a
reader must allocate (count) x 256 bytes but read only (count − 1) x 256, or it
reads a sector of rubbish past the end.

**Field framing on a nibble track.** For the 16-sector format, an address field
is the prologue `D5 AA 96`, eight bytes carrying four values in the two-bytes-
per-byte encoding (volume, track, sector, checksum), then the epilogue
`DE AA EB`; a data field is the prologue `D5 AA AD`, 343 encoded bytes, then the
same epilogue. The address checksum is the exclusive-OR of volume, track and
sector. The **track is a circular buffer of 6,656 bytes** and both the prologue
search and the field extraction must wrap.

**The two-bytes-per-byte encoding.** A value *V* is stored as ((*V* shifted
right one) OR 0xAA) then (*V* OR 0xAA). Decoding is ((first byte shifted left
one) OR 1) AND (second byte). *Worked examples:* 0xFE encodes as `FF FE`, and
(0xFF ≪ 1 | 1) AND 0xFE = 0xFE; 0x03 encodes as `AB AB`, and (0xAB ≪ 1 | 1) AND
0xAB = 0x03.

**The six-and-two data encoding.** A 256-byte sector is stored as 343 encoded
bytes: 86 carrying the low two bits of each byte, then 256 carrying the high
six bits, then a checksum. The sixty-four legal disk bytes, indexed by their
six-bit value, are:

```
 0..7 : 96 97 9A 9B 9D 9E 9F A6
 8..15: A7 AB AC AD AE AF B2 B3
16..23: B4 B5 B6 B7 B9 BA BB BC
24..31: BD BE BF CB CD CE CF D3
32..39: D6 D7 D9 DA DB DC DD DE
40..47: DF E5 E6 E7 E9 EA EB EC
48..55: ED EE EF F2 F3 F4 F5 F6
56..63: F7 F9 FA FB FC FD FE FF
```

Every other byte value is invalid and its appearance means the field is
unreadable. Every entry is 0x96 or above and every entry has bit 7 set — the
defining property.

Decoding, with a running value starting at 0:

*Stage one, the 86 two-bit bytes.* For each, at index *i* from 0 to 85: look it
up; exclusive-OR the six-bit value into the running value; then take three
two-bit groups from the running value **with the bits of each group swapped** —
bits 1 and 0 give the low two bits destined for output byte *i*, bits 3 and 2
for byte *i* + 86, bits 5 and 4 for byte *i* + 172. (For a group whose high bit
is *h* and low bit is *l*, the stored pair is 2*l* + *h*.) This produces 258
two-bit values, of which only the first 256 are used.

*Stage two, the 256 six-bit bytes.* For each, at index *i*: look it up;
exclusive-OR into the running value; output byte *i* is ((running value shifted
left two) OR the two-bit value for position *i*) taken modulo 256.

*Stage three.* Look up the 343rd byte and exclusive-OR it into the running
value; **the result must be zero**.

*Worked example.* With a seed of 0, suppose the first three two-bit bytes are
`9F 9A 96` and the first three six-bit bytes are `FF 96 B4`.

| stage | position | disk byte | value | running | two-bit for *i* |
|---|---|---|---|---|---|
| one | 0 | `9F` | 6 (`000110`) | 6 | bits 1:0 = `10`, swapped = 1 |
| one | 1 | `9A` | 2 | 4 | bits 1:0 = `00`, swapped = 0 |
| one | 2 | `96` | 0 | 4 | 0 |

| stage | position | disk byte | value | running | output |
|---|---|---|---|---|---|
| two | 0 | `FF` | 63 | 59 | (59 ≪ 2) OR 1 = 0xED |
| two | 1 | `96` | 0 | 59 | (59 ≪ 2) OR 0 = 0xEC |
| two | 2 | `B4` | 16 | 43 | (43 ≪ 2) OR 0 = 0xAC |

The first three recovered bytes are `ED EC AC`. Were the field to end there, the
running value of 43 would demand a checksum byte of `E7`, the table entry at
index 43.

**Five-and-three encoding is not covered here.** The thirteen-sector format uses
it with a different prologue set, and a thirteen-sector disk must be named and
refused rather than attempted.

**Bit-preserving images.** Offsets 0x00-0x07 are the signature; 0x08 is a
little-endian CRC-32 (reflected, polynomial 0xEDB88320, initial and final
complement) of everything from 0x0C onward; chunks begin at 0x0C. **A stored CRC
of zero means "not computed" and must be accepted, not refused.** Each chunk is
a four-byte ASCII identifier, a four-byte little-endian size, and that much
data; three are meaningful and all three must be present.

- The information chunk **must be exactly 60 bytes**. Byte 0 is the format
  version; **byte 1 is the disk type and only value 1 (5.25-inch) is
  supported**; byte 2 write-protected; byte 3 synchronised; byte 4 cleaned;
  bytes 5-36 a creator string; from version 2, **byte 37 is the number of sides
  and only 1 is supported**; byte 38 the boot sector format; byte 39 the optimal
  bit timing; bytes 40-41 a compatible-hardware mask; 42-43 required RAM; 44-45
  the largest track in blocks; and, from version 3, 46-47 and 48-49 flux fields.
- The track-map chunk **must be exactly 160 bytes**, one per quarter track;
  **entry 4*n* names the track-data index for whole track *n***, and 0xFF means
  the track is empty.
- The track-data chunk differs by version. In version 1 it is a sequence of
  **6,656-byte records**, one per track index: bytes 0-6,645 the bitstream,
  6,646-6,647 the byte count used, 6,648-6,649 the bit count, 6,650-6,651 the
  splice point, 6,652 the splice nibble, 6,653 the splice bit count, 6,654-6,655
  reserved. In version 2 it begins with **160 eight-byte index entries**: bytes
  0-1 the starting block, 2-3 the block count, 4-7 a 32-bit bit count. A block
  is **512 bytes** and the starting block is an **absolute offset within the
  file**, so a track's bitstream begins at (starting block x 512).

**Recovering a nibble stream from a bitstream.** The bitstream is **circular**
and its byte 0 is an arbitrary rotation of the physical track.

1. **Find the index** by locating a run of self-sync bytes, the only
   unambiguous landmark. Two patterns are searched for, at every **bit**
   position, not merely every byte: a sixteen-sector run of five consecutive
   ten-bit sync bytes — `11111111 00` repeated five times, 50 bits — or a
   thirteen-sector run of eight consecutive nine-bit sync bytes — `11111111 0`
   repeated eight times, 72 bits.
2. **Rotate** the circular stream left so it begins immediately after the run:
   by (found bit position + 50) or (found bit position + 72). If no run is
   found, rotate left by (bit count − 72) to bring the last 72 bits to the front
   in case the run straddles the arbitrary cut, and search again. If it is still
   not found, proceed unrotated but report it.
3. **Extract nibbles**: skip forward over 0 bits; take the **eight bits
   beginning at the next 1 bit** as one disk byte; advance past them. Because
   every disk byte has bit 7 set, this rule is self-synchronising. Continue
   until the bit count is exhausted or 6,656 nibbles are produced, then zero-pad
   to 6,656.
4. **Assemble** 35 tracks of 6,656 nibbles in whole-track order, giving 232,960
   bytes; track *t* occupies bytes *t* x 6,656 onward and a sector is found by
   searching that window circularly.

*Worked example.* A bitstream beginning `00 2F A8` is the bits
`0000 0000 0010 1111 1010 1000`. Skipping the eight zeros of the first byte and
the two of the second, the next 1 bit is at position 10, and the eight bits from
there are `10111110` = **0xBE**, a valid disk byte (index 25). The position
advances to bit 18.

**From container to data.** This path does not produce a memory image; it
produces the contents of a file, and a **binary file begins with a four-byte
prologue: bytes 0-1 the load address, little-endian, bytes 2-3 the data length,
with the data from byte 4**. So the array's byte 4 corresponds to the address in
its own bytes 0-1; there is no fixed platform base. The database offset applied
on top for these releases is **0x135**, with the game header proper at **0x016D**
and fifty bytes taken in front to match the Commodore 64 array shape; if the
parse there fails, two fallbacks are tried at 0x3D00 and 0x3803 further in.
These three offsets are release-mastering constants, not container properties.

**Which file inside.** By name: the first catalogue entry whose normalised name
is exactly `DATABASE`, exactly `SORCEROR OF CLAYMORGUE CASTLE` (the spelling on
the disk), exactly `THE INCREDIBLE HULK`, or which matches the pattern of a
first character `A`, a `.` in the third position and `DAT` in the fourth
through sixth. Two further names are used when present: `PAK.INVEN` or
`PAC.INVEN` for the inventory picture, whose four-byte prologue is stripped; and
`M2`, which in some releases carries a picture-descrambling table taken as the
0x182 bytes at file offset 0x174B — **but only when the 31 bytes at 0x172C read
exactly `COPYRIGHT 1983 NORMAN L. SAILER`**. That string test is a good model:
validate before trusting a fixed offset.

**Refusals**, beyond the three negatives above: thirteen-sector disks; 3.5-inch
or multi-sided bit-preserving images; an information chunk that is not 60 bytes
or a track map that is not 160; a missing chunk; and a disk whose database file
carries none of the four recognised names, since there is no largest-file
fallback here. Two tolerances worth stating as tolerances rather than
oversights: verifying only the first two epilogue bytes, and not verifying that
an address field's track matches the track being read, both of which help with
imperfectly dumped disks and both of which can silently return data from the
wrong place. **Verify both checksums** and report a mismatch.

### 7.5 Base addresses, gathered

| platform | result of decoding | first byte corresponds to |
|---|---|---|
| ZX Spectrum | exactly 49,152 bytes | **address 0x4000**, fixed |
| Commodore 64 | a program file | the address in **its own first two bytes** (array byte 2 is that address) |
| Atari 8-bit | a slice of the flat disk image | **no memory address at all** — a file offset only; the game header is at file offset 0x04F9 |
| Apple II | a DOS file | the address in **its own bytes 0-1** (array byte 4 is that address) |
| TI-99/4A | the file itself | address 0x0380, absorbed by the baseline (§3.1) |

**Only the ZX Spectrum has a fixed base.** Assuming 0x0801 for a Commodore 64
release is a mistake: the catalogued copy instructions use absolute addresses in
the tens of thousands, which only make sense against a load address read from
the file.

---

## 8. SAGA picture data

Scope: decoding bytes to a pixel array with a palette index per pixel.
Presentation — windows, scaling, timing — is out of scope except where it
changes which pixels exist.

**Five distinct picture formats** exist across this family, two British and
three American.

| family | used by |
|---|---|
| **A** — character-cell display list | The Adventure International UK releases: Adventureland, Secret Mission, Claymorgue, Hulk, Spider-Man, Savage Island I and II, Gremlins (four language variants), Supergran, Robin of Sherwood, Seas of Blood — each in a ZX Spectrum and a Commodore 64 form |
| **B** — vector / line drawing | The ten Mysterious Adventures titles, ZX Spectrum and Commodore 64, flagged by a picture-format-version value of 99 |
| **C** — four-colour strip bitmap | US disk releases for Commodore 64 and Atari 8-bit: Adventureland, Pirate Adventure, Voodoo Castle, The Count, Claymorgue, Hulk |
| **D** — Apple II high-resolution page | The same US titles on Apple II, in a plain and a "scrambled" sub-variant |
| **E** — CGA bitmap | The MS-DOS release of Hulk |

**The family is never a magic number inside the picture data.** It follows from
the container and the identified release: an Atari disk image selects C with
Atari colour handling; an Apple II image selects D; a Commodore 64 disk selects
C for a US release and A or B for a UK one; a ZX snapshot selects A or B. For a
UK release the identified record carries a picture-format-version number: **99
selects family B; 0 through 4 select family A** and pick its sub-version. A
record with a picture count of zero has no pictures — which is the case for the
UK Pirate Adventure, Voodoo Castle, Strange Odyssey and Buckaroo Banzai.

### 8.1 Family A — character-cell display list

**Canvas.** 32 cells wide by 12 high, that is **256 x 96 pixels**, cells of
8 x 8, one bit per pixel in the shape plane with colour supplied separately at
cell resolution. A picture occupies (width x height) cells from the cell
position in its header; cells outside it are untouched, and successive pictures
composite onto the same canvas — which is exactly how item overlays work.

Within a cell the eight bytes are the eight pixel rows top to bottom, and **the
most significant bit of a byte is the leftmost pixel**. There is no
display-file interleave: whole cells are stored consecutively. Cells within a
picture are row-major, index 0 being the top-left.

**Locating the data.** Three per-release constants plus a derived value: the
**character-set address** (exactly 2048 bytes, 256 characters of 8), the
**offset-table address** (which may be "immediately after the character set",
i.e. character-set address + 0x800), and a **picture-data base**, an additive
constant converting an offset-table value into a file position — typically
about −0x3FE5 for a ZX snapshot, reflecting the 0x4000 base, and an absolute
positive value for a Commodore 64 release. Everything is additionally shifted by
the whole-file baseline of §4.3. The offset table is little-endian 16-bit
values, one per picture in index order; picture *n* begins at (table value *n*)
+ (picture-data base) + (baseline). Picture counts run from 21 to 139.

**Per-picture header:** byte 0 width in cells, clamped to 32; byte 1 height in
cells, clamped to 12; byte 2 horizontal placement in cells, replaced by 4 if
above 32; byte 3 vertical placement in cells, replaced by 0 if above 12.
**Version 0 has no bytes 2 and 3.** There, pictures 10 through 27 take their
placement from a separate 36-byte coordinate table — horizontal from
(table + index − 10), vertical from 18 bytes further on — and all other pictures
sit at 0,0. Version 0 also uses **four** separate pointer tables rather than one:
pictures 0-10 from the offset table, 11-27 from an item-picture table,
28-33 from a look-picture table, 34 upward from a special-picture table, each
indexed by (index − the range's start) x 2, and the value found is increased by
a further per-release constant before the general rule applies.

**No length is stored.** The picture ends when the decode terminates on cell
count, so every read must be bounded against the file length.

**The shape display list.** A cell cursor starts at 0; the stream ends when it
reaches (width x height), and a decoder must also stop if it would exceed 767.

A **byte below 0x80** is a bare character index, written into the current cell
in replace mode with no transformation; the cursor advances by one. In versions
3 and 4 only, if the *previously selected* index was above 127, 128 is added to
this one.

A **byte of 0x80 or above** is a mode byte:

| bit | meaning |
|---|---|
| 0 | add 128 to the character index that follows, if that index is below 128 |
| 1 | a repeat-count byte follows the mode byte, before the character index |
| 2-3 | combining mode for the overlay that follows: 0 none, 4 OR, 8 AND, 12 XOR |
| 4-5 | rotation: 0 none, 0x10 90 degrees clockwise, 0x20 180, 0x30 270 clockwise |
| 6 | mirror horizontally, applied after any rotation |
| 7 | always set; this is what marks the byte as a mode byte |

The mode byte is followed by an optional repeat count — present only when bit 1
is set, the run length being that byte **plus one** — then one character index.
The selected character is written into (run length) consecutive cells from the
cursor. **If bits 2-3 are non-zero the base character is written in replace
mode**, the combining bits stripped for this first write, because those bits
describe how the *next* shape combines with this one.

When bits 2-3 are non-zero an **overlay chain** follows. Remember that combining
mode, then read the next byte. If it is **below 0x80** it is a bare overlay
index — in version 4 only, +128 if the previous mode byte had bit 0 set — and
that character is combined into the same cells with the remembered mode, no
rotation or mirroring, and the chain ends. If it is **0x80 or above** it is
another mode byte followed by one index, with +128 applied unconditionally if
the new mode byte's bit 0 is set; that character is combined into the same cells
using the new mode byte's rotation and mirror but the *remembered* combining
mode, and if the new mode byte's bits 2-3 are also non-zero they become the new
remembered mode and the chain continues, otherwise it ends.

The cursor advances by the run length after the base write and the whole chain:
**a chain builds one composite cell or run, it does not advance the cursor.** A
character index above 255, which the +128 rules can produce, selects nothing and
that write is skipped.

**The transformations, exactly.** With *source(c, r)* the source pixel at column
*c*, row *r*, both 0-7, column 0 leftmost and row 0 top:

- rotation 0: *result(x, y) = source(x, y)*
- rotation 0x10: *result(x, y) = source(y, 7 − x)*
- rotation 0x20: *result(x, y) = source(7 − x, 7 − y)*
- rotation 0x30: *result(x, y) = source(7 − y, x)*

and the mirror bit then maps *result(x, y)* to *result(7 − x, y)*. The combining
modes act bitwise per pixel row against whatever is in the destination cell.

**There is no fill primitive in this family.** Solid areas are made of solid
characters; a decoder that implements a flood fill here is wrong.

**Colour.** Immediately after the shape stream, with no separator, comes one
colour byte per cell in the same row-major order, run-length compressed over
exactly (width x height) cells. Read a byte: **bit 7 clear** means the byte is a
colour applying to one cell. **Bit 7 set** introduces a run, and the two version
groups differ — in **versions 3 and 4** the run length is the low seven bits and
the colour is the *previously emitted* one, with no colour byte following; in
**versions 0, 1 and 2** the run length is the low seven bits **plus one** and
the colour is the *next* byte, which is consumed.

Attribute byte layout also differs. **Versions 3 and 4**: bits 0-2 ink, bits 3-5
paper, bit 6 bright (which adds 8 to *both* ink and paper), bit 7 ignored.
**Versions 0, 1 and 2**: bits 0-2 paper, bit 3 bright, bits 4-6 ink, bit 7
ignored. Additionally, **in versions 0 and 1 the bright bit is forced on**
whatever its stored value. The flash bit is discarded everywhere; nothing in
any family animates a flashing attribute.

Composition: within each cell, every pixel whose shape bit is set takes the ink
index and every pixel whose shape bit is clear takes the paper index, so an
overlay's background rectangle erases what was under it. One placement
adjustment: **in versions 1 and 2 only**, the picture's horizontal cell offset
is reduced by 4 before use.

**Palettes.** The index space is 16 entries, 0-7 normal and 8-15 bright. The
optimised Sinclair palette, which every ZX release but one selects:

| index | colour | R,G,B | index | colour | R,G,B |
|---|---|---|---|---|---|
| 0 | black | 0,0,0 | 8 | bright black | 0,0,0 |
| 1 | blue | 0,0,202 | 9 | bright blue | 0,0,255 |
| 2 | red | 202,0,0 | 10 | bright red | 255,0,20 |
| 3 | magenta | 202,0,202 | 11 | bright magenta | 255,0,255 |
| 4 | green | 0,202,0 | 12 | bright green | 0,255,0 |
| 5 | cyan | 0,202,202 | 13 | bright cyan | 0,255,255 |
| 6 | yellow | 202,202,0 | 14 | bright yellow | 255,255,0 |
| 7 | white | 202,202,202 | 15 | bright white | 255,255,255 |

The measured Sinclair palette, selected only by the German Gremlins release,
differs in using 154 in place of 202 for indices 1-7 and, for the bright half,
0,0,170 / 186,0,0 / 206,0,206 / 0,206,0 / 0,223,223 / 239,239,0 / 255,255,255.

The Commodore 64 palette, used by every Commodore 64 release:

| index | colour | R,G,B | index | colour | R,G,B |
|---|---|---|---|---|---|
| 0 | black | 0,0,0 | 8 | orange | 186,134,32 |
| 1 | white | 255,255,255 | 9 | brown | 116,105,0 |
| 2 | red | 191,97,72 | 10 | light red | 231,154,132 |
| 3 | cyan | 153,230,249 | 11 | dark grey | 69,69,69 |
| 4 | purple | 177,89,185 | 12 | grey | 167,167,167 |
| 5 | green | 121,213,112 | 13 | light green | 192,255,185 |
| 6 | blue | 95,72,233 | 14 | light blue | 162,143,255 |
| 7 | yellow | 247,255,108 | 15 | light grey | 200,200,200 |

**The remap step.** Commodore 64 releases store the *ZX Spectrum's* colour
indices in their attribute bytes — the artwork was converted, the numbering was
not — so every ink and paper index passes through a sixteen-entry remap table
before the palette lookup. Four tables exist:

| source | A | B | C | D |
|---|---|---|---|---|
| 0 | 0 | 0 | 0 | 0 |
| 1 | 6 | 6 | 6 | 6 |
| 2 | 2 | 9 | 2 | 2 |
| 3 | 4 | 4 | 4 | 4 |
| 4 | 5 | 5 | 5 | 5 |
| 5 | 3 | 14 | 14 | 3 |
| 6 | 7 | 8 | 8 | 8 |
| 7 | 1 | 12 | 12 | 1 |
| 8 | 8 | 0 | 0 | 0 |
| 9 | 1 | 6 | 6 | 6 |
| 10 | 1 | 2 | 2 | 2 |
| 11 | 1 | 4 | 4 | 4 |
| 12 | 7 | 5 | 5 | 5 |
| 13 | 12 | 3 | 3 | 3 |
| 14 | 8 | 7 | 7 | 7 |
| 15 | 7 | 1 | 1 | 1 |

Table A serves the Mysterious Adventures Commodore 64 releases (family B) and
is an eight-colour mapping, which is why its upper half collapses. Table B
serves Hulk, Adventureland, Secret Mission, Claymorgue, Savage Island I and II,
every Gremlins variant, Supergran and Robin of Sherwood on Commodore 64. Table C
serves only Spider-Man and table D only Seas of Blood. **These tables were
derived by eye from emulator captures and are acknowledged as possibly
containing mistakes.** For ZX releases the remap is the identity, and an index
outside 0-15 must be reported as invalid rather than clamped.

**Compression.** The shape stream's run counts and the attribute stream's runs
are the only compression, and there is no length field: the picture's extent is
its cell count. **A decoder must not begin reading the attribute stream until
the shape stream has consumed exactly (width x height) cells**, because the two
are adjacent with no marker between them.

**Worked example, format version 3.** A two-cell picture, one cell high, at the
canvas origin. The byte string is constructed for this document.

Character-set entries used: character 0x05 is `FF 81 81 81 81 81 81 FF`, a
hollow box, and character 0x06 is `00 00 18 18 18 18 00 00`, a small block.
The picture bytes:

```
02 01 00 00   05   84 06 05   47 81
```

Header: width 2, height 1, offsets 0 and 0; two cells to fill.

Shape stream. `05` is below 0x80, a bare index; the previously selected index
was 0, so no +128 applies; character 5 goes into cell 0 in replace mode and the
cursor moves to 1. `84` is a mode byte: bit 0 clear, bit 1 clear (run length 1),
bits 2-3 hold 4, meaning the overlay that follows combines with OR, bits 4-6
clear. `06` is the character index; because bits 2-3 are non-zero the base is
written in **replace** mode, so cell 1 becomes character 6. `05` is the next
byte, below 0x80, so a bare overlay index; this is version 3 not 4, so no +128;
character 5 is OR-ed into cell 1 and the chain ends. The cursor moves to 2,
which equals width x height, so the shape stream is complete after four bytes.
Cell 0 holds `FF 81 81 81 81 81 81 FF` and cell 1 holds character 6 OR character
5 = `FF 81 99 99 99 99 81 FF`.

Attribute stream. `47` has bit 7 clear, so one cell of colour 0x47: version 3
layout gives ink = 0x47 AND 7 = 7, paper = (0x47 ≫ 3) AND 7 = 0, and bit 6 is
set so both gain 8 — cell 0 has ink 15 on paper 8. `81` has bit 7 set: version 3
rule, run length = low seven bits = 1, colour = the previously emitted 0x47, so
cell 1 is the same. Two cells consumed; the stream ends.

The resulting 16 x 8 grid, `#` for palette index 15 and `.` for index 8, cell 0
in columns 0-7 and cell 1 in columns 8-15:

```
row 0:  # # # # # # # #   # # # # # # # #
row 1:  # . . . . . . #   # . . . . . . #
row 2:  # . . . . . . #   # . . # # . . #
row 3:  # . . . . . . #   # . . # # . . #
row 4:  # . . . . . . #   # . . # # . . #
row 5:  # . . . . . . #   # . . # # . . #
row 6:  # . . . . . . #   # . . . . . . #
row 7:  # # # # # # # #   # # # # # # # #
```

Under the optimised Sinclair palette that is bright white on bright black. On a
Commodore 64 release using remap table B, index 15 becomes entry 1 (white) and
index 8 becomes entry 0 (black).

The example demonstrates three things about the format: the mode byte's
combining bits describe the *following* shape, not itself; the base character
therefore goes down in replace mode with the overlay OR-ed on top; and a version
3 or 4 attribute run byte carries no colour of its own.

### 8.2 Family B — the Mysterious Adventures vector format

**Canvas.** A plain **255 x 94** pixel canvas, one palette index per pixel, no
cell structure and no attribute plane. The stored coordinate system is bottom-up
over a taller space: **a stored vertical value *v* means canvas row 190 − *v***,
so pictures occupy the top part of what was originally a 192-line screen. Any
point resolving to a row of 94 or more, or a negative row, is discarded.
Horizontal values are used directly, 0 to 254.

**Locating the data.** All the images for a game sit in one contiguous block
whose start is either an explicit per-release address or "immediately after the
item-location table". It is a simple concatenation, one image per room in room
order, and **the picture count equals the room count**. The block begins with a
byte of 0xFF; each image is a background-colour byte followed by an opcode
stream terminated by 0xFF, and that terminator simultaneously introduces the
next image. **There is no index table**: reaching image *n* means walking *n*
terminators. A block that runs past the end of the file part-way through leaves
every remaining room with no picture.

**The opcodes.** Two state variables are carried: a current point, initially
(0, 0), and a line colour fixed for the whole image.

| opcode | operands | effect |
|---|---|---|
| 0xC0 | *v*, then *h* | move: set the current point to horizontal *h*, vertical (190 − *v*); draws nothing. **Vertical operand first.** |
| 0xC1 | *colour*, *v*, *h* | flood fill from horizontal *h*, vertical (190 − *v*), in *colour* |
| 0xFF | none | end of image |
| 0x00-0xBF | *h* | draw a line: **the opcode byte is itself the vertical operand**, so the line runs from the current point to horizontal *h*, vertical (190 − opcode), in the line colour; the current point becomes that endpoint |

Because the opcode byte doubles as a vertical coordinate, only 0x00-0xBF can
express an endpoint and only 0x61-0xBE resolve to a visible row (93 down to 0).
The three reserved values correspond to rows −2, −3 and −65, which is precisely
why they are safe to steal.

**The line colour is not stored.** It is derived: **if the background colour
index is 0 the line colour is 7; otherwise it is 0.**

Lines are drawn with an integer Bresenham rasteriser, both endpoints inclusive,
one pixel per step along the major axis. For bit-exact agreement: take the
absolute deltas and step signs; if the horizontal delta is the larger, double
both deltas, set the error accumulator to (vertical delta − horizontal delta),
and step horizontally, adding a vertical step and subtracting the horizontal
delta whenever the accumulator is non-negative, then adding the vertical delta;
otherwise do the same with the roles exchanged. Plot before each step and once
more at the end.

**The fill algorithm, exactly** — this is the one place where fill order changes
the output.

- **4-connected**, seeded at the single point the opcode gives.
- **Boundary condition:** a pixel is filled only if its current value equals
  **the image's background colour index** — not the colour at the seed, and not
  "anything other than the boundary". A fill therefore stops at lines *and* at
  anything an earlier fill painted, and a fill whose seed is not on a
  background-coloured pixel does nothing at all.
- **Breadth-first over a first-in-first-out queue**, not recursion and not
  scanline. Enqueue the seed; repeatedly take the front point, and if it is
  inside the canvas and its current value is the background colour, paint it and
  enqueue its four neighbours in the order below, above, right, left. **The test
  and the paint both happen at dequeue time**, so the queue routinely holds
  duplicates.
- **The queue is bounded at 1024 points, and further enqueues are silently
  dropped.** That is the reference behaviour, and reproducing it is the safe
  choice. **Measured, it makes no difference at this canvas size.** Decoding
  every picture of all eleven Commodore 64 releases of §10.4 — 516 images and
  2,227 individual fills — the queue's greatest depth ever reached is **677**,
  well under the bound, so not one enqueue is dropped and the bounded and
  unbounded decoders produce byte-identical canvases. A single unobstructed fill
  of the whole empty 255 x 94 canvas paints all **23,970** pixels either way and
  peaks at a depth of 421. The reason is structural: a breadth-first frontier on
  a grid this shape is bounded by the perimeter it has reached, not by the area
  it has painted. So the bound is a fact about the reference and not, on any
  measured specimen, a fork in the output; an implementer who omits it should
  say so, and one who keeps it need not defend the choice with a comparison.
- Fills and lines must run in strict stream order, since each fill's boundary
  test sees everything done before it.

Neighbour coordinates are computed as unsigned bytes, so a neighbour left of
column 0 becomes column 255 and one above row 0 becomes row 255; both are then
rejected by the bounds test, and signed arithmetic gives the same result.

**Colour.** The canvas is first set to the background index. Every value written
is a raw index in the same 0-15 space as family A and passes through the same
remap and palette machinery: ZX releases use the optimised Sinclair palette with
an identity remap, Commodore 64 releases the Commodore 64 palette with **remap
table A**. There is no attribute plane, no cell resolution, no bright bit and no
flash bit. There is no compression; the opcode stream is the storage form.

**One bounds quirk.** The horizontal test admits column 255 as well as 0-254,
even though rows are 255 pixels wide, so a pixel plotted at column 255 of row
*r* lands at column 0 of row *r* + 1. Reproduce it only if bit-exactness
matters; otherwise clamp to 0-254 and note the deviation.

### 8.3 Family C — Commodore 64 and Atari 8-bit US bitmaps

**Pixels.** Two bits per pixel, four colours, stored **four pixels to a byte,
most significant pair leftmost**. Each stored pixel is two device pixels wide,
so a byte covers eight horizontal positions; the width field is in those
8-pixel columns and the height field in pixel rows. Nominal canvas 280 x 158.

**Storage order is column-major in 8-pixel strips.** Bytes are consumed in
pairs, and a pair paints two consecutive rows of the *same* column, the first
byte upper and the second lower. The row counter then advances by two; when it
passes the height, the column advances by one 8-pixel step and the row counter
resets to the picture's vertical offset. Writing stops when the horizontal
position exceeds (width − 3) x 8; further pairs are consumed and discarded.

**Header**, twelve bytes: 0-1 the container load address, ignored; 2-3 a
little-endian data size, informational; **4 the horizontal placement in 8-pixel
columns plus 3** (subtract 3 to use it); 5 the vertical placement in pixel rows;
6 the width in columns; 7 the height in rows; 8-11 four colour bytes. Data
begins at byte 12 and runs to two bytes before the record's end.

**Compression**, a byte-pair run-length scheme repeated until the record is
exhausted: read a control byte; if **bit 7 is set**, the repeat count is the low
seven bits **plus one** and the next two bytes are a pixel-byte pair emitted
that many times; if **bit 7 is clear**, the literal count is the byte **plus
one** and that many pixel-byte pairs follow, each emitted once. **The Count and
Voodoo Castle use a variant with no literal mode**: every control byte is a
repeat count, bit 7 is not masked, the count is the byte's full value, and two
is subtracted from the stored height before decoding.

**Colour.** The four header bytes define the picture's palette, but **entry 0 is
forced to black** and the four stored bytes load into entries 1 through 4. Since
pixel values are only 0-3, the effective mapping is: value 0 always black,
whatever the file says; value 1 the first stored byte; value 2 the second; value
3 the third; **and the fourth stored byte is never used.** This is a known
deviation from what the original machines did, where a background register and
three playfield registers are all live, and an implementer should refuse to
guess a better mapping without evidence.

**Atari colour bytes** index a 256-entry palette in the conventional
hue x luminance arrangement, index = hue x 16 + luminance, whose luminance-only
row (indices 0-15) is 0x00, 0x0E, 0x1D, 0x2C, 0x3B, 0x4A, 0x59, 0x68, 0x77,
0x86, 0x95, 0xA4, 0xB3, 0xC2, 0xE0, 0xE0, each value used for all three
channels. That base table must be transcribed from an Atari palette reference;
it cannot responsibly be reconstructed from prose. Sixteen entries are
hand-substituted and must be overridden to match: index 14 to 0xE0E0E0; indices
18, 37, 50, 54, 58 and 247 to 0xAD5F64; index 86 to 0x4B1EAD; index 133 to
0x3468EE; index 198 to 0x2B5800; index 199 to 0x3A6700; index 216 to 0x637000;
index 228 to 0x944C02; index 248 to 0x8D5900; index 255 to 0xBA8600.

**Commodore 64 colour bytes are not palette indices at all.** Their meaning was
recovered empirically and the mapping is a bare lookup with no arithmetic
structure, onto thirteen colours: black 0,0,0; white 255,255,255; red 191,97,72;
purple 177,89,185; green 121,213,112; blue 95,72,233; yellow 247,255,108; orange
186,134,32; brown 131,112,0 (note this differs from family A's brown); light red
231,154,132; grey 167,167,167; light green 192,255,185; light blue 162,143,255.
The recognised values:

| result | stored bytes |
|---|---|
| white | 2, 3, 4, 8, 9, 10, 12, 14, 15, 137, 142, 255 |
| brown | 35, 36, 38, 40, 244, 246, 248 |
| yellow | 16, 24, 26, 30, 46, 230, 237, 238, 252 |
| orange | 50, 51, 52, 53, 54, 56, 58, 59, 60, 62, 66 |
| red | 67, 68, 69, 70, 71 |
| purple | 0, 77, 81, 84, 85, 86, 87, 97, 101, 102, 103, 105, 224 |
| light red | 89 |
| blue | 1, 7, 116, 135, 148, 151 |
| light blue | 110, 157 |
| grey | 161 |
| green | 17, 20, 179, 182, 183, 194, 195, 196, 197, 198, 199, 200, 212, 214, 215, 216 |
| light green | 201 |

**Any other value is unrecognised and an implementer must surface it rather than
invent a colour.**

**Locating the data.** On the Commodore 64 the pictures are named files inside
the disk image: a file is a picture if its name is at least four characters, its
first character is `R`, `B` or `S`, and its second through fourth are digits;
the picture index is the three-digit field at positions 3-5. On the Atari there
is no filesystem walk — each title has a hard-coded list of (usage, index, byte
offset) triples into the second disk of the pair, roughly 30 to 70 entries per
title. A picture's record starts **two bytes before** the listed offset, and its
length is the little-endian word at the listed offset plus two. The volume-table
splice of §7.3 applies to any record spanning 0xB390-0xB40F.

### 8.4 Family D — Apple II

Decoding is two stages: bytes are placed into a simulated 8192-byte
high-resolution page, and the page is then resolved to colour by an artifact
model. Nominal picture size 280 x 160.

**Byte placement, plain sub-variant.** The address for pixel row *y* and byte
column *x*, relative to the page base, is

> 1024 x (*y* mod 8) + 128 x ((*y* ÷ 8) mod 8) + 40 x (*y* ÷ 64) + *x*

— the standard three-way interleave, 40 bytes per row, 192 rows. Any address of
0x2000 or more ends the picture. Bytes are consumed in **pairs**, the first to
row *y* and the second to row *y* + 1 in the same column; *y* then advances by
two, and when (*y* − 2) reaches the stored height the column advances by one and
*y* resets to the vertical offset. Decoding also stops if the column exceeds the
stored width. The header is four bytes: horizontal byte-column offset, vertical
row offset, width in byte columns, height in rows. The Count's release omits the
two offsets, taking both as zero, and the first non-zero pair read supplies
width and height; in every case, if width and height both read as zero, keep
consuming two bytes at a time until a non-zero pair appears.

**Byte placement, scrambled sub-variant.** The four header bytes are horizontal
offset, vertical offset, width and height, but here **width and height are
absolute limits**, compared directly against the running column and row. The row
address is not computed but read from a 0x182-byte table taken off the game
disk: the address for row *y* is (table byte at *y*) + 256 x (table byte at
0xC0 + *y*) − 0x2000. A row index of 0xC0 or above, or an address above 0x1FFF,
ends the picture. The sub-variant is selected by the presence of a file named
`M2` containing the string named in §7.4 at file offset 0x172C.

**Pixel meaning within a byte.** Each byte carries **seven** pixels in its low
seven bits, **least significant bit leftmost**; bit 7 is not a pixel but a
per-byte palette-select flag. So a byte column is seven pixels wide and a
40-byte row is 280 pixels.

**Colour.** Six colours: black 0,0,0; purple 0xD53EF9; green 0x64D440; blue
0x458FF7; orange 0xD7762C; white 0xFFFFFF. Two ordered sets are used — with the
byte's high bit **clear** the set is (black, purple, green, white), and with it
**set** the set is (black, blue, orange, white).

Build the artifact lookup for each three-bit neighbourhood value *i* from 0 to 7
and each parity *j* of 0 or 1. If **bit 1 of *i* is set** (the centre pixel is
on): if bit 0 or bit 2 is also set the selector is *white*, otherwise it is
*green* when *j* is 1 and *purple* when *j* is 0. If **bit 1 is clear**: if bits
0 and 2 are **both** set the selector is *purple* when *j* is 1 and *green* when
*j* is 0, otherwise it is *black*. The selector names a position in the
high-bit-clear set, and the same position in the high-bit-set set is used when
the byte's high bit is set. The two "centre pixel off but both neighbours on"
cases produce a coloured pixel where the bitmap says nothing is lit, which is
the artifact this model exists to reproduce.

To resolve a row: gather its forty bytes with a notional zero byte before the
first and after the last; for each byte column form a 21-bit window from the low
seven bits of the previous, current and next bytes, in that order from least
significant; select the colour set from the **current** byte's high bit; and for
each of the seven pixels *b* take the three-bit field of the window starting at
bit (*b* + 6) and combine it with a parity bit of ((*b* XOR the column index)
AND 1). The pixel's position is 7 x column + *b*.

**Compression.** The plain sub-variant uses family C's scheme exactly, with The
Count again using the no-literal variant. The **scrambled sub-variant** uses a
different one: read a byte; **if it is zero** it is an escape, the next byte is
the repeat count and the one after that is the first data byte; if it is
non-zero it is itself the first data byte with a repeat count of one. Either
way, one more byte is then read as the second data byte, and a repeat count that
decodes to zero is treated as one. The pair is written the stated number of
times.

**Locating the data.** A hard-coded per-title list of (usage, index, offset,
length), the record read being the listed length **plus four**; six such lists
exist. Two extra images come from the boot disk: the inventory picture from
`PAK.INVEN` or `PAC.INVEN` with its four-byte prologue discarded, and for one
Claymorgue release a fixed 0x4F8-byte block at file offset 0x7E84.

### 8.5 Family E — MS-DOS

Four colours, two bits per pixel, most significant pair leftmost, as family C;
nominal canvas 280 x 158. **Storage is row-major with the CGA two-bank
interleave**: bytes are emitted left to right along a row, and when the
horizontal position reaches (width + horizontal offset) the vertical position
advances by **two** and the horizontal resets. When the row counter passes the
height, the vertical position resets to (vertical offset + 1) and a pass counter
increments — the picture is stored as all its even rows followed by all its odd
rows — and decoding ends when the second pass completes.

Each picture is one file named for its role: `R01nn` for a room picture,
`B01nnR` for a room-object picture, `B01nnI` for an inventory-object picture,
with a two-digit index at name positions 3-4 for a room name and 4-5 for an
object name. **Header fields sit at fixed file positions**: 0x05-0x06 the
little-endian size of the graphics chunk, which bounds decoding; **0x0D**, where
a value of 0xFF means the picture is "unlined" and any other value means
"lined"; 0x0F-0x10 a little-endian raw start offset; 0x11-0x12 a raw end offset;
0x13 the width in 4-pixel units; and the first compressed byte at 0x17.
Derivations: the horizontal pixel offset is (raw start mod 80) x 4 − 24; the
vertical pixel offset is (raw start ÷ 40) **rounded down to an even number**;
the height is (raw end − raw start) ÷ 80; the width in pixels is the byte at
0x13 times 4. The lined flag selects the pixel aspect: unlined means each stored
pixel is two device pixels wide and the horizontal position advances by two,
lined means one and one.

**Palette**, fixed and not stored: value 0 black, value 1 cyan 0,255,255, value
2 magenta 255,0,255, value 3 white — CGA palette 1 at high intensity, with no
intensity or background selection.

**Compression** is single-byte, not pairs: read a control byte; **bit 7 set**
means the count is the low seven bits **plus one** and one data byte follows, to
be emitted that many times; **bit 7 clear** means the count is the byte **plus
one** and that many data bytes follow, each emitted once.

### 8.6 Picture-to-room association

**Family A.** Room-to-picture is a **table**, not an identity: a per-release
address points at (room count + 1) bytes, one per room including room 0, giving
that room's picture number, with **255 meaning no picture** and only the low
seven bits used as the number (bit 7 is reserved and must be masked). A release
with no such table defaults a room's picture to (room number − 1).

**Item pictures overlay the room picture.** Two more parallel arrays of (item
count + 1) bytes are addressed by their own per-release constants: an **item
picture array** giving each item's picture number, where **0 means none**; and
an **item flag array** whose low seven bits give the room in which that item's
picture is drawn. The complete rule: after the room picture, walk every item and
draw its picture if it has a non-zero picture number, **and** it is in the
player's room, **and** the low seven bits of its flag byte equal the player's
room. Note the asymmetry of sentinels — 255 for rooms, 0 for items. The Hulk
releases invert it, treating 255 as the item sentinel, and ignore the flag test
entirely.

**The overlay's position is not in these tables.** It comes from the item
picture's own header offset bytes, so an item picture is a sub-image that
already knows where on the 32 x 12 grid it belongs — which is exactly why
version 0, having no header offsets, needs its separate coordinate table.

**Family B.** Pure identity: room *n* shows vector image *n* − 1. Rooms whose
image data was truncated have no picture. There are no item overlays.

**Families C, D and E.** Each stored picture carries a **usage** and an
**index**: a *room* picture is shown when the player is in the room with that
index; a *room object* picture overlays it when the item with that index is in
the player's room; an *inventory object* picture is drawn on the inventory
screen when the item with that index is carried. For the Commodore 64 and DOS
the usage comes from the filename — leading `R` for room, leading `B` for object
with a trailing `R` or `I` distinguishing the two; a leading `S` carries no
usage and defaults to a room picture, and no known release uses it. For Atari
and Apple II the usage is a field of the hard-coded list. Three indices are
reserved: **0** is the "too dark to see" picture drawn instead of the room when
the room is dark, **98** is the inventory backdrop, and **99** is the title
picture. Overlays here have no position field at all; each object picture
carries its own absolute placement. Overlays are drawn in load order — filename
order for Commodore 64 and DOS, list order for Atari and Apple II — and the
order is observable, because later pictures overwrite earlier ones.

**Title and intro images.** Families A and B have no title picture format at
all: their opening screens are text. In the US families the title picture is an
**ordinary room picture in every respect**, distinguished only by its reserved
index 99; nothing special is needed to decode it, and a picture set lacking
index 99 simply has no title screen.

---

## 9. Runtime behaviour that varies by dialect

Everything here is stated as behaviour observable from outside the program, with
a test that would catch getting it wrong.

### 9.1 The TI-99/4A dialect

**Automatic actions are independent.** Each has its own probability, several may
fire on one turn, none can suppress another, and none can continue into the
next. *Test:* three automatic records at 100%, each printing a distinct message,
the second failing a condition after printing. All three messages appear on the
first turn, in order; under the reference format's continuation semantics an
equivalent construction would not produce the third.

**Command dispatch reports three outcomes.** When no record in a verb's chain
matches the noun, or the verb has no chain, the interpreter reports that it does
not understand. When at least one matched but every match failed, it reports
"I can't do that yet." A success produces no acknowledgement of its own. This
holds for every verb except the three the interpreter handles itself (below),
where the built-in handling always produces an answer of its own instead.
*Test:* define a verb whose chain holds only a record keyed to noun 5 with a
condition that can never hold; typing it with noun 5 must give the "can't do
that yet" wording and with noun 6 the "don't understand" wording.

**Condition evaluation is interleaved, and side effects persist.** *Test:* a
record that sets bit flag 4, then fails a condition, then ends. The action
reports failure **and** flag 4 is set, observable through a second verb guarded
on it.

**Failure handlers are an if/else.** *Test:* a record of handler marker
targeting a "print message B" opcode, a failing condition, "print message A",
end, "print message B", end. Message B is printed and the record succeeds; flip
the condition to one that holds and message A is printed instead.

**Exactly three verbs are built in, and they are named by index, not by word.**
Verb 1 is go, verb 10 is take, verb 18 is drop — fixed numbers, the same in
every game of this dialect, and consistent with the dictionary order §3.6
tabulates. **No other verb has any interpreter handling whatever.** In
particular inventory, quit, stop, score and save are *not* built in: each is an
ordinary chain that the database spells out in opcodes, in several games as a
single link-0 record, which is why §3.7's link byte has to be read as a link
(worked through in §3.8). An interpreter that supplies its own handling for
those verbs will either double the output or override what the game says.

**Go is handled before the chain; take and drop after it.** Verb 1 with a noun
of 1 through 6 — the six directions, which are always nouns 1 through 6 — never
reaches verb 1's chain at all. The interpreter warns first if it is dark, then
consults the current room's exit for that direction: if there is one it moves
the player and describes the new room; if there is none it prints "I can't go in
that direction. " in the light, and in the dark instead clears the darkness,
moves the player to the highest-numbered room and prints the broken-neck
message. An exit that exists is taken even in the dark, after the warning. Verb
1 with any other noun goes to the chain as usual, and verb 1 with no noun asks
for a direction. So a database cannot override compass movement by writing
records for it.

Take and drop work the other way round: the verb's chain is walked first and the
built-in handling runs only if no record succeeded — **including when a record
matched the noun and failed**, which is where this dialect parts company with
the reference format, in which a line matching the noun exactly suppresses the
built-in. The consequence is observable: a failing take or drop chain never
produces "I can't do that yet."; the built-in answers instead, with the carry
limit, the "I already have it. " / "I don't see it here. " / "I'm not carrying
it. " / "It is beyond my power to do that. " distinctions, and the noun-to-item
link table (§3.4) as the *only* naming authority — an item with no link byte
cannot be taken or dropped by name however its description reads.

*Test:* with no records for the take verb at all, taking a linked noun still
moves the item to the inventory and acknowledges it with `OK. `; and with a take
record keyed to that same noun whose condition can never hold, the answer is
still the built-in's, never "I can't do that yet."

**Death is a side effect, not a return.** The kill opcode prints the death
message, restores light, moves the player to the highest-numbered room and runs
the end-of-game sequence, **and then the rest of the record continues to
execute**; only the end-the-game opcode stops a record immediately. *Test:* a
record of kill, then "print message C", then end. Message C is printed after
the death text.

**Automatic inventory is on by default**, displayed after every room
description, toggled by two opcodes, and its current state must survive save and
restore. *Test:* confirm the carried-items line appears unbidden; run a record
containing the "off" opcode and confirm it disappears on the following turn;
save, restore, confirm it is still absent.

**Movement is acknowledged.** A successful compass move prints "OK. " before
the new room is described. *Test:* move between two connected rooms.

**The message set is this dialect's own**, distinct from both the reference and
the ZX Spectrum sets. Notable strings: `I am in a `, a newline then
`Visible items are : `, `Obvious exits : `, `I am carrying : `, `Nothing. `,
`What shall I do? `, `I don't understand the command. `, `I can't do that yet. `,
`It is beyond my power to do that. `, a newline then
`I fell down and broke my neck.`, `I'm dead... `, `I am carrying too much.`,
`Light went out! `, `Light is growing dim `, `This adventure is over. Play
again?`, `Resume a saved game? `. The item delimiter is `", "`, the message
delimiter a single space, the exits delimiter `", "`. **Taking, dropping and
generic acknowledgement all use `OK. `** — this dialect does not distinguish
"Taken." from "Dropped.". The room description is terminated with a period when
any items are visible, and the inventory listing appends a period (unless the
last item's text already ends in `.` or `!`) followed by a space. *Test:*
compare a full turn's transcript; any interpreter emitting "Taken." or
"Exits: " is not running this set.

**Start-up.** Before the first turn the interpreter explains that any word may
be abbreviated to its first *word-length* letters and directions to one letter,
using the header's actual word length, then asks whether to restore a saved
game. *Test:* with a word length of 4, the number 4 appears in the opening text
and `NORT` is accepted for north.

**Output substitutions** (§3.5) apply to every string printed, not only the
title screen. *Test:* a message containing byte 0x40 renders as a copyright sign
followed by a space.

**No graphics.** Every room's picture index is the "no picture" value.

### 9.2 Light and lamp handling, all dialects

The lamp is always item 9. Two options govern its behaviour, and **every
Mysterious Adventures release and every TI-99/4A release forces both on**.

**The countdown.** While the lamp is not in room 0 and the header's duration is
not −1, the duration decrements once per turn.

**Warning style.** With the "authentic light messages" option **off** — the
Adventure International default — the interpreter prints a "light is growing
dim" message once every fifth turn once fewer than 25 turns remain, and only
while the lamp is carried or in the current room. With it **on**, it instead
prints, **every turn** below 25, a three-part line: a "light runs out in"
string, the exact remaining count as a decimal, and a "turns" string.

**Exhaustion.** At zero the interpreter always sets bit flag 16 and, if the lamp
is carried or present, prints a "light has run out" message. With the
"prehistoric lamp" option **on**, the lamp item is additionally **moved to room
0 and thereby removed from play**. Databases written for the later interpreter
instead leave it in place and rely on their own action lines testing bit flag
16; the Mysterious databases do not test that flag, so unless the lamp is
destroyed the player can go on using it.

A duration of **−1** means "never runs out" and suppresses the whole mechanism.

*Test.* Load the release, take the lamp, and step the counter down. At 24 turns
remaining the output must be the countdown form naming 24, repeating with a
decreasing number every subsequent turn, never the "growing dim" form. On the
turn the counter reaches zero the lamp must both announce that it has run out
**and cease to be in the inventory** — an inventory command immediately
afterwards must not list it. An interpreter that leaves the lamp in the
inventory has the prehistoric option off and is wrong for this series.

### 9.3 First person versus second person

§6.4 tabulates the two message sets. An interpreter should expose the choice as
an option and force it on for a recognised Mysterious Adventures **ZX Spectrum**
release. **Do not force it on the series as a whole**: the Commodore 64
releases of the same eleven titles carry a first-person block in the file, and
§6.4 gives it in full. The wording follows the platform, not the series.

*Test, ZX.* Enter a room in a Mysterious ZX release: the preamble must read
`You are in a `, and the inventory heading `You are carrying:`. Any transcript
containing `I'm in a ` is running the wrong set.

*Test, Commodore 64.* Enter a room in a Mysterious Commodore 64 release: the
preamble must read `I'm in a `, the inventory heading `I am carrying:`, and the
visible-objects heading `Things I can see:`. A transcript containing
`You are in a ` is running the wrong set — and since the eleven files contain no
second-person string at all, that wording can only have come from the
interpreter overriding what the file says.

### 9.4 Command execution order

The reference format's rule, which the memory-image dialects share and the
TI-99/4A dialect does not: all five conditions of a line are evaluated before
any of its four commands runs, so a line that fails changes nothing. The
TI-99/4A dialect interleaves (§9.1). *Test:* the flag-4 test above, run against
both a reference-format and a tokenised encoding of the same logic; they must
differ.

---

## 10. Specimens

Every specimen below is publicly fetchable. Sizes and SHA-256 digests are those
observed when this document was written; a mirror may differ, and the digest is
the check.

**In this repository they are already on disk — look there before fetching
anything.** They live under the gitignored `stories/scott-dialects/`, one
directory per platform, each with a `README.txt` naming its source and the
sha256 of every file in it. The zips below are not retained; each has been
unpacked and its contents stored in a flat directory structure under the
platform directory:

| directory | holds | section |
|---|---|---|
| `ti99/` | the twelve tokenised TI-99/4A releases | §10.2 |
| `spectrum/` | the twenty ZX snapshots, four of them negative controls | §10.3 |
| `mysterious-dat/` | the eleven Mysterious reference-format conversions | §10.3 |
| `c64/` | the two Mysterious compilation D64s, the thirteen program files extracted from them, and the two Questprobe D64s | §10.4, §10.7 |
| `atari/` | the seven US S.A.G.A. Atari 8-bit two-sided disk sets, plus one Atari executable | §10.5 |
| `apple/` | the seven US S.A.G.A. Apple II two-sided disk sets | §10.6 |
| `msdos/` | the two MS-DOS Questprobe releases | §10.7 |

Nothing there is committed and nothing there may be staged.

### 10.1 The reference-format oracle

| file | size | sha256 |
|---|---|---|
| `/if-archive/scott-adams/games/scottfree/AdamsGames.zip` | 107,198 | — |

Contains `adv01.dat` through `adv14b.dat`, `quest1.dat`, `quest2.dat` and
`sampler1.dat`. The twelve numbered games, with their digests:

| file | size | sha256 | game |
|---|---|---|---|
| adv01.dat | 15,896 | `b6f9293bd3cf2759d2240d2ac9a2b55877d71d0d242116f816fe29b3c619dedf` | Adventureland |
| adv02.dat | 16,325 | `e8f5099105147503be7d2dd663f3782573957eb5c8fee390722e17ca634728ce` | Pirate Adventure |
| adv03.dat | 15,275 | `78cb69e0db2f74b6c2eeeba781d42e0093bba6264c85b22ec1ca3952bbe3fd36` | Secret Mission |
| adv04.dat | 15,852 | `ded986066675bfcc04dded6f6ffbd8e21f939b71fdd91f583e7e30a6d94fe3db` | Voodoo Castle |
| adv05.dat | 17,476 | `c7488eee39e1267ba75fe767f0dc91703ec87fd1415f7abd231601e4962fb20d` | The Count |
| adv06.dat | 17,489 | `27c43b6701b2d5d5963156f496dd2ecd27c9f9856a4dbf56e88e1ba8a1af26ee` | Strange Odyssey |
| adv07.dat | 17,165 | `0bfecbf88c6b39fdb787eadb710188d0f2d10eaf3c0ab456937a04bb16c98429` | Mystery Fun House |
| adv08.dat | 17,673 | `1874eb2e68df930044dc4c0c45719083494e84fe1d4ff71d066b454271ac9dfa` | Pyramid of Doom |
| adv09.dat | 17,831 | `37e1a5cbadcd5e855918eafd31a662404216ca97b35d82538d7a79d12bb6b3cf` | Ghost Town |
| adv10.dat | 19,533 | `a26127a51e0815e35e392a7ffd9b3e5c6c49c430a19fcfd9f6ef6a6e366e9027` | Savage Island part I |
| adv11.dat | 18,367 | `fd1718e6e3c0bda8d6a3875407cbef119299d69dd209ae647e8f665e9387058c` | Savage Island part II |
| adv12.dat | 18,513 | `4e7bd5c09adb57e392e7d8a9dcb0b308beab0bcd63d3e607fa19191824606073` | The Golden Voyage |

### 10.2 TI-99/4A

`/if-archive/scott-adams/games/ti99/scott_adams_ti99_games.zip`, 82,552 bytes,
sha256 `21c1b58f2ece9b5975e773e3741e1a505104e5017fb946ff52467f38e7ba6fe8`.
Contains `adv01.fiad` through `adv12.fiad`, the same twelve titles in the same
order, each with a 128-byte file-descriptor header and the detection signature
at file offset 0x589.

| file | size | sha256 | matching database |
|---|---|---|---|
| adv01.fiad | 10,774 | `87ce6c457eb454f6230aa3a6abf4987e362fdf3c9d04882eea80ecd138e063b3` | adv01.dat |
| adv02.fiad | 10,488 | `51ad7b942c81da2c95de7e771b450c26152bcecf3f88342e1109eca4e317dd9a` | adv02.dat |
| adv03.fiad | 10,562 | `5a78bdd2dcc78fd46b931f67bda16c66cdfdc095a97787d7b5cdfc52d63d6935` | adv03.dat |
| adv04.fiad | 10,424 | `fdfcccefefdf05e734188614e2a5cc1b1df7540d2f76fb98435de01a95acbb99` | adv04.dat |
| adv05.fiad | 10,326 | `0408c83dfc2c554451c95996b4b486fc3158a9ba9db32cf0c6a225f163084e13` | adv05.dat |
| adv06.fiad | 10,170 | `61c9dfd4e0909313f444654b8c3155754e9e7e5fcfc1277eb51c062528e50c29` | adv06.dat |
| adv07.fiad | 10,594 | `68f764d48b941240a1a722c5bb79c80ec62b2f416d0d8d6349f425b10cd1d456` | adv07.dat |
| adv08.fiad | 10,242 | `640dfe16017c6e48e679f86d9e7e5623215bd01a4145ca186d8c2323e3bcd98d` | adv08.dat |
| adv09.fiad | 10,170 | `4ed9f27e002947e697640d3c80afcd96c61769103ef93afc236afe1c16740bb2` | adv09.dat |
| adv10.fiad | 10,170 | `d04de725f564b80e862e38f6c35226e87dc55f2f2717f5340099e34fff69c9f7` | adv10.dat |
| adv11.fiad | 12,616 | `2e409f6d8511923de88e8e25e13552e182354899f464a882938a5ca6d5325707` | adv11.dat |
| adv12.fiad | 10,346 | `37b33eb2d703d5684c710f6450a47a3d5b9ce6a5b3ec1e9e76dfb30b86c0526d` | adv12.dat |

**How strong an oracle this is, honestly.** The item counts agree exactly for
Adventureland, Voodoo Castle, Mystery Fun House, Ghost Town, Savage Island part
II and The Golden Voyage, and differ for the rest — Pirate Adventure has 64
items in the tokenised release against 66 in the conversion, Pyramid of Doom 92
against 100. **These are different releases of the same games**, so exact table
equality is the right check only for the six titles where the counts already
agree; for the others the useful checks are structural — that the vocabulary
overlaps heavily, that room descriptions read as the same rooms, and that the
decoded action semantics match the published conversion where the two releases
share a puzzle.

A further homebrew set, `/if-archive/scott-adams/games/ti99/home_brew_ti99_
scottadams_games.zip`, 94,041 bytes, sha256
`d4eb4d63c16de7a100a09b83c992760f7d59b6fa67bd45e1de0bf685377eaeff`, holds
sixteen more files in the same format with no reference-format counterpart —
useful for exercising the loader, useless as an oracle.

### 10.3 ZX Spectrum snapshots

`/if-archive/games/spectrum/mystsoft.zip`, 546,758 bytes. Contains twenty
snapshots, of which sixteen are in scope. All are 48K; the version and the
address at which the dictionary signature appears **after decompression** were
measured and are given below.

| file | size | sha256 | container | signature at | dialect |
|---|---|---|---|---|---|
| m1goldba.z80 | 32,060 | `b115bc6645b672d74d183058e0779f430d8c30dc9e74093282876bbc9a3cc567` | version 2 | `AUTO\0GO\0` at 0x873A | Mysterious, early |
| m2tmachi.z80 | 30,928 | `938156ac680129b08b4f01d3c9d2cd5fd52bcbe759d8e4268dfd77e23b6da101` | version 1, compressed | — | Mysterious, early |
| m3arrow1.z80 | 34,105 | `0f1562ba1f87c44a63028e91688f55f7e453ba129d1c7e051913a74d18249bbf` | version 1, compressed | — | Mysterious, early |
| m4arrow2.z80 | 38,043 | `48bcbae098b9f09732abee0d1282642c87825742a9854b4141d5088a2362a1ec` | version 2 | — | Mysterious, early |
| m5pulsar.z80 | 29,961 | `9e6c60da54ab7ac0165b71f7f1bcf3db88b0e41a3b48b3a086eec3adb14c397b` | version 1, compressed | `AUTO\0GO\0` at 0x8B1D | Mysterious, early |
| m6circus.z80 | 27,746 | `d549242483fc49023f2e0a98ef7af42618923767bf4dd83522990ba9ded01c5d` | version 1, compressed | — | Mysterious, early |
| m7feasib.z80 | 37,395 | `d4b82b9d6dbaea17d2ff0a94a3a8637fad2db008ea18fb4633b8e54f0098c44c` | version 2 | — | Mysterious, early |
| m8akyrtz.z80 | 33,753 | `669ace0cc60a90869b75a4bec5f40265dee200bcd1d55d6c663c987e2149f6cc` | version 2 | — | Mysterious, early |
| m9perseu.z80 | 31,504 | `9abe65d7654ad574be9b8affcdd98aa62a9c568573cfa9416d881dc40a342427` | version 2 | — | Mysterious, early |
| m10india.z80 | 31,664 | `d15f5b872d66aaa0d49572f240bb9115379859b654a38be6a91ac00a196863ed` | version 2 | — | Mysterious, early |
| m11waxwo.z80 | 32,662 | `8e989f1ac73a3e45a173ed0bbca7613fae5c81329c58bf24d6ddf82f27052271` | version 2 | — | Mysterious, early |
| gremlins.z80 | 45,332 | `04909ab3fd4f0770037e6032ca0cc7e30f1b270b73b73c3c455da02d6b0659d9` | version 1, compressed | `AUTO\0GO\0` at 0x85CA | compressed actions, plain text |
| supergra.z80 | 40,432 | `dabc81fdb3d85cdbf0ea68c50c74b328bf7271eb5ea1d5da5a1e73ef59191536` | version 2 | `AUTO\0GO\0` at 0x82E2 | compressed actions, plain text |
| sherwood.z80 | 44,264 | `0939f2ebe4a4ac52ed302c81259eeaea29f2d12e88d25920aba925ab70f5e249` | version 1, compressed | `aUTOgO\0` at 0x8DA8 | compressed actions **and** text |
| seablood.z80 | 46,257 | `9d97e3bc2dfcce6e8113c5e5317cf37c5ea4e002cddb147e0b00bf5b904e7a63` | version 2 | `aUTOgO\0` at 0x9900 | compressed actions **and** text |
| rbplanet.z80 | 46,442 | `f03d35d2613ae7754c2f993bc508e275a146c462fa9de8e2ba9bd29efa04cb5e` | version 1, compressed | — | — |

The remaining four snapshots in the archive — `blizzard.z80`, `heman.z80`,
`kayleth.z80`, `temple.z80` — carry **no** Scott Adams dictionary signature and
are useful **negative controls**: a detector that claims them is wrong.

The matching reference-format databases for the eleven Mysterious titles are in
`/if-archive/scott-adams/games/scottfree/mysterious.tar.gz`, 49,153 bytes,
sha256 `f9e63477709ba85ab438dafa3e6db03413380647da91076e6b9dc2f415b7f33d`:

| file | size | sha256 |
|---|---|---|
| 1_baton.dat | 13,373 | `747c32f3ea33f650afdf449ddb6779174e9c97d1a062e56592d419d196978743` |
| 2_timemachine.dat | 13,742 | `f672593358c609a02fee7244fe6d2ec303ce24364b0ed510bcb15cda2eca9b97` |
| 3_arrow1.dat | 13,412 | `b5f98a0da6f4f4366f785d82e1238e9a50a6380ea76999f0c719084a3ba0d972` |
| 4_arrow2.dat | 15,554 | `eaad6bc4f94c5bdf0ad38cb692c22db61139823d6f2fa058ce722a5ed52fa414` |
| 5_pulsar7.dat | 17,777 | `93721a0213ec25e0fe62f8ca492cc9fa8190443147208b0e10affec6cb610852` |
| 6_circus.dat | 13,621 | `3ee953a6b69956464b98f23768a0fe5b15394232e6915c8803d58c1811eb57eb` |
| 7_feasibility.dat | 13,441 | `4992e65695bd1982182eeea4e9448a73110b79042ac89d5851eabd7d68e9a6c4` |
| 8_akyrz.dat | 16,803 | `46fa29f11be3141a9b4ebb5714e099bf6c0ba975d1154626b23d417072b79964` |
| 9_perseus.dat | 15,080 | `050111b15a6f8abe3df9c996bb63ef07644e94ef383fec6c986ebbadf19c2cdf` |
| A_tenlittleindians.dat | 14,215 | `64ccc01fb56b552dc91a7c0153d443b17d3e21bed94791ed287d7fe971175b6c` |
| B_waxworks.dat | 16,068 | `da32b53fecc0d6bcaf3ab77ec4a72e81876a6b06b603d9b742b112ff7fb08bb5` |

These conversions were made from the ZX Spectrum and Commodore 64 releases, so
they are the right oracle — with the same caveat as §10.2. Escape from Pulsar 7
matches exactly: the eleven header numbers recovered from the snapshot are
identical to the eleven in the conversion. The Golden Baton does not: the
snapshot reports 171 actions, 76 words, 5 carried and 99 messages against the
conversion's 166, 78, 6 and 99. **Check the header numbers before assuming
table equality is the right test.**

### 10.4 Commodore 64: the Mysterious Adventures compilation disks

`/if-archive/scott-adams/games/c64/mystadv.zip`, 187,015 bytes, sha256
`08907754669907ab651bb5f0caf0e974d3d938563385e582297065f7cb943bcd`, contains
the two Mysterious Adventures compilation disk images:

| file | size | sha256 |
|---|---|---|
| MYSTADV1.D64 | 174,848 | `acfcedccc858e6ffb7f5833e4bce80e989c5bd242876a72c66a2aee1870d3798` |
| MYSTADV2.D64 | 174,848 | `17eb01436ab33eb715bdfc10a46d0a21b259ca35a15d841d4ed15ab76a1e6ca2` |

Both are 35-track images with no error map, both carry an ordinary CBM DOS
directory at track 18 sector 1, and neither uses any copy protection,
non-standard interleave or bad-sector trick. Their file names are those listed
in §6.5, plus a `BOOT` program on each which is a tokenised BASIC menu carrying
no game data, and **a reader must be told which game is wanted** — nothing in
the container distinguishes one from the others except its name.

**Extract the program file before quoting an offset.** Earlier drafts of this
section gave the file offsets `0x4739` and `0x4B29` at which the plain
`AUTO\0GO\0` signature falls in the raw D64 bytes. Those are artefacts of where
the sectors happen to lie and are of no use to a loader; the figures worth
having are the per-title addresses below, which are addresses in the extracted
memory image.

Each extracted program file keeps its two-byte load address, which is `$4000` in
all eleven, so array byte 2 corresponds to `$4000`. None is crunched (§7.2).

| game file | bytes | sha256 (first 12) | dictionary | header | pictures |
|---|---|---|---|---|---|
| `BATON` | 27,555 | `7dd995cff54e` | `$685F` | `$5DD7` | `$78EF` |
| `TIME MACHINE` | 27,622 | `4297dd1e11d3` | `$680F` | `$5DD7` | `$7870` |
| `ARROW I` | 30,534 | `b447f1205d2e` | `$675F` | `$5DD7` | `$78E0` |
| `ARROW II` | 34,054 | `76c14159832e` | `$68FF` | `$5DD7` | `$7CAA` |
| `PULSAR 7` | 26,162 | `b8c93d0e45db` | `$69E1` | `$5DD7` | `$7BF2` |
| `CIRCUS` | 24,400 | `e57513c61102` | `$684F` | `$5DD7` | `$7912` |
| `EXPERIMENT` | 33,742 | `c13a8cee97de` | `$67C1` | `$5DD7` | `$7874` |
| `WIZARD OF AKYRZ` | 29,071 | `64bdf4ba53e0` | `$6A6F` | `$5DD7` | `$7BCC` |
| `PERSEUS` | 27,785 | `f72fa51dc1f9` | `$6851` | `$5DD7` | `$7D8F` |
| `INDIANS` | 28,174 | `faf18b1edee8` | `$680E` | `$5DD7` | `$7A44` |
| `WAXWORKS` | 28,644 | `adb73a99dc4b` | `$69D1` | `$5DD7` | `$7F2F` |

Every dictionary address here is **also** readable from the file itself, at
§6.2's pointer block, and the picture address is derivable as the first `$FF` at
or after the pointer block's sixth address; both columns are given so a reader
can check its arithmetic rather than because either must be tabulated.

The headers those addresses decode to, with the §4.5 shape each uses:

| game file | shape | items | acts | words | rooms | carry | start | treas | wlen | lamp | msgs |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `BATON` | Mysterious C64 | 48 | 166 | 78 | 31 | 6 | 1 | 0 | 4 | 200 | 99 |
| `TIME MACHINE` | Mysterious C64 | 62 | 161 | 85 | 44 | 6 | 43 | 0 | 4 | 200 | 73 |
| `ARROW I` | Mysterious C64 | 64 | 150 | 90 | 52 | 5 | 1 | 0 | 4 | 32767 | 82 |
| `ARROW II` | Arrow of Death part 2 | 90 | 176 | 82 | 65 | 9 | 1 | 0 | 4 | 400 | 87 |
| `PULSAR 7` | early | 88 | **195**, see §5.3 | 145 | 45 | 6 | 1 | 0 | 4 | 200 | 74 |
| `CIRCUS` | Mysterious C64 | 65 | 165 | 97 | 36 | 6 | 1 | 0 | 4 | 150 | 72 |
| `EXPERIMENT` | early | 65 | 156 | 79 | 59 | 6 | 10 | 10 | 4 | 200 | 65 |
| `WIZARD OF AKYRZ` | Mysterious C64 | 49 | 199 | 85 | 40 | 6 | 1 | 4 | 4 | 65535 | 99 |
| `PERSEUS` | early | 59 | 165 | 130 | 40 | 6 | 1 | 0 | 4 | 200 | 96 |
| `INDIANS` | Ten Little Indians | 73 | 161 | 82 | 63 | 5 | 61 | 0 | 4 | **500** | 67 |
| `WAXWORKS` | early | 57 | 189 | 105 | 41 | 6 | 1 | 0 | 4 | 250 | 91 |

**How strong an oracle the §10.3 conversions are for these, honestly.** Decoding
each program file with §4.4's encodings and comparing every table against the
same title's conversion, field for field:

| game file | actions | room descs | connections | messages | item descs | item locations |
|---|---|---|---|---|---|---|
| `BATON` | equal | equal | equal | equal | equal | equal |
| `ARROW I` | equal | equal | equal | equal | equal | equal |
| `ARROW II` | equal | equal | equal | equal | equal | equal |
| `EXPERIMENT` | equal | equal | equal | equal | equal | equal |
| `PERSEUS` | equal | equal | equal | equal | equal | equal |
| `WAXWORKS` | equal | equal | equal | equal | equal | equal |
| `CIRCUS` | equal | 4 differ | equal | 25 differ | 8 differ | equal |
| `INDIANS` | 1 differs | 4 differ | equal | 10 differ | 2 differ | equal |
| `TIME MACHINE` | 162 v 165 | 18 differ | equal | 22 differ | 62 v 63 | equal |
| `WIZARD OF AKYRZ` | 200 v 202 | 36 differ | equal | 80 differ | 32 differ | 1 differs |
| `PULSAR 7` | 191 v 221 | 39 differ | equal | 75 v 76 | 89 v 91 | 89 v 91 |

So for **six of the eleven** the program file and the conversion hold the same
database exactly, and exact table equality is the right test. For the other five
it is the wrong test: they are **different releases of the same games**, the
same situation §6.1 records for the ZX *Golden Baton*, and their tables are
nonetheless raw and decode cleanly. Compare the header counts first, exactly as
§10.2 and §10.3 say. (Note that the room-connection table is identical in all
eleven, including the five divergent releases — a revision changed the prose and
the script, never the map.)

### 10.5 Atari 8-bit: the US S.A.G.A. disk releases

Not on the IF Archive. Seven two-sided releases were fetched from
<https://www.atarimania.com/machines/atari-400-800-xl-xe?q=saga> as one zip per
title. **All fourteen images are exactly 92,176 bytes and carry §7.3's
six-byte header `96 02 80 16 80 00`** — the single-density, 720-sector,
128-bytes-per-sector shape, and the only shape §7.3 accepts.

| title | side A | side A sha256 | side B | side B sha256 |
|---|---|---|---|---|
| #1 Adventureland | `SAGA #1 - Adventureland [side A].atr` | `7c48bc7779d1` | `SAGA #1 - Adventureland [side B].atr` | `a308387c9220` |
| #2 Pirate Adventure | `SAGA #2 - Pirate Adventure [side A].atr` | `547c0e902c73` | `SAGA #2 - Pirate Adventure [side B].atr` | `e2cfea52ea58` |
| #3 Mission Impossible | `SAGA #3 - Mission Impossible [side A].atr` | `7575154bd458` | `SAGA #3 - Mission Impossible [side B].atr` | `b28ab6872c41` |
| #4 Voodoo Castle | `SAGA #4 - Voodoo Castle [side A].atr` | `3f7db7281806` | `SAGA #4 - Voodoo Castle [side B].atr` | `2a417fb62f14` |
| #5 The Count | `SAGA #5 - The Count [side A].atr` | `5dc27fa23830` | `SAGA #5 - The Count [side B].atr` | `37fd7e4fd1cf` |
| #6 Strange Odyssey | `SAGA #6 - Strange Odyssey [side A].atr` | `83985e1ac74e` | `SAGA #6 - Strange Odyssey [side B].atr` | `9783df409f69` |
| #13 Claymorgue Castle | `SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr` | `d0dcba442113` | `SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr` | `9542de4bb9d8` |

Side A is the database side and side B the companion picture side, which is what
§7.3's companion-disk paragraph expects. Two cheap checks confirm it, and both
were run on the images rather than taken on trust. First, **each side A decodes a
coherent US-shape header (§4.5) at §7.3's mastering offset `0x04F9`**:

| title | side A sha256 (first 12) | wlen | words | acts | items | msgs | rooms | carry | start | treas | lamp | treasure room |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| #1 Adventureland | `7c48bc7779d1` | 3 | 69 | 169 | 65 | 75 | 33 | 6 | 11 | 13 | 125 | 3 |
| #2 Pirate Adventure | `547c0e902c73` | 3 | 79 | 177 | 66 | 88 | 26 | 6 | 1 | 2 | 150 | 1 |
| #3 Mission Impossible | `7575154bd458` | 3 | 64 | 161 | 53 | 81 | 23 | 7 | 2 | 0 | 10000 | 1 |
| #4 Voodoo Castle | `3f7db7281806` | 3 | 89 | 189 | 65 | 99 | 25 | 9 | 1 | 0 | 15000 | 0 |
| #5 The Count | `5dc27fa23830` | 3 | 79 | 219 | 72 | 88 | 22 | 7 | 1 | 0 | 175 | 0 |
| #6 Strange Odyssey | `83985e1ac74e` | 4 | 79 | 223 | 55 | 94 | 35 | 6 | 1 | 5 | 10000 | 22 |
| #13 Claymorgue Castle | `d0dcba442113` | 5 | 109 | 267 | 75 | 79 | 32 | 10 | 1 | 13 | 3000 | 19 |

Second, **the declared word length agrees with the §4.1 signature actually
present**: three-letter cells `AUT\0GO\0` in #1 to #5, four-letter cells
`AUTO\0GO\0` in #6. Claymorgue's word length is 5 and none of §4.1's plain
signatures occurs in it; its dictionary arrangement is unexamined. No side B
decodes a plausible header at `0x04F9`.

A family-C loader starts from §7.3 for the container and the mastering
constants, §4.5 for the US header field order, §4.4 for the table encodings, and
§8.3 with §8.6 for the pictures. It will still need the per-title (usage, index,
offset) picture lists §8.3 describes; nothing in these images supplies them.

**One Atari file here is not a disk image at all.** `The Hulk.xex` in the `atari/`
directory, 21,821 bytes, sha256 `c5c18baa36348e04a0a932bafad3f901f2777324a03019ceea5894b26bf075ab`,
from myabandonware.com, is an Atari binary-load executable: it begins `FF FF`,
the segmented-load signature, followed by the load address `$4000` and the end
address `$9530`. It carries the plain `AUTO\0GO\0` signature at file offset
`0x2CC4`, so it is a memory image of the family §4 describes arriving in a
container this document does not specify. §7.3's refusal rule — that header-less
images of the same content are refused by name rather than guessed at by size —
covers it correctly today.

### 10.6 Apple II: the US S.A.G.A. disk releases

Not on the IF Archive. Seven two-sided releases were fetched from the Asimov
mirror at <https://mirrors.apple2.org.za/ftp.apple.asimov.net/images/games/adventure/scott_adams/>.
Every `.dsk` is exactly **143,360 bytes** — 35 × 16 × 256, §7.4's flat sector image —
and none is a `.woz`, `.nib`, `.po` or `2IMG`.

| title | boot side | boot side sha256 (first 12) | non-boot side sha256 (first 12) |
|---|---|---|---|
| #1 Adventureland v2.1-416 | `Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side B - boot.dsk` | `53064cac8ddc` | `b2d542032ba7` |
| #2 Pirate Adventure v2.1-408 | `Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side B - boot.dsk` | `0a01e8bb97e3` | `8afd74db4799` |
| #3 Mission Impossible v2.1-306 | `Scott Adams Graphic Adventure 3 - Mission Impossible v2.1-306 (4am crack) side B (boot).dsk` | `773479102468` | `5395140182c3` |
| #4 Voodoo Castle v2.1-119 | `Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk` | `c34dba003a7f` | `0b0572df3f37` |
| #5 The Count v2.1-115 | `Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk` | `ba3d66fbcce6` | `1ad44a16aecc` |
| #6 Strange Odyssey v2.1-119 | `Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side B - boot.dsk` | `6039602205a6` | `623bbe2759bc` |
| #13 Claymorgue Castle v2.2-122 | `Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk` | `5ca192b45d2f` | `98a734f7ef97` |

Here the **boot side** is the database side. Walking §7.4's VTOC at track 17
sector 0 and the catalogue chain finds on each boot disk exactly one file whose
normalised name is one of §7.4's recognised database names — and note that
Claymorgue's is `DATABASE`, not the `SORCEROR OF CLAYMORGUE CASTLE` spelling
§7.4 also lists:

| title | boot side | boot side sha256 (first 12) | database file | family-D sub-variant |
|---|---|---|---|---|
| #1 Adventureland | side B | `53064cac8ddc` | `A1.DAT` | plain |
| #2 Pirate Adventure | side B | `0a01e8bb97e3` | `A2.DAT` | plain |
| #3 Mission Impossible | side B | `773479102468` | `A3.DAT` | plain |
| #4 Voodoo Castle | side B | `c34dba003a7f` | `A4.DAT` | **scrambled** |
| #5 The Count | side B | `ba3d66fbcce6` | `DATABASE` | **scrambled** |
| #6 Strange Odyssey | side B | `6039602205a6` | `A6.DAT` | plain |
| #13 Claymorgue Castle | side B | `5ca192b45d2f` | `DATABASE` | **scrambled** |

The sub-variant column was settled by §7.4's own string test, which is exactly
the cheap check it recommends: read the file named `M2` through its track/sector
list and take the 31 bytes at file offset `0x172C`. On #4, #5 and #13 they read
exactly `COPYRIGHT 1983 NORMAN L. SAILER` — and the crack logs in the same zips
credit "graphic routines by Norm Sailer" — which selects §8.4's **scrambled**
byte placement and its distinct compression scheme. On #1, #2, #3 and #6 the
`M2` file is 3,584 bytes rather than 6,656, so `0x172C` is past its data, the
test fails, and the release is plain.

The split shows a second time, independently: the three scrambled titles are
exactly the three whose picture files (`PAK.INVEN`, `PAK.LET0` … `PAK.LET11`)
sit on the boot disk and whose side A is not a readable DOS 3.3 disk at all,
while the four plain titles keep an ordinary DOS 3.3 side A holding `Rnnnn`- and
`Bnnnnn`-named picture files.

The non-boot side digests, in the same title order: `b2d542032ba7`,
`8afd74db4799`, `5395140182c3`, `0b0572df3f37`, `1ad44a16aecc`, `623bbe2759bc`,
`98a734f7ef97`.

A family-D loader starts from §7.4 for the container, the catalogue walk and the
`0x135`/`0x016D` database constants, §4.5 for the US header field order, §4.4
for the table encodings, and §8.4 with §8.6 for the pictures — including both of
§8.4's byte placements and both of its compression schemes, since these fourteen
images exercise all of them. The per-title (usage, index, offset, length) picture
lists §8.4 requires are not in these images.

### 10.7 MS-DOS and the Commodore 64 Questprobe releases

All four from <https://www.myabandonware.com/>.

**MS-DOS.** Two Questprobe releases, supplied as zips. §8.5 names only the MS-DOS *Hulk*; the
*Fantastic Four* release is new to this catalogue and is the **same** family-E
CGA picture format with a **different filename convention**.

| zip | size | sha256 |
|---|---|---|
| The-Hulk_DOS_EN.zip | 107,013 | `5869c4dc461a9f9a21b4285293ae1bfb25ed09c0bfb720614b1284942c7451fd` |
| Questprobe-Featuring-Human-Torch-and-the-Thing_DOS_EN.zip | 119,448 | `d4cdebcc7b8b385ccd5811415b089ac17c463c42b7e16d50f2ec62d9b233200a` |

*The Hulk* holds `START.EXE`, `HULK.BAT`, and `ADVENT.DAT` (18,897 bytes, sha256
`f07a57451e6d95c12a093310d6d0bf194f4161258265fc81090ff0b75ce9bbcc`). **The
database is not a memory image**: it is the plain reference text format of §2,
carriage-return terminated, and its §2.2 header reads (unused first integer
4730) items 54, actions 261, words 128, rooms 20, max carried 10, start room 1,
treasures 17, word length 4, lamp 150, messages 99, treasure room 16 — the
reference format's own field order. No §4.1 signature occurs in
it, which is as it should be — those are memory-image signatures. Its pictures
are the `.PAK` files, named exactly as §8.5 and §8.6 say: `R01nn`, `B01nnR`,
`B01nnI`. Sampling `R0100.PAK` against §8.5's fixed header positions gives
graphics chunk 1,490, `0x0D` = `0xFF` (unlined), raw start 6, raw end 6,316,
width byte 70 (280 pixels), height 78.

*Fantastic Four* holds `FANTFOUR.EXE`, `FANTFOUR.TXT` and `SPL53P.DAT` (41,472
bytes, sha256
`321787ad12ca0316f6f4a987c977b4891f27a779c5133e9f0f97308e6c0ed87c`). **Its
database is neither a memory image nor the reference text format**, and this
document does not describe it: it opens with a quoted title followed by a
comma-separated number list, `"FF #1 ",48,6987,96,53,34,12,6,178,1,172,14,6,523,
0,40,0,22,20`, and then one number per line. No §4.1 signature occurs in it. Its
**pictures are family E**, though: `R001.PAK` read at §8.5's fixed header
positions gives graphics chunk 2,315, `0x0D` = `0xFF`, raw start 6, raw end
6,316, width byte 70 (280 pixels), height 78 — the same shape and canvas as the
Hulk's. Two of §8.6's naming rules nevertheless fail on it, and an implementer
should treat them as rules about the *Hulk* release rather than about MS-DOS:

- the names are three-digit `Rnnn` / `Bnnn`, with no `01` prefix and no trailing
  `R` or `I`, so §8.6's "two-digit index at name positions 3-4 for a room and
  4-5 for an object" and its trailing-letter usage rule both mis-read them;
- the set uses `S`-prefixed names, `S000.PAK` through `S020.PAK`, where §8.6
  says a leading `S` "carries no usage and defaults to a room picture, and no
  known release uses it". **This release uses it, twenty-one times**, and
  `S000.PAK` reads `0x0D` = `0x00`, i.e. *lined*, where both sampled `R`
  pictures are unlined.

**Commodore 64.** Two Questprobe disk images, both 174,848-byte 35-track D64s
with an ordinary directory.

| image | image sha256 | source |
|---|---|---|
| `QUESTPR1.D64` | `5035c0ae93ebfd144e8c473c29ed24c30e07f9e5e5457730e573bbb28d95a4e4` | The Hulk |
| `QUESTPR3.D64` | `68c72e55ff77a4ef025c67a79f8007114aca6073dc614e7c0bc6f6fb55ac53a8` | Fantastic Four |

*The Hulk* on `QUESTPR1.D64` is the **US family-C release**, not the UK
family-A one, and the disk says so twice. Its database file is named `SHULK.DB`,
one of §7.2's fixed names; extracted through the block chain it is 9,858 bytes,
loads at `$8030`, is **not crunched**, and carries §4.1's plain `AUTO\0GO\0`
(four-letter cells, plain text) at `$8308` rather than the packed `aUTOgO` of the
compressed dialect. Its picture files are named `R01nnn`, `B01nnnR` and
`B01nnnI`, which is §8.3's Commodore 64 rule and §8.6's usage convention exactly.

*Fantastic Four* on `QUESTPR3.D64` is **not identified**, and an implementer
should know that before spending time on it. Its picture files are `Rnnn`,
`Bnnn` and `Snnn` — the `S` prefix again, twenty-one of them. But none of §7.2's
fixed database names appears; the data-looking files are `SAGA.OBJ` at `$1300`,
`SAGA.C64` at `$3A00`, `SAGA.TED` at `$2400`, and a matched pair `DATA` at
`$9000` and `BATA` at `$8000` of 16,386 bytes each; and **none of §4.1's nine
signatures occurs anywhere in the image or in any of those files**. No file
begins with a `$0801` depacker stub, so this is not simply a §7.2 cruncher case
either. Its database encoding is unknown to this document.

---

## 11. Refusal cases

Recognised-but-unspecified situations an implementer should name and refuse
rather than guess.

**TI-99/4A.** A signature match where a header pointer resolves outside the file,
or where the baseline would be negative, or where the header start is beyond end
of file — all mean "not this dialect", not "a corrupt file", because other
detectors have yet to run. The declared object-table pointer targets a structure
of unknown layout and must be left alone entirely. The unassigned header byte
has no meaning. Opcode bytes 202-211 and 213 are unassigned and their operand
counts are unknown, so encountering one makes the rest of the record
undecodable: abandon the record rather than skipping the byte as a no-op. (No
byte in that range is reached as an opcode anywhere in the twelve §10.2
specimens, so this is a guard against unknown files, not a case any known game
exercises.) A verb index beyond the highest verb index is reachable and must be
bounds-checked against the dispatch table; a dispatch entry that does not lie
wholly within the file is a *short file*, not a refusal, and reads as zero — see
§3.1, which works `adv07.fiad` through. Zero-length dictionary entries must not
stall a reader; entries of 20 characters or more should be rejected. String chunk
lengths of 0 or above 100 mark a malformed string. And one per-game patch exists
with no basis in the format — item index 3 whose description begins with the
letters "bird" has its take/drop name forced to `BIRD`, repairing one title
whose link byte is wrong; reproduce it for compatibility if you like, but do not
generalise it.

**Memory images.** A dictionary signature that matches but for which no
catalogued release validates is the normal outcome for an unknown release of a
known game, and there is no fallback: with no baseline there is no header
address and with no header address there are no counts. Refuse by name — "a
four-letter plain-text memory image, but no known release matches its header" —
rather than attempting a heuristic scan, except for the early-shape case that
§4.6 measures. A compressed string chain whose accumulated length walks past the
end of the image, or whose output exceeds 255 characters, indicates a wrong
address or a wrong release match with no recovery. A compressed action record
claiming more than five conditions or more than two commands is impossible;
clamping keeps a reader running but silently mis-aligns everything after it, so
a strict reader should refuse. Line-drawn picture data whose first byte is not
0xFF, or which truncates mid-record, leaves the remaining rooms with no
recoverable picture — a partial load, not a fatal error, and the player should
be told how many were lost.

**The US S.A.G.A. releases are a different format, and §12 now specifies it.**
The American disk editions — Adventureland, Pirate Adventure, Mission
Impossible, Voodoo Castle, The Count, Strange Odyssey, Claymorgue and the
Questprobe *Hulk*, on the Atari 8-bit, the Apple II and the Commodore 64 — and
the UK *Hulk* releases all carry a **binary database** that shares §4's model
and almost none of its encoding: a fifteen-word header in §4.5's US field order
at a fixed offset, a dictionary of nouns then verbs found by scanning for the
literal bytes `ANY`, **length-prefixed** strings rather than NUL-terminated
ones, item locations two bytes apart, a **column-major** action table,
**direction-major** room connections, and three tables of memory-address
pointers. §§4-6 do not apply to any of it. This document no longer refuses it —
read §12, which specifies the format, the container step for each platform, and
the per-release facts, all measured on fifteen databases against their
reference-format twins.

**Questprobe 3, *Fantastic Four*, is still refused**, on both the Commodore 64
(`QUESTPR3.D64`) and MS-DOS (`SPL53P.DAT`); see §12.14 for what was ruled out
and why.

**Containers.** Every refusal listed in §7 stands: unknown snapshot header
lengths and hardware modes; appended level-data blocks; a snapshot stream that
does not expand to exactly the right length; duplicate page numbers;
register-dump snapshots; Commodore 64 files not in the catalogue and Commodore
64 images of unlisted size; Atari images of any other sector size or capacity,
and header-less Atari images; raw Apple II nibble images, ProDOS-ordered images,
and the `2IMG` wrapper; thirteen-sector Apple II disks; 3.5-inch or multi-sided
bit-preserving images; and Apple II disks whose database file carries none of
the four recognised names.

**Pictures.** Eighteen byte-level patches exist, keyed on (release, picture
number, byte offset, length), correcting broken bytes in specific pictures of
seven Commodore 64 releases and two ZX ones. They are not a format feature: an
implementer should either enumerate them as a compatibility table or state that
unpatched data may decode visibly corrupt. Two Claymorgue releases ship with
broken picture tables — the Commodore 64 release is detected by picture 13
having both dimensions zero, after which pictures 12 through 27 except 16 are
unavailable; the ZX release by picture 9 having zero dimensions, after which
pictures 9 through 35 except 14 are unavailable. Reproduce the "unavailable"
behaviour; the forced-size repairs those releases also need are
compatibility hacks, not format.

**Two titles do not use the room-picture table at all**, building each room's
screen from a private display list that composites family A pictures. *Seas of
Blood* carries a 2010-byte block of one variable-length program per room
separated by 0xFF terminators, whose opcodes are: 0xFF end; 0xFE mirror the left
sixteen columns onto the right, reversing each cell's pixel rows and copying its
attribute; 0xFD with two operands recolour, every cell whose ink equals (first
operand AND 7) gaining ink (second AND 7) and likewise for paper; 0xFC with four
operands write an attribute byte into that many consecutive cells from a cell
position; 0xFB set the bright bit on all 384 cells; 0xFA mirror the whole image
horizontally; 0xF9 with two operands draw the current room's object picture at
that cell position and suppress the default object pass; anything else is a
picture number followed by a cell position. *Robin of Sherwood* treats rooms 11
through 73 as procedurally assembled forest scenes driven by a 555-byte
variable-stride table. Both must be recognised by title, and an implementer may
reasonably refuse them.

**Per-game association overrides** that no table expresses: Gremlins substitutes
picture 45 for room 17 under an item condition, draws 42 in room 34 under
another, draws 82 (and 90 in one release) in room 10, and drives a timed
animation over a list of frames; the Hulk ignores the item flag test, uses 255
rather than 0 as the item sentinel, suppresses two item pictures under
conditions, and maps room numbers 81-89 onto pictures 34-42 for close-ups;
Robin of Sherwood draws pictures 3, 12, 15, 36 and 70 on item and room
conditions; Savage Island draws picture 9 in place of item 20's picture in room
8; The Count, Voodoo Castle and the US Hulk each add hard-coded overlays. Name
these rather than infer them.

**Known-wrong colour handling**, restated so it is not mistaken for format: the
fourth colour register of every family C picture is discarded and pixel value 0
is forced to black; the Commodore 64 US colour byte mapping is an incomplete
empirical lookup with no rule; the Atari 256-entry palette contains sixteen
hand-substituted entries that match no standard Atari palette; and the four
remap tables were derived by eye and may contain mistakes.

---

## 12. The US S.A.G.A. binary database

This is the format §11 used to refuse by name: the encoding of the American
"S.A.G.A." (Scott Adams Graphic Adventure) disk releases for the Atari 8-bit,
the Apple II and the Commodore 64, and of the Questprobe *Hulk*. It is a binary
database like §4's, and it decodes to the same model, but **almost nothing else
is shared** — it has no dictionary signature, no per-release address catalogue,
a different string encoding, a transposed action table, transposed room
connections, and its own pointer tables. It is specified here rather than folded
into §4 because none of §4's locating rules apply to it.

Every layout statement below was measured on the fifteen databases of §12.13 —
seven Atari 8-bit, seven Apple II and one Commodore 64 — and every table was
checked field for field against the freely redistributable reference-format
conversion of the same game (§12.13 names the twin for each).

### 12.1 Shape of the whole thing

A database is one flat byte array. Its layout, in file order:

| # | region | size | § |
|---|---|---|---|
| 1 | front matter, carrying the version and adventure numbers | 0x38 bytes | 12.2 |
| 2 | header | 15 little-endian words, of which the first 11 carry fields | 12.4 |
| 3 | gap (present on Atari and Apple, absent on the Commodore 64) | per release | 12.4 |
| 4 | dictionary: **nouns then verbs**, (words + 1) cells each | (words + 1) x 2 cells | 12.5 |
| 5 | room descriptions, length-prefixed | (rooms + 1) strings | 12.6 |
| 6 | messages, length-prefixed | (messages + 1) strings | 12.6 |
| 7 | item descriptions, length-prefixed | (items + 1) strings | 12.6 |
| 8 | one separator byte | 1 | 12.7 |
| 9 | item locations | (items + 1) words | 12.7 |
| 10 | item-description pointers | (items + 1) words | 12.7 |
| 11 | item locations again, an identical copy | (items + 1) words | 12.7 |
| 12 | actions, **column-major** | 16 x (actions + 1) bytes | 12.8 |
| 13 | room-description pointers | (rooms + 1) words | 12.7 |
| 14 | room connections, **direction-major** | 12 x (rooms + 1) bytes | 12.9 |
| 15 | message pointers, in a 228-byte allocation | (messages + 1) words | 12.7 |

Every count in that table is a *highest index*, so a table of *N* things has
*N* + 1 records, exactly as in §2 and §4.4. There are no gaps other than region
3 and the slack at the end of region 15: on every specimen each region begins
exactly where the previous one ended, which is what makes reading the whole
thing sequentially a sound check on the header.

### 12.2 Detection

There is no magic number. **Read the first 0x38 bytes as twenty-eight
little-endian words; the first non-zero value below 500 is the format version
and the next value below 500 is the adventure number.** A database whose
adventure number comes out zero is not one of these.

**The zero skip is load-bearing.** A scan that lets a zero stand as the version
answers (0, 0) on the Apple II *Claymorgue* database, whose words at offsets
0x02 through 0x21 are every one of them zero; the real pair sits past them.

Measured, the pair is at a fixed place per platform but not across platforms,
which is why the scan exists rather than two constants: **offsets 0x22 and 0x26
on all fourteen Atari and Apple databases, 0x34 and 0x36 on the Commodore 64
*Hulk***.

Neither number is a game identity on its own. The version is the release build
number — 416, 408, 306, 119, 115, 119 and 125 for the seven Atari titles, the
same six plus 122 for the Apple II — and the adventure number is the Adventure
International series number. **Only the pair identifies a title**, because
adventure 1 is *Adventureland* at version 416 and the *Hulk* at version 127.
Three pairs are known to name variants rather than the plain series entry:
version 127 with adventure 1 is the US *Hulk*; version 126 with adventure 2 and
version 126 with adventure 13 are two further variants an existing catalogue
distinguishes. Any other pair means "the series entry named by the adventure
number".

### 12.3 Reaching the database, per platform

The container work is §7's; what is per-platform here is only *where in the
container the array starts*, and each platform's constant is chosen so that
**the header always lands 0x38 bytes into the array**.

**Atari 8-bit** (§7.3). The database is the byte range of the disk image
beginning at **file offset 0x04C1** and running to the end, taken as-is — the
header then falls at file offset 0x04F9. It is not a file in the disk's own
directory: walking the Atari DOS catalogue in sectors 361-368 of these images
finds only `DOS.SYS`, `AUTORUN.SYS` and (on *Strange Odyssey*) `NIR.SYS`. The
range is raw, with no per-sector link trailers in it; that is what lets a flat
slice work at all, and it is a property of how these disks were mastered rather
than of the container. Side A holds the database and side B the pictures
(§10.5).

**Apple II** (§7.4). Read the DOS 3.3 catalogue on the **boot** side, take the
file whose normalised name is `DATABASE` or matches `A?.DAT`, and read it
through its track/sector list. That gives the raw sector concatenation,
beginning with the binary file's four-byte prologue (load address, then data
length). **The database array begins at offset 0x135 of that concatenation**, so
the header falls at 0x16D. Every one of the seven is 42 catalogue sectors, of
which 41 hold data (§7.4's "count − 1" rule), loads at `$4000`, and declares a
data length of 10,335 bytes.

**Commodore 64** (§7.2). Read the named file out of the disk image through its
block chain; for the *Hulk* the name is `SHULK.DB`, and the array is the program
file **including** its two load-address bytes. The header falls at offset 0x38 of
that array with no gap after it at all. `QUESTPR1.D64` sums to 0x2918 in the low
sixteen bits, which is §7.2's identification checksum, and its `SHULK.DB` is
9,858 bytes loading at `$8030`.

**The memory-image route, and the number 645.** A release of this format can
also arrive as a §4 memory image, located by a §4.1 dictionary signature rather
than by a container offset — this is how the UK *Hulk* releases are handled. The
baseline is then §4.3's, with a correction: baseline = signature hit − the
catalogued dictionary address − **645**. The 645 is not arbitrary. §4.1's plain
signature is `AUTO\0GO\0`, which is the head of the **verb** block, while this
format's dictionary proper begins at the **noun** block; measured on the
Commodore 64 *Hulk*, the noun block is at array offset 0x55 and `AUTO\0GO\0` at
0x2DA, and 0x2DA − 0x55 = **645**. The constant is the distance between the two
blocks in that database, and it is not a general property of the format — a
reader taking the container route never needs it.

**"Scrambled" is a picture matter, not a database one.** §7.4's `M2` string test
and §8.4's two byte placements select how a *picture* is decoded. Verified:
*Voodoo Castle*, *The Count* and *Claymorgue* are the three Apple II titles whose
`M2` file carries `COPYRIGHT 1983 NORMAN L. SAILER` at file offset 0x172C, and
all three decode their **databases** by the rules of this section unchanged, with
no transform of any kind. The 0x182-byte table those three releases carry at
`M2` file offset 0x174B is byte-identical between them, and it is a row-address
table for §8.4: the address for row *y* is
(table byte at *y*) + 256 x (table byte at 0xC0 + *y*) − 0x2000. *Worked
example*, from the *Voodoo Castle* boot disk: the table's first eight bytes are
`00 00 00 00 00 00 00 00` and its bytes at 0xC0 through 0xC7 are
`20 24 28 2C 30 34 38 3C`, so rows 0 to 7 resolve to 0x0000, 0x0400, 0x0800,
0x0C00, 0x1000, 0x1400, 0x1800 and 0x1C00 — the ordinary Apple high-resolution
interleave, spelled out in a table instead of computed.

### 12.4 The header, and the anchoring rule

**Fifteen little-endian 16-bit words at array offset 0x38.** The field order is
§4.5's **US** shape: word length 0, words 1, actions 2, items 3, messages 4,
rooms 5, max carried 6, start room 7, treasure count 8, lamp turns 9, and the
treasure room in the **high byte** of word 10. Words 11 to 14 carry no field —
on the Commodore 64 *Hulk* they are already the dictionary.

**A reader consumes twenty-nine bytes, not thirty.** The fifteenth word's low
byte is re-read: the next region begins at header start + 29. This is not a
nicety. On the *Hulk*, the header is at 0x38 and the noun cell `ANY` begins at
**0x55**, which is header start + 29; a reader that advances a full thirty bytes
steps over the `A` and never finds the dictionary at all.

**Sanity limits** for accepting a header, all inclusive: items 10-500, actions
100-500, words 50-200, rooms 10-100, messages 10-255. A header failing any of
them means "not this format", not "a corrupt file". Scanning a whole Atari disk
image for any window of fifteen words satisfying those limits together with max
carried 1-20, word length 3-6 and a start room within the room count yields
**exactly one** hit on each of the seven side-A images, at file offset 0x04F9 —
so the mastering constant of §12.3 is checkable rather than merely asserted.

### 12.5 The dictionary

**Located by scanning forward from the end of the header for the literal bytes
`A`, `N`, `Y`.** There is no signature table and no back-off; the three letters
are the first noun cell and the dictionary begins on the `A`. Measured, the scan
walks **0 bytes** on the Commodore 64 *Hulk*, whose noun block starts exactly at
header + 29, and **179 bytes** on every Atari and Apple release, whose
dictionaries all begin at array offset 0x108.

**Order is nouns first.** (words + 1) noun cells, then (words + 1) verb cells —
**the reverse of §4.2's memory-image order**, which is why the §4.1 signature
lands in the middle of the block rather than at its start.

**Cells** are the word length in characters, with two escapes that consume an
extra byte each: a NUL where a cell should begin is skipped and the following
byte taken as the first character, and a `*` marking a synonym restarts the
character count so that the word after it still gets the full width. **Each
escape fires at most once per cell.** A NUL *inside* a cell is not an escape but
an ordinary character that counts towards the word length; trailing NULs are
trimmed from the finished word. A byte above 127 terminates. There is no space
escape here either (§4.2): across all fifteen databases, a space followed by a
non-space never once occurs at a position where such a rule would fire.

**"At most once" is normative, and letting the leading-NUL escape repeat loses
the dictionary.** A run of pad bytes where a cell should begin is not alignment
slack to be skipped over; it is an **empty dictionary entry**, and the
reference-format conversions carry those entries too. A reader that skips every
leading NUL it meets swallows the run, puts the *next* word in that slot, and is
one entry short for the whole of the rest of the block — after which every
vocabulary index in the action table names the wrong word.

*Worked example*, the first twenty bytes of Adventureland's dictionary — array
offset 0x108, word length 3, on both the Atari and the Apple II release:

```
41 4E 59 00 | 4E 4F 52 00 | 53 4F 55 00 | 45 41 53 00 | 57 45 53 00
A  N  Y     | N  O  R     | S  O  U     | E  A  S     | W  E  S
```

Three characters are read and the pad NUL is left for the next cell's leading-NUL
escape. Two cells later `55 50 00 00` gives `UP` — two letters and two pads, the
second of which the escape absorbs. The verb block begins 279 bytes further on,
at 0x21F, and reads `00 41 55 54 | 00 47 4F 00 | 2A 45 4E 54 | 2A 52 55 4E` —
a leading pad, `AUT`, `GO`, then the synonyms `*ENT` and `*RUN`, each of which
spends a byte on its `*`.

*Worked example of the empty entry*, *Pirate Adventure*'s verb block — array
offset 0x247, word length 3, and byte-identical on the Atari 8-bit and the
Apple II release:

```
0x032B  00 57 45 49    escape, W E I      -> verb 57 = WEI
0x032F  00 00 00 00    escape, 00 00 00   -> verb 58 = (empty)
0x0333  00 42 52 45    escape, B R E      -> verb 59 = BRE
```

Cell 58's four bytes are every one of them NUL: the escape absorbs the first,
the remaining three are the cell's three characters, and trimming the trailing
NULs leaves the empty string — which is exactly what `adv02.dat` gives for verb
58. Let the escape repeat and all four bytes are skipped, `BRE` is read as verb
58, and **every verb after it is one slot early**. This release's verb block has
fifteen empty cells (58, 62, 65, 66, and 69 through 79); a repeating escape
consumes them one by one, runs the dictionary 46 bytes past its true end at
0x387, and produces `\x0eFl` for verb 65 — the length byte and first two
characters of room 1's description, `Flat in london`, read as vocabulary. The
correct reading ends the dictionary exactly where the room block begins, which
is the cheap check to make.

Verified against the conversions: the dictionary decodes to the twin's verb and
noun lists exactly (truncated to the release's word length, with the `*` marker
not counting towards it) on all seven Atari titles, on six of the seven Apple II
titles, and on the Commodore 64 *Hulk*.

**The Apple II *Claymorgue* differs from its twin in nineteen verbs and two
nouns.** It is version 122 where the Atari release is 125, and the Atari one
matches the twin exactly, so this is a release difference and not a decoding
one. The nineteen verbs fall into three groups:

- **five mid-table synonyms** — verbs 17, 26, 33, 45 and 54 — that v122 spells
  as the placeholder `.` where v125 has `*PICKU`, `*TOWAR`, `*TUG`, `*USING`
  and `*SLICE`;
- **one substitution**, verb 51, which is `LOWER` in v122 and `*INVOK` in v125;
- **thirteen entries v122 leaves empty**: verbs 88 and 89 (`*SING` and `*WASH`
  in v125) and the eleven-entry run 96 to 106, which in v125 is `LIGHT`
  followed by its ten synonyms `*BURN`, `*IGNIT`, `*BUILD`, `*MAKE`, `*COOK`,
  `*WASH`, `*LOWER`, `*KILL`, `*SLAY` and `*CLEAN`.

The two nouns are 107 and 108, `EDGE` and `*EDGES`, likewise empty in v122.

**Both releases' dictionaries occupy the same bytes**, which is what makes this
a release difference rather than a reader drifting: each begins at array offset
0x108 and each ends at 0x62F, where its room block begins, after 110 noun cells
and 110 verb cells. Verb 96 is `00 4C 49 47 48 54` on the Atari — escape, then
`LIGHT` — and `00 00 00 00 00 00` on the Apple II; six bytes either way, and
the cell after it starts in the same place on both.

### 12.6 Rooms, messages and item descriptions

All three are **length-prefixed strings**, back to back: one byte of length, then
that many bytes of text, no terminator. **A length of 0 means the string `.`** —
the reference format's placeholder — and consumes no further bytes.

*Worked example*, Adventureland's room block at array offset 0x337:
`00 | 0C 64 69 73 6D 61 6C 20 73 77 61 6D 70 | 1A 74 6F 70 …` — room 0 is the
placeholder, room 1 is twelve bytes of `dismal swamp`, room 2 is twenty-six.

The reference format's conventions carry over unchanged: a leading `*` on a room
description means "print literally", a leading `*` on an item description marks
a treasure, and an item's auto-get word is the text between the first `/` and
the next, with the literal remainders `//` and `/*` meaning "none" — the rule of
§2.5 applies as written.

**An item whose description carries an auto-get word is followed by one extra
byte, outside the length prefix, and that byte is the item's own index.**
Measured on Adventureland: 31 of the 66 item records carry the extra byte and 35
do not, the split falls exactly on whether the text contains a `/WORD/`, and
every one of the 31 bytes equals its own item number — item 2
(`*Pot of RUBIES*/RUB/`) is followed by `02`, item 7 (`Evil smelling mud/MUD/`)
by `07`, item 62 by `3E`. The same holds on all fifteen databases. The pointer
table of §12.7 confirms the framing independently: item 2's record runs from
0x1204 to 0x1219, twenty-two bytes for a one-byte length, twenty of text and the
one extra. It is a back-reference, not a picture number; nothing in this format
associates an item with a picture (§12.10).

**Comparing text against a conversion needs one normalisation.** The reference
format quotes strings with `"`, and the published conversions render an embedded
double quote as a backtick. Map backtick to `"` before comparing, or a third of
the message and item strings of every title will look wrong.

### 12.7 Item locations, the pointer tables and the trailer

Between the item descriptions and the actions sit one separator byte and three
tables. On every specimen the separator is **0x00**.

**Item locations** — (items + 1) little-endian words, i.e. entries **two bytes
apart**, with the location in the low byte and the high byte zero on every entry
of every specimen. **255 means carried** and **0 out of play**, as everywhere
else.

**Item-description pointers** — (items + 1) little-endian words, each the
**memory address** at which that item's length byte sits.

**Item locations again** — a second copy, byte-identical to the first on all
fifteen specimens.

Two more pointer tables of the same kind appear later: **(rooms + 1) pointers to
the room descriptions**, between the action table and the connections, and
**(messages + 1) pointers to the messages** after the connections, in a
**228-byte (0xE4)** allocation — the Commodore 64 *Hulk*'s database file ends
exactly 228 bytes past its connection table, and no specimen's message pointers
fill more than 200 of them.

These are addresses, not offsets, and that makes them useful twice over. They
**validate a parse**: on fourteen of the fifteen specimens every one of the
three tables resolves, entry for entry, onto the string starts a sequential read
produced. And they **recover the array's memory base**, which is this format's
equivalent of §4.3's baseline and, unlike it, is derivable: subtract the room
block's offset from the first room pointer. Measured, that base is `$4131` on
every Apple II release (`$4000` + 0x135 − 4, the load address adjusted for the
four-byte prologue and the mastering offset), `$3031` on every Atari release,
and `$802E` on the Commodore 64 *Hulk* (its load address `$8030` less the two
load-address bytes).

### 12.8 Actions, column-major

The action table is the same sixteen bytes per record as §4.4 and the same
packing of verb, noun, conditions and commands — but **transposed**. Instead of
(actions + 1) records of sixteen bytes, it is a fixed sequence of columns, each
column holding one field of every record:

| column | width | contents |
|---|---|---|
| 1 | 1 byte x (actions + 1) | the vocabulary word's **verb** |
| 2 | 1 byte x (actions + 1) | the vocabulary word's **noun** |
| 3 | 1 byte x (actions + 1) | command 1 |
| 4 | 1 byte x (actions + 1) | command 2 |
| 5 | 1 byte x (actions + 1) | command 3 |
| 6 | 1 byte x (actions + 1) | command 4 |
| 7-11 | 2 bytes x (actions + 1) each | conditions 1 to 5, little-endian |

So with *n* = actions + 1 and the table starting at offset *T*: record *k*'s verb
is the byte at *T* + *k*, its noun at *T* + *n* + *k*, its four command bytes at
*T* + 2*n* + *k*, *T* + 3*n* + *k*, *T* + 4*n* + *k* and *T* + 5*n* + *k*, and
its condition *j* (counting from 0) is the little-endian word at
*T* + 6*n* + 2*jn* + 2*k*. The whole table is 16*n* bytes, exactly as row-major
would be.

Reassembly into reference-format numbers is unchanged: the vocabulary word is
verb x 150 + noun; the first command word is 150 x (command 1) + (command 2) and
the second is 150 x (command 3) + (command 4); the condition words are already
in reference form, condition code + 20 x value.

*Worked example*, Adventureland (Apple II, `A1.DAT`). The table starts at array
offset 0x19D4, *n* = 170, so the columns begin at 0x19D4, 0x1A7E, 0x1B28,
0x1BD2, 0x1C7C, 0x1D26 and the five condition columns at 0x1DD0, 0x1F24, 0x2078,
0x21CC, 0x2320. Record 0 takes the first byte of each:

| field | offset | byte(s) | value |
|---|---|---|---|
| verb | 0x19D4 | `00` | 0 |
| noun | 0x1A7E | `4B` | 75 |
| command 1 | 0x1B28 | `75` | 117 |
| command 2 | 0x1BD2 | `3E` | 62 |
| command 3 | 0x1C7C | `00` | 0 |
| command 4 | 0x1D26 | `00` | 0 |
| condition 1 | 0x1DD0 | `A1 00` | 161 |
| condition 2 | 0x1F24 | `82 01` | 386 |
| condition 3 | 0x2078 | `A0 00` | 160 |
| condition 4 | 0x21CC | `C8 00` | 200 |
| condition 5 | 0x2320 | `00 00` | 0 |

Vocabulary word 0 x 150 + 75 = **75**; first command word 150 x 117 + 62 =
**17612**; second 0. `adv01.dat`'s first action line is
`75 161 386 160 200 0 17612 0` — the same eight numbers in the same order.

**How closely the tables agree with the conversions.** Two titles are exact on
both platforms: *Voodoo Castle*, all 190 records, and *The Count*, all 220. The
rest differ in a small number of records, and the count is a property of the
release rather than of the platform:

| title | Atari | Apple II |
|---|---|---|
| #1 Adventureland | 4 of 170 | 4 of 170 |
| #2 Pirate Adventure | 4 of 178 | 4 of 178 |
| #3 Mission Impossible | (specimen damaged) | 7 of 162 |
| #4 Voodoo Castle | **0** of 190 | **0** of 190 |
| #5 The Count | **0** of 220 | **0** of 220 |
| #6 Strange Odyssey | 3 of 224 | 3 of 224 |
| #13 Claymorgue Castle | 2 of 268 | 15 of 268 |
| Questprobe 1: The Hulk (C64) | — | 10 of 262 |

Every difference measured has the same shape: the graphic release carries one
extra command, and sometimes one extra parameter condition, that the text
conversion does not. That extra command is **89 or 90** in every case, and the
rest of this section is about them. These are release differences, not decoding
failures — a column-major misreading cannot produce 166 exact records and four
wrong ones — and *Claymorgue*'s wider Apple II gap is the same version 122
versus 125 split its dictionary shows (§12.5).

**Two command codes these releases use and their conversions never do: 89 and
90.** §2.4 leaves commands 0 through 89 to the reference format's own
`Definition` document; these releases also use **90**, and 89 occurs in the same
tables, so 90 is not a renumbering of 89. Neither appears in any of the eight
reference-format twins. Across every reference-format file this document names —
the §10.1 archive's twelve numbered games, `quest1`, `quest2` and `sampler1`, and
the eleven *Mysterious Adventures* conversions of §10.4 — **89 appears twice, in
*Spider-Man* (`quest2.dat`), and 90 never appears at all**. Measured occurrences
over the fourteen databases that decode:

| release | 89 | 90 |
|---|---|---|
| #1 Adventureland — Atari, Apple II | 2 each | 2 each |
| #2 Pirate Adventure — Atari, Apple II | 1 each | 3 each |
| #3 Mission Impossible — Apple II | 5 | 2 |
| #6 Strange Odyssey — Atari, Apple II | 0 | 3 each |
| Questprobe 1: The Hulk — Commodore 64 | 0 | 11 |
| #4 Voodoo Castle, #5 The Count, #13 Claymorgue — both platforms | 0 | 0 |

**Their operand counts are the load-bearing fact: command 89 takes NO operand
and command 90 takes ONE.** Operands in this format are smuggled through
condition slots carrying code 0 (§2.4) and consumed left to right by whichever
commands want them, so an arity error does not merely mis-run the one command —
it hands every later command in the same record the wrong operand, silently.

That is also how the arities were measured, and it needs nothing but the oracle.
Thirty-eight records across five titles and both platforms carry an 89 or a 90;
for each of them the operands the record's *other* commands receive can be
compared against the same-numbered record in the reference-format twin, whose
condition list is usually byte-identical. Only one assignment reproduces the twin
in all thirty-eight:

| assumed arity | records reproducing the twin's operand assignment |
|---|---|
| **89 takes 0, 90 takes 1** | **38 of 38** |
| 89 takes 0, 90 takes 0 | 36 of 38 |
| 89 takes 1, 90 takes 1 | 32 of 38 |
| 89 takes 1, 90 takes 0 | 30 of 38 |

*The falsifying example*, Adventureland on both platforms, action 107 — `RUB
LAMP`. Its conditions are `203 169 960 160 0`, identical in the database and in
`adv01.dat`; its commands are `49 89 53 58` in the database and `49 53 58 0` in
the conversion. The three code-0 slots supply operands 48, 8 and 0. With 89
taking none, the record prints message 49 ("A glowing Genie appears, drops
something, then vanishes."), drops **item 48** — `*DIAMOND RING*`, whose start
location is 0, out of play, exactly what a genie conjuring a treasure needs —
and sets bit flag 8, which is `adv01.dat`'s line to the number. With 89 taking
one operand the same record instead drops **item 8**, `*GOLDEN FISH*`, which is
already lying in room 10, and sets **bit flag 0**, the darkness flag. Action 111
is the same record for the bracelet, item 49, and *Pirate Adventure*'s action
104 (`SET SAIL`) falsifies it a third time: an operand for 89 there makes
command 58 set flag 37 rather than flag 4 and leaves command 62 an operand
short.

**What command 90 does: draw the picture whose index is its operand, over the
whole graphics window.** The operand is a **room-usage** picture index in §12.10's
sense — the same numbering a room's own picture is named by — and in every
occurrence measured it is *above* the release's room count, so it never names a
room. The picture side of each release carries exactly the files those operands
name:

| release | rooms | operands measured | picture files present |
|---|---|---|---|
| #1 Adventureland, Apple II | 33 | 90, 91 | `R0190`, `R0191` |
| #2 Pirate Adventure, Apple II | 26 | 90, 91 | `R0290`, `R0291` |
| #3 Mission Impossible, Apple II | 23 | 88, 89 | `R0388`, `R0389` |
| #6 Strange Odyssey, Apple II | 35 | 91, 92, 93 | `R0691`, `R0692`, `R0693` |
| Questprobe 1: The Hulk, Commodore 64 | 20 | 81 to 89 | `R01081` … `R01089` |

Those file names were read out of each release's own DOS 3.3 catalogue and out
of the *Hulk* disk's directory, and the same disks carry the `…98` picture
§12.11's inventory view draws and the `…00` picture its darkness case draws — the
same numbering arrived at from two other directions. The reference draws this
command for the *Hulk* alone, as a full-window picture followed by a wait for
the player to press ENTER, after which the room view returns; for every other
release of this section it consumes the operand and draws nothing.

**What command 89 does is not settled by any specimen here.** It takes no
operand; it occurs only in *Adventureland*, *Pirate Adventure* and *Mission
Impossible*; and within a title every one of its records calls for what looks
like the same effect — both of Adventureland's are the genie, and all five of
Mission Impossible's stand immediately before command 63, "the game is over".
The reference implements this command only for releases outside this section
(blanking the picture for one of them, drawing an operand-selected closeup and
waiting for ENTER for others) and has no case for any release of §12, so it
draws nothing for all fifteen while still consuming an operand — which is
precisely the arity the table above rules out. Each of these three titles carries
high-numbered `R…` pictures that command 90 never names — Adventureland's
`R0180` to `R0189`, Mission Impossible's `R0380` to `R0385` — and those are
where an investigation of this command should begin.

*Worked example of a record carrying command 90*, the Commodore 64 *Hulk*,
action 101. Actions + 1 = 262 and the table starts at array offset 0x1418, so
the columns begin at 0x1418, 0x151E, 0x1624, 0x172A, 0x1830, 0x1936 and the five
condition columns at 0x1A3C, 0x1C48, 0x1E54, 0x2060, 0x226C:

| field | offset | byte(s) | value |
|---|---|---|---|
| verb | 0x147D | `27` | 39 — `LOOK` |
| noun | 0x1583 | `09` | 9 — `MIRR` |
| command 1 | 0x1689 | `01` | 1 |
| command 2 | 0x178F | `02` | 2 |
| command 3 | 0x1895 | `0C` | 12 |
| command 4 | 0x199B | `5A` | **90** |
| condition 1 | 0x1B06 | `2B 00` | 43 — code 3, value 2 |
| condition 2 | 0x1D12 | `35 01` | 309 — code 9, value 15 |
| condition 3 | 0x1F1E | `68 06` | 1640 — code 0, **operand 82** |
| condition 4 | 0x212A | `1A 00` | 26 — code 6, value 1 |
| condition 5 | 0x2336 | `00 00` | 0 — code 0, operand 0 |

Vocabulary word 39 x 150 + 9 = **5859**; first command word 150 x 1 + 2 =
**152**; second 150 x 12 + 90 = **1890**. `quest1.dat`'s action 101 is
`5859 43 309 1640 26 0 152 1800` — the same eight numbers with **1800** where
this database has 1890, which is this record with command 90 removed and the 0
it displaced left in its place. Messages 1, 2 and 12 are `'O.K.'`, `'I see'` and
`'Bruce Banner'`; the operand the conversion still carries but no longer consumes
is 82. Looking in the mirror prints those three lines and draws `R01082`. One
byte of the sixteen is the whole of the difference, which is what the table above
means by "one extra command, and sometimes one extra parameter condition".

### 12.9 Room connections, direction-major

(rooms + 1) x 6 exits, stored **direction-major**: all the north exits for every
room, then all the south, east, west, up and down. Within a direction block the
entries are **two bytes apart**, the exit in the low byte and the high byte zero
on every entry of every specimen. A block is therefore 2 x (rooms + 1) bytes and
the whole table 12 x (rooms + 1). 0 means no exit.

*Worked example*, Adventureland (Apple II), 34 rooms' worth. The table starts at
0x24B8 and the six blocks begin at 0x24B8, 0x24FC, 0x2540, 0x2584, 0x25C8 and
0x260C. Room 1's exits are the second word of each: `17 00`, `00 00`, `1D 00`,
`19 00`, `00 00`, `00 00` — 23, 0, 29, 25, 0, 0, which is `adv01.dat`'s room 1
exit line exactly.

There is no room-image byte anywhere in this table or beside it (§12.10).

### 12.10 Pictures: what the database says

**Nothing.** This is worth stating plainly because §4.4's later-release layout
has three per-object picture tables and it is natural to look for them here.
There is no room-image list, no item-flag list and no item-image list; the
three tables that sit where they might have are the two location copies and the
item-description pointers of §12.7, all of them accounted for byte by byte
above; and the one loose byte in the item block is the item's own index (§12.6).

Pictures live in **separate files on the disk**, one per picture, identified by
filename, and the association is by name and index alone:

- **Commodore 64 and Atari 8-bit** — picture family C (§8.3), named by §8.6's
  convention: `R…` for a room picture, `B…` with a trailing `R` for an object
  drawn in a room and a trailing `I` for one drawn in the inventory. The
  Commodore 64 *Hulk* disk carries `R01nnn`, `B01nnnR` and `B01nnnI` (§10.7).
  Atari pictures are on the companion side (§10.5) and are reached by offset,
  not by the disk's directory.
- **Apple II** — picture family D (§8.4), from `PAK.INVEN` / `PAC.INVEN` and the
  `PAK.LET…` or `Rnnnn`/`Bnnnnn` files (§10.6).

Two consequences for a reader. A room's picture index **is the room number**;
there is no table to consult. And the per-title (usage, index, offset, length)
lists that §8.3 and §8.4 require are not recoverable from the database — they
are the one thing about these releases that must still be tabulated, and this
section cannot help with them.

### 12.11 Runtime behaviour that differs

In the manner of §9, the behaviours a conforming interpreter must produce for
these releases and not for others:

- **A room's picture is drawn from the room number**, with no room-image byte
  and no 255 sentinel (§12.10). After it, every object picture whose item index
  names an item presently in the room is drawn over it, in the order the picture
  files were gathered.
- **Darkness does not blank the graphics window.** Where other dialects paint
  black, these draw **picture index 0** — a dedicated darkness image — and
  return.
- **The inventory command draws a picture.** Beyond listing what is carried, it
  clears the graphics window, draws **picture index 98** as a room picture, then
  draws the inventory-object picture of every carried item, and waits for the
  player to press ENTER before restoring the room view. On the Atari 8-bit the
  index-98 image comes from the companion disk rather than from the database
  side.
- **The *Hulk* remaps five pairs of rooms onto other rooms' pictures** — 5 and 6
  onto 3, 7 and 8 onto 4, 10 and 11 onto 9, 13 and 14 onto 2, 17 and 18 onto 16
  — on the Commodore 64 and Atari 8-bit releases but **not** on the Apple II one.
- **Four titles add hard-coded overlays** that no table expresses: the *Hulk*
  draws object pictures 70, 72 and 13 under item-position conditions; *The
  Count* draws 80, 81 and 82 only in rooms 8, 18 and 9 respectively; *Voodoo
  Castle* draws 80 only in room 14. §11's "per-game association overrides"
  paragraph covers these; name them rather than infer them.
- **Command 90 draws a picture, and an interpreter that draws none still has to
  consume its operand.** §12.8 fixes the two arities: **89 takes no operand and
  90 takes one**. Command 90 draws the room-usage picture its operand names over
  the whole graphics window and waits for the player to press ENTER before the
  room view returns — the same door the inventory picture above goes through, and
  the operand is never a room's own number. Command 89's drawing effect is
  undetermined (§12.8). **A conforming interpreter that draws no pictures must
  still consume one operand for 90 and none for 89**, and must make no other
  state change for either. That is not a cosmetic detail to be skipped along with
  the artwork: get 89's arity wrong and every later command in the same record
  receives the wrong operand, which on *Adventureland*'s `RUB LAMP` conjures the
  wrong treasure and clears the darkness flag.
- **The two lamp options §9.2 describes are not forced.** These are Adventure
  International releases, not Mysterious Adventures ones, and take the host's
  settings.

### 12.12 The per-release table, and what can be re-derived

Fifteen facts per release are tabulated below, and — unlike §4.6's memory-image
catalogue, where roughly seventeen numbers per release have no derivation at all
— **every one of the fifteen is recovered from the bytes**. The table is an
identification checksum, not an input.

| fact | how it is re-derived |
|---|---|
| version, adventure number | §12.2's scan of the first 0x38 bytes |
| word length, words, actions, items, messages, rooms, max carried, start room, treasures, lamp turns, treasure room | the header at array offset 0x38, read in §4.5's US field order |
| dictionary offset | scan forward from header + 29 for the literal `ANY` (§12.5) |
| database base address | first room-description pointer minus the room block's offset (§12.7) |

The only per-release knowledge this format genuinely requires is the picture
lists of §12.10, and the only per-**platform** knowledge is the three container
constants of §12.3.

**Atari 8-bit**, side A of each two-sided set, database at file offset 0x04C1,
base `$3031`, dictionary at array offset 0x108 in every one:

| title | ver | adv | wlen | words | acts | items | msgs | rooms | carry | start | treas | lamp | t.room | twin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| #1 Adventureland | 416 | 1 | 3 | 69 | 169 | 65 | 75 | 33 | 6 | 11 | 13 | 125 | 3 | `adv01.dat` |
| #2 Pirate Adventure | 408 | 2 | 3 | 79 | 177 | 66 | 88 | 26 | 6 | 1 | 2 | 150 | 1 | `adv02.dat` |
| #3 Mission Impossible | 306 | 3 | 3 | 64 | 161 | 53 | 81 | 23 | 7 | 2 | 0 | 10000 | 1 | `adv03.dat` |
| #4 Voodoo Castle | 119 | 4 | 3 | 89 | 189 | 65 | 99 | 25 | 9 | 1 | 0 | 15000 | 0 | `adv04.dat` |
| #5 The Count | 115 | 5 | 3 | 79 | 219 | 72 | 88 | 22 | 7 | 1 | 0 | 175 | 0 | `adv05.dat` |
| #6 Strange Odyssey | 119 | 6 | 4 | 79 | 223 | 55 | 94 | 35 | 6 | 1 | 5 | 10000 | 22 | `adv06.dat` |
| #13 Claymorgue Castle | 125 | 13 | 5 | 109 | 267 | 75 | 79 | 32 | 10 | 1 | 13 | 3000 | 19 | `adv13.dat` |

**Apple II**, the boot side of each two-sided set, database at offset 0x135 of
the named DOS 3.3 file, base `$4131`, dictionary at array offset 0x108 in every
one:

| title | file | ver | adv | wlen | words | acts | items | msgs | rooms | carry | start | treas | lamp | t.room | twin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| #1 Adventureland | `A1.DAT` | 416 | 1 | 3 | 69 | 169 | 65 | 75 | 33 | 6 | 11 | 13 | 125 | 3 | `adv01.dat` |
| #2 Pirate Adventure | `A2.DAT` | 408 | 2 | 3 | 79 | 177 | 66 | 88 | 26 | 6 | 1 | 2 | 150 | 1 | `adv02.dat` |
| #3 Mission Impossible | `A3.DAT` | 306 | 3 | 3 | 64 | 161 | 53 | 81 | 23 | 7 | 2 | 0 | 10000 | 1 | `adv03.dat` |
| #4 Voodoo Castle | `A4.DAT` | 119 | 4 | 3 | 89 | 189 | 65 | 99 | 25 | 9 | 1 | 0 | 15000 | 0 | `adv04.dat` |
| #5 The Count | `DATABASE` | 115 | 5 | 3 | 79 | 219 | 72 | 88 | 22 | 7 | 1 | 0 | 175 | 0 | `adv05.dat` |
| #6 Strange Odyssey | `A6.DAT` | 119 | 6 | 4 | 79 | 223 | 55 | 94 | 35 | 6 | 1 | 5 | 10000 | 22 | `adv06.dat` |
| #13 Claymorgue Castle | `DATABASE` | 122 | 13 | 5 | 109 | 267 | 75 | 79 | 32 | 10 | 1 | 13 | 3000 | 19 | `adv13.dat` |

**Commodore 64**, `SHULK.DB` from `QUESTPR1.D64`, base `$802E`, dictionary at
array offset 0x55 — immediately after the header, with no gap:

| title | file | ver | adv | wlen | words | acts | items | msgs | rooms | carry | start | treas | lamp | t.room | twin |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Questprobe 1: The Hulk | `SHULK.DB` | 127 | 1 | 4 | 128 | 261 | 54 | 99 | 20 | 10 | 1 | 17 | 150 | 16 | `quest1.dat` |

Two things the tables show that are worth reading off them. The Atari and Apple
II releases of titles 1 to 6 are **the same build** — identical version numbers
and identical header counts — and differ only in container and picture family;
*Claymorgue* is the exception, 125 on the Atari and 122 on the Apple. And the
Commodore 64 *Hulk*'s eleven header numbers are **identical to those of the
MS-DOS `ADVENT.DAT`** of §10.7, which is the plain reference text format — the
same database in two encodings, which is exactly the oracle §12.13 recommends.

### 12.13 Specimens, and the oracle

The specimens are catalogued in §10.5 (Atari 8-bit, seven two-sided sets), §10.6
(Apple II, seven two-sided sets) and §10.7 (the Commodore 64 Questprobe images),
with the sha256 of every file. The ones this section was measured on:

| platform | specimen | sha256 |
|---|---|---|
| Atari | `SAGA #1 - Adventureland [side A].atr` | `7c48bc7779d15b75ef66cddf384f77c6a3a37e5372340e71a573bbec41ba2d75` |
| Atari | `SAGA #2 - Pirate Adventure [side A].atr` | `547c0e902c732c11d158a4c3e46a2e41a6da861516e60b763f2157b2785d3041` |
| Atari | `SAGA #3 - Mission Impossible [side A].atr` | `7575154bd45890d411fcf19cb566523934ff884b471b25cfa9e3002b88c3d136` |
| Atari | `SAGA #4 - Voodoo Castle [side A].atr` | `3f7db728180666d1e4d1b8b0a96f75d1ca22cc26d9bc8651546d5d9bc8a5f2bd` |
| Atari | `SAGA #5 - The Count [side A].atr` | `5dc27fa238301de48ddc4d2d70dc1fa8fde94dfeb83647a7e25ba2ae13da87e4` |
| Atari | `SAGA #6 - Strange Odyssey [side A].atr` | `83985e1ac74e67d35d37f1364bd65bb0829f83eb16073fca9f9d1ec2842b0a78` |
| Atari | `SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr` | `d0dcba442113b7ca60b1f83f085fe5e5a5f65b15a8deaeafa4e8fec9354ceb96` |
| Apple II | Adventureland side B - boot | `53064cac8ddcb0bb61638d1fffce292fe6ccb175c24c62570fe99c0012531f32` |
| Apple II | Pirate Adventure side B - boot | `0a01e8bb97e3b8463ff6ac2283d8589bd42ca2e0b08dd53a8bbc5c853b8bdf03` |
| Apple II | Mission Impossible side B (boot) | `7734791024687997fa3025c7e52a71a56dd70dd62d7d4601d61c51759ba03a1b` |
| Apple II | Voodoo Castle side B (boot) | `c34dba003a7f5f8e3e32ee682dc96dc1e611a9c9a2236920070fe59882ebf84e` |
| Apple II | The Count side B - boot | `ba3d66fbcce65ab753b886fad52ce3d4ef54ce829c6bd3667bf10a5fd1529d2a` |
| Apple II | Strange Odyssey side B - boot | `6039602205a6dd03f8b0aa15e5da246d19dcccb3d3495371641c8ed8d156a1fc` |
| Apple II | Claymorgue Castle side B (boot) | `5ca192b45d2fdc1e883e1db29961919cc8b3437673a463f816f8072e8fc65910` |
| Commodore 64 | `QUESTPR1.D64` | `5035c0ae93ebfd144e8c473c29ed24c30e07f9e5e5457730e573bbb28d95a4e4` |

**The oracle, and it is a good one.** Every title here has a reference-format
twin in the §10.1 archive — `adv01` through `adv06` for the six S.A.G.A.
numbers, `adv13` for *Claymorgue*, and `quest1` for the *Hulk*. Decode the
binary database and compare table for table against the twin; nothing from any
implementation is needed, and nothing from this document either once the
comparison is running. What such a comparison should be expected to produce, as
measured here:

- **the eleven header numbers match exactly** on all fifteen databases;
- **item start locations match exactly** on fourteen of the fifteen (all but the
  damaged specimen below);
- **room connections match exactly** on fourteen (all but the damaged specimen
  below), including every one of Adventureland's 34 rooms and Claymorgue's 33;
- **the dictionary matches** as §12.5 describes;
- **room, message and item texts match** after mapping the conversion's backtick
  to `"`, except where the graphic release genuinely differs — the Atari *Count*
  is the widest such gap, 3 rooms, 8 messages and 7 items away from `adv05.dat`,
  where the Apple II *Count* is identical to it;
- **actions match to within a handful of records per title**, as §12.8 sets out;
  *Voodoo Castle* and *The Count* are exactly identical on both platforms and are
  the two cases to bring up first — a column-major reader that gets either of
  them wrong is wrong about the format, not about the release.

**Release by release, in numbers.** Each cell below is a count of records that
disagree with the twin; **0** means byte for byte identical. Two normalisations
are applied and no others: the conversion's backtick maps to `"` (§12.6), and a
zero-length record and the conversion's `.` placeholder are the same thing, since
some conversions spell that placeholder as an empty string instead. The eleven
header numbers match exactly on every row.

| release | verbs | nouns | room texts | exits | messages | item texts | item locations | actions |
|---|---|---|---|---|---|---|---|---|
| #1 Adventureland, Atari | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 4 of 170 |
| #1 Adventureland, Apple II | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 4 of 170 |
| #2 Pirate Adventure, Atari | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 4 of 178 |
| #2 Pirate Adventure, Apple II | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 4 of 178 |
| #3 Mission Impossible, Apple II | 0 | 0 | 0 | 0 | 2 | 0 | 0 | 7 of 162 |
| #4 Voodoo Castle, Atari | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **0** of 190 |
| #4 Voodoo Castle, Apple II | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **0** of 190 |
| #5 The Count, Atari | 0 | 0 | 3 | 0 | 8 | 7 | 0 | **0** of 220 |
| #5 The Count, Apple II | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **0** of 220 |
| #6 Strange Odyssey, Atari | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 of 224 |
| #6 Strange Odyssey, Apple II | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 of 224 |
| #13 Claymorgue, Atari (v125) | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 2 of 268 |
| #13 Claymorgue, Apple II (v122) | 19 | 2 | 0 | 0 | 5 | 0 | 0 | 15 of 268 |
| Questprobe 1: The Hulk, C64 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 10 of 262 |

The fifteenth database, the Atari *Mission Impossible*, is absent from that table
on purpose: it is damaged, and the answer for it is a refusal rather than a row
(below). Four things are worth reading off the rest. **Three of the fourteen —
both *Voodoo Castle* releases and the Apple II *Count* — disagree with their twin
in no table at all**, which is a stronger statement than any individual field's
and makes them the first cases to reach for when a reader is suspect. **Both
*Count* releases are exactly identical in the
action table while the Atari one is the corpus's widest text gap** — 3 rooms, 8
messages and 7 items — so text agreement and table agreement are independent and
a reader must not infer one from the other. **Exits and item start locations
agree everywhere**, including all 34 of Adventureland's rooms and all 33 of
Claymorgue's; a single disagreement in either column means the parse is wrong,
not the release. And **the only dictionary disagreement in the corpus is the
Apple II *Claymorgue*'s**, which §12.5 accounts for entry by entry.

Pin these numbers rather than a floor. A test asserting "at most *n*
disagreements" passes a reader that has quietly stopped agreeing anywhere it used
to, and it also fails an honest release the day a better specimen of it turns up;
an exact count says which of the two happened.

**One specimen is damaged and cannot serve as an oracle.** The Atari
*Mission Impossible* side A carries about fifty corrupt bytes in the middle of
its room-description block, beginning at file offset 0x912: the text of room 16,
`*I'm on a ledge outside of a window…`, is cut off after `window` and overwritten
with a fragment containing the ASCII string `D:SAGA3.DAT`. Its header and
dictionary decode correctly and match `adv03.dat`; everything after the sixteenth
room string does not. The disk holds a second, undamaged copy of the same
database, reachable by stripping the three-byte link trailer from the end of
every 128-byte sector and then scanning for a §12.4 header — that copy's room
connections match `adv03.dat` exactly. Use the **Apple II** `A3.DAT`, which is
clean, or that second copy; do not conclude from the flat-slice reading that this
document is wrong about the format. This is what §7.3 means when it says the
0x04C1 constant is a property of how these disks were mastered and an implementer
wanting generality must walk the filesystem.

**The rule for that side is: refuse it as damaged, do not decode it.** This is
the one case in the corpus where every cheap acceptance test passes and the
result is nonsense, so it is worth naming what does catch it. The version and
adventure numbers scan correctly; the header at array offset 0x38 satisfies
§12.4's limits and matches `adv03.dat` in all eleven fields; the dictionary is
found at 0x108 and decodes to `adv03.dat`'s verbs and nouns without a single
disagreement. The room block is reached at array offset 0x30F, and the read stays
in step until room 16 at array 0x42F (file 0x8F0), whose length byte promises
more text than the record holds. **Two of §12.14's checks then fail, either of
them on its own sufficient**: the first room-description pointer minus the room
block's offset gives a base of `$82E5`, where every undamaged Atari release in
§12.12 gives `$3031`, so §12.7's pointer tables do not resolve onto the string
starts the sequential read produced; and the two copies of the item-location
table (regions 9 and 11 of §12.1), byte-identical on all fourteen sound
databases, disagree here. A reader that ignores both and carries on decodes 8
wrong room texts, 82 wrong messages, 53 wrong item texts, 42 wrong item
locations, 24 wrong exit rows and all 162 action records — a self-consistent
parse of nothing, and one that would be reported as a bug in this document
rather than as a bad disk. Refuse, and say the file is damaged rather than that
it is not this format.

### 12.14 Refusals

- A first-0x38-byte scan yielding an adventure number of **zero**, or a header at
  offset 0x38 failing §12.4's limits, means "not this format" — hand the file to
  the next detector rather than reporting corruption.
- A forward scan from header + 29 that reaches the end of the array without
  finding the bytes `ANY` is the same answer. Do not fall back to §4.1's
  signatures: they match the **verb** block here, 645 bytes late on the one
  release measured, and reading from there produces a self-consistent parse of
  nothing.
- A sequential read whose regions do not meet — a string table that overruns the
  next table's start, or an action table that does not end exactly where the
  room-description pointers begin — indicates a wrong array start, not a
  recoverable file. §12.7's three pointer tables are the cheap check: if they do
  not resolve onto the string starts already read, stop.
- **A file that passes detection, the header limits and the dictionary and then
  fails one of those structural checks is DAMAGED, not "not this format", and
  must be refused as damaged.** The two signals to act on are §12.7's pointer
  tables failing to resolve, and the two copies of the item-location table (§12.1
  regions 9 and 11) disagreeing — identical on every sound specimen. The Atari
  8-bit *Mission Impossible* side A is the corpus's one instance and §12.13 works
  it through: its header and dictionary are perfect and everything after the
  sixteenth room string is rubble. Do not decode it, do not pass it to the next
  detector, and do not report it as an unknown format; tell the player the file
  is damaged and name the clean alternative, which for this title is the Apple II
  `A3.DAT`.
- **Questprobe 3, *Fantastic Four*, remains unidentified.** Neither its Commodore
  64 release (`QUESTPR3.D64`, §10.7) nor its MS-DOS one (`SPL53P.DAT`, §10.7) is
  this format: no §4.1 signature occurs in either, the first 0x38 bytes of
  `SPL53P.DAT` are a quoted title and a comma-separated number list rather than a
  binary header, and the Commodore 64 image carries none of §7.2's fixed database
  names. **An existing implementation does not name it either** — its release
  catalogue runs to Questprobe 1 (*The Hulk*) and Questprobe 2 (*Spider-Man*) and
  has no entry for Questprobe 3 on any platform. Its pictures are family E and
  family C respectively and decode normally (§10.7); its database encoding is
  described nowhere this document has looked. Refuse it by name.
- The Atari 8-bit *Hulk* executable `The Hulk.xex` (§10.5) is a **memory image**
  of §4's family in a container this document does not specify, not a database of
  this section's. §7.3's refusal covers it.

---

## Appendix A — How lanthorn uses this document

lanthorn's `scott` crate reads the reference text format, **§3's TI-99/4A
tokenised releases** (`crates/scott/src/ti994a.rs`, SQ-1414), the **eleven
Commodore 64 *Mysterious Adventures* releases** (`crates/scott/src/c64.rs`,
SQ-1455) and **§12's US S.A.G.A. binary database** — all fifteen Atari 8-bit,
Apple II and Commodore 64 databases (`crates/scott/src/saga_us.rs`, SQ-1414 /
SQ-1464) — and refuses the remaining dialects by name rather than loading them: a
file that fails the text parse is checked against the TI-99/4A signature, the
`aUTOgO\0` compressed signature and the three plain dictionary signatures, so a
player is told "this is a Commodore 64 memory snapshot" rather than "invalid
data".

**Implemented from this document:** §1, §2, §3 (all of it — §3.1 detection and
the baseline, §3.2 endianness, §3.3 the header, §3.4 the two table shapes, §3.5
strings and the derived message count, §3.6 the two dictionaries, §3.7 the
action encoding), the Commodore 64 slice of §4 (§4.2's cell reading, §4.3's
locating, §4.4's encodings), §5.3's repairs for that family, §6, §7.2's
uncrunched disk-image path, §8.2's Family B pictures, §9.1 and §9.2 as they
apply, §11's TI-99/4A refusals, and **§12 in full** (§12.1-§12.9 as the format,
including §12.8's command codes 89 and 90 and their operand counts, §12.11's
database-borne runtime facts, §12.14's refusals; §12.10 says the database carries
no picture data, so there is none to read). **Not
implemented:** the rest of §4-§8, §9.3, and §11 apart from the above. The
S.A.G.A. **container** step is the host's — `scott` reads no disk images — so
§12.3's three array offsets live in the crate and the mount does not.

The TI-99/4A implementer raised three questions about §3 while building that
loader, each found by measuring the §10.2 specimens against the §10.1 oracle.
All three are now resolved in the normative sections; recorded here is what
changed and, where the answer went the other way, what the code has to change.

1. **"Carried" — the specification was wrong, the measurement was right.**
   §3.4 used to claim this dialect had no in-band value meaning "carried". It
   has one, and it is the reference format's own: **255**, on exactly the items
   whose `.dat` twins give −1 or 255, in five of the twelve specimens. §3.4 now
   says so, adds that 0 and 255 are the only reserved values, and spells out
   that opcodes 200 and 201 compare against the 255 literally. Reading 255 as
   `CARRIED` at load time is correct.
2. **Pointer validation versus table extent — the specification was
   incomplete.** All eleven header pointers resolve within the file in all
   twelve specimens, yet `adv07.fiad` ends two bytes before the last entry of
   its own dispatch table. §3.1 now states the general rule — a validated
   pointer proves a table's start, never its extent, and any 16-bit value whose
   two bytes do not both lie within the file reads as 0 — and works `adv07`
   through. Reading a dispatch entry that does not fit as "this verb has no
   records" is correct.
3. **The link-0 record — the specification was misleading, and the code is
   wrong.** Both statements the implementer weighed were right as far as they
   went: §9.1's built-ins really are only go, take and drop, and §3.7's
   chain-final record really is "a real record, eligible to match and to run".
   What was wrong was calling byte 1 a *length*. It is a **link**: it locates
   the next record, and 0 means only that there is none. A link-0 record's
   opcode stream is not empty — it begins at byte 2 like every other record's
   and ends at its own opcode 255. Adventureland's `INVENTORY` chain is
   `00 00 E9 FF`: list the inventory, succeed. §3.7 is rewritten around the
   link, and §3.8 works that record through byte by byte alongside the same
   game's `QUIT`, `STOP`, `SCORE` and `SAVE GAME`.

   **What this means for the crate:** `read_chain` must stop giving a link-0
   record an empty `ops`. It has to recover that record's stream by walking the
   opcode arities to the 255 that ends it (bounded by the end of the file), the
   way §3.7 describes, and the doc comments on `Ti99Record` and `read_chain`
   that repeat the old "empty `ops`, so it always fails" reading must go with
   it. Until then this dialect answers "I can't do that yet." to inventory,
   quit, stop, score and save in every game that spells them this way.

   **Done (SQ-1414).** `read_chain` recovers a link-0 record's stream with a
   new `walk_ti99_ops`, sharing `Vm`'s own command-arity table
   (`crate::vm::ti99_command_operands`, made `pub(crate)` for this) as the one
   source of truth, so the loader and the interpreter cannot disagree about
   where a record ends. The stale doc comments are gone. Verified against
   `adv01.fiad`: `INVENTORY` answers `I am carrying : Nothing. ` and `SCORE`
   answers the real score line, neither "I can't do that yet."; the twelve
   §10.2 specimens reproduce this section's own measured 1,870 explicit / 378
   automatic / 448 link-0 records, every one walking to its own 255.

**And the Commodore 64 implementer raised three more, about §4.2, §5.3 and
§8.2.** All three were reconciled on 2026-09-09 against the §10.4 specimens and,
where the question was about the reference's behaviour, against the source named
in the Sources table. All three went the implementer's way; the normative
sections now say so, and no code has to change.

4. **§4.2's dictionary reader was wrong for this family — the measurement was
   right.** The escape-driven reading (word length characters per cell, with a
   `*` restarting the count and a space rewinding it) loses alignment on two
   shapes these releases contain: a cell whose word fills all (word length + 1)
   bytes, and two adjacent all-space cells. §4.2 is rewritten around the plain
   (word length + 1)-byte NUL-padded grid, with a `*` occupying one of the
   cell's own bytes and a single leading-NUL alignment escape whose scope is now
   stated — safe on the Commodore 64 releases, where no dictionary contains an
   all-NUL cell, and unsafe on the ZX Spectrum ones, where they are common. The
   grid reproduces every verb and noun cell of the six titles whose tables are
   byte-identical to their conversions; the escape reading reproduces four.
   *The Golden Baton*'s final cell reads `CAST` through the one escape that
   fires in the whole corpus, and the space rewind is not a rule of any dialect
   here — it never fires on any of the 37 dictionaries measured.
5. **§5.3's Mysterious Commodore 64 direction repair is unnecessary and
   harmful.** `ANY` and the six direction words are already noun cells 0-6 in
   all eleven, read straight out of the grid, and applying the repair truncates
   *The Time Machine*'s and *Arrow of Death* part 2's stored `NORTH` and
   `SOUTH`. §5.3 records what the repair is for — a reader that has lost cell
   alignment before it reaches noun cell 0 — names the family the reference
   applies it to (records marked both *Mysterious* and *Commodore 64*, which is
   this eleven and no others), and drops it. *The Golden Baton*'s `CAST`
   substitution goes with it; the word-count reduction stays.
6. **§8.2's fill-queue bound is not a behavioural fork at this canvas size.**
   Across 516 pictures and 2,227 fills in the eleven releases, the queue's
   greatest depth is 677, so the 1024-point bound is never reached and the
   bounded and unbounded decoders produce identical canvases; an unobstructed
   fill of the empty canvas paints all 23,970 pixels either way. §8.2 keeps the
   bound as the reference's behaviour and replaces "a real behavioural fork, and
   an implementer must choose deliberately" with the measurement.
7. **§8.2 is a display list, and drawing it larger is not a matter of drawing
   the same primitives on a bigger canvas.** This section describes a *drawing*
   over a 255 x 94 canvas, not a bitmap, so an implementer may reasonably want
   it at the resolution the display has. Re-running the lines and the fills at
   an integer supersample does not give that: with the lines drawn where the
   geometry actually puts them rather than where 1x rounding put them, a fill
   sealed at 1x by two lines a pixel apart, or by two the rounding placed in
   one row, finds a half-pixel seam and floods past it — measured over the
   eleven Commodore 64 releases, **25 of 516 images leaked at scale 2, 3 or 4,
   the worst repainting 6,118 of 23,970 native pixels**. Nor is a wider pen a
   fix: the 1x pixel a shallow line lands in is up to a whole pixel from the
   true line, so nothing under a two-pixel stroke covers it, and a two-pixel
   stroke is no longer this artwork's line. An implementer who wants both
   should take the REGIONS from the 255 x 94 raster (which is what the machine
   drew, and what the player saw) and redraw only the LINES at the larger size,
   colouring every device pixel by the native pixel it lies in. lanthorn does
   that; the corpus check that a supersample, majority-voted back to the native
   grid, disagrees nowhere the ink cannot explain is
   `every_picture_keeps_its_regions_at_every_supersample`.

**And the US S.A.G.A. implementer raised four more, about §12.5, §12.8 and
§12.13** — three corrections and one omission, every one of them found by
measuring the §12.13 specimens against the §10.1 twins. All four were reconciled
on 2026-09-09 on the specimens; the reference was consulted for one thing only,
the identity of the two command codes, and where it and the specimens disagree
the specimens won. Three went the implementer's way and cost no code change; the
fourth is a code change.

8. **§12.5's leading-NUL escape must fire at most once per cell — the
   specification was silent, and the measurement was right.** A run of pad bytes
   where a cell should begin is an **empty dictionary entry**, not slack: let the
   escape repeat and the run is swallowed, the next word takes that slot, and
   every entry after it is one slot early. §12.5 now states the rule normatively
   and works *Pirate Adventure*'s verb block through — cell 58 is `00 00 00 00`,
   which is the escape plus three NUL characters, and `adv02.dat` agrees that
   verb 58 is empty. A repeating escape loses that release's fifteen empty verb
   cells one by one, runs 46 bytes past the dictionary's true end, and reads room
   1's description as verb 65. Byte-identical on the Atari and the Apple II.
9. **§12.5's Apple II *Claymorgue* gap is nineteen verbs, not six.** Re-measured
   against `adv13.dat`: nineteen verbs and two nouns. §12.5 now breaks the
   nineteen down — five mid-table synonyms v122 spells `.`, one substitution
   (verb 51, `LOWER` for `*INVOK`), and thirteen entries it leaves empty, of
   which eleven are the trailing `LIGHT` run at 96 to 106 — and adds the fact
   that makes it a release difference rather than a drift: both dictionaries
   occupy 0x108 to 0x62F, 110 noun cells and 110 verb cells, and the cell after
   the divergence starts in the same place on both.
10. **§12 never mentioned command code 90, and both its arity and 89's were
    wrong for these releases — the specification was incomplete, and the code
    IS wrong.** Command 90 occurs in five of the eight titles and in none of
    their conversions; 89 occurs in the same tables, so 90 is not a
    renumbering. §12.8
    now carries both: the occurrence counts, the identity of 90 (draw the
    room-usage picture its operand names over the whole graphics window, wait for
    ENTER; every operand measured names a picture file that is present on that
    title's picture side and is never a room's own number), the fact that 89's
    drawing effect is undetermined, and — the load-bearing part — **89 takes no
    operand and 90 takes one**, which is the only assignment of the four that
    reproduces the twins' operand assignment in all thirty-eight records that
    carry either code. §12.11 states the obligation for an interpreter that draws
    nothing, and §12.8 hand-decodes the Commodore 64 *Hulk*'s action 101, whose
    sixteen bytes differ from `quest1.dat`'s line in one.

    **What this means for the crate:** the two arities are currently the wrong
    way round — command 89 consumes an operand and command 90 consumes none,
    which is the reference format's own numbering and is right for the dialects
    that numbering came from, but not for §12's. On the five §12 databases that
    use command 89 — both *Adventureland* releases, both *Pirate Adventure*
    releases and the Apple II *Mission Impossible* — it misroutes every operand
    after an 89 in the same record: *Adventureland*'s
    `RUB LAMP` drops the golden fish already lying in room 10 instead of
    conjuring the diamond ring, and clears the darkness flag instead of setting
    flag 8; *Pirate Adventure*'s `SET SAIL` sets the wrong flag and leaves the
    next command an operand short. The fix is per dialect, not global — 89 keeps
    its operand everywhere else, including the one reference-format file that
    uses it — so the arity has to travel with the database the way §9's runtime
    differences do. Command 90 is a no-op for a host that draws no pictures, but
    it must still consume its operand.

    **Implemented (SQ-1472).** `vm.rs`'s `Vm::run_commands` now branches on
    `Database::saga_us.is_some()`: 89 consumes an operand and sets
    `pending_picture` only for the reference format, and for a S.A.G.A.
    database it takes none and leaves every field untouched (its drawing
    effect stays undetermined); 90 consumes one operand and sets
    `pending_picture` only for a S.A.G.A. database, through the same
    `pending_picture`/`current_picture()` door opcode 89 already used —
    waiting for ENTER is left to the host, the way opcode 88's pause always
    has been. `decompile.rs`'s mnemonic table was fixed from the same
    dialect check so a debug listing shows the right operand count too.
    Verified against the real Atari/Apple II Adventureland and Pirate
    Adventure databases: `RUB LAMP` (action 107) now drops item 48 (`*DIAMOND
    RING*`) in the player's room and sets flag 8, and `SET SAIL` (action 104)
    sets flag 4 and moves the pirate ship (item 37) to room 21 — both
    matching `adv01.dat`/`adv02.dat` exactly, and both wrong (the golden
    fish moved, flag 0 cleared; flag 37 set) with the arity reverted.
11. **The oracle deserved to be written down, and now is.** §12.13 gained the
    per-release, per-table count of disagreements with the twin — fourteen rows
    by eight columns, exact counts rather than a floor — together with the two
    normalisations it assumes and the four things worth reading off it. §12.13
    and §12.14 also gained the **damaged-side rule**: the Atari *Mission
    Impossible* side A passes detection, the header limits and the dictionary
    and is rubble after its sixteenth room string, so it must be **refused as
    damaged** rather than decoded or passed on as an unknown format. The two
    signals that catch it are §12.7's pointer tables failing to resolve (its
    first room pointer gives a base of `$82E5` where every sound Atari release
    gives `$3031`) and the two copies of the item-location table disagreeing.

The in-memory model this document's dialects decode *to*, in that crate, is a
database of rooms (six exits and a description, plus a flag for the leading-`*`
literal convention), items (text, a treasure flag, an optional auto-get noun and
a start location where −1 rather than 255 means carried), a flat verb and noun
vocabulary with the `*` synonym convention preserved, a message pool, and an
action table of (verb, noun, five conditions, four commands) — that is, the
reference format's shape. **Every dialect here decodes to that shape**, which is
why §2 is written as the common target and why every later section states its
differences against it.

Two consequences for anyone working on that crate. First, a dialect loader
belongs behind the same entry point as the text parser and must produce the same
model, not a parallel one. Second, the runtime differences of §9 — the two
message sets, the two lamp behaviours, and the TI-99/4A dialect's interleaved
condition evaluation — are properties of the *database*, not of the host, and
have to travel with it.
