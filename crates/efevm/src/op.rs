//! The instruction set: opcode numbers, mnemonics and operand shapes (SPEC.md section 5).
//!
//! Opcode numbers are permanent once released. Numbering scheme (hex):
//!
//! | range | group |
//! |---|---|
//! | 0x000-0x00F | control and misc (nop, unreachable, yield, ret, jmp, br_*, call*) |
//! | 0x010-0x01F | constants, mov, select, globals, memory.size/grow/copy/fill |
//! | 0x020-0x03F | i32 arithmetic (0x20-0x32) and comparisons (0x34-0x3D) |
//! | 0x040-0x05F | i64 arithmetic (0x40-0x52) and comparisons (0x54-0x5D) |
//! | 0x060-0x07F | plain loads (0x60-0x63) and stores (0x64-0x67) |
//! | 0x080-0x09F | narrow loads (0x80-0x89) and stores (0x90-0x94) |
//! | 0x0A0-0x0BF | f32 binary, unary, comparisons |
//! | 0x0C0-0x0DF | f64 binary, unary, comparisons |
//! | 0x0E0-0x0FF | integer width conversions and sign extensions |
//! | 0x100-0x10F | trunc (0x100-0x107), trunc_sat (0x108-0x10F) |
//! | 0x110-0x11F | convert, demote, promote, reinterpret |

/// Operand shape of an instruction; see `Inst` in `module.rs` for the field mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// nop, unreachable, yield
    None,
    /// memory.size d
    D,
    /// d, a: mov, unary ops, eqz, conversions, memory.grow
    DA,
    /// d, a, b: binary arithmetic and comparisons
    DAB,
    /// d, imm32
    ConstI32,
    /// d, imm64
    ConstI64,
    /// d, f32 bits
    ConstF32,
    /// d, f64 bits
    ConstF64,
    /// d, cond, a, b
    Select,
    /// d, base, off
    Load,
    /// src, base, off
    Store,
    /// three source registers (memory.copy, memory.fill)
    ABC,
    /// d, g
    GlobalGet,
    /// g, a
    GlobalSet,
    /// L
    Jmp,
    /// a, L
    Br,
    /// a, [L...], Ldefault
    BrTable,
    /// f, base
    Call,
    /// type, a, base
    CallIndirect,
    /// base
    Ret,
}

impl Shape {
    /// Human-readable operand list.
    pub fn operands(self) -> &'static str {
        match self {
            Shape::None => "",
            Shape::D => "d",
            Shape::DA => "d, a",
            Shape::DAB => "d, a, b",
            Shape::ConstI32 => "d, imm32",
            Shape::ConstI64 => "d, imm64",
            Shape::ConstF32 => "d, f32",
            Shape::ConstF64 => "d, f64",
            Shape::Select => "d, cond, a, b",
            Shape::Load => "d, base, off",
            Shape::Store => "src, base, off",
            Shape::ABC => "a, b, c",
            Shape::GlobalGet => "d, g",
            Shape::GlobalSet => "g, a",
            Shape::Jmp => "L",
            Shape::Br => "a, L",
            Shape::BrTable => "a, [L...], Ldefault",
            Shape::Call => "f, base",
            Shape::CallIndirect => "type, a, base",
            Shape::Ret => "base",
        }
    }
}

macro_rules! define_ops {
    ($($var:ident = $num:literal, $name:literal, $shape:ident;)*) => {
        /// An instruction opcode. Discriminants are the permanent opcode numbers.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum Op {
            $($var = $num,)*
        }

        impl Op {
            /// Every op, ascending by number.
            pub const ALL: &'static [Op] = &[$(Op::$var,)*];

            /// The op with this opcode number, if any.
            pub fn from_u16(n: u16) -> Option<Op> {
                match n {
                    $($num => Some(Op::$var),)*
                    _ => None,
                }
            }

            /// Mnemonic as written in SPEC.md.
            pub fn name(self) -> &'static str {
                match self {
                    $(Op::$var => $name,)*
                }
            }

            /// Operand shape.
            pub fn shape(self) -> Shape {
                match self {
                    $(Op::$var => Shape::$shape,)*
                }
            }
        }
    };
}

define_ops! {
    Nop = 0x000, "nop", None;
    Unreachable = 0x001, "unreachable", None;
    Yield = 0x002, "yield", None;
    Ret = 0x003, "ret", Ret;
    Jmp = 0x004, "jmp", Jmp;
    BrIf = 0x005, "br_if", Br;
    BrIfz = 0x006, "br_ifz", Br;
    BrTable = 0x007, "br_table", BrTable;
    Call = 0x008, "call", Call;
    CallIndirect = 0x009, "call_indirect", CallIndirect;
    ConstI32 = 0x010, "const.i32", ConstI32;
    ConstI64 = 0x011, "const.i64", ConstI64;
    ConstF32 = 0x012, "const.f32", ConstF32;
    ConstF64 = 0x013, "const.f64", ConstF64;
    Mov = 0x014, "mov", DA;
    Select = 0x015, "select", Select;
    GlobalGet = 0x016, "global.get", GlobalGet;
    GlobalSet = 0x017, "global.set", GlobalSet;
    MemorySize = 0x018, "memory.size", D;
    MemoryGrow = 0x019, "memory.grow", DA;
    MemoryCopy = 0x01A, "memory.copy", ABC;
    MemoryFill = 0x01B, "memory.fill", ABC;
    I32Add = 0x020, "i32.add", DAB;
    I32Sub = 0x021, "i32.sub", DAB;
    I32Mul = 0x022, "i32.mul", DAB;
    I32DivS = 0x023, "i32.div_s", DAB;
    I32DivU = 0x024, "i32.div_u", DAB;
    I32RemS = 0x025, "i32.rem_s", DAB;
    I32RemU = 0x026, "i32.rem_u", DAB;
    I32And = 0x027, "i32.and", DAB;
    I32Or = 0x028, "i32.or", DAB;
    I32Xor = 0x029, "i32.xor", DAB;
    I32Shl = 0x02A, "i32.shl", DAB;
    I32ShrS = 0x02B, "i32.shr_s", DAB;
    I32ShrU = 0x02C, "i32.shr_u", DAB;
    I32Rotl = 0x02D, "i32.rotl", DAB;
    I32Rotr = 0x02E, "i32.rotr", DAB;
    I32Clz = 0x02F, "i32.clz", DA;
    I32Ctz = 0x030, "i32.ctz", DA;
    I32Popcnt = 0x031, "i32.popcnt", DA;
    I32Eqz = 0x032, "i32.eqz", DA;
    I32Eq = 0x034, "i32.eq", DAB;
    I32Ne = 0x035, "i32.ne", DAB;
    I32LtS = 0x036, "i32.lt_s", DAB;
    I32LtU = 0x037, "i32.lt_u", DAB;
    I32GtS = 0x038, "i32.gt_s", DAB;
    I32GtU = 0x039, "i32.gt_u", DAB;
    I32LeS = 0x03A, "i32.le_s", DAB;
    I32LeU = 0x03B, "i32.le_u", DAB;
    I32GeS = 0x03C, "i32.ge_s", DAB;
    I32GeU = 0x03D, "i32.ge_u", DAB;
    I64Add = 0x040, "i64.add", DAB;
    I64Sub = 0x041, "i64.sub", DAB;
    I64Mul = 0x042, "i64.mul", DAB;
    I64DivS = 0x043, "i64.div_s", DAB;
    I64DivU = 0x044, "i64.div_u", DAB;
    I64RemS = 0x045, "i64.rem_s", DAB;
    I64RemU = 0x046, "i64.rem_u", DAB;
    I64And = 0x047, "i64.and", DAB;
    I64Or = 0x048, "i64.or", DAB;
    I64Xor = 0x049, "i64.xor", DAB;
    I64Shl = 0x04A, "i64.shl", DAB;
    I64ShrS = 0x04B, "i64.shr_s", DAB;
    I64ShrU = 0x04C, "i64.shr_u", DAB;
    I64Rotl = 0x04D, "i64.rotl", DAB;
    I64Rotr = 0x04E, "i64.rotr", DAB;
    I64Clz = 0x04F, "i64.clz", DA;
    I64Ctz = 0x050, "i64.ctz", DA;
    I64Popcnt = 0x051, "i64.popcnt", DA;
    I64Eqz = 0x052, "i64.eqz", DA;
    I64Eq = 0x054, "i64.eq", DAB;
    I64Ne = 0x055, "i64.ne", DAB;
    I64LtS = 0x056, "i64.lt_s", DAB;
    I64LtU = 0x057, "i64.lt_u", DAB;
    I64GtS = 0x058, "i64.gt_s", DAB;
    I64GtU = 0x059, "i64.gt_u", DAB;
    I64LeS = 0x05A, "i64.le_s", DAB;
    I64LeU = 0x05B, "i64.le_u", DAB;
    I64GeS = 0x05C, "i64.ge_s", DAB;
    I64GeU = 0x05D, "i64.ge_u", DAB;
    I32Load = 0x060, "i32.load", Load;
    I64Load = 0x061, "i64.load", Load;
    F32Load = 0x062, "f32.load", Load;
    F64Load = 0x063, "f64.load", Load;
    I32Store = 0x064, "i32.store", Store;
    I64Store = 0x065, "i64.store", Store;
    F32Store = 0x066, "f32.store", Store;
    F64Store = 0x067, "f64.store", Store;
    I32Load8S = 0x080, "i32.load8_s", Load;
    I32Load8U = 0x081, "i32.load8_u", Load;
    I32Load16S = 0x082, "i32.load16_s", Load;
    I32Load16U = 0x083, "i32.load16_u", Load;
    I64Load8S = 0x084, "i64.load8_s", Load;
    I64Load8U = 0x085, "i64.load8_u", Load;
    I64Load16S = 0x086, "i64.load16_s", Load;
    I64Load16U = 0x087, "i64.load16_u", Load;
    I64Load32S = 0x088, "i64.load32_s", Load;
    I64Load32U = 0x089, "i64.load32_u", Load;
    I32Store8 = 0x090, "i32.store8", Store;
    I32Store16 = 0x091, "i32.store16", Store;
    I64Store8 = 0x092, "i64.store8", Store;
    I64Store16 = 0x093, "i64.store16", Store;
    I64Store32 = 0x094, "i64.store32", Store;
    F32Add = 0x0A0, "f32.add", DAB;
    F32Sub = 0x0A1, "f32.sub", DAB;
    F32Mul = 0x0A2, "f32.mul", DAB;
    F32Div = 0x0A3, "f32.div", DAB;
    F32Min = 0x0A4, "f32.min", DAB;
    F32Max = 0x0A5, "f32.max", DAB;
    F32Copysign = 0x0A6, "f32.copysign", DAB;
    F32Abs = 0x0A8, "f32.abs", DA;
    F32Neg = 0x0A9, "f32.neg", DA;
    F32Sqrt = 0x0AA, "f32.sqrt", DA;
    F32Ceil = 0x0AB, "f32.ceil", DA;
    F32Floor = 0x0AC, "f32.floor", DA;
    F32Trunc = 0x0AD, "f32.trunc", DA;
    F32Nearest = 0x0AE, "f32.nearest", DA;
    F32Eq = 0x0B0, "f32.eq", DAB;
    F32Ne = 0x0B1, "f32.ne", DAB;
    F32Lt = 0x0B2, "f32.lt", DAB;
    F32Gt = 0x0B3, "f32.gt", DAB;
    F32Le = 0x0B4, "f32.le", DAB;
    F32Ge = 0x0B5, "f32.ge", DAB;
    F64Add = 0x0C0, "f64.add", DAB;
    F64Sub = 0x0C1, "f64.sub", DAB;
    F64Mul = 0x0C2, "f64.mul", DAB;
    F64Div = 0x0C3, "f64.div", DAB;
    F64Min = 0x0C4, "f64.min", DAB;
    F64Max = 0x0C5, "f64.max", DAB;
    F64Copysign = 0x0C6, "f64.copysign", DAB;
    F64Abs = 0x0C8, "f64.abs", DA;
    F64Neg = 0x0C9, "f64.neg", DA;
    F64Sqrt = 0x0CA, "f64.sqrt", DA;
    F64Ceil = 0x0CB, "f64.ceil", DA;
    F64Floor = 0x0CC, "f64.floor", DA;
    F64Trunc = 0x0CD, "f64.trunc", DA;
    F64Nearest = 0x0CE, "f64.nearest", DA;
    F64Eq = 0x0D0, "f64.eq", DAB;
    F64Ne = 0x0D1, "f64.ne", DAB;
    F64Lt = 0x0D2, "f64.lt", DAB;
    F64Gt = 0x0D3, "f64.gt", DAB;
    F64Le = 0x0D4, "f64.le", DAB;
    F64Ge = 0x0D5, "f64.ge", DAB;
    I32WrapI64 = 0x0E0, "i32.wrap_i64", DA;
    I64ExtendI32S = 0x0E1, "i64.extend_i32_s", DA;
    I64ExtendI32U = 0x0E2, "i64.extend_i32_u", DA;
    I32Extend8S = 0x0E3, "i32.extend8_s", DA;
    I32Extend16S = 0x0E4, "i32.extend16_s", DA;
    I64Extend8S = 0x0E5, "i64.extend8_s", DA;
    I64Extend16S = 0x0E6, "i64.extend16_s", DA;
    I64Extend32S = 0x0E7, "i64.extend32_s", DA;
    I32TruncF32S = 0x100, "i32.trunc_f32_s", DA;
    I32TruncF32U = 0x101, "i32.trunc_f32_u", DA;
    I32TruncF64S = 0x102, "i32.trunc_f64_s", DA;
    I32TruncF64U = 0x103, "i32.trunc_f64_u", DA;
    I64TruncF32S = 0x104, "i64.trunc_f32_s", DA;
    I64TruncF32U = 0x105, "i64.trunc_f32_u", DA;
    I64TruncF64S = 0x106, "i64.trunc_f64_s", DA;
    I64TruncF64U = 0x107, "i64.trunc_f64_u", DA;
    I32TruncSatF32S = 0x108, "i32.trunc_sat_f32_s", DA;
    I32TruncSatF32U = 0x109, "i32.trunc_sat_f32_u", DA;
    I32TruncSatF64S = 0x10A, "i32.trunc_sat_f64_s", DA;
    I32TruncSatF64U = 0x10B, "i32.trunc_sat_f64_u", DA;
    I64TruncSatF32S = 0x10C, "i64.trunc_sat_f32_s", DA;
    I64TruncSatF32U = 0x10D, "i64.trunc_sat_f32_u", DA;
    I64TruncSatF64S = 0x10E, "i64.trunc_sat_f64_s", DA;
    I64TruncSatF64U = 0x10F, "i64.trunc_sat_f64_u", DA;
    F32ConvertI32S = 0x110, "f32.convert_i32_s", DA;
    F32ConvertI32U = 0x111, "f32.convert_i32_u", DA;
    F32ConvertI64S = 0x112, "f32.convert_i64_s", DA;
    F32ConvertI64U = 0x113, "f32.convert_i64_u", DA;
    F64ConvertI32S = 0x114, "f64.convert_i32_s", DA;
    F64ConvertI32U = 0x115, "f64.convert_i32_u", DA;
    F64ConvertI64S = 0x116, "f64.convert_i64_s", DA;
    F64ConvertI64U = 0x117, "f64.convert_i64_u", DA;
    F32DemoteF64 = 0x118, "f32.demote_f64", DA;
    F64PromoteF32 = 0x119, "f64.promote_f32", DA;
    I32ReinterpretF32 = 0x11A, "i32.reinterpret_f32", DA;
    I64ReinterpretF64 = 0x11B, "i64.reinterpret_f64", DA;
    F32ReinterpretI32 = 0x11C, "f32.reinterpret_i32", DA;
    F64ReinterpretI64 = 0x11D, "f64.reinterpret_i64", DA;
}

impl Op {
    /// The opcode number.
    pub fn code(self) -> u16 {
        self as u16
    }

    /// Instructions a function may end with: ret, jmp, br_table, unreachable.
    pub fn is_terminator(self) -> bool {
        matches!(self, Op::Ret | Op::Jmp | Op::BrTable | Op::Unreachable)
    }
}

/// Renders the opcode table as Markdown (written to OPCODES.md by a test).
pub fn opcodes_markdown() -> String {
    let mut s = String::from(
        "# efevm opcodes\n\nGenerated from `src/op.rs`. Numbers are permanent once released.\n\n\
         | hex | dec | mnemonic | operands |\n|---|---|---|---|\n",
    );
    for &op in Op::ALL {
        s.push_str(&format!(
            "| 0x{:03X} | {} | `{}` | {} |\n",
            op.code(),
            op.code(),
            op.name(),
            op.shape().operands()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_ascending_and_roundtrip() {
        for w in Op::ALL.windows(2) {
            assert!(w[0].code() < w[1].code());
        }
        for &op in Op::ALL {
            assert_eq!(Op::from_u16(op.code()), Some(op));
        }
        let present: HashSet<u16> = Op::ALL.iter().map(|o| o.code()).collect();
        for n in 0..=0x200u16 {
            assert_eq!(Op::from_u16(n).is_some(), present.contains(&n));
        }
        assert!(Op::from_u16(0x1E).is_none());
        assert!(Op::from_u16(0xFFFF).is_none());
    }

    #[test]
    fn hot_ops_are_one_byte() {
        for &op in Op::ALL {
            let hot = matches!(
                op.shape(),
                Shape::ConstI32 | Shape::ConstI64 | Shape::Jmp | Shape::Br | Shape::Call | Shape::Ret
            ) || op.name().starts_with("i32.") && !op.name().contains("trunc") && !op.name().contains("reinterpret") && !op.name().contains("load") && !op.name().contains("store") && !op.name().contains("extend") && !op.name().contains("wrap");
            if hot {
                assert!(op.code() < 128, "{}", op.name());
            }
        }
        for op in [Op::I32Load, Op::I64Load, Op::I32Store, Op::I64Store, Op::Mov] {
            assert!(op.code() < 128);
        }
    }

    #[test]
    fn names_unique() {
        let names: HashSet<&str> = Op::ALL.iter().map(|o| o.name()).collect();
        assert_eq!(names.len(), Op::ALL.len());
    }

    #[test]
    fn spec_mnemonics_present() {
        let mut expected: Vec<String> = Vec::new();
        for m in [
            "const.i32", "const.i64", "const.f32", "const.f64", "mov", "select",
            "i32.wrap_i64", "i64.extend_i32_s", "i64.extend_i32_u", "i32.extend8_s",
            "i32.extend16_s", "i64.extend8_s", "i64.extend16_s", "i64.extend32_s",
            "f32.demote_f64", "f64.promote_f32", "i32.reinterpret_f32", "i64.reinterpret_f64",
            "f32.reinterpret_i32", "f64.reinterpret_i64",
            "i32.load", "i64.load", "f32.load", "f64.load", "i32.load8_s", "i32.load8_u",
            "i32.load16_s", "i32.load16_u", "i64.load8_s", "i64.load8_u", "i64.load16_s",
            "i64.load16_u", "i64.load32_s", "i64.load32_u", "i32.store", "i64.store",
            "f32.store", "f64.store", "i32.store8", "i32.store16", "i64.store8", "i64.store16",
            "i64.store32", "memory.size", "memory.grow", "memory.copy", "memory.fill",
            "global.get", "global.set", "jmp", "br_if", "br_ifz", "br_table", "call",
            "call_indirect", "ret", "yield", "unreachable", "nop",
        ] {
            expected.push(m.to_string());
        }
        for t in ["i32", "i64"] {
            for o in [
                "add", "sub", "mul", "and", "or", "xor", "shl", "shr_s", "shr_u", "rotl", "rotr",
                "div_s", "div_u", "rem_s", "rem_u", "clz", "ctz", "popcnt", "eqz", "eq", "ne",
                "lt_s", "lt_u", "gt_s", "gt_u", "le_s", "le_u", "ge_s", "ge_u",
            ] {
                expected.push(format!("{t}.{o}"));
            }
        }
        for f in ["f32", "f64"] {
            for o in [
                "add", "sub", "mul", "div", "min", "max", "copysign", "abs", "neg", "sqrt", "ceil",
                "floor", "trunc", "nearest", "eq", "ne", "lt", "gt", "le", "ge",
            ] {
                expected.push(format!("{f}.{o}"));
            }
            for t in ["i32", "i64"] {
                for s in ["s", "u"] {
                    expected.push(format!("{t}.trunc_{f}_{s}"));
                    expected.push(format!("{t}.trunc_sat_{f}_{s}"));
                    expected.push(format!("{f}.convert_{t}_{s}"));
                }
            }
        }
        let have: HashSet<&str> = Op::ALL.iter().map(|o| o.name()).collect();
        for e in &expected {
            assert!(have.contains(e.as_str()), "missing {e}");
        }
        assert_eq!(have.len(), expected.len(), "unexpected extra ops");
    }

    #[test]
    fn terminators_and_markdown() {
        assert!(Op::Ret.is_terminator() && Op::Jmp.is_terminator());
        assert!(Op::BrTable.is_terminator() && Op::Unreachable.is_terminator());
        assert!(!Op::BrIf.is_terminator() && !Op::Nop.is_terminator());
        let md = opcodes_markdown();
        assert!(md.contains("`i64.trunc_sat_f64_u`"));
    }
}
