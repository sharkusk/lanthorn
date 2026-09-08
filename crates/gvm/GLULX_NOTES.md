# GLULX_NOTES — authoritative tables (transcribed from the spec)

Source: **Glulx specification 3.1.2**, Andrew Plotkin —
<https://www.eblong.com/zarf/glulx/glulx-spec.txt> (and the HTML at
<https://www.eblong.com/zarf/glulx/glulx-spec.html>). Glk dispatch selector
numbers from the canonical `gi_dispa.c`
(<https://raw.githubusercontent.com/erkyrath/cheapglk/master/gi_dispa.c>).

Where the implementation plan's prose and the spec disagree, **the spec wins.**
Everything in `gvm` is implemented against THIS file.

## 1. Header (first 36 bytes, all 32-bit big-endian)

| Offset | Field          | Meaning                                            |
|--------|----------------|----------------------------------------------------|
| 0x00   | Magic          | `47 6C 75 6C` = ASCII `"Glul"`                      |
| 0x04   | Version        | upper 16 bits = major, next 8 = minor, low 8 = sub |
| 0x08   | RAMSTART       | first writable address (end of ROM)                |
| 0x0C   | EXTSTART       | end of the stored initial memory in the file       |
| 0x10   | ENDMEM         | end of the memory map at startup                   |
| 0x14   | Stack size     | bytes of stack the program needs                   |
| 0x18   | Start function | address of the first function to execute           |
| 0x1C   | Decoding table | string-decoding table address (0 = none); used 2b  |
| 0x20   | Checksum       | sum of the whole initial memory as 32-bit ints     |

**Version acceptance:** a 3.x interpreter accepts game versions 2.0.0 through
3.1.*. We accept major version 2 or 3; reject everything else as
`UnsupportedVersion`.

## 2. Memory map

- `[0, RAMSTART)` — ROM. Read-only at runtime; writes are a fault (we treat as
  a no-op + diagnostic, games never write ROM).
- `[RAMSTART, EXTSTART)` — RAM, initialized from the image file.
- `[EXTSTART, ENDMEM)` — RAM, zero-initialized on load.
- RAMSTART, EXTSTART, ENDMEM are all multiples of 256.
- The image file length equals EXTSTART (the stored initial memory).
- `getmemsize` returns the current ENDMEM. `setmemsize(newval)` resizes:
  newval must be a multiple of 256 and **not less than the original ENDMEM**;
  growth is zero-filled. Returns 0 on success, 1 on failure.

## 3. Instruction encoding

### Opcode number (variable length, decode by the top two bits of byte 0)

- top bit `0` (byte `< 0x80`): 1 byte, value = byte.
- top bits `10`: 2 bytes, value = (u16 big-endian) − 0x8000.
- top bits `11`: 4 bytes, value = (u32 big-endian) − 0xC0000000.

### Operand addressing modes (one nibble each)

Packed two nibbles per byte, **low nibble = earlier operand**, in argument
order; an odd final operand leaves the high nibble zero. The mode run is
`ceil(n_operands / 2)` bytes, read in full *before* the operand data bytes that
follow (constants/addresses), which appear in operand order after the mode run.

| Mode | LOAD                                   | STORE                      | extra bytes |
|------|----------------------------------------|----------------------------|-------------|
| 0x0  | constant 0                             | discard (throw away)       | 0           |
| 0x1  | constant, 1 byte, sign-extended        | (invalid as store)         | 1           |
| 0x2  | constant, 2 bytes, sign-extended       | (invalid as store)         | 2           |
| 0x3  | constant, 4 bytes                      | (invalid as store)         | 4           |
| 0x5  | contents of address (1-byte addr)      | store to memory addr       | 1           |
| 0x6  | contents of address (2-byte addr)      | store to memory addr       | 2           |
| 0x7  | contents of address (4-byte addr)      | store to memory addr       | 4           |
| 0x8  | pop off stack                          | push on stack              | 0           |
| 0x9  | call-frame local (1-byte offset)       | store to local             | 1           |
| 0xA  | call-frame local (2-byte offset)       | store to local             | 2           |
| 0xB  | call-frame local (4-byte offset)       | store to local             | 4           |
| 0xD  | contents of RAM addr (1-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 1         |
| 0xE  | contents of RAM addr (2-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 2         |
| 0xF  | contents of RAM addr (4-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 4         |

Modes 0x4 and 0xC are unused/illegal. Address/local-offset operand bytes are
read **unsigned**; constant operand bytes are **sign-extended**.

## 4. Functions and the call frame

A function begins with a type byte:
- `0xC0` — stack-argument function (args left on the new frame's stack).
- `0xC1` — local-argument function (args written into locals).

Then the **locals-format** list: `(LocalType, LocalCount)` byte pairs, where
LocalType ∈ {1,2,4}, terminated by a `(0,0)` pair. Instructions start right
after the terminator.

### Call frame layout (byte-addressed on the stack)

```
+------------+  FramePtr
| FrameLen   |   u32   total frame length (to the start of the values area)
| LocalsPos  |   u32   offset from FramePtr to the locals
| Format of  |   2*n bytes   the locals-format (LocalType,LocalCount) pairs...
|   Locals   |               ...terminated by (0,0)
| Padding    |   0/2 bytes   to align the locals
+------------+  FramePtr+LocalsPos
| Locals     |   1/2/4 bytes each, at natural alignment
| Padding    |   0..3 bytes
+------------+  FramePtr+FrameLen
| Values     |   4 bytes each (the operand stack for this frame)
+------------+  StackPtr
```

Alignment: 16-bit values on even addresses, 32-bit on multiples of 4 (relative
to FramePtr). Locals are laid out in format order, each at its natural
alignment; LocalsPos points just past the format list (padded to a multiple of
4 minus... we align locals start so the first local sits at its natural
alignment — in practice format list + (0,0) is padded so locals begin aligned).

### Argument passing

- `0xC1`: pop args off the **caller** stack; write them into the locals in
  format order (truncated to the local's width). Too many → extras dropped; too
  few → remaining locals stay zero.
- `0xC0`: all locals zeroed; push the args onto the new frame's value stack
  (last arg first so the first arg ends up on top), then push the arg count.

### Call stub (four u32 values, pushed before the new frame on a call)

```
DestType   u32
DestAddr   u32
PC         u32     (address to resume at after return)
FramePtr   u32     (caller frame pointer)
```

DestType: `0` discard, `1` store to main memory at DestAddr, `2` store to local
at `(FramePtr+LocalsPos)+DestAddr`, `3` push on stack. (For glk/string
intermediate stubs the spec also defines 0x10/0x11/0x12/0x13 — not needed in
2a.)

### Return

Set StackPtr back to FramePtr, pop FramePtr, PC, DestAddr, DestType, store the
return value per DestType, resume at PC. Returning from the **topmost** frame
(no call stub beneath) ends execution → `StepResult::Quit`.

## 5. Branch convention

Branch operand `Offset`:
- `0` → return 0 from the current function.
- `1` → return 1 from the current function.
- otherwise → `PC = (addr_of_next_instruction) + Offset - 2`.

## 6. Opcode numbers (Phase 2a subset; L=load operands, S=store operands)

| Opcode      | Num   | L | S |   | Opcode     | Num   | L | S |
|-------------|-------|---|---|---|------------|-------|---|---|
| nop         | 0x00  | 0 | 0 |   | jleu       | 0x2D  | 3 | 0 |
| add         | 0x10  | 2 | 1 |   | call       | 0x30  | 3 | 1 |
| sub         | 0x11  | 2 | 1 |   | return     | 0x31  | 1 | 0 |
| mul         | 0x12  | 2 | 1 |   | tailcall   | 0x34  | 2 | 0 |
| div         | 0x13  | 2 | 1 |   | copy       | 0x40  | 1 | 1 |
| mod         | 0x14  | 2 | 1 |   | copys      | 0x41  | 1 | 1 |
| neg         | 0x15  | 1 | 1 |   | copyb      | 0x42  | 1 | 1 |
| bitand      | 0x18  | 2 | 1 |   | sexs       | 0x44  | 1 | 1 |
| bitor       | 0x19  | 2 | 1 |   | sexb       | 0x45  | 1 | 1 |
| bitxor      | 0x1A  | 2 | 1 |   | stkcount   | 0x50  | 0 | 1 |
| bitnot      | 0x1B  | 1 | 1 |   | stkpeek    | 0x51  | 1 | 1 |
| shiftl      | 0x1C  | 2 | 1 |   | stkswap    | 0x52  | 0 | 0 |
| sshiftr     | 0x1D  | 2 | 1 |   | stkroll    | 0x53  | 2 | 0 |
| ushiftr     | 0x1E  | 2 | 1 |   | stkcopy    | 0x54  | 1 | 0 |
| jump        | 0x20  | 1 | 0 |   | streamchar | 0x70  | 1 | 0 |
| jz          | 0x22  | 2 | 0 |   | streamnum  | 0x71  | 1 | 0 |
| jnz         | 0x23  | 2 | 0 |   | streamstr  | 0x72  | 1 | 0 |
| jeq         | 0x24  | 3 | 0 |   | streamunichar | 0x73 | 1 | 0 |
| jne         | 0x25  | 3 | 0 |   | getmemsize | 0x102 | 0 | 1 |
| jlt         | 0x26  | 3 | 0 |   | setmemsize | 0x103 | 1 | 1 |
| jge         | 0x27  | 3 | 0 |   | quit       | 0x120 | 0 | 0 |
| jgt         | 0x28  | 3 | 0 |   | glk        | 0x130 | 2 | 1 |
| jle         | 0x29  | 3 | 0 |   | getiosys   | 0x148 | 0 | 2 |
| jltu        | 0x2A  | 3 | 0 |   | setiosys   | 0x149 | 2 | 0 |
| jgeu        | 0x2B  | 3 | 0 |   | callf      | 0x160 | 1 | 1 |
| jgtu        | 0x2C  | 3 | 0 |   | callfi     | 0x161 | 2 | 1 |
|             |       |   |   |   | callfii    | 0x162 | 3 | 1 |
|             |       |   |   |   | callfiii   | 0x163 | 4 | 1 |

Arithmetic is 32-bit two's complement. `div`/`mod` truncate toward zero;
div/mod by zero is a fault (diagnostic + Quit). Signed compares: jlt/jge/jgt/jle;
unsigned: jltu/jgeu/jgtu/jleu. Shifts by ≥ 32: `shiftl`/`ushiftr` yield 0,
`sshiftr` yields 0 or -1 (the sign bit).

## 7. I/O system and stream opcodes

`setiosys(mode, rock)` / `getiosys → (mode, rock)`:
- mode `0` — null: all output discarded.
- mode `1` — filter: rock is a function address; each output character is passed
  as a single argument (its code point) to that function via a VM call
  (`run_call_to_return`), per spec §7.2. Re-entrant filter calls are bounded by a
  native-stack depth guard.
- mode `2` — Glk: stream opcodes route to `Output::print`.
- any other mode — normalized to `0` (null) with rock `0` (spec §2.11: "If the
  system L1 is not supported by the interpreter, it will default to the null
  system"). Modes `0` and `2` always zero the rock too, matching glulxe's
  `stream_set_iosys` (`string.c`) — only mode `1` (filter) keeps the caller's
  rock, since there it names the filter routine (SQ-1415).

- `streamchar L1` — emit `L1 & 0xFF` as one Latin-1 char.
- `streamunichar L1` — emit the 32-bit Unicode code point `L1`.
- `streamnum L1` — emit `L1` as a signed decimal number (ASCII).
- `streamstr L1` — print a string object at address L1: type `0xE0` =
  unencoded C-string (Latin-1 bytes until a 0 terminator), `0xE2` = unencoded
  Unicode (E2 + 3 pad bytes + u32 code points until a 0). Compressed `0xE1`
  decoding is deferred to 2b (diagnostic).

## 8. The `glk` opcode (0x130) — minimal dispatch for 2a

`glk selector argc store`: pop `argc` 32-bit args off the stack (the first arg
is on top), invoke the Glk function, push/store the return value. 2a implements
only the put-char/buffer family; everything else pops its args and returns 0.

Glk dispatch selectors (from `gi_dispa.c`):

| Selector              | Num    | 2a behavior                                  |
|-----------------------|--------|----------------------------------------------|
| glk_put_char          | 0x0080 | print `arg0 & 0xFF` as Latin-1               |
| glk_put_char_stream   | 0x0081 | print `arg1 & 0xFF` (ignore stream id)       |
| glk_put_string        | 0x0082 | print the C-string at `arg0`                 |
| glk_put_buffer        | 0x0084 | print `arg1` Latin-1 bytes from address `arg0` |
| glk_put_char_uni      | 0x0128 | print Unicode code point `arg0`              |
| glk_put_string_uni    | 0x0129 | print the Unicode string at `arg0`           |
| glk_put_buffer_uni    | 0x012A | print `arg1` u32 code points from `arg0`     |
| glk_window_open       | 0x0023 | return 0 (no real window model in 2a)        |
| glk_set_window        | 0x002F | return 0                                     |
| glk_stream_set_current| 0x0047 | return 0                                     |
| glk_stream_get_current| 0x0048 | return 0                                     |
| (any other selector)  | —      | pop args, return 0                           |

## 9. String objects + compressed-string decoding (Phase 2b)

A string object is identified by its first byte (spec §1.6.1):

- `0xE0` — unencoded Latin-1 C-string: the bytes follow, terminated by a `00`.
- `0xE2` — unencoded Unicode: an `E2` byte, **three padding `00` bytes**, then
  big-endian 32-bit code points, terminated by a `0000_0000` word.
- `0xE1` — compressed: an `E1` byte, then a Huffman bit stream.

`streamstr L1` (0x72) prints the string object at L1 to the current iosys. A
"printable object" can also be a **function** (`0xC0`/`0xC1`); when an indirect
decode node names a function it is *called* and its output streamed.

### Compressed (E1) bit stream (spec §1.6.1.3)

Bits are read **low bit first**: bit 0 is the `0x01` bit of the first byte after
the `E1`, up through the `0x80` bit, then on to the next byte. Decoding walks the
string-decoding table (a Huffman tree) from its root: at each branch read one
bit, `0` → left child, `1` → right child, until a leaf. Print the leaf's entity,
return to the root, repeat. The string-terminator leaf ends decoding.

### String-decoding table (spec §1.6.1.4)

Header (three big-endian 32-bit words), then the node data:

| Offset | Field           |
|--------|-----------------|
| +0     | Table Length    |
| +4     | Number of Nodes |
| +8     | Root Node Addr (**absolute** address, not an offset) |
| +12…   | Node data       |

The table root is at `decode_table()` (header field 0x1C) but is **overridable**
at run time by `setstringtbl`/`getstringtbl`; the current value lives on the
Machine. Decoding an `E1` string with **no** table set is illegal (fault).

### Node types (distinguished by their first byte)

| Type | Name                         | Layout                                       |
|------|------------------------------|----------------------------------------------|
| 0x00 | Branch                       | `00` · Left addr (4) · Right addr (4)        |
| 0x01 | String terminator            | `01`                                         |
| 0x02 | Single character             | `02` · char (1)                              |
| 0x03 | C-style string               | `03` · Latin-1 bytes… · NUL (1)              |
| 0x04 | Single Unicode character     | `04` · char (4)                              |
| 0x05 | C-style Unicode string       | `05` · 32-bit chars… · NUL word (4)          |
| 0x08 | Indirect reference           | `08` · addr (4) → print string / call func   |
| 0x09 | Double-indirect reference    | `09` · addr (4); `*addr` → object            |
| 0x0A | Indirect, with arguments     | `0A` · addr (4) · argc (4) · args (4·argc)   |
| 0x0B | Double-indirect, with args   | `0B` · addr (4) · argc (4) · args (4·argc)   |

For 0x08/0x0A the address names the object directly; for 0x09/0x0B it names a
4-byte field whose contents are the object address. If the object is a string it
is printed (args ignored); if it is a function it is **called** (with the given
args, or none for 0x08/0x09) and its output is streamed in place.

### Calling functions from within strings (spec §1.3.4)

The spec models a string-called function with type-10/11/13/14 call stubs so
that a normal `return` resumes string decoding. We instead execute the call
**synchronously**: decoding is a recursive Rust walk, and a function node calls
the function and runs the VM run-loop until that frame returns (tracked by the
frame pointer), then resumes the walk. This is observably equivalent for
well-behaved veneer functions (their output is streamed in order). A recursion
depth limit guards against pathological/cyclic tables (fault, never a Rust stack
overflow).

### `getstringtbl` / `setstringtbl`

| Opcode       | Num   | L | S | Behavior                                       |
|--------------|-------|---|---|------------------------------------------------|
| getstringtbl | 0x140 | 0 | 1 | store the current decoding-table address (0=none) |
| setstringtbl | 0x141 | 1 | 0 | set it (0 = none; does not touch the ROM header)  |

## 10. The memory-array opcodes (Phase 2b, spec §2.4)

All take a main-memory base in L1 and a **signed** index in L2; loads
zero-extend, stores truncate. Address is `L1 + width*L2`.

| Opcode    | Num   | L | S | Address / effect                              |
|-----------|-------|---|---|-----------------------------------------------|
| aload     | 0x48  | 2 | 1 | read 32-bit @ `L1+4*L2`                        |
| aloads    | 0x49  | 2 | 1 | read 16-bit @ `L1+2*L2` (zero-extended)        |
| aloadb    | 0x4A  | 2 | 1 | read 8-bit @ `L1+L2` (zero-extended)           |
| aloadbit  | 0x4B  | 2 | 1 | bit `L2 mod 8` of `L1 + L2/8` → 0/1            |
| astore    | 0x4C  | 3 | 0 | write 32-bit L3 @ `L1+4*L2`                    |
| astores   | 0x4D  | 3 | 0 | write 16-bit (low) L3 @ `L1+2*L2`              |
| astoreb   | 0x4E  | 3 | 0 | write 8-bit (low) L3 @ `L1+L2`                 |
| astorebit | 0x4F  | 3 | 0 | set (L3≠0) / clear (L3=0) bit `L2 mod 8`       |
| mzero     | 0x170 | 2 | 0 | zero L1 bytes at L2                            |
| mcopy     | 0x171 | 3 | 0 | copy L1 bytes from L2 to L3                    |

Bit indexing uses **flooring** division (`div_euclid`/`rem_euclid`), so a
negative `L2` reaches bytes before `L1` (spec examples: base 1002, `L2=-1` →
bit 7 of 1001). `mcopy` copies forward when `to < from`, otherwise backward, so
overlapping ranges move correctly.

## 11. The allocation heap (Phase 2b, spec §2.9)

Dynamically-allocated blocks live above ENDMEM. The first `malloc` activates the
heap: the heap-start address is the `getmemsize` value at that moment, and the
memory map is extended to fit the block. While the heap is active, `setmemsize`
is illegal (it fails — stores 1). When the last block is freed, the heap goes
inactive and the memory map shrinks back to the heap-start address.

| Opcode | Num   | L | S | Effect                                            |
|--------|-------|---|---|---------------------------------------------------|
| malloc | 0x178 | 1 | 1 | allocate L1 bytes; store the block address or 0   |
| mfree  | 0x179 | 1 | 0 | free the extant block at L1                        |

**Implementation:** we track the heap-start address (0 = inactive) and a list of
extant blocks `(addr, size)` kept sorted by address. `malloc` walks the free
gaps from heap-start (reusing freed space) and places the block in the first gap
large enough, extending the memory map only if it must append past the committed
end. Coalescing is implicit: a freed block simply leaves a gap that the next
`malloc` can reuse or grow across. Allocation fails (stores 0) for a
non-positive size or if the block would push memory past a fixed ceiling
(`MAX_MEMSIZE`). `mfree` of an address that is not an extant block faults.

## 12. Search opcodes (Phase 2b, spec §2.16)

A collection of fixed-size structs each hold a `KeySize`-byte key at byte
`KeyOffset`. Searches find a struct whose key matches the given key.

| Opcode       | Num   | L | S | Operands                                            |
|--------------|-------|---|---|-----------------------------------------------------|
| linearsearch | 0x150 | 7 | 1 | Key, KeySize, Start, StructSize, NumStructs, KeyOffset, Options |
| binarysearch | 0x151 | 7 | 1 | same; structs sorted ascending by key; NumStructs exact |
| linkedsearch | 0x152 | 6 | 1 | Key, KeySize, Start, KeyOffset, NextOffset, Options |

**Options bitfield:**
- `KeyIndirect` (0x01): Key is the *address* of the key bytes. Otherwise Key is
  the value itself and KeySize must be 1/2/4 (its low bytes are used).
- `ZeroKeyTerminates` (0x02): stop and fail at an all-zero struct key (a real
  match on an all-zero search key still takes precedence). linear/linked only.
- `ReturnIndex` (0x04): return the array index (or 0xFFFFFFFF on failure)
  instead of the struct address (or 0 on failure). linear/binary only.

`linearsearch` scans in order; `NumStructs` may be 0xFFFFFFFF for no limit.
`binarysearch` compares keys as big-endian unsigned integers (byte-wise
lexicographic on equal-length keys). `linkedsearch` follows the 4-byte link at
`NextOffset` until it is zero. All key reads are bounds-checked.

## 13. gestalt + verify (Phase 2b, spec §2.18)

`gestalt L1 L2 S1` (0x100) — test selector L1 (with optional arg L2). Unknown
selectors return 0. `verify S1` (0x121) — a **real image checksum check** (spec
§1.4: sum the initial memory as big-endian 32-bit words with the 0x20 field
zeroed, compare to the stored checksum); stores 0 if it matches, else 1.
Selector numbers and meanings are from the spec; the **returned capability values
reflect what this VM actually implements** (single-precision float is
implemented → Float(11) = 1; double-precision is deferred → Double(12) = 0; see
§17 for acceleration, which is implemented).

| Sel | Name         | Num | Returns                                              |
|-----|--------------|-----|------------------------------------------------------|
| 0   | GlulxVersion | 0   | 0x00030102 (spec 3.1.2)                              |
| 1   | TerpVersion  | 1   | 0x00000100 (this terp, v0.1.0)                       |
| 2   | ResizeMem    | 2   | 1 (setmemsize supported)                            |
| 3   | Undo         | 3   | 1 (saveundo/restoreundo — 2c)                       |
| 4   | IOSystem     | 4   | 1 if L2 ∈ {0 null, 1 filter, 2 Glk}, else 0         |
| 5   | Unicode      | 5   | 1                                                    |
| 6   | MemCopy      | 6   | 1 (mzero/mcopy)                                      |
| 7   | MAlloc       | 7   | 1 (malloc/mfree)                                     |
| 8   | MAllocHeap   | 8   | heap-start address (0 if the heap is inactive)      |
| 9   | Acceleration | 9   | 1 (interception implemented for 13 well-known functions) |
| 10  | AccelFunc    | 10  | 1 for function numbers 1–13, else 0 (0 is also the accelfunc cancel sentinel) |
| 11  | Float        | 11  | 1 (single-precision implemented; double-precision Sel 12 deferred → 0) |

The null (0), filter (1), and Glk (2) I/O systems are all implemented and
reported supported.

## 14. Save / restore serialization (Phase 2c, spec §1.8)

The save state is a Glulx-Quetzal `FORM IFZS` container (IFF: a 4-byte type,
4-byte big-endian length, then chunks, each `id(4) · len(4) · data · even-pad`).
`Machine::save_state()` produces these bytes; `restore_state()` consumes them and
returns `GError::BadSave` on any corruption (never a panic).

| Chunk  | Contents                                                              |
|--------|----------------------------------------------------------------------|
| `IFhd` | the first 128 bytes of memory (identity / game-file header)          |
| `CMem` | 4-byte current memsize, then the RLE-compressed RAM diff             |
| `Stks` | the live stack bytes `[0, sp)`, big-endian, padding included         |
| `MAll` | heap-start (4), block count (4), then `(addr, len)` per block        |
| `GReg` | sp, fp, pc, iosys_mode, iosys_rock, cur_stringtbl, protect_addr, protect_len (8×u32) |
| `Glk ` | the Glk window/stream model (window tree + streams + current state); see §20 |

**`CMem` compression (spec §1.8 / Quetzal RLE):** the memory area saved is
`[RAMSTART, memsize)`. Each byte is XORed against the **original loaded image**
(extended with zeros at/above EXTSTART). The resulting diff stream is
run-length-encoded: a non-zero byte is literal; a run of 1..=256 zero bytes is a
`0x00` byte followed by `(count − 1)`. Restore reverses this: reset RAM to the
original image, then apply the diff (so bytes absent from the save return to
their load-time values).

**A foreign writer may end the `CMem` stream early, and that is not corruption
(SQ-1415).** glulxe's own writer (`serial.c` `write_memstate`) omits the
trailing zero run once every remaining byte is unchanged from the original
image — "it's possible we've got a run left over, but we don't write it" — and
its reader treats running out of stream as "the final, unstored run", filling
the rest with zero diff. `decompress_ram` does the same: exhausting the `CMem`
bytes before `memsize` fills `[addr, memsize)` from the original image rather
than returning `BadSave`. Only a *decoded* run whose length would write past
`memsize` is still rejected — glulxe's writer can never produce one, so that
shape means a corrupted or hostile file. This is what makes
`tests/fixtures/startsavetest.gblorb` (glulxe's own save/restore unit-test
game, `tests/startsavetest_boots.rs`) and Counterfeit Monkey's shipped
resource `9998` boot save restore instead of being rejected.

**`Stks` (spec §1.8 / §1.3.1):** the stack is one byte-addressed buffer already
in the spec's call-frame layout, so the chunk is simply `stack[0..sp]`. We store
sp/fp/pc explicitly in `GReg` rather than deriving them from a top-of-stack call
stub (a real Quetzal reader's job); `GReg` is this implementation's extension to
keep `save_state`/`restore_state` self-contained for headless testing.

**Restore order:** parse chunks → decompress `CMem`/`UMem` (reset+diff), SKIPPING
any byte inside the *live* protect range → load the stack and registers from
`Stks`/`GReg` → rebuild the heap from `MAll` → recompute the frame cache. The
**protected range** (§16) is preserved across restore: bytes in the current
protect range keep their pre-restore values, because `decompress_ram`/
`load_umem` simply never write them (SQ-1415: no longer a snapshot-then-
reimpose over a materialised `Vec` of every protected byte, which for a
hostile multi-GiB `@protect` range was itself an unbounded allocation).

Per the spec, an interpreter's Glk state, RNG internal state, protect range, and
I/O-system/string-table settings are not part of a real Quetzal *file*; our
internal snapshot additionally carries iosys/string-table/protect in `GReg` so
that **`restore_state`** (the host Save State path) restores the full VM state
exactly, including the saved protect range. **`saveundo`/`restoreundo` (§15) are
different: per spec §2.16 the protect range is explicitly not part of the saved
undo state**, so `restoreundo` puts the LIVE range (as it stood right before the
undo) back after the shared restore core runs, rather than trusting the
snapshot's `GReg` the way `restore_state` does (SQ-1415).

**A second, spec-conformant standard serializer (`@save`/`@restore`, SQ-0283).**
`save_state`/`restore_state` above back the host **Save State** (Layer 2, a
cold cross-session snapshot into a fresh `Machine`). The game's own
`@save`/`@restore` (opcodes 0x0123/0x0124) instead use a second pair,
`save_quetzal()`/`restore_quetzal()`, that produces a real, portable, standard
Glulx-Quetzal save — the same shape as the Z-machine's `.qzl`:

- **`save_quetzal()`** emits `FORM IFZS` with **`IFhd`/`CMem`/`Stks`/`MAll`
  only** — no `GReg`, no `Glk `. Per §1.8, PC/FP/SP are not serialized as
  registers at all: `@save` pushes a 4-word call stub (`DestType, DestAddr,
  PC, FramePtr` — §1.3.2, reusing `call_function`'s stub machinery, exec.rs
  ~1134) for its `S1` before suspending, so the resume point self-describes
  from the saved stack instead.
- **`restore_quetzal(blob)`** restores RAM/stack/heap through the same shared
  core as `restore_state` (`restore_vm_core`, honoring the live protect
  range; accepts `UMem` as well as `CMem`), then **pops that call stub back
  off the restored stack and stores `-1`** into it — the "just restored"
  sentinel, spec §2.9 — to resume execution just after the original `@save`.
  Per **§1.8.5 it leaves every other piece of live interpreter state
  untouched**: the Glk model (windows/streams/VFS), `iosys_mode`/`iosys_rock`,
  the current string-decoding table, and the protect range keep their live
  values, rather than being replaced from `GReg`/`Glk ` the way
  `restore_state` does.

**Why two serializers.** `save_state` targets a *cold* restore into a fresh
`Machine` with no live Glk model to inherit, so it must be self-contained —
hence `GReg` (registers) and the `Glk ` chunk (the whole window/stream/VFS
tree). `save_quetzal` is a *mid-session* in-game save: it always resumes
inside the same running interpreter via the call stub it pushed, so PC/FP
recover from the stack and every other piece of live state is already
correct and must NOT be reset out from under the game.

**The "Save failed." verification (SQ-0292).** `@save` delivers its bytes to
the host, not to the game's own output stream `L1` — so the Glk library's
post-save verification on `L1` would otherwise conclude the save is empty and
report failure. `@save` calls `glk.note_stream_write(l[0], save_quetzal().len())`,
which (a) bumps the stream's write count without storing bytes and (b) records
the slot's byte length keyed by fileref name. That length + a create-mode open
marking existence let `glk_fileref_does_file_exist` return true and a
reopen-for-read + seek-to-end report the true save size — the two checks CM's
SAVE verb runs after `@save` (`fileref_exists` / `stream_saveload_info` /
`saved_game_files` in `glk.rs`). A `fileusage_SavedGame` stream is a
`StreamKind::Null` conduit (writes discarded, reads EOF, no VFS entry) — so
`@save`/`@restore` never depend on, or pollute, the story's Glk file VFS (§20).

**Routing by fileref creation method (SQ-0296).** `@save`/`@restore` capture
their `L1` stream's fileref `{name, by_prompt}` via `glk.stream_saveload_info`
(covering both `SavedGame` `Null` streams and `Data`-usage VFS `File` streams —
a game may save to either; CM's init cache is the latter) and stash them on the
suspended `PendingSaveLoad`. The host reads them through
`Machine::pending_saveload_request() -> SaveLoadRequest { name, by_prompt,
restore }`. `by_prompt` is `true` only for a `glk_fileref_create_by_prompt`
fileref (the player's SAVE/RESTORE verb — bound with `fileref_create_prompted`
in `supply_filename`); `create_by_name`/`create_by_usage`/`create_temp` are
`false`. Hosts service `by_prompt = false` silently against a fixed
`<game-dir>/<name>.qzl` (no UI); `by_prompt = true` surfaces the save UI. The
flag is session-transient (defaults `false` on a Glk-snapshot restore) and adds
no `StepResult` variant (it stays `Copy`).

## 15. Undo (Phase 2c, spec §2.11)

| Opcode      | Num   | L | S | Effect                                            |
|-------------|-------|---|---|---------------------------------------------------|
| saveundo    | 0x125 | 0 | 1 | snapshot state; store 0 (success), 1 (fail), or −1 (after restore) |
| restoreundo | 0x126 | 0 | 1 | restore the newest snapshot; store 1 on failure   |

These use the §14 core, entirely in memory (no Glk streams). The
**destination-write convention** matches `@save`/`@restore` (spec §2.11):

- Before snapshotting, `saveundo` pushes a four-value **call stub** (DestType,
  DestAddr of S1; the resume PC; the FramePtr) so the snapshot records where the
  result must go. It then pops the stub and stores **0** in S1 (normal success).
- `restoreundo` pops the newest snapshot, restores it (the stub is back on the
  stack), pops that stub, and stores **−1** (`0xFFFFFFFF`) at the *original
  saveundo's* destination — i.e. the saved `saveundo` "returns again" with −1, so
  the game can branch (continue vs. just-restored). `restoreundo` itself stores
  nothing on success.
- With no snapshot, `restoreundo` stores **1** (failure) and leaves state intact.

The undo stack is bounded to `UNDO_CAP` (16) snapshots; the oldest is dropped
when full. (`@save`/`@restore` to a real file are now implemented — see §14's
`save_quetzal`/`restore_quetzal` — and `hasundo`/`discardundo` are implemented too.)

## 16. protect (Phase 2c, spec §2.11)

| Opcode  | Num   | L | S | Effect                                              |
|---------|-------|---|---|-----------------------------------------------------|
| protect | 0x127 | 2 | 0 | preserve RAM `[L1, L1+L2)` across restore/restoreundo; `L2 == 0` clears |

The protected range `(addr, len)` lives on the `Machine`, stored as `(start,
len)` rather than `(start, end)` — glulxe's own `op_protect` (exec.c) computes
`end = start + len` and validates nothing, so a hostile range is ordinary
input there too, and every reader of the range here (`decompress_ram`/
`load_umem`/`reset_ram`) computes its end with `saturating_add` rather than
trusting one (SQ-1415). During restore (§14), `decompress_ram`/`load_umem`
simply never write a byte inside the live protected range, so it keeps its
**current** value rather than the restored image's — a live skip, not a
snapshot-then-reimpose, precisely so a hostile multi-GiB range is never
materialised into a `Vec`. `protect(_, 0)` clears protection.

**`restart` and `restoreundo` treat the protect range oppositely, and both
match the spec (SQ-1415):**

- **`@restart` (spec §2.9) honors it and does NOT reset it.** `reset_ram`
  (memory.rs) reloads `[RAMSTART, EXTSTART)` from the original image and
  zeroes `[EXTSTART, ENDMEM)` while skipping `[protectstart, protectend)`,
  mirroring glulxe's `vm_restart` (`vm.c`) exactly — it reloads the game file
  byte-by-byte, `if (lx >= protectstart && lx < protectend) continue;`, and
  its own comment says "we do not reset the protection range". `op_restart`
  leaves `self.protect` untouched (previously it zeroed it, which the spec
  never asked for).
- **`saveundo`/`restoreundo` (§15) do NOT honor it — per spec §2.16 the
  protect range is explicitly not part of the saved undo state.** Our
  internal snapshot format (`GReg`) DOES carry the range (so **`restore_state`**,
  the unrelated host Save State path, can restore it exactly), but
  `restoreundo` captures the LIVE range before calling the shared restore
  core and puts it back afterward, rather than trusting what `GReg` says —
  otherwise an undo would silently reinstate whatever range was live when
  `saveundo` ran, which is exactly the "part of saved state" the spec denies.

## 17. Acceleration (Phase 2c, spec §2.18 / §1.4)

| Opcode     | Num   | L | S | Effect                                              |
|------------|-------|---|---|-----------------------------------------------------|
| accelfunc  | 0x180 | 2 | 0 | assign accelerated-function number L1 to the VM function at address L2 (L1 == 0 cancels) |
| accelparam | 0x181 | 2 | 0 | store value L2 in the accel parameter table at index L1 |

Acceleration is a pure **speed optimization**; Inform 7 games run correctly
without it. `accelfunc`/`accelparam` **store** the assignments (`accel_funcs`:
address → number) and parameters (`accel_params`: index → value), and the 13
well-known functions (numbers 1–13: `Z__Region`, `CP__Tab`, `RA__Pr`, `RL__Pr`,
`OC__Cl`, `RV__Pr`, `OP__Pr`, in both V1 and V2 parameter-offset variants) are
now **intercepted and executed natively**, bypassing normal frame construction
and opcode dispatch entirely. Interception happens at the two call choke
points — `call_function` and `op_tailcall` — so it applies uniformly whether a
game calls an accelerated function directly or tail-calls into one. This is
behaviorally transparent (the transcript is byte-identical with acceleration on
or off, including the three functions' `[** Programming error: … **]`
diagnostics on malformed input — `accel_error` writes them through the current
Glk stream, matching glulxe's `accel.c`, and only when the I/O system is Glk;
SQ-1416) and is **on by default**, with an `--accel on|off` flag (`gvm-cli` and the
app) as an escape hatch for diagnosing any mismatch. On CounterfeitMonkey-11,
acceleration cuts the dispatched-opcode count from init to the first prompt by
roughly 7.9× (23.78M → 3.00M). Accordingly the `Acceleration` (9) and
`AccelFunc` (10) gestalt selectors report support. The stored assignments
remain readable via `Machine::accel_func_for` / `accel_param`.

## 18. PRNG (Phase 2c, spec §2.14)

| Opcode    | Num   | L | S | Effect                                              |
|-----------|-------|---|---|-----------------------------------------------------|
| random    | 0x110 | 1 | 1 | `[0, L1)` if L1 > 0; `(L1, 0]` if L1 < 0; any 32-bit value if L1 == 0 |
| setrandom | 0x111 | 1 | 0 | seed the generator with L1 (L1 == 0 → reseed)       |

A xorshift32 generator on the `Machine`. `random(L1)`: for `L1 > 0` return
`next() mod L1`; for `L1 < 0` return `-(next() mod |L1|)` (values from `L1+1` to
0); for `L1 == 0` return the raw 32-bit value. A fixed seed yields a fully
reproducible sequence.

`setrandom(0)` reseeds from true entropy, as the spec requires. No dependency is
involved: `Machine::entropy_seed` draws from std's `RandomState` (the OS-seeded
hasher `HashMap` uses), and coerces a zero result to `DEFAULT_SEED` (xorshift32
is absorbing at zero).

`DEFAULT_SEED` is the seed a bare `Machine` starts from — fixed, so an unseeded
machine is reproducible and the unit tests here mean something. Handing a PLAYER
that same sequence on every launch is a bug (SQ-0811), so the host seeds the
machine before the boot drive with `Machine::set_rng_seed`: lanthorn uses the
`random_seed` config key when the user pinned one, and an entropy draw when they
did not. `restart` deliberately leaves the state alone rather than snapping back
to `DEFAULT_SEED`, so a restarted roguelike is a new game and not the last one.

## 19. Glk I/O model (Glulx sub-project 3a, Glk spec 0.7.5)

The Glulx `@glk` opcode (0x130) dispatches to the Glk library: a window/stream/
event model. We implement the **interactive-fiction subset** in `glk.rs` (the
`Model` — window tree, streams, current stream, per-stream style) plus a
pluggable `GlkBackend` display trait. The `Output`/`BufferOutput` placeholder is
gone; printing routes **current stream → window → backend** (or → Glulx memory,
for a memory stream). Phase 3a-1 is **output-only** (input events + `glk_select`
suspend/resume are 3a-2). All constant values below are from `glk.h`.

### Window types (`wintype`, the glk_window_open argument)

| Type             | Value | Notes                                  |
|------------------|-------|----------------------------------------|
| wintype_Pair     | 1     | internal layout node (split-created)   |
| wintype_Blank    | 2     | real window: no text, no output, `glk_window_get_size` always (0,0) — pure layout filler (SQ-1416) |
| wintype_TextBuffer | 3   | scrolling main window                  |
| wintype_TextGrid | 4     | fixed character grid / status window   |
| wintype_Graphics | 5     | out of scope                           |

### Split methods (`winmethod`, bitfield)

Direction (`winmethod_DirMask` = 0x0f): Left 0x00, Right 0x01, Above 0x02,
Below 0x03. Division (`winmethod_DivisionMask` = 0xf0): Fixed 0x10
(`size` = a character count), Proportional 0x20 (`size` = percent 0–100).
Border 0x000 / NoBorder 0x100 (ignored). On `glk_window_open(split, method,
size, wintype, rock)` a new **Pair** node replaces `split` in the tree, with
`split` and the new window as its children; the new window is the **key** window
and gets `size` units on the side named by the direction; the old window gets the
rest. An oversized request collapses the old window to zero (spec §3.3 — no
ancestor renegotiation). The root window fills the screen; the model computes
child rects top-down (`Model::relayout`), and the backend supplies the screen
size (`GlkBackend::screen_size`).

### Style classes (`style_*`, 0–10; `style_NUMSTYLES` = 11)

Normal 0, Emphasized 1, Preformatted 2, Header 3, Subheader 4, Alert 5, Note 6,
BlockQuote 7, Input 8, User1 9, User2 10. A style is carried on the **stream**
(`glk_set_style` sets the current stream's style); every `put_text`/`grid_put`
is tagged with it. The backend maps classes → display attributes (SGR in the CLI).

### gestalt selectors (the `glk_gestalt` selector, distinct from the Glulx
### `gestalt` opcode)

Version 0, CharInput 1, LineInput 2, CharOutput 3 (returns CannotPrint 0 /
ApproxPrint 1 / ExactPrint 2), MouseInput 4, Timer 5, Graphics 6, Unicode 15,
LineInputEcho 17, LineTerminators 18, DrawImageScale 24, … We report `Version`
= 0x00000705 (0.7.5, not 0.7.6: `0x00EC glk_image_draw_scaled_ext` is not
implemented — its `imagerule_WidthRatio` in a `wintype_TextBuffer` window is
dynamic, re-resolved on every window resize, not a one-shot draw — so
`DrawImageScale` truthfully answers unsupported too; SQ-1416), `CharInput` = 1,
`LineInput` = 1, `CharOutput` answered per code point from
`GlkBackend::char_output_gestalt` (default: CannotPrint for the eight-bit
control ranges the spec names, ExactPrint for the rest of Latin-1, ApproxPrint
beyond that — not a blanket ExactPrint; SQ-1416), `Unicode` = 1, and **0** for
mouse/timer/graphics/sound/hyperlinks/echo/terminators (truthful: not
supported).

### Dispatch selector codes implemented (output subset; from `gi_dispa.c`)

| Selector | Code | Selector | Code |
|----------|------|----------|------|
| glk_exit | 0x0001 | glk_gestalt | 0x0004 |
| glk_gestalt_ext | 0x0005 | glk_window_iterate | 0x0020 |
| glk_window_get_rock | 0x0021 | glk_window_get_root | 0x0022 |
| glk_window_open | 0x0023 | glk_window_close | 0x0024 |
| glk_window_get_size | 0x0025 | glk_window_set_arrangement | 0x0026 |
| glk_window_get_arrangement | 0x0027 | glk_window_get_type | 0x0028 |
| glk_window_get_parent | 0x0029 | glk_window_clear | 0x002A |
| glk_window_move_cursor | 0x002B | glk_window_get_stream | 0x002C |
| glk_set_window | 0x002F | glk_window_get_sibling | 0x0030 |
| glk_stream_iterate | 0x0040 | glk_stream_get_rock | 0x0041 |
| glk_stream_open_memory | 0x0043 | glk_stream_close | 0x0044 |
| glk_stream_set_position | 0x0045 | glk_stream_get_position | 0x0046 |
| glk_stream_set_current | 0x0047 | glk_stream_get_current | 0x0048 |
| glk_put_char | 0x0080 | glk_put_char_stream | 0x0081 |
| glk_put_string | 0x0082 | glk_put_string_stream | 0x0083 |
| glk_put_buffer | 0x0084 | glk_put_buffer_stream | 0x0085 |
| glk_set_style | 0x0086 | glk_set_style_stream | 0x0087 |
| glk_stylehint_set | 0x00B0 | glk_stylehint_clear | 0x00B1 |
| glk_put_char_uni | 0x0128 | glk_put_string_uni | 0x0129 |
| glk_put_buffer_uni | 0x012A | glk_stream_open_memory_uni | 0x0139 |

### Output routing

`@glk` put selectors write to the current stream (or an explicit stream for the
`_stream` variants), **independent** of the VM's I/O system. The Glulx
`streamchar`/`streamnum`/`streamstr` opcodes (§7) emit through the **current Glk
stream** only under I/O system 2 (Glk). A window stream → `put_text` (text
buffer) or grid-cursor writes with wrap (text grid) via the backend; a memory
stream → Glulx main memory at `addr + pos·elsize` (1 byte, or 4 for a Unicode
stream), advancing `pos` and the write count; a null/invalid/zero stream is
discarded. Nothing here panics on a bad window/stream id.

### Input events + `glk_select` (3a-2)

The IF input subset: **line** and **character** input, delivered through
`glk_select`. The model mirrors `zvm`'s `NeedLine`/`NeedChar` suspend/resume.

**The `event_t` struct** (4 `glui32` words, big-endian, written at the address
passed to `glk_select`):

| Offset | Field | Line input        | Char input          |
|--------|-------|-------------------|---------------------|
| +0     | type  | `evtype_LineInput` 3 | `evtype_CharInput` 2 |
| +4     | win   | the window id     | the window id       |
| +8     | val1  | chars entered     | the key code        |
| +12    | val2  | 0 (terminator)    | 0                   |

`evtype_*`: None 0, Timer 1, CharInput 2, LineInput 3, MouseInput 4, Arrange 5,
Redraw 6, SoundNotify 7, Hyperlink 8.

**Special keycodes** (`keycode_*`, from glk.h) occupy the top of the `glui32`
range, `keycode_Func12` `0xffffffe4` … `keycode_Unknown` `0xffffffff`
(`keycode_MAXVAL` = 28): Unknown ffffffff, Left fffffffe, Right fffffffd, Up
fffffffc, Down fffffffb, Return fffffffa, Delete fffffff9, Escape fffffff8, Tab
fffffff7, PageUp fffffff6, PageDown fffffff5, Home fffffff4, End fffffff3,
Func1 ffffffef … Func12 ffffffe4.

**Request selectors:** `glk_request_line_event` 0x00D0 (`win, buf, maxlen,
initlen`), `glk_request_line_event_uni` 0x0141, `glk_request_char_event`
0x00D2 (`win`), `glk_request_char_event_uni` 0x0140, `glk_select` 0x00C0
(`event`). A request is recorded on the window (line and char are mutually
exclusive per window). `glk_select` then: (1) delivers any queued non-input
event (arrange) first; else (2) **suspends** on the first window with a pending
request — `step()` returns `NeedLine{win}` / `NeedChar{win, unicode}` — until the
host calls `supply_line(text)` / `supply_char(key)`, which writes the buffer +
`event_t` and resumes; else (3) with nothing to wait for, writes `evtype_None`
and continues (a malformed program would otherwise deadlock).

`supply_line` writes `text` into the request buffer (truncated to `maxlen`;
Latin-1 bytes, or 32-bit words for `_uni`) and sets `val1` to the char count.
`supply_char` maps the key for a non-Unicode request: a Latin-1 code (≤ 0xff) or
a special keycode passes through; any other code point becomes `keycode_Unknown`.
A `_uni` char request passes the full code point. The model does **not** echo
line input — like a stdio Glk, the display backend/terminal handles echo.

**Cancel + other event selectors:** `glk_cancel_line_event` 0x00D1 (drops the
request, reports `evtype_LineInput` with the `initlen` chars already in the
buffer, else `evtype_None`), `glk_cancel_char_event` 0x00D3 (drops the request),
`glk_select_poll` 0x00C1 (never suspends; returns the first queued Timer/
Arrange/Redraw/SoundNotify/VolumeNotify event, skipping over — and leaving
queued — any Char/Line/Mouse/Hyperlink ahead of it, per Glk spec §4.2's "does
not check for or return evtype_CharInput, evtype_LineInput, or
evtype_MouseInput"; Hyperlink is excluded the same way, a per-window
player-input request like Mouse; SQ-1416). **Arrange:** `glk_window_set_arrangement`
queues an `evtype_Arrange` (win 0), and so does `deliver_arrange` when nothing
is currently blocked on a select (SQ-1416: it used to drop the event on the
floor instead, unlike its sound/timer/mouse/hyperlink siblings); `glk_select`
delivers any queued non-input event before suspending for input. A suspended
`glk_select`'s S1 (always 0) is stored only once the event is actually
delivered on resume, AFTER any stack-pushed event words — Glulx spec §2.18:
"Stack output references are pushed after the Glk call, but before the S1
result value is stored" (SQ-1416 item 3; `PendingInput`/`PendingEvent` carry
the deferred store target the same way `PendingFileref` already did for
`glk_fileref_create_by_prompt`). **Diagnosed no-ops (out of scope):**
`glk_request_timer_events` 0x00D6, `glk_request_mouse_event` 0x00D4,
`glk_cancel_mouse_event` 0x00D5. **Accepted best-effort:**
`glk_set_echo_line_event` 0x0150, `glk_set_terminators_line_event` 0x0151.

### Unicode case + the dispatch reference convention (glulxercise conformance)

The parser path (and glulxercise) needs three more pieces, all transcribed from
the Glulx Glk dispatch (gi_dispa) and glk.h:

- **Character case:** `glk_char_to_lower` 0x00A0 / `glk_char_to_upper` 0x00A1
  (Latin-1 case folding), and the in-place Unicode buffer folds
  `glk_buffer_to_lower_case_uni` 0x0120 / `_upper_` 0x0121 / `_title_` 0x0122
  (a fold may change the length; the result is clamped to the buffer and the full
  length returned).
- **The dispatch -1 reference convention.** A Glk output reference/struct pointer
  argument is **not** a plain address: `0` is a C NULL (discard the result);
  **`0xFFFFFFFF` (-1) means "use the VM stack"** — the output value(s) are
  **pushed** (last field on top), and the game pops them back. Otherwise the value
  is written to memory at the address. This applies to `glk_stream_close` /
  `glk_window_close` (push `readcount` then `writecount`), `glk_window_get_size`,
  `glk_window_get_arrangement`, the `*_iterate` rocks, and the `event_t*` of
  `glk_select`/`glk_select_poll`. Inform's veneer (`PrintAnyToArray`) relies on
  this to read a memory stream's write count without a stat buffer.
- **The same -1 convention has an INPUT direction too** (SQ-1416 item 2): a
  reference to a Glk INPUT structure at `-1` is popped off the stack instead of
  read from memory, field 0 topmost (Glulx spec §2.18: "an input structure is
  popped off first-topmost" — the mirror image of the output rule above, and
  matching cheapglk `glkop.c`'s `ReadStructField` macro, which pops once per
  field in increasing field-index order). `read_timeval`/`read_glkdate` (used
  by the six §2.10 date/time selectors `0x0168`, `0x0169`, `0x016C`–`0x016F`)
  honor it; they used to always read from memory even when handed `-1`.
- **Fileref name simplification** (`Model::sanitize_fileref_name`, SQ-1416 item
  6): a `glk_fileref_create_by_name`/`_by_prompt` name is simplified per the
  Glk spec's recommended rule (cheapglk `cgfref.c`'s comment on
  `glk_fileref_create_by_name`) — delete `" \ / > < : | ? *`, keep only the
  part before the first `.`, `"null"` if that leaves nothing, then append the
  usage's suffix (`.glkdata` Data, `.glksave` SavedGame, `.txt`
  Transcript/InputRecord). This is what other Glk interpreters do, so a file a
  Glulx story writes exchanges with them; it also means a `SavedGame` fileref's
  on-disk name now carries a `.glksave` suffix it did not before.

### Core opcodes completed alongside (Glulx spec §2)

`jumpabs` 0x104 (jump to an absolute address) and the exception pair `catch`
0x32 / `throw` 0x33 (a catch stub — DestType/DestAddr/PC/FramePtr, like a call
stub — records a token = the stack pointer; `throw value token` unwinds the stack
to the token and resumes at the catch with `value` stored to its destination).

### glulxercise capstone

`gvm-cli/tests/glulxercise.rs` drives the vendored `glulxercise.ulx` headlessly
and asserts the in-scope groups pass — now including `filter`/`nullio`/`iosys2`/
`iosys3`/`gestalt` (filter I/O system implemented) and `gidispa` (SQ-0251: the
Glk dispatch layer's output-argument marshalling — `glk_put_string`/
`glk_put_string_stream`/`glk_put_string_uni` decode the type-tagged Inform string
object they are handed instead of streaming the tag byte: E0/E2 directly, and a
compressed `E1` object via the string machinery, capturing its decoded output
(including embedded string/function nodes) and writing it straight to the Glk
stream (SQ-0252). A failed E1 decode (e.g. no string table set) records a
diagnostic and skips rather than faulting the VM). Out of scope: double-precision float.
(`acceleration` and single-float are implemented; the Glulxercise groups for them
just aren't in the assertion list. The gi_dispatch *introspection* API is
unreachable from Glulx bytecode and unimplemented, but `gidispa` doesn't use it.)

### Deferred / out of scope

Filerefs/file streams (`@save`/`@restore` via Glk), echo streams, timers,
hyperlinks, mouse, graphics, and sound remain out of scope.

## 20. The `Glk ` save chunk (Glk model snapshot)

The Glulx save core (§14) serializes RAM/stack/heap/registers but not the Glk
display state. Without it, restoring a snapshot into a **fresh** `Machine`
(a new session) would leave the VM holding window/stream ids that no longer
exist. `save_state` therefore appends a `Glk ` chunk — `Model::serialize()` —
and `restore_state` reinstalls the model from it via `Model::deserialize()`.

**Body format** (all fields 32-bit big-endian, like the rest of the save):

```
version (=1) · root · cur_stream
nwindows · per slot: present(0|1); if present:
    id · wintype · rock · parent · stream · child1 · child2 · key · method · size
    rect(left,top,width,height) · grid(width,height,cx,cy)
    line_req: present(0|1)[ · buf · maxlen · initlen · unicode ]
    char_req: present(0|1)[ · unicode ]
nstreams · per slot: present(0|1); if present:
    id · rock · style · read_count · write_count
    kind: 0=Window( win ) | 1=Memory( addr · len · pos · unicode )
```

The window/stream slot vectors are emitted in full (`None` slots included) so
ids — the `id - 1` index into each vector — survive the round-trip and the next
`glk_window_open`/stream allocation keeps numbering where it left off.

**What is and isn't stored.** The chunk carries window *structure* (the tree:
types, rocks, pair splits, geometry), text-grid *dimensions + cursor*, the
streams (memory stream addr/len/position/rock/window association, styles, I/O
counts), and `root` + `cur_stream`. It does **not** store rendered cell glyphs
or text-buffer scrollback: those live in the pluggable `GlkBackend`, not the
`Model`, and the host re-renders them on restore (the app's transcript persists
the buffer history; the grid is repainted from the game's next status redraw).
The transient `glk_select` event queue is not stored either.

**Routing after restore.** Because the stream table and `cur_stream` are
restored, the output funnel (`glk_stream_put`) resolves the current stream to
the correct window id immediately — a post-restore `glk_put_*` or text-grid op
lands in the right window with no replay.

**Back-compat.** A snapshot without a `Glk ` chunk (an older gvm save) restores
with an empty `Model` and returns `Ok` — no panic. Malformed chunk bytes
(truncation, a bad window/stream tag, trailing garbage) yield `GError::BadSave`;
a corrupt slot count cannot trigger a large allocation (slots are pushed, not
pre-reserved, so the reader runs out of bytes first).
