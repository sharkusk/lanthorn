//! Structured crash stack trace (zero-dep value type + text formatter).

/// A single call frame captured at a fault. Innermost (faulting) frame first
/// in `StackTrace::frames`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TraceFrame {
    /// Routine entry address (0 = unknown; gvm always reports 0).
    pub func_addr: u32,
    /// PC to resume in the caller.
    pub return_pc: u32,
    /// Local variables, widened to i64.
    pub locals: Vec<i64>,
    /// This frame's working eval-stack values, widened to i64.
    pub operands: Vec<i64>,
}

/// A crash stack trace: the fault site plus the live call stack.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StackTrace {
    /// Human-readable fault, e.g. "memory fault: read16 @0x004a1c".
    pub fault: String,
    /// Start-PC of the faulting instruction.
    pub fault_pc: u32,
    /// Decoded mnemonic of the faulting instruction, e.g. "loadw".
    pub fault_op: String,
    /// Hex render width in bytes: 2 = u16 (zvm), 4 = u32 (gvm).
    pub width: u8,
    /// Call frames, innermost (faulting) first.
    pub frames: Vec<TraceFrame>,
}

impl StackTrace {
    /// Canonical multi-line text form, shared by every host surface. One string
    /// per line, no trailing newlines.
    pub fn to_lines(&self) -> Vec<String> {
        let digits = (self.width as usize) * 2;
        let hexw = |v: i64| format!("0x{:0width$x}", v as u64 & mask(self.width), width = digits);
        let list = |xs: &[i64]| {
            xs.iter().map(|&v| hexw(v)).collect::<Vec<_>>().join(",")
        };
        let mut out = vec![
            "*** VM FAULT ***".to_string(),
            self.fault.clone(),
            format!("PC=0x{:06x}  op={}", self.fault_pc, self.fault_op),
        ];
        for (i, f) in self.frames.iter().enumerate() {
            let mut line = format!(
                "  #{i}  fn@0x{:06x}  ret=0x{:06x}  locals=[{}]",
                f.func_addr,
                f.return_pc,
                list(&f.locals),
            );
            if !f.operands.is_empty() {
                line.push_str(&format!("  stack=[{}]", list(&f.operands)));
            }
            out.push(line);
        }
        out
    }
}

fn mask(width: u8) -> u64 {
    match width {
        2 => 0xFFFF,
        4 => 0xFFFF_FFFF,
        _ => u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_lines_formats_header_pc_and_frames() {
        let t = StackTrace {
            fault: "memory fault: read16 @0x004a1c".to_string(),
            fault_pc: 0x004a1c,
            fault_op: "loadw".to_string(),
            width: 2,
            frames: vec![
                TraceFrame { func_addr: 0x4980, return_pc: 0x4a20, locals: vec![1, 0, 0xffff], operands: vec![0x2a] },
                TraceFrame { func_addr: 0x1200, return_pc: 0x32f0, locals: vec![], operands: vec![] },
            ],
        };
        let lines = t.to_lines();
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert_eq!(lines[1], "memory fault: read16 @0x004a1c");
        assert_eq!(lines[2], "PC=0x004a1c  op=loadw");
        // width=2 → 4 hex digits per value; operands present → stack=[..]
        assert_eq!(lines[3], "  #0  fn@0x004980  ret=0x004a20  locals=[0x0001,0x0000,0xffff]  stack=[0x002a]");
        // empty locals + empty operands → no stack=[]
        assert_eq!(lines[4], "  #1  fn@0x001200  ret=0x0032f0  locals=[]");
    }
}
