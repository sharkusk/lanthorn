# Architecture

[← back to README](../../README.md)

lanthorn is a Rust workspace. Two ideas shape it: the **interpreter and the
mapper are decoupled** (a VM reports *where you are*; the mapper turns the stream
of locations and movements into a spatial graph, knowing nothing about the
engine), and **three different story formats render through one neutral screen
model** so a single renderer draws them all.

## Crates

| Crate | Responsibility |
|-------|----------------|
| `zvm` | A from-scratch Z-machine virtual machine — executes story files, standard Quetzal save/restore. Zero-dependency. It also owns the **machine table** (`interpreter.rs`, SQ-0872): one row per ZMSD §11.1.3 interpreter number, carrying what that machine IS — the byte it writes into `$1E`, the default page and ink it reports in `$2C`/`$2D`, the palette its colour numbers resolve through, and the §8.3 screen rules the standard gives it by name — each value sourced out of Infocom's own interpreter for the machine and quoted at its constant. The table is keyed by the **number**, not by an enum, for the same reason `blorb::medium` answers one: a number is a compact published encoding that needs no shared type, which is what lets three crates kept deliberately independent talk about the same machine. It landed here because zvm had been carrying a per-machine rule for one machine since SQ-0740 (the Amiga's global colour pens) as a special case beside a lone constant; when the Macintosh arrived it landed in `app::session` instead, so one concept sat in two crates and `zvm-cli` could see only half of it — it set the interpreter number and never the colours, telling a story off a release press which machine it was on and leaving it to infer what that machine looked like from the generic §8.3.2 seed. Both front-ends now read this table, so they cannot present different machines off the same disk, and `machine()` answering `None` for an unmodelled number is what lets a front-end *say* it does not model one instead of quietly substituting an IBM PC. What stays in `app::interpreter` stays for charter reasons: reading a disk to work out which machine pressed it is I/O policy, the art-flavour preference needs `blorb`, and a standard window is a Version 6 picture space stated by an archive. And since SQ-1118, `objects::ParseNames` reads what an object can be **called** rather than what it is printed as. Both compiler families keep the words as an array of dictionary addresses in a property, and disagree about which: Inform hard-codes `name` at 1 (`Inform6/src/objects.c`, and Inform 7 keeps it), while ZIL numbers `SYNONYM` per game and the number really moves — 14 in Seastalker, 17 in Zork II, 18 in Zork I, 31 in Spellbreaker, 63 in Nord and Bert, with **nothing in the image naming it**. So Infocom's is detected: tally which properties are word arrays over the whole object table, then take the one whose objects CONTAIN every other candidate's. Containment and not size, because from V4 the adjectives are word arrays too and lead by too little to separate (Zork Zero 432 to 306) while an object cannot have adjectives without nouns — and because the V6 games' dictionary flags mark almost nothing a noun (24 of Zork Zero's 1624 words), so no part-of-speech filter reaches them. Where neither containment nor a 2x margin settles it, it refuses: Journey and Scopa have no parser, and `advent.z8` tokenises against its own word table with the Z-machine dictionary declaring zero entries. SQ-1120 then took the runner-up containment discards and made it the answer to a second question — an object's **adjectives**, which Infocom keeps in a property of their own and which a player types (`take brass lantern`, `examine baby prams`, both confirmed under `zvm-cli`). It ships for **V4 and up only**, where that property holds dictionary addresses: all fifteen V4–V6 titles in `stories/` agree, Zork Zero p51/p46 and Shogun p45/p32 among them. A V1–3 story keeps its adjectives as one-byte *numbers* the property scan cannot see at all, so its runner-up is noise — one to four objects, against leaders of 136 to 246 — and the version gate refuses it rather than answering `win` for Zork I's kitchen window. **What makes shipping half of this safe is that the half is stated in the type**: `Adjectives::Unavailable` means the story cannot be asked and `Adjectives::Read { words: [] }` means this object has none, so a word list never quietly means two things on two story versions. |
| `gvm` | A Glulx virtual machine (Glk I/O) for modern Inform 7 games — accelerated Inform veneer, full float opcodes. Zero-dependency. Since SQ-1102 it also reads Inform's **grammar tables** (`grammar.rs`), the counterpart of `zvm::grammar` and the same questions asked of the modern half of the corpus: which words are verbs, what sentence shapes each verb accepts, which prepositions it expects, what parts of speech the dictionary marks. The formats are near-identical — Plotkin describes the lines as "nearly identical to the grammar version 2 format in Z-machine Inform" — but where the Z-machine names the table's address in its header, **a Glulx image records it nowhere**, by design rather than oversight: `glulxdump`, written by the man who designed both, requires the address on its command line and its header comment asks for a layout field that was never added. So the module derives it, by a chain that has to close exactly — the dictionary (a run of `$60`-tagged records at a constant stride whose length matches the count before it), then the actions table ending precisely where the dictionary begins, then the grammar table ending precisely where the actions table begins, with every verb, line and token walked to prove it lands on that byte and no other. The last step earns its keep: **889 byte offsets across the 22-story corpus satisfy the pointer-array precondition — 279 in one game — and exactly 22 survive the walk.** Verified against `glulxdump` on all 22, 6,911 grammar lines, zero differences; the reference tool cannot find these tables but can read them once handed what this module derives. Locating a table is not the same as reading the numbers in it, and SQ-1114 is the other half: a dictionary record holds its verb's index *inverted*, and Inform in Glulx mode counted down from the Z-machine's `$FF` until v6.32 widened it to `$FFFF` (`Inform6/verbs.c`), so the four pre-6.32 stories in the corpus — `advent.blb` among them — read as a complete grammar table with **not one verb word attached to it**, which is a thing no story has. The base is now decided per file by checking both against the grammar table's own verb count, since which one a story uses is a fact about the compiler that built it and not about the format. SQ-1118 added `objects.rs` on the same footing and for the same reason — a Glulx image records the OBJECT tree's address nowhere either — deriving it from Plotkin's §2 structure: a `$70`-tagged linked list whose every next-link names the object exactly one stride along, ending at a `0`, with `NUM_ATTR_BYTES` recovered from the stride that makes the walk close. A clean walk is not proof on its own (the list from object *k* is also a list, so the head is the lowest address that closes), and the verification is the objects' own `name` arrays: every entry of every array must land on a `$60` dictionary record of the table `locate` already found. Confirmed against the parser — `advent.blb`'s lantern answers `lamp`, `headlamp`, `headlight`, `lantern`, `light`, `shiny` and `brass`, and `gvm-cli` takes and drops it by all of them. Inform 7 objects have no hardware short name at all, so there the word list is the only text in the image that identifies the object. |
| `scott` | A Scott Adams (ScottFree `.dat`) virtual machine for the classic text adventures. Zero-dependency. Since SQ-1118 `Database::item_words` answers the same question the other two engines answer — what an item IS and what it can be CALLED — from the only material the format has: the `/NOUN/` marker in an item's description and the `*`-prefixed synonyms following that noun in the table. Adventureland's empty bottle answers `bot` and `con`, so a player may type either "bottle" or "container", which nothing in the printed name could have told you. |
| `mapper` | A VM-agnostic map model: rooms, connections, layered 2-D layout, overlap removal, edge routing. Serializable. |
| `app` | The `lanthorn` TUI binary (ratatui + crossterm): play loop, live map rendering, debug inspector, all interactive features. `assets.rs` is its counterpart to `blorb::medium`: **one enumeration of every place a story's files can live** — the directory beside it, and the volume it was mounted out of — so a caller looking for a game's assets filters that one list instead of learning that disk images exist. `launch_options::discover_art_candidates` is the only filter over it today; before SQ-0843 it was a bare `read_dir`, which is why a Macintosh disk's two picture archives were unpickable while `blorb` had been reading them for a week. A new asset **source** is an arm in `assets::files`, a new asset **kind** is a filter beside that one, and a new disk **format** is still just a row in `blorb::medium::FORMATS`. `disk_set` is a second small enumeration in the same spirit — **which files are volumes of one multi-disk release** — and it answers from filenames alone, never opening a disk, because the question is about how a collection was pressed rather than what is on it. It lived here until `zvm-cli` needed it and now lives in `cli-host` (SQ-0874), re-exported as `app::disk_set` so every call site is unchanged; see that crate's row for why moving it beat copying it. It feeds three callers: `picker::StorySource` (what a launch argument *means* — a directory, the release a named volume belongs to, or, since SQ-0962, the games on a volume that belongs to no release at all: that arm asked `disk_set::members` and gave up on `None`, so a single compilation disc launched whatever story its own tiebreak preferred and the other thirty-two were unreachable, because "is this a volume of a set?" was standing in for "is there a choice to make?". The mount that answers the wider question is asked only after the cheap name-only rule declines, and only of files that really are disk images; the cross-volume IFID fold is now conditional on there BEING other volumes, since a lone hybrid disc carries one build per machine on purpose), `picker::scan_stories` (which folds a set's duplicate builds together by IFID, since the ST shelf carries 39 stories for 33 games), and `disk_set::mount_at`, the one way either front-end opens a named volume. The volume label was weighed as the grouping signal and rejected on measurement, not taste: nine of the corpus's volumes report none at all, and Zork Zero's two DOS presses both label their first disk `ZORK0 1`, so it would leave one family ungrouped and merge the one pair the filename rule correctly separates (SQ-0844). |
| `zvm-cli` / `gvm-cli` / `scott-cli` | Standalone DOS-style command-line players (no map): save/restore, single-key input, terminal-bell bleeps — and, piped, a clean deterministic harness for testing/scripting. `zvm-cli` declines graphical **v6** stories at load: they drive a windowed display it cannot present, and every one of them runs away at its first input prompt. `zvm` itself supports v6 fully — play those in `lanthorn`. `zvm-cli` also opens an original release disk image — **every format `blorb` reads, without naming one of them** (`blorb::medium` mounts it, so this costs no dependency) — and picks between several stories on one disk with a startup menu or `--story <n|name>`; the medium also sets the interpreter number it advertises, exactly as the TUI's does, with `-I` still overriding — see [the interpreter's disk-image notes](interpreter.md#the-command-line-player-takes-a-floppy-too). A v6 disk still gets the v6 refusal rather than a disk error: the mount worked, the renderer is what is missing. |
| `cli-host` | The plumbing those three CLIs share: terminal escapes, the input/EOF rule, an RAII terminal restore, and `--help`/`--version`. Not the renderers — see below. Since SQ-0850 it also owns the one thing the CLIs share with the **TUI**: `storage.rs`, which answers *what do I call this game's save directory* for every host, and the `titles.rs` catalogue the readable half of that name comes from. `app` depends on it for exactly that, because a story taken off a disk image is keyed by its own release and serial and two implementations of that rule would be two directories. SQ-0874 sent a second rule down the same road for the same reason. `disk_set.rs` says **which files are volumes of one multi-disk release**, from filenames alone; it lived in `app`, and `zvm-cli` therefore could not reach it, so the CLI mounted every disk with `MountedDisk::mount` — `mount_set` with no companions — and no multi-volume release opened there at all. *Trinity* played in the TUI and not at the prompt, and the Apple II 5.25" presses answered "no story file on this disk image" off a disk whose game is simply on the next floppy. The choice was to move the rule down or copy it sideways, and a copy is how two front-ends end up with two ideas of what a release is — invisible until a game goes missing from one of them. It moved cleanly because it is pure filename logic over a directory listing, reading its extension census off `blorb::medium` (which `cli-host` already depended on) and opening nothing; `app::disk_set` is now a re-export, so every existing call site still spells it the way it did. `disk_set::mount_at` sits beside it as the one way either front-end opens a named volume, and it keeps the laziness that makes the seam affordable: the companions closure is called only when the named volume has no story of its own, so an ordinary floppy and every compilation disk cost exactly the one read they always did. SQ-0941 gave that lazy arm its other half. `mount_set` reassembles a story the release *pages* across its volumes, which is the Apple II and Commodore case; it can say nothing about a release whose volumes are independent filesystems holding distinct files, because there is no container spanning them — and that is the DOS press, which keeps the story whole on one floppy and the installer and the artwork on the others. So a volume that still has no story asks `members_indexed` for its siblings and takes the game off whichever one has it, mounting them plainly because anything paged was already found. Only when the release carries **exactly one** game, which is `app::assets::volumes`'s threshold and is here for the same reason: widening across *The Lost Treasures of Infocom* would hand whoever opened its launcher disk one of thirty unrelated games, and a shelf is a browser's job. Measured on *Zork Zero* release 393 / serial 890714, whose 360K press puts `INSTALL.EXE` on disk 1, `ZORK0.ZIP` on disk 2 and `ZORK0.EG1` on disk 3 — the disk a player opens first was the one that could not work. SQ-0961 gave the module the other half of its job, **enumeration**: `stories_across_the_release` answers "every story this path can reach", which is what `app::assets::volumes` has answered about artwork since SQ-0874 and what nothing answered about stories at all. Each front-end therefore decided for itself and they drifted — `zvm-cli` on the Amiga *Lost Treasures* disk 1 offered the six games on that platter while the TUI listed all twenty across the six-volume release. The named volume's own list comes through untouched (a compilation's menu does not move because the shelf around it became visible) and the siblings follow in disk order with a build already offered dropped, keyed on the release and serial `storage::disk_story_key` names a save directory with — which is what keeps SQ-0941's widening from listing the DOS *Zork Zero* twice. Three bugs from one seam was enough evidence that reasoning would not hold the line, so `release_enumeration::no_production_code_mounts_the_platter_alone` fails any production call to `MountedDisk::mount` outside this file; unit tests inside `src/` are exempt, because several of them legitimately mount a platter to establish a premise. The same quest widened the *grouping* rule, which was a prerequisite rather than an aside: the Macintosh DiskCopy press names each volume after the games on it (`- Disk 1 - Beyond Zork, Lurking Horror`), so its five stems share a prefix and no suffix whatever, and a prefix-index-suffix rule grouped none of them. The suffix is dropped from the key only when the digit run is introduced by one of `disk`/`disc`/`side`, which is the qualifier that keeps `Ultima 1`, `Ultima 2 - Revenge`, `Ultima 3` from becoming one release. `gvm-cli` and `scott-cli` compile the module and gain nothing and no dependency, which is the price of the shared crate being genuinely shared. |
| `blorb` | Blorb container parsing — bundled story, cover art, and sound/image resources — plus the release-media readers beside it: Infocom's native picture archives, Amiga `.adf` floppies (`adf.rs`), Macintosh DiskCopy 4.2 / HFS disks (`hfs.rs`, which SQ-0870 taught to open a **hybrid CD** as well — not through a new reader but through `cd.rs`, a wrapper in the same shape as the DiskCopy unwrap: it measures a raw disc's sector stride from the distance between sync patterns rather than matching 2352, reads the mode byte to place the user data, and walks the Apple Partition Map to the `Apple_HFS` entry, which on the *Masterpieces* disc starts at block 513 and is the only one of the three the crate reads — the ISO9660 side is SQ-0871. A cooked `.iso` falls out of the same code as an offset with nothing copied, since the absence of a sync pattern *is* the cooked case. The same quest fixed what made the partition unreadable even after extracting it by hand: `volume_is_sane` bounded a volume by the size its MDB claims, and a hybrid disc's Macintosh partition is sized for the medium — 665,589,248 bytes of allocation blocks against 307,992,064 present, every one of them allocated, only free tail missing. The bound is now on the blocks a reader **follows**: the catalogue and extents-overflow extents the MDB names must be inside the image, and per-file truncation is caught where it already was, in `read_fork`, which refuses a fork whose chain runs short rather than handing back a partial one), DOS **and** Atari ST floppies (`fat12.rs` — one FAT12 reader for both, because GEMDOS put its BPB at the DOS offsets; the machine is decided by whether the boot sector opens with an x86 jump, which DOS's load protocol requires and TOS has no use for), and Apple II ProDOS disks (`prodos.rs` — a `2IMG` wrapper whose declared data length reads zero on every image in the corpus, so the block count is the fallback; then seedling/sapling/tree/extended files, sparse blocks and nested directories). ProDOS gained a **third** wrapper in SQ-0864, and it is the only one that moves bytes rather than offsetting them: a 5.25" `.dsk` is the same filesystem with its sectors in the order the drive numbers them, so `dos_order.rs` de-interleaves 35 tracks of 16 sectors back into ProDOS block order and hands the result to the same reader. That module is deliberately nothing else — no format, no row, no verdict; it re-orders and the volume directory decides, so a DOS 3.3 or Pascal 5.25" disk comes through it just as willingly and is then declined. A **fourth** wrapper landed in SQ-0889 and cost no new decoder at all: `Shogun.po` is an 800 KB ProDOS volume behind an 84-byte DiskCopy 4.2 header (`dataSize` 819,200 plus `tagSize` 19,200 plus the header being its 838,484 bytes exactly), so `volume_at` gained the placement and borrowed `hfs.rs`'s unwrap for it — DiskCopy is a wrapper and not a filesystem, so each reader sniffs 84 bytes in and declines what is not its own, and a Macintosh DiskCopy image is unwrapped by the ProDOS reader as willingly as a DOS 3.3 floppy is de-interleaved by it and turned away just as fast. It opens the Apple *Shogun* press (release 311, serial 890510) off one disk instead of five. SQ-0868 gave it a **second traversal of the same table**: read the interleave grid row-wise instead of column-wise and it is DOS 3.3's own *logical* sector order, so `logical_order` sits beside `prodos_order` with one `PHYSICAL_OF` behind both (ProDOS block `b` of a track is DOS logical sectors `b` and `b + 8`, and the ProDOS order is now derived from that relation rather than restated). That second order is what `infocom_boot.rs` needs — the reader for Infocom's **raw self-booting** Apple II floppies, which are the same 143,360 bytes in the same sector order as a 5.25" ProDOS `.dsk` and have no filesystem under them at all: no volume directory in any order, no DOS 3.3 VTOC, just Infocom's loader and a run of Z-code its own RWTS reads off known tracks. It is therefore the one row in `medium.rs` whose bytes are not a volume, and the story is located not by a boot signature (which would fit a corpus of one) but by de-interleaving and taking the first sector boundary whose story **verifies against its own ZMSD §11.1.6 header checksum** — shared with `infocom_packed.rs` rather than copied. That check is decisive about *which* sectors and blind to their order, since it is a byte sum, so the order was settled twice more: only the logical order puts a real Version 3 dictionary where the header points, and only it produces a game that boots. Two rows now claim the `.dsk` spelling and stay disjoint **by construction, not by table order** — the boot-disk sniff declines a ProDOS volume outright — which is what keeps `DiskImage::detect`'s promise that `FORMATS` order is "a formality rather than a precedence" true now that two formats share a size, a sector order and a name. SQ-0869 added the one other row whose bytes are not a volume, `d64.rs` — Infocom's **Commodore 1541** releases. A `.d64` needs no de-interleave (the container stores each track's sectors in ascending order by definition), and Commodore DOS is present on all three disks in the corpus and used by none of them: *Trinity* writes its story over its own directory sector and its BAM reports the whole disk free, while *Hitchhiker's* keeps a directory whose one file is a BASIC loader and stamps its DOS bytes `TG` rather than `2A`. Two things make it unlike `infocom_boot.rs`. First, the presses disagree about the layout — the 1984 disk spends sixteen of each track's twenty-one sectors and skips the loader and directory tracks whole, the 1986 disk spends every sector and skips only the BAM — so the reader carries both plans and keeps whichever one's reassembly verifies, and where a press stops on a disk falls out of the media rather than a table, since a 1541 `FORMAT` leaves each block as `$4B` then 255 x `$01`. Second, and new to the crate, a story can be **larger than a disk**: *Trinity* is Version 4, so its `$1A` field counts in fours and the story is 262,064 bytes against a floppy's 174,848, with side 1 holding 344 sectors and a header and side 2 holding 680 and nothing that identifies it at all. That is why `MountedDisk` now carries the set's raw IMAGES beside the set's files — the Apple's packed volume pages a story across segments that are files, and the Commodore's pages one across sectors that are not, so `story_across_the_set` asks `infocom_packed` and then `d64`, both of which verify against the story's own checksum before answering. The layout was settled the way SQ-0868's correction demands rather than by the order-blind checksum alone: the dictionary at each header's pointer decodes as a textbook one, an FNV-1a fingerprint over every sector in order is pinned, and *Trinity*'s reassembly is byte-identical from `$40` on to `stories/trinity-r12-s860926.z4`, an independent dump of the same build. The three bytes below `$40` that differ are the press declaring a high-memory mark of 22,527 where the reference says 63,423 — a 64 KB machine paging a 256 KB story — which is legal precisely because the checksum starts at `$40`, and which cost `adf.rs`'s `looks_like_story` its assumption that high memory begins at or above static memory. SQ-1095 added the ninth row, `g64.rs`, and it is the first that is not a container at all: a `.g64` holds the raw GCR **bitstream** a 1541's head reads — sync marks, encoded header and data blocks, gaps, and whatever the mastering house did to the parts of the disk that are not data — so every sector it hands on is computed rather than copied. Decode it and it *is* a D64, which is the whole design: the module ends at `sector_image`, hands `d64.rs` a 174,848-byte image, and takes no interest in what a story is. The container layout is Peter Schepers' `G64.TXT` rev 1.9 and the 4-bit-to-5-bit nybble table is VICE's `gcr.c` cross-checked against Linus Åkesson's *GCR decoding on the fly*, with the inverse table computed from the forward one at compile time rather than transcribed. Two things are worth carrying forward. First, **decode what decodes**: Infocom's Commodore protection lives in the loader, which lanthorn never executes, so a track whose bitstream is not sectors is skipped rather than refused — on `plundered_hearts[infocom_1987](r26)(!).g64` that is five whole tracks (36-40) plus one unreadable block, 682 of 683 sectors decoding and the story untouched. Second, the one leniency: both block types end in "off" bytes that exist only to pad the block to a multiple of five, and a drive's write splice lands in exactly those nybbles — six sectors of that press have a corrupt final GCR byte and are otherwise perfect, so an invalid code past the last meaningful byte is passed over and the XOR checksum over what means something is what actually decides. Reverting that concession loses the whole story and no synthetic test notices, which is why the oracle matters: what comes off the bitstream is byte-identical to `stories/plunderedhearts-r26-s870730.z3`, all 128,962 bytes. It also found a **third** Commodore sector layout — the 1987 press spends seventeen sectors a track from track 5, skipping track 17 and starting after the BAM and directory sector on track 18 — so `d64.rs` now carries three plans and still keeps whichever one's reassembly verifies. The row answers Commodore 128 (7) like its `.d64` neighbour, because §11.1.3 asks which machine the interpreter runs on and a container cannot change that. All hand-rolled; the crate takes no dependencies. Beside the filesystems sits `infocom_packed.rs`, which is not one: the Apple II press of *Arthur* and *Journey* stores no story file at all but a **packed volume** — an index in block 0 of the first `.D1` segment, then per-segment runs mapping story pages to blocks scattered across every floppy in the set, so reading is a scatter-gather rather than a file read (SQ-0852). It takes named byte blobs rather than a `Volume` because it is not a filesystem's business; and it assembles and then **verifies the story's own header checksum** before handing anything back, because a wrong page map yields a file just as plausible as a right one. SQ-0864 corrected one thing SQ-0852 recorded about it: the 5.25" pressings of *Shogun* and *Zork Zero* do **not** carry the packed volume bare on a filesystem-less disk — they are ordinary ProDOS volumes in DOS sector order, and what looked like a hand-rolled per-disk block map is a ProDOS index block. Two of Shogun's segments are ProDOS *tree* files, which the hand-rolled reading could not have addressed at all. `medium.rs`'s provided `Volume::stories` asks it on every format, so a story that is not a file is still a story on the list — and `MountedDisk::mount_set` asks it once more across a whole multi-disk release, which is the only way *Shogun* opens at all, since its story is on no single one of its five floppies. That set path is format-neutral and above the table: no reader implements anything for it and none can opt out, the companion volumes are opened through whichever row claimed the one you named, and the closure that supplies them is called **only** when the named volume has no story of its own — so a compilation disk costs exactly the one read it always did. Which files are one release is `cli_host::disk_set`'s answer, from filenames alone; `blorb` is handed bytes and never learns what a directory is. `medium.rs` is the seam on top, and it is the **only place in the workspace that names a disk format**: a `FORMATS` table of one row per format, a `Volume` trait each reader implements by delegation, and a `MountedDisk` every front-end holds. Ask it whether bytes are an image, open them, list the stories on the volume, take the one to play, take the disk's own artwork, name the container for the picker, and get the Z-machine interpreter number the machine implies. Detect and mount walk the same table, so a format lanthorn can recognise is a format it can open — the guarantee that was missing when `zvm-cli` detected an Amiga floppy and refused a Macintosh disk `blorb` had read for a month (SQ-0840). The row also carries the filename extensions a directory scan pre-filters on, which is the newest column and the one that had to be retrofitted: the TUI's story picker kept its own list, never heard about the DOS and ST rows, and left a shelf of mountable `.ima` and `.st` floppies out of the story list for two quests (SQ-0849). Extensions decide nothing — content still does — they only say which files are worth opening. Adding a format is a row here plus the reader it names, and every front-end gains it in the same commit — DOS and the Atari ST landed as **two rows over one reader**, which is what the row/reader split is for, since they are one filesystem and two machines, and ProDOS then landed as one row over one new reader with nothing outside `blorb` touched at all (SQ-0836); the interpreter-number default lives in the same row for the same reason two copies of "an `.adf` means interpreter 4" went stale in one place and not the other (SQ-0839). An explicit number always outranks it. |
| `audio` | Sound playback (rodio) — synthesized bleeps and sampled AIFF / Ogg / ProTracker MOD. |
| `buildinfo` | A tiny zero-dep helper: a `build.rs` that stamps the git commit hash into non-release build versions. |
| `grammar-model` | The **answer** a grammar reader returns, and none of the reading: `Token`, `NounKind`, `RoutineRef`, `Slot`, `SyntaxLine`, `Verb`, `WordRoles`, with Inform's elementary-token numbering and the six token types. Zero-dependency, and depended on by `zvm` and `gvm` alike (SQ-1103). It exists because the two READERS share nothing — a Z-machine grammar table is at a header-named address and a Glulx image records its own nowhere; verb numbers count down from $FF against $FFFF (or against $FF, before Inform 6.32 — SQ-1114); line headers are 2 bytes against 3; tokens 1+2 against 1+4; dictionaries Z-encoded against `$60`-tagged; five table shapes against one — while the two ANSWERS are the same question answered about two story formats. Each engine keeps what is about its FORMAT rather than its answer: `zvm::grammar::GrammarFormat` (which of the five shapes), `gvm::grammar::Tables`/`locate` (where the derived addresses were found), and each crate's own `GrammarError`. SQ-1118 added `ObjectWords` on the same principle: an object's id, its printed name, the dictionary words that refer to it and the length its vocabulary truncates at, as ONE value — a caller holding the words without the name cannot say which thing they belong to, and one holding the name without the words is offering a player something the parser never agreed to accept. All three engines return it. SQ-1120 added `Adjectives` beside it on the same reasoning: only Infocom splits adjectives out of the name array and only from V4 can they be read, so the value distinguishes *unavailable* from *absent* rather than flattening both to an empty list — `ObjectWords::new` still takes five arguments and defaults to `Unavailable`, and `with_adjectives` is the only way to say otherwise. SQ-1108 then lifted the **container** those answers arrive in, which SQ-1103 had deliberately left behind: `Vocabulary` holds the verbs, the spelling index, the prepositions, the dictionary roles and the action routines, and answers the ten questions each engine's `Grammar` used to answer with bodies that matched character for character. It **derives** the spelling index and the preposition list from the verbs rather than being handed them, because both are functions of the verbs alone and both were previously built by the same dozen lines in each reader — a caller that could supply them could supply them inconsistently. Each engine's `Grammar` composes one and delegates to it explicitly, one method for one, rather than exposing it: the public API of `zvm::grammar::Grammar` still reads on its own, which is what `zvm` being embeddable outside lanthorn asks of it, and every call site and both `grammar_tables.rs` suites went through unchanged — which is how the readers' verification survives a restructuring of their insides. The loaders share nothing and were not touched. |
| `verb-synonyms` | The bridge from a word a player typed to one the story knows, when the two are related by **meaning** rather than by spelling. Guess-the-verb's motivating case — `illuminate` → `light` — is unreachable by edit distance (8 on a 10-letter word), by stemming (`illuminat-` reaches nothing) or by grammar shape, because all three operate on FORM and the bridge required is meaning. Ships a generated TSV of synonym groups beside its reader, `include_str!`'d and parsed lazily behind a `OnceLock`: 3,068 groups, 80 KB, greppable and diffable, so a regeneration shows in review as changed lines rather than as one changed blob. A word appears in as many groups as it has senses — `illuminate` is in the *light* group and the *explain* group — and the groups are never merged, because collapsing everything transitively connected joins senses through polysemous words and the table starts confidently suggesting nonsense. The consumer intersects a group with THIS story's dictionary, so nothing is ever offered that the parser would reject: the table proposes, the story disposes (SQ-1110, SQ-1115, SQ-1119). A **second** table sits beside it and answers the other half of the same problem: `irregular_forms.tsv`, WordNet's own exception lists as `form → base`, for the inflections no suffix rule can produce — `lit` → `light`, `took` → `take`, `went` → `go`, `mice` → `mouse`. `app`'s `vocab::stems` strips regular endings and then asks it, always rather than only on a miss, because it is one hash lookup and some words are reached both ways. Nouns are in it as well as verbs, deliberately: `stems` serves every position in a command, so `mice` → `mouse` is the same case as `lit` → `light` one slot to the right. The lookup hands back a SLICE of bases, because a spelling can inflect two ways — `axes` is `ax` and `axis`, `singing` is `sing` and `singe` — and only the story's own dictionary can settle which was meant (SQ-1113). |
| `verb-synonyms-gen` | The generator, shipped with its table because a derived artifact whose inputs are unrecorded cannot be regenerated or audited. Harvests the real IF verb vocabulary from every story it is given (`Grammar::verb_words` on Z-machine and Glulx, `Database::verbs` on Scott), expands it through WordNet **offline**, and inverts the result — so a word outside IF's vocabulary never enters the table and the size stays bounded by the domain rather than by English. `if_groups.tsv` carries the corpus's own verb groupings, which outrank the thesaurus: an author writing `Verb 'examine' 'x' 'inspect'` has stated what a word means IN A PARSER, which is a better authority here than a lexicographer's view of English (SQ-1115). Coverage is measured, not asserted — 90% of the common-verb basis — and a relaxation that scored higher was rejected for putting `fish`, `hook` and `net` in a group with `grab`. Its third subcommand, `irregulars`, is the odd one out: no corpus and no frequency list, because an irregular inflection is a fact about English rather than about interactive fiction — it copies WordNet's `verb.exc` and `noun.exc` out as the shipped `irregular_forms.tsv`, which is the only honest way to hold that data, a hand table being a second copy to reconcile with the first every time either moved (SQ-1113). Licence terms for every input are recorded in `THIRD-PARTY-NOTICES.md`. |

The crates layer `zvm`/`gvm`/`scott` → `mapper` → `app`; the CLIs are thin VM
front-ends. The mapper has **no dependency on any VM**, so layout logic can be
tested in isolation, and the VM crates stay **zero-dependency** (image/audio/
resource types live in `app`, `blorb`, and `audio`). "Zero-dependency" means no
EXTERNAL crates: `zvm` and `gvm` both depend on `grammar-model`, which is itself
dependency-free, in the same way other crates depend on `blorb`. `app` depends on
`verb-synonyms` for the same reason and with the same freedom — it is `app`'s to
use because knowing about English is emphatically not the VM crates' business,
and `verb-synonyms-gen` is a dev tool that may take whatever dependencies it
likes because nothing ships it.

### What `cli-host` does and does not share

The three CLIs share their *plumbing* and keep their *renderers*. The line is
drawn where it is because of what actually went wrong. Five escape helpers were
byte-identical in `zvm-cli` and `gvm-cli`, which was merely untidy — but the same
stdin-EOF bug (a 0-byte read taken for a blank command, so the game is fed a
fabricated newline forever) shipped **three** times: fixed in `zvm-cli`'s char
path long ago, still live in `gvm-cli` until SQ-0604, and still live in
`zvm-cli`'s own *line* path until SQ-0605 found it. Three copies, three chances
to get it wrong, and the terminal was left un-restored on the paths nobody was
thinking about.

So `cli-host` owns: the escape sequences, [`HostMode`] (may we emit escapes? may
we take over line editing?), the EOF-honest readers, `TerminalGuard` (restores on
every exit *including* a panic), and `--help`/`--version`.

It also owns the **save-directory key** — and that one is shared with `app`, which
is otherwise no CLI at all. The reason is the same drift argument one layer up:
the rule now has cases in it (a story mounted out of a disk image keys on its
release and serial, not on the image's filename, because one compilation carries
six games; a story out of a zip keys on its entry's basename, because a zip has
no release to be keyed by and one archive can carry two games just as easily) and
a second copy of a rule with a case in it is a second answer waiting to happen.
`app::storage` re-exports it rather than restating it. SQ-1098 turned the rule's
inputs into one value, `storage::StoryOrigin` — the path, the entry inside it and
the build — because they were two positional arguments and the third was simply
missing, which is the refactoring policy's shape exactly: five call sites
reassembling one decision, and the omission produced a *plausible* key rather
than an error.

It owns the **pin placement** for the same reason, and that one is worth setting
out because the reasoning is not guessable from the code (SQ-0909). Both `zvm-cli`
and `gvm-cli` keep rows fixed while the story scrolls under them — a v3 status
bar, a Glk grid window, BeyondZork's compass, an InvisiClues menu — and both did
it by confining the screen with DECSTBM. The cost was invisible until somebody
looked for it: **a terminal only archives a line that scrolls off the top of the
screen, and it judges that by the scroll region's top margin.** Pin at the top and
the region starts at row 2, so a line leaving row 2 has not left the screen and is
simply dropped. Every line of narrative the player had read was thrown away to
keep one status bar in view.

Measured against Ghostty's core, feeding 30 lines to a 10-row screen:

| region | rows reaching history |
|---|---|
| none | 21 |
| rows 2–10, pinned at the **top** | **0** |
| rows 1–9, pinned at the **bottom** | **22** |

So it is not pinning that costs the history — pinning *at the top* is. Move the
same rows to the bottom, the region starts at row 1 again, and everything
scrolling past is archived exactly as it would be with no region at all. That is
the whole of `--pin bottom` (alias `--scrollback`), and the reason **lanthorn
implements no scrollback of its own**: the history a player wants is the one their
terminal already keeps, complete with its wheel, its selection and its search, and
the only question was whether we were preventing it. The alternative — a ring
buffer, a pager, re-wrapping on resize, SGR replay, and mouse reporting that would
have *disabled* the terminal's own text selection — would have been more code and
a worse result.

The default stays `top`, where Infocom put the status line. An earlier attempt
bought the same history by *unpinning* one-row status bars and letting them flow
into the transcript; it worked, and it was the wrong trade, because it gave up the
thing the player looks at every turn to get the thing they occasionally want.

`cli_host::pin` therefore owns the placement, the region helpers, the `/pin`
parser and the exit teardown; `gvm-cli`'s `enter_region` stays local because it
confines a band between two explicit rows, where the shared helper places N rows
of chrome at one end. The measurement lives with the code it justifies, in that
module's own tests, against `qwertty-term-vt` rather than against our own decoder
— checking a renderer with the decoder that renders it only proves it agrees with
itself.

The teardown is shared for a related reason. Dropping the region without moving
the cursor leaves the shell prompt wherever the game left its `>`, so the next
prompt is drawn *into* the story text — and the paths that got that wrong were the
ones that never reach `main`: Ctrl-C and Ctrl-D in raw mode are keypresses rather
than signals, so nothing else stops the process. Both placements need the same
treatment, because the bottom row is occupied either way: by the last line of
story under a top pin, by the chrome itself under a bottom pin.

`scott-cli` needs none of this and takes none of it. It emits no escape sequences
at all and has no status window to pin, so it always had native scrollback — which
is the same property that gives it the escape-free `TerminalGuard` below.

It owns none of the drawing. `gvm-cli/glk_term.rs` and `zvm-cli/screen.rs` have
essentially no logic in common, and `scott-cli` — which emits no escape sequences
at all — would only pay for machinery it does not need. That last property is
load-bearing rather than incidental, so the guard comes in two flavours and
`scott-cli` takes the one that restores raw mode and emits nothing.

[`HostMode`]: ../../crates/cli-host/src/mode.rs

## Three engines, one renderer — and Glk only for Glulx

All three VMs implement one `Engine` trait whose `screen()` returns an
engine-neutral **`ScreenModel`** (a window tree the app knows how to draw). The
one generic renderer draws every engine from that model. But *how* each engine
arrives at its `ScreenModel` differs, and this is a deliberate design decision:

- **Glulx (`gvm`) uses Glk.** A Glulx game drives Glk display calls (open/close/
  arrange windows, `put_text`, `grid_put`, …). The app's `AppGlk`
  (`app/src/glk_backend.rs`) records those calls and *projects them* onto the
  `ScreenModel`. Glk lives entirely in this **app-layer translator** — `gvm`
  itself just makes the calls; the VM crate carries no terminal or Glk types.
- **Z-machine (`zvm`) is native.** `zvm` has its **own** `ScreenState` + `Output`
  model (v3 status line, v4+ cursor-addressed upper window). The app *mirrors*
  that state into the same `ScreenModel` — no Glk involved.
- **Scott Adams (`scott`) is native.** The `scott` VM has no screen model of its
  own at all; the app builds a `ScreenModel` directly from its output. No Glk.

So **Glk is confined to the Glulx path.** Z-machine and Scott are implemented
against their own I/O models and converge with Glulx only at the neutral
`ScreenModel` layer.

**Which engine gets the file is decided by evidence, and all four of them are
tested now** (SQ-0889). `hints::extract_story` classifies a story image: a Blorb
proves itself by its `FORM`/`IFRS` magic, a Glulx image by `Glul`, a Scott Adams
database by a content sniff of its leading integers — and, until SQ-0889, a
Z-machine story by being none of those. Z-code was the else-branch, so the only
gate a file had left was `zvm::header::parse_header`'s `3..=8` on byte 0, which
about **2.3% of arbitrary containers pass**. One did: an 838 KB Apple II DiskCopy
image whose name-length byte is `0x06` opened as a Version 6 story, paired itself
with a sidecar archive belonging to a different game, printed "story ended
without asking for input" and exited **0** — a message that reads as a game bug
and sends the reader looking somewhere else entirely. Z-code now proves itself
like the other three, by `blorb::adf::looks_like_zcode`: dynamic memory ends
below `$0e`, the writable object and global tables are inside it, the dictionary
is in static memory, the serial is six printable bytes, and the declared file
length does not over-run the bytes present (ZMSD §1.1, §11.1.6). That check is
**borrowed from the disk readers rather than restated** — it is the same one that
decides which file on a mounted volume is the game, and two of its clauses are
corrections that cost a real release its visibility when they were assumed
instead of measured (`SQ-0856`'s high-ASCII serial, `SQ-0869`'s Commodore
*Trinity* whose high-memory mark sits below its static base). A second copy would
be a second place for that to go stale. A container that passes nothing is
refused with its length and the head of its file, which is where a wrapper writes
its name, and the process exits non-zero.

### Why confine Glk to Glulx

- **Spec-faithful.** Glulx's I/O *is defined* in terms of Glk — using Glk there
  matches the standard. The Z-machine and Scott Adams formats are **not** defined
  against Glk; they have their own display models. Implementing each format's I/O
  the way its spec describes keeps every engine honest.
- **No leaky abstraction.** Routing the Z-machine's cursor-addressed upper window
  or Scott's fixed two-window layout *through* Glk's windowing model would be an
  impedance mismatch — format-specific behavior would be distorted or lost. Each
  engine keeps its exact semantics.
- **Unification at the right layer.** Cross-engine render unification is banked at
  the `ScreenModel`, so one renderer serves all three — *without* forcing a single
  I/O library onto formats that don't use it.
- **Smaller, self-contained VMs.** `zvm` and `scott` don't pull in a Glk layer
  they'd never use, so they stay zero-dependency and easy to reason about; Glk
  code lives in exactly one place (`app`'s Glulx backend).

## Graphical v6: a fourth window kind on the same model

Graphical Z-machine **v6** stories (*Zork Zero* and kin) don't fit the plain
window tree — pictures and text share one pixel-addressed screen. Rather than
build a second renderer, v6 gets one more `ScreenModel` node,
`WinNode::Layered`, carrying the game's windows z-ordered background-first:
`session.rs`'s `v6_screen_model` builds it from `zvm`'s native v6 window
state; `render/screen.rs`'s `Layered` arm composites it — per-cell without an
image protocol, or (with one) as one native-pixel-space canvas assembled by
`render/v6_layout.rs`'s classification/geometry helpers and drawn by
`render/graphics.rs::draw_v6_canvas`. Same generic renderer, same neutral
model — v6 is a fourth leaf kind, not a parallel pipeline. See [Graphical
v6](v6-graphics.md) for what that composite looks like from the
player's side.

**A modal forces the cell path**, and that changes which graphics a dialog has
to keep clear of. `dialog_bounds` subtracts every graphics window so a dialog
never lands under an image placement — but the cell path draws no frame art, so
on a v6 composite the only thing still placed is a chrome window entirely
*beside* the story (Journey's picture column). Subtracting the rest put the
dialog in the strip below the frame's own stamp, clipped to eight rows with its
buttons off the pane. `v6_layout::cell_path_side_columns` is now the single
statement of which windows those are, called by the cell path and by
`dialog_bounds` both: they had measured it on two different bases — pane-
proportional cells against the game's native cells — and agreed only near an
80-column pane, which is every pane anybody had tested.

## One transcript wrap, for both ways of drawing it

Both render paths wrap the whole scrollback and then show forty rows of it, and
they used to disagree about when that was necessary — in opposite directions.
The cell path (which hybrid's story text also uses) had a whole-product cache
keyed on a generation counter that moves on *every* transcript mutation, so each
turn threw the wrap away and rebuilt it from line zero; its idle frame sat flat
while its post-turn frame grew to 35 ms at 20,000 turns. The raster path had no
cache at all, behind a whole-canvas gate that hashes the live input line — so one
keystroke re-wrapped the lot.

`render/wrap_cache.rs` is now the single owner of the question. `WrapKey` gathers
every fact that can move a wrap boundary — width, filter, the picker's cell, the
screen-clear anchor, the machine and window pages, the period look, and the pen's
own advance table — in one constructor, so a caller cannot supply a subset and get
a plausible wrong answer. `WrapKey::plan` answers **reuse**, **append**, or
**rebuild**, and both paths obey the same answer: content only ever grows at the
end, so a turn extends the wrapped rows, while a resize or a theme change drops
them. There are two cache structs because the two products are different types —
`WrappedRow`s carrying kinds, styles, runs and image bands against the raster's
glyph rows and emphasis bits — but only one copy of the rule, because two copies
of a measurement rule is precisely what drifted.

Raster is the degenerate case rather than a second design: its columns come from
the native v6 screen rect, i.e. the game's own coordinate space, so they do not
move with the pane and it takes the append branch essentially always. The cell
path wraps to the terminal's columns and takes the rebuild branch on a resize.

Two details are worth knowing before touching it. The wrap carries state across
lines — an open margin float narrows the rows beside it — so an append resumes
from the float the last line left open, and the trailing flush of a picture that
outran its text is *not* final: the next prose line to arrive claims those strips,
so an append truncates back past the flush before extending. And the append/rebuild
choice is stated by each mutator (`TranscriptEdit::Appended` / `Rewrote`) rather
than inferred, with the last consumed line's fingerprint in the key as the guard
that catches a mutator which picked wrong. `cargo run --release -p app --example
scroll_bench` measures all of it.

## Input: a suspend/resume handshake

Input is engine-neutral too. A VM's `step()` returns a request —
`NeedLine` / `NeedChar` / `NeedEvent` — and the host resolves it with
`supply_line` / `supply_char` / `supply_filename`. The values are neutral (no
terminal types cross the boundary), so the same host loop drives every engine and
the CLIs can feed input from a pipe for deterministic testing.

## Asking the game a question it cannot be asked out loud

`app::probe` forks the live session into a **shadow** — a second `Engine` on the
same story, driven from a host snapshot of the live one — runs commands in it,
reads the answer off it and throws it away. `Engine::save_state` /
`restore_state` are engine-neutral and already in the trait, so this works on all
three VMs; the shadow is booted lazily and reused, and the live session is never
stepped, saved or restored (restoring under a running game is the SQ-0587/0588
hazard — the game never learns it happened).

Two things about it are load-bearing and easy to get wrong:

- **How a story says no is discovered, not assumed.** Every family words its
  refusals differently, and a table of English phrases is broken by the next
  game. So `Refusals` is built from what the shadow prints in reply to
  deliberate nonsense, run beside the real question — one control the parser
  cannot have understood, plus a pair of the same command carrying two different
  nouns, believed only when both replies reduce to the same sentence and neither
  changed the world. `ProbeRun::did_something` combines that with `WorldPrint`,
  which is a changed world's proof of success (an unchanged one proves nothing —
  `examine` legitimately changes nothing).
- **The controls belong to the ROOM, not the session.** Zork I answers `light
  rug` with `You don't have that!` in the field and `You don't have the carpet.`
  in the living room. A signature learned once at boot is a signature of the
  wrong room, so controls and question run in the same `run`, off the same
  snapshot.

- **It runs on a worker thread, and a late answer is dropped** (SQ-1124). Only
  the story interpreter belongs on the main thread, so `ShadowProbe::ask` hands
  the worker a snapshot and returns; the event loop collects the answer with
  `poll` and the offer arrives a beat after the game's reply. There is no
  budget and no too-slow latch: a slow story simply answers later, and an answer
  that arrives after the player has typed again is *stale* — it would attach a
  suggestion to a command that never provoked it — so it is discarded. Measured
  on Zork I, the player's turn now pays ~1 ms (a snapshot and a world hash)
  against SQ-1121's whole 12–15 ms run.
- **A shadow boots the way the LIVE game boots.** It reads the live game's own
  per-story store and Glk VFS — read-only, through
  `glulx_session::GameStore::read_only`, so it can never write what it reads.
  Booting with neither is not "isolated" so much as a *different launch*:
  Counterfeit Monkey checks a 52-byte VFS marker and then `@restore`s
  `_Counterfeit_Monkey-startup-data.qzl`, and a shadow given neither re-ran the
  whole initialisation the live session skips (2.4 s against 0.53 s, measured).
  Both halves are needed: the `.qzl` alone is never asked for.

Isolation is explicit rather than assumed: the shadow boots with sound and
graphics off, no Blorb, a read-only store it may never write, and an in-game
`@save`/`@restore` or a Glk filename prompt inside a probe is answered *failed*
so the VM unwinds where it stands. `app::vocab` is the first consumer (SQ-1121,
vetting a suggestion before it is offered) and the return probe below is the
second. SQ-1043's irreversible-move caution is a **reading of the second**, not a
third consumer: see the last bullet there.

### The second consumer: the return probe (SQ-0785)

`app::return_probe` asks the shadow a structurally different question — *am I
back where I started?* — and everything that differs between the two consumers
follows from that.

- **It reads a room number, not prose.** Success is `step.location == origin`,
  the same `snap.number` `session::apply_turn` keys rooms by, so none of the
  `Refusals` machinery above applies. Landing *somewhere* is not landing back: a
  probe that comes out in a third room records the attempt and nothing else — no
  edge, no room, no trace it was seen — because an invented edge is worse than
  the gap it replaced.
- **Its answers are never stale.** SQ-1124 drops an answer whose `turn_epoch`
  has moved, because a vocabulary suggestion is about *this* turn. "South from
  here returns to A" is about the *map*, so it is recorded whenever it lands. A
  new **move** does end the search — the move may itself be the walk back — and
  that is a different rule from staleness.
- **One snapshot serves the whole search.** Attempts go out one at a time so each
  answer is durable (`MapGraph::mark_probed`, one direction per answer, so an
  aborted search resumes rather than restarts), and `ShadowProbe::snapshot` is
  split out of `ask` so the player's thread pays for one host snapshot per search
  instead of one per attempt — 102 ms each on Counterfeit Monkey in a debug
  build, and twelve of those is exactly the main-thread cost SQ-1124 removed.
- **The edge goes in through the mapper's own door.** `Mapper::mint_passage` is
  the extracted body of `observe_inner`'s minting branch, and both a walked
  crossing and `Mapper::record_probed_passage` call it — one path, so the two
  cannot drift in shape, in `?`-stub hygiene, or in placement. What the probe
  path skips is everything about the *player*: the current pointer, `arrived_via`
  and the layer suggestion. `ProbedPassage` carries the three facts as one value
  and deliberately cannot name the outbound passage, which is how reciprocity is
  made unwriteable rather than merely unwritten.
- **Two consumers, one channel, one collector.** `ShadowProbe::poll` takes
  whatever has arrived without knowing who wanted it, so a consumer polling for
  itself would sometimes take the other's answer off the channel and drop it.
  `loop_tick::poll_shadow_answers` collects once and routes by token.

Measured per attempt, worker time, debug build: Zork I **0.7 ms**, Coloratura
**4.3 ms**, Counterfeit Monkey **343 ms**. In play the priority order usually
stops at the first success — Zork I's North of House takes three commands
(2.7 ms), Counterfeit Monkey's Back Alley one (407 ms). `cargo run -p app
--example return_probe_cost` is the instrument.

## Reading back the bytes we actually emit

Every other harness in the repo renders into a ratatui `Buffer` and asserts on
cells — lanthorn's own model of the screen. None of them can see the *stream*,
so a defect that is right in the model and wrong on the glass is invisible to
all of them. `crates/app/tests/pty_stream/` closes that gap: it runs the real
`lanthorn` binary under a pty, plays the part of the terminal, and decodes the
escape bytes that come back.

Five parts, and the split matters for Windows:

| file | what it is |
| --- | --- |
| `tests/pty_stream/driver.rs` | The pty (`posix_openpt` + `libc`, no new dependency), the terminal-query answers, the keystroke script. **Unix only** — a pty is. |
| `tests/pty_stream/decode.rs` | Bytes → named sequences → a screen model: cursor, SGR, kitty APC commands, U+10EEEE placeholder cells. **Portable**, and unit-tested on every platform. |
| `tests/pty_stream/oracle.rs` | The same bytes through a real terminal emulator — see [the placement oracle](#a-second-reader-for-the-same-bytes-the-placement-oracle). **Portable.** |
| `tests/pty_stream/raster.rs` | That resolved screen drawn as a PNG — see [looking at the frame](#looking-at-the-frame-the-rasteriser). **Portable.** |
| `tests/pty_stream/inflate.rs` | Undoes the kitty protocol's `o=z` before the oracle sees it — see below. **Portable.** |
| `tests/pty_stream/mod.rs` | The report — protocol verdict, uploads, placement rects, a background map, and the finding. |

**Compressed uploads have to be undone for the oracle, and only for it.** A
graphics-window upload is transmitted zlib-compressed (`o=z`) whenever the
terminal answered the compression probe — which the pty harness does — and that
is a transport encoding sitting at exactly the level base64 sits at. Our own decoder
never noticed — it counts payload bytes and does not decode pixels — but the
oracle's terminal core deliberately links no codecs at all: its image decoder is
a seam and the byte-stream entry point wires the null one, so a compressed
transmit fails with `EINVAL: decompression failed`, the image is never stored,
and every placement naming it vanishes. `Capture::bytes` therefore stays the wire
stream (`Flush` offsets index it, and the wire size is a measurement worth
having) and `Capture::terminal_bytes()` is what the oracle is handed.

**It verifies the protocol first, and says so out loud.** lanthorn picks its
graphics backend from `Picker::from_query_stdio`, which asks the terminal three
questions before the UI starts and falls back to half-blocks when nobody
answers. A bare pty answers nothing, so a naive harness silently measures the
half-block path and every number it produces is worthless. The driver answers
the kitty capability query, DA1, `CSI 16 t` (the cell size — not cosmetic: v6 art
is scaled by pixel and placed by cell) and the OSC 10/11 colour probes, and the
capture then *proves* kitty from the stream rather than from hope: no APC `_G`
traffic means no kitty, and the test refuses to go on.

**What it can tell apart that nothing else can.** A kitty placement is virtual:
the upload (`a=T,U=1`) says how big the image is and nothing about where it
goes, and the position comes from the placeholder cells printed afterwards. So
"this row is that colour" has two entirely different causes — an image is placed
over it, or a background was painted into the cells — and they are different
bugs with different fixes. The decoder builds a grid, marks which cells carry
placeholders, and the report's background map names each row's runs with
`(image)` on the ones an upload covers. SQ-0747's flank-panel fill was settled
this way in one run: the overrun rows were **painted cells, not a placement
rect**.

Ad hoc:

```sh
cargo build -p app                       # the harness drives the REAL binary
cargo run -p app --example pty_capture -- \
    --story "stories/Journey - The Quest Begins.adf" \
    --size 117x64 --keys "wait:1500,cr,wait:800,cr,wait:800,cr,wait:1200" \
    --out /tmp/journey.stream.txt
```

`--size` is the terminal, not the story pane: at `117x64` with the map hidden
(the default here) the frame border and the help row leave the story pane the
`115x61` a finding is usually quoted at. Exit status 3 means the run did not
negotiate kitty. `cargo run -p app --example pty_capture -- --help` lists the
rest.

From a test: `cargo test -p app --test pty_emitted_stream -- --nocapture`, which
writes its report to `target/pty-capture/`. It asserts that the harness measured
the right backend and could read a placement back, and deliberately does **not**
pin any particular defect's presence — a test that fails when a bug is fixed is
a trap for the next person, so the image-versus-paint reading is printed as a
finding instead. On Windows the whole thing compiles and the decoder's unit
tests run; the pty case is an explicit skip.

Its complement is `/dump-cells` ([Graphical v6](v6-graphics.md)), which
dumps the same screen from the *inside*: that shows what we computed, this shows
what we sent. Disagreement between the two is the interesting case.

## A second reader for the same bytes: the placement oracle

`pty_stream/decode.rs` is *our* reading of the emitted stream — a hand-rolled
decoder that shares whatever assumptions we built it with. When the model
looks right (Layer 1) and the stream also looks right by our own reading
(Layer 2) but the screen is still wrong, the next question is whether our
reading of the stream is itself the bug. `crates/app/tests/pty_stream/oracle.rs`
(SQ-0764) answers that by resolving the same captured bytes through
`qwertty-term-vt`, a dev-dependency that is a pure-Rust port of Ghostty's
terminal core (tracking upstream Ghostty commit `2da015cd6`, including the
297-entry diacritic table matching kitty's published list) — one dependency,
no build script, builds on all three platforms. Reach for it for placement
lifetime, z-order, overlap, stale placements, missing deletes, and anything
turning on the unicode-placeholder continuation rules our decoder doesn't
model.

**It's a port, not Ghostty.** `qwertty-term-vt` tracks Ghostty's algorithm
faithfully enough to answer "does this placement cover these cells" — but a
port can diverge from what a real terminal does in ways nobody's hit yet.
Before writing up a user-visible bug on the oracle's word alone, eyeball it
on a real terminal too.

**The two decoders name images differently.** Ours keys an image by the low
24 bits of the placeholder's foreground colour; the oracle keys it by the
full 32-bit `i=` value (`full = low24 | (high_byte << 24)`). Comparing a
lanthorn-side id against an oracle-side id means masking the oracle's down to
the low 24 bits first, not comparing them raw.

**The two decoders agree on image coverage — now.** They didn't when the
oracle landed: ours attributes a cell to an image by foreground colour alone
and doesn't model the diacritic continuation rule, so it counted 33 runs of
orphaned placeholder cells a real terminal declined to draw. That was
SQ-0772, and it was lanthorn's bug, not the harness's: virtual placements
were emitted as one anchored cell per row plus bare continuations, invisible
to ratatui's damage model, so a later frame could destroy the anchor and
strand the rest. Every placeholder cell now carries its own row, column and
id high byte and lives in the buffer like any other content, and the real
capture asserts agreement on *both* axes. Ours still can't read a high byte
(see above), so a disagreement there remains an id-masking question, not a
coverage one.

**What the oracle reports is a function of the bytes, and that had to be made
true twice.** `resolve_placements` documents itself as returning "placements in
arbitrary order" — it walks a `HashMap` — so anything downstream that reads a
candidate list positionally gets a fresh random permutation on every call.
`resolve_rects` did that in two places (SQ-0982). It took a cell's `source_y`
from whichever candidate ended up LAST in the list, and it read a pin
placement's declared cell grid off whichever placement of that image the map
yielded FIRST. Both are now decided: candidates are sorted by
`(z, image id, cell offset, source rect)` and the one on TOP supplies the cell,
because `OracleCell::source_y` answers "which pixel row lands here" and what
lands is the topmost draw; the declared grid is matched by pin POSITION, so an
image pinned twice at two sizes no longer lends both of its rects an arbitrary
one of the two. The z-then-id key is the same expression `raster.rs` sorts
draws by, for the same reason and off the same protocol sentence (see the
rasteriser section below); same z and same id is undefined upstream, so the
position tail is arbitrary-but-stable. `the_same_bytes_always_resolve_the_same_way`
and `several_placements_on_one_cell_report_the_topmost_source_row` are the
guards. An oracle that answers differently on different runs is worse than one
that is merely wrong, because nobody can tell which answer they got.

**A stronger oracle exists in principle but isn't built.** For literal
Ghostty ground truth (not a port of it), `libghostty-vt` — Ghostty's own C
library — is reachable in theory, but only as a prebuilt artifact. Building
it from source needs zig plus a full ghostty source checkout, which drags in
the entire GUI dependency graph (sentry, imgui, freetype, glslang, …) even
to get the headless VT core, and it doesn't build at all on macOS 26 with the
pinned toolchain. The viable route, not yet set up, is a GitHub Actions
matrix (IPv4-only runners, so no fetch-wall failures) that publishes
`libghostty-vt.a` plus headers and the generated `.pc`, consumed on a dev
machine through the `-sys` crate's `pkg-config` feature — which skips zig
entirely. Full findings live on SQ-0764; don't re-derive this, extend it.

## Looking at the frame: the rasteriser

Everything above answers questions *about* a frame. `pty_stream/raster.rs`
(SQ-0775) draws it. The oracle already resolves a capture to a cell grid with
per-cell colours plus every placement's source rect, destination size and
position; the rasteriser composites that into an RGBA canvas at the capture's
own cell size and writes a PNG. Development happens over ssh as often as not,
and half the render quests in the tracker end in "the user must go look at it" —
this turns that into "here is the picture, is this right?", and a before/after
pair makes a render change reviewable with no terminal at all.

```sh
cargo run -p app --example pty_capture -- \
    --story "stories/Journey - The Quest Begins.adf" \
    --size 117x64 --keys cr,wait:800,cr,wait:800,cr \
    --out /tmp/j.txt --png /tmp/j.png
```

A before/after pair is one more flag, not a second mode: capture the old build
to a PNG, then run the new one with `--png-diff /tmp/before.png --png
/tmp/pair.png` and the two frames come back side by side with a divider between
them.

**It is not a screenshot, and the difference is not cosmetic.** Text is drawn
with the repo's own bitmap fonts (`render/bitfont.rs`, the ones the v6 pixel
composite uses), scaled to fill each cell: no hinting, no ligatures, and bold and
italic are synthesized from the roman master rather than being real faces. It is an oracle for
**layout, art placement and colour** — where the panes are, where the art
landed, what was painted under it, which of two overlapping things won — drawn
with our glyphs from what Ghostty's *algorithm* resolved. Judge geometry from
it; never judge typography from it. Two more honest limits: cells the app never
painted show the emulator's own default background (palette entry 0, Ghostty's
`#1D1F21`) rather than whatever the real terminal answered the OSC 11 probe
with, because the capture only sees the app→terminal direction; and a
below-background placement (kitty `z < -1073741824`) is bucketed on the z the
*renderer* sorts by, which upstream hardcodes to `-1` for every virtual
placement whatever the client asked for.

**It refuses to hide the bug it was built beside.** Each placement is
rasterised from its OWN resolved source rect, one draw per resolved placement,
never from the aggregated cell rect. A virtual placement resolves one entry per
screen row, and an orphaned run redraws the image's *first* row down the whole
rect (SQ-0772) — sampling per draw means the picture shows that as the banded
smear it is on the glass. A rasteriser that drew each image once into its
bounding box would render a clean, plausible, wrong picture of exactly the
defect worth seeing.

**The picture is a function of the bytes, and that had to be made true**
(SQ-0968). Draws are sorted by `(z, image id, position)`, not by `z` alone: the
protocol settles a tie itself — "if two images with the same z-index overlap then
the image with the lower id is considered to have the lower z-index" — and there
is no resolver order to fall back on, because `resolve_placements` walks a
`HashMap` and hands back a fresh random permutation on every call. Sorting on `z`
alone therefore made two overlapping same-z placements a coin flip: measured at
six orderings in ten renders of one stream inside a single process, with the
losing half putting a superseded image on top and blending the live one's
transparency into it — which is exactly what a stale placement looks like.
`the_same_bytes_always_draw_the_same_picture` is the guard.

**What SQ-0968 reported, and what it turned out to be.** It was filed as "`--png`
composites a band's transparency onto stale content and showed a block the
emitted bytes prove was gone", off the SQ-0948 Shogun frame. That does not
reproduce: captured on `stories/shogun-r322-s890706.z6` (release 322, serial
890706, IBM PC) at 117x40 with 8x18 cells, two turns in (`cr` off the boot menu,
then `look`), the band's own texels under the reported block read `[0,0,0,0]` and
the picture draws the terminal default there; with the SQ-0948 fix reverse-applied
and nothing else changed, the same texels read opaque white and the picture draws
the block. The instrument tracked the bytes in both directions — the third
left-flank band the lane eventually found really was still carrying the fill. The
ordering defect above is what the audit did turn up, and it is a different one.

The tests are in `tests/pty_oracle.rs`'s `raster` module: hand-authored streams
whose expected picture can be stated exactly, asserting **colours at
coordinates** — a PNG writer's obvious failure mode is emitting a plausible
blank, and "a file appeared" accepts one.

## The gallery: the same capture, meant to be looked at

`pty_stream/gallery.rs` and `--example gallery` (SQ-0942) turn the harness into
a picture-maker for the project page. One committed recipe,
`crates/app/examples/gallery.toml`, names every frame — the medium, the key
script, the pane size, the backend, the v6 render mode, the pinned seed and a
caption — and one
command regenerates the whole set into `target/gallery/`, with a proof-sheet
`index.html` and a `gallery.json` recording what was actually captured.

```sh
cargo build -p app
cargo run -p app --example gallery                  # the whole manifest
cargo run -p app --example gallery -- --list        # what it would take
cargo run -p app --example gallery -- --only journey-amiga
```

Six things are deliberate:

- **The output is labelled a render inside its own pixels.** Every frame gets a
  footer strip saying so, drawn in the bitmap face whatever the frame above it
  used. An image gets separated from its page the first time somebody drags it
  into a chat window, and the only claim that survives that trip is the one in
  the pixels. This is the price of the next bullet.
- **It draws with a real typeface** (`--font`, else the first candidate that
  loads, else the bitmap master; `fontdue` is a dev-dependency).
  `raster::render` keeps the bitmap face and the tests never pass the flag, so
  the geometry oracle goes on looking as synthetic as it should — giving *that*
  a real font would make it 90% convincing at a job it cannot do.

  The default face and its size are a **measurement** (SQ-0963). A half-block
  sample is one cell wide and half a cell tall, so square samples want a cell of
  exactly 1:2 — and because a cell is `round(advance · px)` by `round(line · px)`,
  what matters is how often the *rounded* cell lands there. Fira Code (0.615 /
  1.231 em = 2.000) does at ten sizes in 6..24 px/em — 5x10, 6x12, 7x14, 8x16,
  9x18, 10x20, 11x22, 13x26, 14x28, 15x30, the historical terminal cells — where
  JetBrains Mono (2.200), which this list used to lead with, manages one. So Fira
  Code leads the list, and the rasterisation size comes from the face's own line
  metrics rather than a `0.78 × cell_h` guess, with a printed complaint if some
  other face's cell does not match the box it sits in.

  **The kitty cell is 16x32** (SQ-1001; it was 8x18, then 8x16), which is 26 px/em
  of that face and lands exactly. The absolute size is not a taste either: a v6
  press draws its text on an 8x16 *game*-pixel cell, hybrid gives each of those
  characters one terminal cell, and the art beside it is magnified by `s` — so a
  cell of `8s × 16s` puts one game character in one terminal cell and anything
  smaller renders the prose at a fraction of the size the game laid out. At 8x16
  against art at 2x it was rendering it at half. The knock-on is that a kitty shot
  cannot magnify by less than 2: at 1x the game's 80-column screen would get 40
  cells and its text overruns its own windows. Half-blocks keep 10x20 because
  `Picker::halfblocks()` hardcodes that cell whatever the terminal reports.

  Coverage stopped being the deciding question at the same time. `Face::draw`
  used to send a fixed RANGE — U+2500..=U+259F — to the bitmap master and
  everything else to fontdue, so the map's arrowheads (Arrows and Geometric
  Shapes) came out as `.notdef` boxes under Monaco. It now asks the face whether
  it **has** the glyph (`fontdue::Font::has_glyph`) and falls back for anything it
  does not, which fixes every face rather than one; the structural range still
  goes to the bitmap master even when the face has it, because a text face draws
  those with gaps at the cell seams. Anything neither can draw is named in the
  run's output, since a blank cell is quieter than a tofu box and this quest
  exists because a tofu box went unnoticed.

- **A shot's `size` is a magnification.** A v6 press lays out on a fixed native
  screen (640x400 for most of the manifest) and lanthorn letterboxes it into the
  story pane at `min(box_w / native_w, box_h / native_h)`, unrounded; the
  composite is then resized once to `round(native · s)` with every band a 1:1
  crop out of it. So `s` is the only place softness can enter, and the manifest's
  sizes are the ones where it is a whole number — 82x28, 122x41 and 162x53 for a
  640x400 press at a 16x32 cell (2x, 3x, 4x), 130x43 at half-blocks' 10x20. The
  first draft was 117x40 throughout, which is 1.4375x. `Provenance` derives the
  native screen from the mounted medium through `startup.rs`'s own chain, the
  tool prints the magnification under every frame, and
  `every_v6_shot_magnifies_by_a_whole_number` fails the gate if a size drifts off
  it. This cannot become one constant: Arthur's Apple II press is 560x384, and
  the Macintosh's monochrome plates are 480x300 — which is why `zork0-mac-mono`
  is 92x32 and every other kitty shot is 82x28.
- **A `--pictures` name is part of the provenance** (SQ-1001). A shot may pass
  `args = ["--pictures", "Pic.data"]` to choose which rendition of the artwork to
  draw, and the archive's own flavour then picks the machine. `Provenance::read`
  takes that name and resolves the override the way `startup.rs` does, because the
  named archive changes the picture space the press lays out on: read without it,
  the native screen, the magnification and the profile all belong to the rendition
  the shot did not draw, and all of them stay self-consistent. `expect` cannot
  catch that — two renditions of one scene look like each other. A named archive
  that will not load fails the shot outright rather than falling back to the
  Blorb, which in the app is the right call for a player and in a gallery is a
  caption describing a picture that is not there.
- **Nothing about a frame is declared twice.** The release and serial come from
  the header of the bytes the medium mounted; the turn count is counted off the
  key script. A manifest that tries to state either is refused.
- **Every shot carries a non-vacuity guard** (`expect`, `expect_art_cells`), and
  a shot that fails it never becomes a picture. This is not ceremony: pointed at
  a DOS floppy that lanthorn opens a browser for, the first draft captured
  *Ballyhoo* off a neighbouring disk while the release, serial and medium in the
  record all went on correctly describing the Zork Zero image the manifest
  named — because those are read from the file and not from the frame.
- **One shot renders more than one frame: the composite** (SQ-1165). A shot that
  names `machines = [2, 3, 4, 6, 7, 8]` is captured once per §11.1.3 interpreter
  number — each launch with `--interpreter N --colour machine` appended, which is
  the pair SQ-1154 made reach a bare story file — and the results are tiled into
  a single picture, each tile badged with its machine. It is a shot KIND rather
  than a second example beside this one, because the provenance, the guards, the
  pinned seed, the burnt-in label, the proof sheet and `gallery.json` are all
  things a composite needs exactly as much as a single frame does; a renderer of
  its own would have grown them again and then drifted.

  `machine-colours` is the one in the manifest: *Deadline* r27/s831005, a
  **Version 3** story chosen for its status line, so every tile carries two
  coloured surfaces rather than one and the reverse-video band is where the IBM
  PC's white-on-EGA-blue and the Commodore 128's cyan show hardest. Six tiles,
  not nine, because nine machines are not nine looks: the three Apple rows share
  `APPLE_PERIOD_LOOK` and the Atari ST shares the Macintosh's pair.

  **Its guard is `check_machines_differ`, and it is the reason the kind is worth
  having.** Every tile draws the same story at the same moment, so every string
  `expect` could name is on all of them and six copies of one palette pass
  unanimously — the SQ-1164 failure one shape along. The guard reads each tile's
  page, ink and pair set off its own story pane and refuses the frame if two
  machines `zvm::interpreter` measured APART came out the same. The obligation is
  derived per pair rather than listed, because interpreters 2 and 8 were measured
  alike — both white on black with a full reverse — and differ only in caret
  shape, so a rule of "every tile must differ" would refuse a correct frame.
  Falsified by swapping the appended `--colour machine` for `--colour theme`: all
  six tiles collapse onto the terminal default and the guard names eleven pairs.

  Two layout facts are derived rather than chosen, and both because a TILE is a
  terminal and therefore already landscape. `Shot::tile_columns` picks the count
  that makes the finished PICTURE closest to square, not the squarest grid:
  `ceil(sqrt(6))` gives 3x2 and a 3128x1402 banner that gets scaled down until the
  prose is unreadable, where 2 columns give 2090x2016. And the machine's name is
  a BADGE on the tile rather than a caption above it, drawn in the harness's own
  bitmap face on a near-black plate under a hairline — a treatment lanthorn's
  theme has nowhere, since the one thing the tag must not do is read as something
  the app drew. `badge_anchor` finds the lowest clear two-row band in each pane
  off that tile's own resolved screen, so it can never land on the status band,
  the prose or the caret; a tile with no clear ground goes unbadged and says so.

`tests/suites/gallery_manifest.rs` runs the whole validator over the committed
recipe, so a manifest that has gone stale fails the gate rather than failing
whoever is trying to cut a release. It needs no gitignored media.

## Casts: the same capture, as a moving image

`pty_stream/cast.rs` and `--example cast` (SQ-0943) serialise a capture as an
[asciinema v2](https://docs.asciinema.org/manual/asciicast/v2/) file. It is a
small tool because the harness already collects exactly the right data: a
`Flush` is a timestamped byte range from the real binary under a real pty, and a
v2 cast is a header line followed by `[seconds, "o", data]` events. Same shape as
the gallery — one committed recipe (`crates/app/examples/casts.toml`), output
under `target/casts/`, a required guard per entry.

```sh
cargo build --workspace
cargo run -p app --example cast              # the whole manifest
cargo run -p app --example cast -- --list
cargo run -p app --example cast -- --only zork-map --gif
asciinema play target/casts/machines.cast
```

**`--gif` is what makes a cast publishable.** A `.cast` is JSON and needs a
player, which a GitHub README cannot run — so the flag renders each recording as
an animated GIF beside it with [`agg`](https://docs.asciinema.org/manual/agg/),
asciinema's own renderer (`brew install agg`). `docs/automapping.gif`,
`docs/beyond-zork.gif` and `docs/anchorhead.gif` are that output.

**`agg` and not `svg-term`, and the reason is geometry rather than taste.** The
SVG route was built first and discarded: `svg-term` lays columns out 1.002 units
apart while a box-drawing glyph is one unit wide, so every cell boundary carries a
hairline seam. It is invisible in prose and cumulative along a rule — enough to
render lanthorn's own window borders as dashed lines — and no flag adjusts it,
because it is baked into the emitted geometry. (It also runs `svgo` over its own
output, which *deletes* the `font-family` declaration and drops the whole page
onto the viewer's proportional default; `--no-optimize` fixes that half but not
the seams.) `agg` rasterises with a real font at whole-pixel cell positions, so a
`│` column is solid and a `─` run is continuous.

What that costs is GIF's 256-colour palette, and it is why **no cast is a Version
6 recording**. A half-block v6 frame carries two 24-bit colours per cell and is
exactly the content the palette cannot hold; more to the point, half-blocks is the
fallback for a reader without a graphics protocol rather than a preview of what v6
looks like. Version 6 is shown with the gallery's kitty stills, and motion is kept
for what only motion shows. Every entry in `casts.toml` is text or 16-colour,
which GIF holds exactly.

**These recordings deliberately do NOT answer the kitty capability query.** The
asciinema player renders cells and SGR and drops kitty's APC graphics, so a
kitty recording replays with no artwork at all and lanthorn looks like it draws
nothing. Left unanswered, `ratatui-image` falls back to half-blocks — the same
v6 *pixel* path resolved into `▀` with a foreground and a background, which is
glyphs and 24-bit SGR and replays exactly. Measured on a Journey recording:
1,624 `▀`, 1,499 `▄`, 3,649 truecolour foreground and 3,689 truecolour
background sequences, none in iTerm2's colon-separated form (the one gap in the
player). The tool refuses any recording that emits real graphics commands
anyway, and every file says in its own header why there is no kitty artwork in
it.

`Spec::answer_kitty` is what selects that, and `Spec::argv` is what lets the same
driver record `zvm-cli`, `gvm-cli` and `scott-cli` — the CLI clients are
text-only by design, so a cast captures them *completely*.

One driver bug fell out of building this, and it is worth knowing about: sending
a keystroke used to reset the same clock the flush grouping read, so the app's
reply — a few milliseconds after the key — always looked like a continuation of
the previous burst however many seconds earlier it was, and **every run
collapsed into one flush at `at: 0`**. Invisible to the decoder, which only
wants the grouping for attribution; fatal to a recorder, for which those
timestamps *are* the recording. Reads and keystrokes now keep separate clocks.

## See also

- [Interactive-fiction standards lanthorn implements](../reference/standards.md) (Z-Machine,
  Glulx, Glk, Quetzal, Blorb, Treaty of Babel).
- [What CI cannot see](ci-fixture-coverage.md) — which integration suites skip
  vacuously without `stories/`, which of them an authored fixture can reach,
  and which are about a particular commercial release and never will be.
- Design/strategy notes under [`docs/design/`](../design/).
