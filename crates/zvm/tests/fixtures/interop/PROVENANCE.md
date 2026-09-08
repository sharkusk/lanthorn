# Interop golden saves — provenance (SQ-0158)

These are reference-interpreter-produced save files, checked in so the READ-direction
interop tests (`crates/zvm/tests/save_interop.rs`) can run in CI without any external
binary. Regenerate with `scripts/gen-interop-goldens.sh`.

## `minizork-at-P.qzl`

- **Story:** `crates/zvm/tests/fixtures/minizork.z3` (Mini-Zork I, Infocom 1988).
- **Reference interpreter:** `dfrotz` (FROTZ V2.55, Dumb interface; homebrew `frotz`).
- **Point P — prefix commands (verbatim):** `open mailbox` → `take leaflet` → `north`.
  Resulting state: room = *North of House*, the leaflet is in the player's inventory.
- **Save command:** the game's `save` verb, written to this path.
- **Format:** Quetzal `FORM … IFZS` (`IFhd` + `CMem` + `Stks`), ~366 bytes.
- **Probe (used by the test):** `look` (reveals room) + `inventory` (reveals the leaflet).
  A broken restore would place the player elsewhere or drop the leaflet — so the
  cross-load equivalence assertion cannot pass vacuously.

## `curses-at-P.qzl` (SQ-1421)

- **Story:** `crates/zvm/tests/fixtures/curses.z5` ("Curses", Graham Nelson 1993,
  Release 16/Serial 951024) — added alongside minizork as a second, spec-diverse
  story for `save_interop.rs`'s matrix: a real parser game with its own `save`
  verb, not a synthetic opcode-suite or menu exerciser.
- **Reference interpreter:** `dfrotz` (FROTZ V2.55, Dumb interface; homebrew
  `frotz`; commit `acf205585a9472d27c07c0fe62da4b8bc89d1ec7`, 2025-02-01 — see
  `crates/zvm/tests/fixtures/README.md` for the full `dfrotz -v` banner).
- **Point P — prefix commands (verbatim):** `east` → `take scarf`. Resulting
  state: room = *Servant's Room*, the scarf is in the player's inventory
  (alongside the chocolate biscuit, electric torch and crumpled paper the game
  starts the player carrying).
- **Save command:** the game's `save` verb, written to this path (a RELATIVE
  filename — dfrotz's "Please enter a filename" prompt did not accept an
  absolute path containing the scratch directory's punctuation-heavy name
  during regeneration; `-m` disables the `***MORE***` pager, whose prompts
  otherwise consume extra lines from the piped script).
- **Format:** Quetzal `FORM … IFZS`, ~544 bytes.
- **Probe (used by the test):** `look` (reveals room) + `inventory` (reveals the
  scarf) — same non-vacuity reasoning as minizork's golden above.
- **Not byte-reproducible across regenerations, unlike minizork's**: running
  `scripts/gen-interop-goldens.sh` twice produced two different-but-both-valid
  saves (544 bytes both times, different SHA-256 each time) — Curses evidently
  carries some RNG- or clock-influenced state in dynamic memory unrelated to
  the player's own commands, where minizork's regenerated save came back
  byte-identical. Both saves still restore to the SAME probed room/inventory
  text under both interpreters, which is everything `save_interop.rs` asserts
  — the committed file here is one specific run, not "the" canonical bytes.
