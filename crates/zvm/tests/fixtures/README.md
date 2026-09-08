# Test fixtures (not committed — binary story files, gitignored)

These regression/story files are fetched on demand; `crate::fixtures::load(name)`
returns `None` when absent so fixture-backed tests skip cleanly.

| file | purpose | source | status |
|------|---------|--------|--------|
| `czech.z5` | CZECH opcode regression suite (primary acceptance oracle, Task 16) | <https://www.ifarchive.org/if-archive/infocom/interpreters/tools/czech_0_8.zip> (unzip, extract czech.z5) | ✅ verified, sha256 9f7e01b94353798e1eb8c3b4521f06db4c830a6120f5b3ab7f0d1ec1bc882b5a |
| `praxix.z5` | Praxix arithmetic/edge-case checker (Task 16) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/praxix.zip> (unzip, extract praxix.z5) | ✅ verified, sha256 bef3bdc2543cc7161833062855aa9bb1682db9eca69d9d36e9101347c21b48ac |
| `minizork.z3` | small real v3 game (smoke tests, Tasks 5/6/15) | <https://ifarchive.org/if-archive/infocom/demos/minizork.z3> | ✅ verified, sha256 c74f01a232e8df4b05d7ebcba14870143f49b3c9a25f194f7a7d2c69e31ea4a6 |
| `gntests.z5` | Graham Nelson's Z-Spec InputCodes/Fonts/Accents/Colours/Header/TimedInput test programs — `zvm-cli`'s `gntests_input_codes` reads its InputCodes screen to prove `@read_char` never sees ZSCII 10 for a bare Enter (SQ-1423) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/etude.tar.Z> (`gzip -dc \| tar xf -`, extract `gntests.z5`) | ✅ verified, sha256 56e483ab0049311ce26b4b8639d2c6f8976ad0e7058113099a8e4e3d49c545a6 |
