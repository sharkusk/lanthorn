# Third-Party Notices

This document lists the permissively-licensed open-source projects from which lanthorn has derived code or data structures.

## glulxe

**Project:** glulxe  
**Copyright:** © 1999–2023, Andrew Plotkin  
**License:** MIT  
**URL:** https://github.com/erkyrath/glulxe  
**Commit read:** Read from master branch, 2026-09-09 (Bocfel migration audit SQ-1444)  
**Lanthorn files:**
- `crates/lanthorn-gvm/src/exec.rs` — floating-point opcodes (`op_fmod`, `op_dmodr`, `op_dmodq`, `op_ftonumz`, `op_dtonumz`, `op_dtonumn`, `glulx_powf` wrapper)

### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

## Bocfel

**Project:** Bocfel  
**Copyright:** © 2009–2025, Chris Spiegel  
**License:** MIT (v2.2.3 onward, as of 2025-02-01)  
**URL:** https://github.com/cspiegel/bocfel  
**Commit read:** Read from garglk/terps/bocfel at master branch, 2026-09-09 (Bocfel migration audit SQ-1444)  
**Lanthorn files:**
- `crates/lanthorn-zvm/src/cpu/exec.rs` (`font3_translate` function, lines 6672–6745) — Z-Machine Standard §16 font table mappings (derived from `garglk/terps/bocfel/unicode.cpp`, function `build_zscii_to_character_graphics_table`)
- `crates/lanthorn-zvm/src/text/encode.rs` (lines 120–148) — shift-lock encoding rule for version 1–2 text compression (derived from `garglk/terps/bocfel/dict.cpp`)

### MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
