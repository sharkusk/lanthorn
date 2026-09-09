# Third-Party Notices

This document lists the permissively-licensed open-source projects from which lanthorn has derived code or data structures.

## glulxe

**Project:** glulxe  
**Copyright:** © 1999–2023, Andrew Plotkin  
**License:** MIT  
**URL:** https://github.com/erkyrath/glulxe  
**Commit read:** Read from master branch, 2026-09-09 (Bocfel migration audit SQ-1444)  
**Lanthorn files:**
- `crates/gvm/src/exec.rs` — floating-point opcodes (`op_fmod`, `op_dmodr`, `op_dmodq`, `op_ftonumz`, `op_dtonumz`, `op_dtonumn`, `glulx_powf` wrapper)

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

## ttyd

**Project:** ttyd  
**Copyright:** © 2016–2026, Shuanglei Tao  
**License:** MIT  
**URL:** https://github.com/tsl0922/ttyd  
**Commit read:** ttyd 1.7.7, commit reference from https://github.com/tsl0922/ttyd/releases/tag/1.7.7 (2026-09-09)  
**Lanthorn files:**
- `crates/app/tests/pty_stream/driver.rs` — hangup behavior (`hang_up` function, signalling process groups with `SIGHUP` on websocket drop)
- `crates/app/tests/suites/pty_hangup_autosave.rs` — test commentary on websocket hangup semantics
- `docs/internals/docker.md` — description of session survival across websocket drops

**Notes:** Lanthorn does not derive ttyd's code; the implementation uses standard POSIX system calls (`kill(-pid, SIGHUP)`). ttyd's 1.7.7 source was read to understand the specific behaviour that the test harness mirrors: when ttyd drops a websocket, it signals the process group with `SIGHUP`, allowing a session manager (dtach) to detach while the underlying process continues.

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
