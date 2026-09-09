# Clean-room specification: the pair protocol

lanthorn is BSD-3-Clause. Several file formats it wants to read are, in
practice, documented only by GPL-licensed implementations. Reading such an
implementation and then writing code is how a permissive project acquires a
licence problem it cannot see: copyright attaches to expression, and expression
is what you carry away when you read code and then type.

The way out is old and well-tested — a **clean room**. Two parties, one
one-directional channel between them, and a written artefact in the middle that
contains facts and no expression.

## The two roles

**The specification half** reads whatever it needs: GPL sources, other
interpreters, disassemblies, format notes, real specimen files. It writes a
functional specification — what the bytes *mean*, what a conforming reader must
*do*, what a program's behaviour must *look like* from outside. It writes no
code in the target language and no pseudo-code.

**The implementation half** reads the specification, the public standards it
cites, and real specimen files. It never reads the sources the specification
half read, and never reads the specification half's working notes or
transcript.

Neither half reads the other's transcript. The specification is the only
channel, which is what makes it auditable: if a fact reached the implementation,
it is written down in a document anyone can review.

## What the specification may contain

- Field widths, byte offsets, endianness, bit positions, enumerated values.
- Signature bytes, magic numbers, and where in a file they may appear.
- Algorithms stated as a **procedure in prose** — "read one byte; if it is
  0xED and the next byte is also 0xED, the two bytes are followed by a count
  and a value" — with a worked example on a byte string the writer constructed.
- Observable behaviour: what the program prints, in what order, under what
  conditions, and a black-box test that would catch getting it wrong.
- Facts that are irreducibly per-game, presented as a data table, together with
  an honest column saying how each entry could be re-derived from a specimen
  without the table.

## What the specification must not contain

- Source code, in any language, from any source.
- Pseudo-code that is a transliteration of source code.
- Function names, variable names, struct names, enumerator names, or file names
  drawn from the sources read.
- Control-flow narration — "it then loops over the table, and if the flag is
  set it calls …". A specification says *the format is* and *a conforming
  reader must*; it does not describe another program's shape.
- Test vectors copied from the sources read. Worked examples are constructed by
  the specification writer, or taken from a real specimen the implementer can
  fetch independently.

The last two are the ones that slip. A sentence that could only have been
written by someone looking at a particular function is a sentence that carries
that function's expression, however carefully paraphrased.

## Citing the sources

The specification names every source it summarised: project, URL, licence, and
the exact commit or release read. This is not an attribution requirement —
facts about a file format are not copyrightable, and a specification of them
carries the repository's own licence. It is a provenance requirement, so a
reviewer can check the claim that the specification contains facts and not
expression, and so a later reader knows what was and was not consulted.

Where the same fact is also available from an independent, non-copyleft source
— a published format note, a hardware manual, a specimen file that can simply
be measured — the specification says so. That is the strongest position: the
fact is then documented twice, and the GPL reading was only a shortcut to
finding it.

## Verification without crossing the channel

The implementation half needs an oracle, and it must be one it can run itself.
The best one available for a file format is a **second encoding of the same
work**: many Scott Adams games exist both as a platform-native binary and as a
plain-text conversion. Decoding the native file and comparing the resulting
tables, field for field, against the text file is a black-box check that needs
nothing from the specification half and nothing from any GPL implementation.

Where such a pair exists, the specification names it.

## In this repository

| Document | Role |
|---|---|
| [`scott-dialects-spec.md`](scott-dialects-spec.md) | The specification for the Scott Adams game-file dialects |

The specification was produced under this protocol in September 2026; the
implementation that consumes it is tracked separately and was written without
access to the sources listed in that document's Sources section.
