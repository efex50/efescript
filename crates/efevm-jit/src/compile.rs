//! Lowering of efevm functions to Cranelift IR and the owner of the compiled code.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::{Ieee32, Ieee64};
use cranelift_codegen::ir::{
    self, AbiParam, Block, Endianness, FuncRef, InstBuilder, MemFlagsData, Signature, StackSlot, StackSlotData,
    StackSlotKind, Type, Value, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module as _};
use efevm::{Inst, Module as VmModule, Op};

use crate::ctx::*;
use crate::helpers::*;

/// Optimisation level of the generated code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptLevel {
    None,
    Speed,
}

/// Options of [`compile`] and [`crate::enable`].
#[derive(Clone, Debug)]
pub struct JitOptions {
    pub opt_level: OptLevel,
    /// Native stack the generated code may use below the address where `run` started. When it
    /// is used up, the fiber continues in the interpreter (so deep recursion behaves exactly
    /// like the interpreter). The thread must have at least this much stack plus headroom for
    /// host functions. Default 1 MiB.
    pub max_native_stack: usize,
    /// Run Cranelift's IR verifier (default: only in debug builds).
    pub verify: bool,
}

impl Default for JitOptions {
    fn default() -> Self {
        JitOptions { opt_level: OptLevel::Speed, max_native_stack: 1 << 20, verify: cfg!(debug_assertions) }
    }
}

/// Why compilation failed. The caller falls back to the interpreter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitError {
    /// The host has no Cranelift ISA, or is not a 64-bit host.
    UnsupportedHost(String),
    /// An op could not be lowered.
    Unsupported { func: u32, op: Op },
    /// Cranelift failed on a function.
    Codegen { func: u32, message: String },
    /// Cranelift's module layer failed.
    Module(String),
    /// A function has more registers than [`MAX_JIT_REGISTERS`]: every register is an SSA
    /// variable (a block parameter of every join point), so compile time and frame size grow
    /// with it, and Cranelift's block-parameter count is capped at 65535. The module stays on
    /// the interpreter.
    TooManyRegisters { func: u32, nregs: u32 },
}

/// Most registers (`nregs`) a function may have to be compiled.
pub const MAX_JIT_REGISTERS: u32 = 4096;

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::UnsupportedHost(m) => write!(f, "jit: unsupported host: {m}"),
            JitError::Unsupported { func, op } => write!(f, "jit: function {func}: unsupported op {}", op.name()),
            JitError::Codegen { func, message } => write!(f, "jit: function {func}: {message}"),
            JitError::Module(m) => write!(f, "jit: {m}"),
            JitError::TooManyRegisters { func, nregs } => {
                write!(f, "jit: function {func} has {nregs} registers (the JIT compiles at most {MAX_JIT_REGISTERS})")
            }
        }
    }
}

impl std::error::Error for JitError {}

/// Compiled code for one module. Owns the executable memory; freed on drop.
pub struct JitCode {
    pub(crate) module: VmModule,
    jit: Option<JITModule>,
    /// Native entry per function index (imports: their thunk).
    pub(crate) entries: Vec<*const u8>,
    pub(crate) table: Vec<TableEntry>,
    pub(crate) max_native_stack: usize,
}

// SAFETY: the code and tables are immutable after `compile`; the `JITModule` is only touched
// again in `Drop`, when no `Arc` (so no running backend) exists any more.
unsafe impl Send for JitCode {}
// SAFETY: see above.
unsafe impl Sync for JitCode {}

impl Drop for JitCode {
    fn drop(&mut self) {
        if let Some(j) = self.jit.take() {
            // SAFETY: nothing can call into the code any more: every user holds an `Arc`.
            unsafe { j.free_memory() };
        }
    }
}

impl fmt::Debug for JitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JitCode").field("functions", &self.entries.len()).finish()
    }
}

#[derive(Clone, Copy)]
struct HelperIds {
    trap: FuncId,
    trap_frame: FuncId,
    save_inner: FuncId,
    save_frame: FuncId,
    stack_low: FuncId,
    resume_frame: FuncId,
    grow: FuncId,
    copy: FuncId,
    fill: FuncId,
}

#[derive(Clone, Copy)]
struct HelperRefs {
    trap: FuncRef,
    trap_frame: FuncRef,
    save_inner: FuncRef,
    save_frame: FuncRef,
    stack_low: FuncRef,
    resume_frame: FuncRef,
    grow: FuncRef,
    copy: FuncRef,
    fill: FuncRef,
}

#[derive(Clone, Copy)]
struct FnInfo {
    np: u32,
    nr: u32,
    /// Activation size, 0 for imports.
    size: u32,
    canon: u32,
}

const I32: Type = types::I32;
const I64: Type = types::I64;
const F32: Type = types::F32;
const F64: Type = types::F64;

fn cg_err(func: u32, e: impl fmt::Display) -> JitError {
    JitError::Codegen { func, message: e.to_string() }
}

/// Compiles every function of `module`.
pub fn compile(module: &VmModule, opts: &JitOptions) -> Result<Arc<JitCode>, JitError> {
    let nimports = module.data().imports.len();
    for (i, f) in module.data().functions.iter().enumerate() {
        if u32::from(f.nregs) > MAX_JIT_REGISTERS {
            return Err(JitError::TooManyRegisters { func: (nimports + i) as u32, nregs: u32::from(f.nregs) });
        }
    }
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| f.set(k, v).map_err(|e| JitError::UnsupportedHost(e.to_string()));
    set(&mut flags, "use_colocated_libcalls", "false")?;
    set(&mut flags, "is_pic", "false")?;
    set(&mut flags, "opt_level", if opts.opt_level == OptLevel::Speed { "speed" } else { "none" })?;
    set(&mut flags, "enable_verifier", if opts.verify { "true" } else { "false" })?;
    let isa = cranelift_native::builder()
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| JitError::UnsupportedHost(e.to_string()))?;
    if isa.pointer_bits() != 64 {
        return Err(JitError::UnsupportedHost("not a 64-bit host".into()));
    }
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    jb.symbol("efevm_jit_trap", h_trap as *const u8);
    jb.symbol("efevm_jit_trap_frame", h_trap_frame as *const u8);
    jb.symbol("efevm_jit_save_inner", h_save_inner as *const u8);
    jb.symbol("efevm_jit_save_frame", h_save_frame as *const u8);
    jb.symbol("efevm_jit_stack_low", h_stack_low as *const u8);
    jb.symbol("efevm_jit_resume_frame", h_resume_frame as *const u8);
    jb.symbol("efevm_jit_grow", h_grow as *const u8);
    jb.symbol("efevm_jit_copy", h_copy as *const u8);
    jb.symbol("efevm_jit_fill", h_fill as *const u8);
    let mut jm = JITModule::new(jb);
    let cc = jm.isa().default_call_conv();
    let fc = jm.isa().frontend_config();

    let sig = |params: &[Type], ret: Option<Type>| {
        let mut s = Signature::new(cc);
        s.params.extend(params.iter().map(|t| AbiParam::new(*t)));
        s.returns.extend(ret.map(AbiParam::new));
        s
    };
    let sig_fn = sig(&[I64, I64, I32], Some(I32));
    let decl = |jm: &mut JITModule, name: &str, s: &Signature| {
        jm.declare_function(name, Linkage::Import, s).map_err(|e| JitError::Module(e.to_string()))
    };
    let hids = HelperIds {
        trap: decl(&mut jm, "efevm_jit_trap", &sig(&[I64, I32, I32, I32], None))?,
        trap_frame: decl(&mut jm, "efevm_jit_trap_frame", &sig(&[I64, I32, I32], None))?,
        save_inner: decl(&mut jm, "efevm_jit_save_inner", &sig(&[I64, I32, I32, I32, I64, I32], None))?,
        save_frame: decl(&mut jm, "efevm_jit_save_frame", &sig(&[I64, I32, I32, I64, I32, I32], None))?,
        stack_low: decl(&mut jm, "efevm_jit_stack_low", &sig(&[I64, I32, I64, I32], None))?,
        resume_frame: decl(&mut jm, "efevm_jit_resume_frame", &sig(&[I64, I32, I64], Some(I32)))?,
        grow: decl(&mut jm, "efevm_jit_grow", &sig(&[I64, I64], Some(I64)))?,
        copy: decl(&mut jm, "efevm_jit_copy", &sig(&[I64, I64, I64, I64], Some(I32)))?,
        fill: decl(&mut jm, "efevm_jit_fill", &sig(&[I64, I64, I64, I64], Some(I32)))?,
    };

    let data = module.data();
    let nimports = data.imports.len();
    // Canonical type ids, exactly as `Instance` computes them.
    let mut type_canon = Vec::with_capacity(data.types.len());
    let mut first: HashMap<&efevm::FuncType, u32> = HashMap::new();
    for (i, t) in data.types.iter().enumerate() {
        type_canon.push(*first.entry(t).or_insert(i as u32));
    }
    let nfuncs = data.num_funcs() as usize;
    let mut infos = Vec::with_capacity(nfuncs);
    for g in 0..nfuncs {
        let (tidx, nregs) = if g < nimports {
            (data.imports[g].type_idx, 0u32)
        } else {
            let f = &data.functions[g - nimports];
            (f.type_idx, f.nregs as u32)
        };
        let t = data.types.get(tidx as usize);
        let (np, nr) = t.map_or((0, 0), |t| (t.params.len() as u32, t.results.len() as u32));
        infos.push(FnInfo {
            np,
            nr,
            size: if g < nimports { 0 } else { nregs.max(np) },
            canon: type_canon.get(tidx as usize).copied().unwrap_or(u32::MAX),
        });
    }

    let mut ids = Vec::with_capacity(nfuncs);
    for g in 0..nfuncs {
        let name = if g < nimports { format!("import{g}") } else { format!("f{g}") };
        ids.push(jm.declare_function(&name, Linkage::Local, &sig_fn).map_err(|e| JitError::Module(e.to_string()))?);
    }

    let mut fbc = FunctionBuilderContext::new();
    let mut ctx = jm.make_context();
    // Import thunks.
    for g in 0..nimports {
        ctx.func.signature = sig_fn.clone();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let e = b.create_block();
            b.append_block_params_for_function_params(e);
            b.switch_to_block(e);
            b.seal_block(e);
            let (c, w) = (b.block_params(e)[0], b.block_params(e)[1]);
            let hc = b.ins().load(I64, MemFlagsData::trusted(), c, JitCtx::OFF_HOST_CALL);
            let s = b.import_signature(sig(&[I64, I32, I64], Some(I32)));
            let imp = b.ins().iconst(I32, g as i64);
            let call = b.ins().call_indirect(s, hc, &[c, imp, w]);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.finalize(fc);
        }
        jm.define_function(ids[g], &mut ctx).map_err(|e| cg_err(g as u32, e))?;
        jm.clear_context(&mut ctx);
    }

    for g in nimports..nfuncs {
        ctx.func.signature = sig_fn.clone();
        let f = &data.functions[g - nimports];
        let href = |jm: &mut JITModule, func: &mut ir::Function| HelperRefs {
            trap: jm.declare_func_in_func(hids.trap, func),
            trap_frame: jm.declare_func_in_func(hids.trap_frame, func),
            save_inner: jm.declare_func_in_func(hids.save_inner, func),
            save_frame: jm.declare_func_in_func(hids.save_frame, func),
            stack_low: jm.declare_func_in_func(hids.stack_low, func),
            resume_frame: jm.declare_func_in_func(hids.resume_frame, func),
            grow: jm.declare_func_in_func(hids.grow, func),
            copy: jm.declare_func_in_func(hids.copy, func),
            fill: jm.declare_func_in_func(hids.fill, func),
        };
        let h = href(&mut jm, &mut ctx.func);
        let mut callees = HashMap::new();
        for i in &f.code {
            if i.op == Op::Call {
                callees.entry(i.a).or_insert_with(|| jm.declare_func_in_func(ids[i.a as usize], &mut ctx.func));
            }
        }
        {
            let b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let np = infos[g].np as usize;
            let nregs = f.nregs as usize;
            let mut c = Fc {
                b,
                vm: module,
                fidx: g as u32,
                nimports: nimports as u32,
                np,
                nr: infos[g].nr as usize,
                nregs,
                vars: Vec::new(),
                mem_base: Variable::from_u32(0),
                mem_len: Variable::from_u32(0),
                ctx: Value::from_u32(0),
                window: Value::from_u32(0),
                regslot: StackSlot::from_u32(0),
                callslot: StackSlot::from_u32(0),
                h,
                callees,
                infos: &infos,
                type_canon: &type_canon,
                sig_fn: sig_fn.clone(),
                blk: Vec::new(),
                sites: Vec::new(),
                idx: 0,
            };
            c.build(f)?;
            c.b.finalize(fc);
        }
        jm.define_function(ids[g], &mut ctx).map_err(|e| cg_err(g as u32, e))?;
        jm.clear_context(&mut ctx);
    }
    jm.finalize_definitions().map_err(|e| JitError::Module(e.to_string()))?;

    let entries: Vec<*const u8> = ids.iter().map(|id| jm.get_finalized_function(*id)).collect();
    let table = data
        .table
        .iter()
        .map(|e| match e {
            Some(g) if (*g as usize) < nfuncs => TableEntry {
                code: entries[*g as usize],
                canon: infos[*g as usize].canon,
                size: infos[*g as usize].size,
                is_host: ((*g as usize) < nimports) as u32,
                _pad: 0,
            },
            _ => TableEntry { code: std::ptr::null(), canon: 0, size: 0, is_host: 0, _pad: 0 },
        })
        .collect();
    Ok(Arc::new(JitCode { module: module.clone(), jit: Some(jm), entries, table, max_native_stack: opts.max_native_stack }))
}

/// Per-function code generator.
struct Fc<'a, 'm> {
    b: FunctionBuilder<'a>,
    vm: &'m VmModule,
    fidx: u32,
    nimports: u32,
    np: usize,
    nr: usize,
    nregs: usize,
    vars: Vec<Variable>,
    mem_base: Variable,
    mem_len: Variable,
    ctx: Value,
    window: Value,
    regslot: StackSlot,
    callslot: StackSlot,
    h: HelperRefs,
    callees: HashMap<u32, FuncRef>,
    infos: &'m [FnInfo],
    type_canon: &'m [u32],
    sig_fn: Signature,
    /// Cranelift block of every leader pc.
    blk: Vec<Option<Block>>,
    /// Resume targets: (return pc, block taking the mode as parameter).
    sites: Vec<(u32, Block)>,
    /// Index of the instruction being lowered.
    idx: u32,
}

fn mf() -> MemFlagsData {
    let mut f = MemFlagsData::new();
    f.set_notrap();
    f.set_endianness(Endianness::Little);
    f
}

fn terminates(op: Op) -> bool {
    matches!(
        op,
        Op::Jmp | Op::BrIf | Op::BrIfz | Op::BrTable | Op::Ret | Op::Unreachable | Op::Call | Op::CallIndirect | Op::Yield
    )
}

impl<'a, 'm> Fc<'a, 'm> {
    fn build(&mut self, f: &efevm::Function) -> Result<(), JitError> {
        let size = self.nregs.max(self.np);
        self.nregs = size;
        let max_win = f
            .code
            .iter()
            .filter_map(|i| match i.op {
                Op::Call => {
                    let n = self.infos[i.a as usize];
                    Some(n.np.max(n.nr) as usize)
                }
                Op::CallIndirect => {
                    let t = &self.vm.data().types[i.a as usize];
                    Some(t.params.len().max(t.results.len()))
                }
                _ => None,
            })
            .max()
            .unwrap_or(0);
        self.regslot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (size.max(1) * 8) as u32, 3));
        self.callslot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (max_win.max(1) * 8) as u32, 3));
        self.vars = (0..size).map(|_| self.b.declare_var(I64)).collect();
        self.mem_base = self.b.declare_var(I64);
        self.mem_len = self.b.declare_var(I64);

        let entry = self.b.create_block();
        self.b.append_block_params_for_function_params(entry);
        let low = self.b.create_block();
        let setup = self.b.create_block();
        let normal = self.b.create_block();
        let resume = self.b.create_block();
        self.b.switch_to_block(entry);
        self.b.seal_block(entry);
        let ps = self.b.block_params(entry).to_vec();
        self.ctx = ps[0];
        self.window = ps[1];
        let mode = ps[2];
        // Native stack check, before anything else.
        let sp = self.b.ins().get_stack_pointer(I64);
        let lim = self.b.ins().load(I64, mf(), self.ctx, JitCtx::OFF_STACK_LIMIT);
        let is_low = self.b.ins().icmp(IntCC::UnsignedLessThan, sp, lim);
        self.b.ins().brif(is_low, low, &[], setup, &[]);
        self.b.set_cold_block(low);
        self.b.seal_block(low);
        self.b.seal_block(setup);

        self.b.switch_to_block(low);
        let fv = self.b.ins().iconst(I32, self.fidx as i64);
        self.b.ins().call(self.h.stack_low, &[self.ctx, fv, self.window, mode]);
        let one = self.b.ins().iconst(I32, 1);
        self.b.ins().return_(&[one]);

        self.b.switch_to_block(setup);
        self.reload_mem();
        self.b.ins().brif(mode, resume, &[], normal, &[]);
        self.b.seal_block(normal);
        self.b.seal_block(resume);

        // Leader blocks.
        let code_len = f.code.len();
        self.blk = vec![None; code_len + 1];
        let leaders: Vec<(u32, u32)> = self.vm.blocks(self.fidx).to_vec();
        for (pc, _) in &leaders {
            self.blk[*pc as usize] = Some(self.b.create_block());
        }

        // Normal entry: parameters from the window, other registers zero.
        self.b.switch_to_block(normal);
        for r in 0..size {
            let v = if r < self.np {
                self.b.ins().load(I64, MemFlagsData::new(), self.window, (r * 8) as i32)
            } else {
                self.b.ins().iconst(I64, 0)
            };
            self.b.def_var(self.vars[r], v);
        }
        let first = self.blk[0].ok_or(JitError::Codegen { func: self.fidx, message: "no leader at pc 0".into() })?;
        self.b.ins().jump(first, &[]);

        // Code.
        for (k, (pc, cost)) in leaders.iter().enumerate() {
            let end = leaders.get(k + 1).map_or(code_len as u32, |l| l.0);
            let block = self.blk[*pc as usize].unwrap();
            self.b.switch_to_block(block);
            self.idx = *pc;
            self.budget_check(*pc, *cost);
            for i in *pc..end {
                self.idx = i;
                let inst = f.code[i as usize];
                self.inst(f, inst, i)?;
            }
            if !terminates(f.code[(end - 1) as usize].op) {
                let next = self.blk[end as usize].ok_or(JitError::Codegen { func: self.fidx, message: "fallthrough off the end".into() })?;
                self.b.ins().jump(next, &[]);
            }
        }

        // Resume entry (after the call sites are known).
        self.b.switch_to_block(resume);
        let fv = self.b.ins().iconst(I32, self.fidx as i64);
        let slot = self.b.ins().stack_addr(I64, self.regslot, 0);
        let call = self.b.ins().call(self.h.resume_frame, &[self.ctx, fv, slot]);
        let pc = self.b.inst_results(call)[0];
        for r in 0..size {
            let v = self.b.ins().stack_load(I64, I64, self.regslot, (r * 8) as i32);
            self.b.def_var(self.vars[r], v);
        }
        let inner = self.b.ins().load(I32, MemFlagsData::new(), self.ctx, JitCtx::OFF_RESUME_INNER);
        let inner_d = self.b.create_block();
        let call_d = self.b.create_block();
        let bad = self.b.create_block();
        self.b.ins().brif(inner, inner_d, &[], call_d, &[]);
        self.b.switch_to_block(inner_d);
        let mut sw = Switch::new();
        for (pc, _) in &leaders {
            sw.set_entry(*pc as u128, self.blk[*pc as usize].unwrap());
        }
        sw.emit(&mut self.b, pc, bad);
        self.b.switch_to_block(call_d);
        let mut sw = Switch::new();
        let sites = std::mem::take(&mut self.sites);
        for (retpc, site) in sites {
            let tramp = self.b.create_block();
            self.b.switch_to_block(tramp);
            let m = self.b.ins().iconst(I32, 1);
            self.b.ins().jump(site, &[m.into()]);
            sw.set_entry(retpc as u128, tramp);
        }
        self.b.switch_to_block(call_d);
        sw.emit(&mut self.b, pc, bad);
        self.b.switch_to_block(bad);
        self.b.set_cold_block(bad);
        self.idx = 0;
        self.emit_trap(TRAP_UNREACHABLE);
        self.b.seal_all_blocks();
        Ok(())
    }

    // ---- small helpers ----

    fn get(&mut self, r: u32) -> Value {
        self.b.use_var(self.vars[r as usize])
    }
    fn set(&mut self, r: u32, v: Value) {
        self.b.def_var(self.vars[r as usize], v);
    }
    fn ic(&mut self, t: Type, v: i64) -> Value {
        self.b.ins().iconst(t, v)
    }
    fn ctx_load(&mut self, t: Type, off: i32) -> Value {
        self.b.ins().load(t, MemFlagsData::new(), self.ctx, off)
    }
    fn ctx_store(&mut self, v: Value, off: i32) {
        self.b.ins().store(MemFlagsData::new(), v, self.ctx, off);
    }
    fn reload_mem(&mut self) {
        let base = self.ctx_load(I64, JitCtx::OFF_MEM_BASE);
        let len = self.ctx_load(I64, JitCtx::OFF_MEM_LEN);
        self.b.def_var(self.mem_base, base);
        self.b.def_var(self.mem_len, len);
    }

    /// Reads register `r` as a value of type `t`.
    fn rd(&mut self, r: u32, t: Type) -> Value {
        let v = self.get(r);
        match t {
            I64 => v,
            I32 => self.b.ins().ireduce(I32, v),
            F32 => {
                let x = self.b.ins().ireduce(I32, v);
                self.b.ins().bitcast(F32, MemFlagsData::new(), x)
            }
            _ => self.b.ins().bitcast(F64, MemFlagsData::new(), v),
        }
    }

    /// Writes a result of any type into register `r` (zero-extending narrow ints, bit-casting floats).
    fn wr(&mut self, r: u32, v: Value) {
        let t = self.b.func.dfg.value_type(v);
        let out = match t {
            I64 => v,
            F32 => {
                let x = self.b.ins().bitcast(I32, MemFlagsData::new(), v);
                self.b.ins().uextend(I64, x)
            }
            F64 => self.b.ins().bitcast(I64, MemFlagsData::new(), v),
            _ => self.b.ins().uextend(I64, v),
        };
        self.set(r, out);
    }

    fn bin(&mut self, i: Inst, t: Type, f: impl FnOnce(&mut FunctionBuilder<'a>, Value, Value) -> Value) {
        let x = self.rd(i.b, t);
        let y = self.rd(i.c, t);
        let r = f(&mut self.b, x, y);
        self.wr(i.a, r);
    }
    fn un(&mut self, i: Inst, t: Type, f: impl FnOnce(&mut FunctionBuilder<'a>, Value) -> Value) {
        let x = self.rd(i.b, t);
        let r = f(&mut self.b, x);
        self.wr(i.a, r);
    }
    fn icmp(&mut self, i: Inst, t: Type, cc: IntCC) {
        self.bin(i, t, |b, x, y| b.ins().icmp(cc, x, y));
    }
    fn fcmp(&mut self, i: Inst, t: Type, cc: FloatCC) {
        self.bin(i, t, |b, x, y| b.ins().fcmp(cc, x, y));
    }

    fn emit_trap(&mut self, kind: u32) {
        let k = self.ic(I32, kind as i64);
        let f = self.ic(I32, self.fidx as i64);
        let i = self.ic(I32, self.idx as i64);
        self.b.ins().call(self.h.trap, &[self.ctx, k, f, i]);
        let two = self.ic(I32, 2);
        self.b.ins().return_(&[two]);
    }

    /// Traps with `kind` when `bad` is nonzero, else continues.
    fn guard(&mut self, bad: Value, kind: u32) {
        let (tb, cb) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(bad, tb, &[], cb, &[]);
        self.b.set_cold_block(tb);
        self.b.seal_block(tb);
        self.b.seal_block(cb);
        self.b.switch_to_block(tb);
        self.emit_trap(kind);
        self.b.switch_to_block(cb);
    }

    /// Traps with `kind` unless `ok` is nonzero.
    fn guard_ok(&mut self, ok: Value, kind: u32) {
        let (tb, cb) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(ok, cb, &[], tb, &[]);
        self.b.set_cold_block(tb);
        self.b.seal_block(tb);
        self.b.seal_block(cb);
        self.b.switch_to_block(tb);
        self.emit_trap(kind);
        self.b.switch_to_block(cb);
    }

    /// Stores every register into the register slot; returns the slot address.
    fn spill(&mut self) -> Value {
        for r in 0..self.nregs {
            let v = self.get(r as u32);
            self.b.ins().stack_store(I64, v, self.regslot, (r * 8) as i32);
        }
        self.b.ins().stack_addr(I64, self.regslot, 0)
    }

    /// Leader entry: stop with a budget suspension when the counter is `<= 0`, else charge.
    fn budget_check(&mut self, pc: u32, cost: u32) {
        let bud = self.ctx_load(I64, JitCtx::OFF_BUDGET);
        let zero = self.ic(I64, 0);
        let le = self.b.ins().icmp(IntCC::SignedLessThanOrEqual, bud, zero);
        let (sb, cb) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(le, sb, &[], cb, &[]);
        self.b.set_cold_block(sb);
        self.b.seal_block(sb);
        self.b.seal_block(cb);
        self.b.switch_to_block(sb);
        let slot = self.spill();
        let (k, f, p, s) = (
            self.ic(I32, SUSPEND_BUDGET as i64),
            self.ic(I32, self.fidx as i64),
            self.ic(I32, pc as i64),
            self.ic(I32, self.nregs as i64),
        );
        self.b.ins().call(self.h.save_inner, &[self.ctx, k, f, p, slot, s]);
        let one = self.ic(I32, 1);
        self.b.ins().return_(&[one]);
        self.b.switch_to_block(cb);
        let c = self.ic(I64, cost as i64);
        let nb = self.b.ins().isub(bud, c);
        self.ctx_store(nb, JitCtx::OFF_BUDGET);
    }

    fn block_at(&self, pc: u32) -> Block {
        self.blk[pc as usize].expect("leader")
    }

    /// Bounds-checked effective address for an access of `size` bytes.
    fn addr(&mut self, base: u32, off: u32, size: i64) -> Value {
        let b = self.get(base);
        let o = self.ic(I64, off as i64);
        let ea = self.b.ins().iadd(b, o);
        let c1 = self.b.ins().icmp(IntCC::UnsignedLessThan, ea, b);
        let s = self.ic(I64, size);
        let end = self.b.ins().iadd(ea, s);
        let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, end, ea);
        let len = self.b.use_var(self.mem_len);
        let c3 = self.b.ins().icmp(IntCC::UnsignedGreaterThan, end, len);
        let c12 = self.b.ins().bor(c1, c2);
        let bad = self.b.ins().bor(c12, c3);
        self.guard(bad, TRAP_OOB);
        let mb = self.b.use_var(self.mem_base);
        self.b.ins().iadd(mb, ea)
    }

    fn load(&mut self, i: Inst, size: i64, f: impl FnOnce(&mut FunctionBuilder<'a>, Value) -> Value) {
        let p = self.addr(i.b, i.c, size);
        let v = f(&mut self.b, p);
        self.wr(i.a, v);
    }
    fn store(&mut self, i: Inst, size: i64, f: impl FnOnce(&mut FunctionBuilder<'a>, Value, Value)) {
        let v = self.get(i.a);
        let p = self.addr(i.b, i.c, size);
        f(&mut self.b, v, p);
    }

    fn fconst32(&mut self, v: f32) -> Value {
        self.b.ins().f32const(Ieee32::with_float(v))
    }
    fn fconst64(&mut self, v: f64) -> Value {
        self.b.ins().f64const(Ieee64::with_float(v))
    }

    /// Trapping float to int conversion with the interpreter's range constants.
    fn trunc_trap(&mut self, i: Inst, from: Type, to: Type, signed: bool) {
        let x = self.rd(i.b, from);
        let nan = self.b.ins().fcmp(FloatCC::Unordered, x, x);
        self.guard(nan, TRAP_INVALID_CONV);
        // (lower bound, lower inclusive, upper bound exclusive)
        let (lo, lo_inc, hi): (f64, bool, f64) = match (to == I32, signed, from == F32) {
            (true, true, true) => (-2147483904.0, false, 2147483648.0),
            (true, true, false) => (-2147483649.0, false, 2147483648.0),
            (true, false, _) => (-1.0, false, 4294967296.0),
            (false, true, _) => (-9223372036854775808.0, true, 9223372036854775808.0),
            (false, false, _) => (-1.0, false, 18446744073709551616.0),
        };
        let (l, h) = if from == F32 {
            (self.fconst32(lo as f32), self.fconst32(hi as f32))
        } else {
            (self.fconst64(lo), self.fconst64(hi))
        };
        let c1 = self.b.ins().fcmp(if lo_inc { FloatCC::GreaterThanOrEqual } else { FloatCC::GreaterThan }, x, l);
        let c2 = self.b.ins().fcmp(FloatCC::LessThan, x, h);
        let ok = self.b.ins().band(c1, c2);
        self.guard_ok(ok, TRAP_OVERFLOW);
        let r = if signed { self.b.ins().fcvt_to_sint_sat(to, x) } else { self.b.ins().fcvt_to_uint_sat(to, x) };
        self.wr(i.a, r);
    }

    fn trunc_sat(&mut self, i: Inst, from: Type, to: Type, signed: bool) {
        let x = self.rd(i.b, from);
        let r = if signed { self.b.ins().fcvt_to_sint_sat(to, x) } else { self.b.ins().fcvt_to_uint_sat(to, x) };
        self.wr(i.a, r);
    }

    fn div_rem(&mut self, i: Inst, t: Type, signed: bool, rem: bool) {
        let x = self.rd(i.b, t);
        let y = self.rd(i.c, t);
        let zero = self.ic(t, 0);
        let z = self.b.ins().icmp(IntCC::Equal, y, zero);
        self.guard(z, TRAP_DIV0);
        let r = if signed {
            let m1 = self.ic(t, -1);
            let is_m1 = self.b.ins().icmp(IntCC::Equal, y, m1);
            if !rem {
                let min = if t == I32 { i32::MIN as u32 as i64 } else { i64::MIN };
                let minv = self.ic(t, min);
                let is_min = self.b.ins().icmp(IntCC::Equal, x, minv);
                let bad = self.b.ins().band(is_min, is_m1);
                self.guard(bad, TRAP_OVERFLOW);
                self.b.ins().sdiv(x, y)
            } else {
                // x % -1 == 0: divide by 1 instead (also avoids the hardware overflow).
                let one = self.ic(t, 1);
                let y2 = self.b.ins().select(is_m1, one, y);
                self.b.ins().srem(x, y2)
            }
        } else if rem {
            self.b.ins().urem(x, y)
        } else {
            self.b.ins().udiv(x, y)
        };
        self.wr(i.a, r);
    }

    fn mem_status_guard(&mut self, st: Value) {
        self.guard(st, TRAP_OOB);
    }

    // ---- calls ----

    /// Loads the call results from the call slot into the window registers.
    fn load_results(&mut self, base: u32, nr: usize) {
        for k in 0..nr {
            let v = self.b.ins().stack_load(I64, I64, self.callslot, (k * 8) as i32);
            self.set(base + k as u32, v);
        }
    }

    /// The common tail of a call: dispatch on the callee status.
    fn finish_call(&mut self, status: Value, base: u32, nr: usize, may_host: bool, restore: Option<(Value, Value)>, next: Block) {
        let (okb, slow) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(status, slow, &[], okb, &[]);
        self.b.set_cold_block(slow);
        self.b.seal_block(slow);
        self.b.seal_block(okb);
        self.b.switch_to_block(okb);
        if let Some((d, ru)) = restore {
            self.ctx_store(d, JitCtx::OFF_DEPTH);
            self.ctx_store(ru, JitCtx::OFF_REGS_USED);
        }
        self.load_results(base, nr);
        self.b.ins().jump(next, &[]);

        self.b.switch_to_block(slow);
        let two = self.ic(I32, 2);
        let is2 = self.b.ins().icmp(IntCC::Equal, status, two);
        let (trapb, rest) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is2, trapb, &[], rest, &[]);
        self.b.seal_block(trapb);
        self.b.seal_block(rest);
        self.b.switch_to_block(trapb);
        let (f, i) = (self.ic(I32, self.fidx as i64), self.ic(I32, self.idx as i64));
        self.b.ins().call(self.h.trap_frame, &[self.ctx, f, i]);
        self.b.ins().return_(&[two]);

        self.b.switch_to_block(rest);
        let susb = self.b.create_block();
        if may_host {
            let three = self.ic(I32, 3);
            let is3 = self.b.ins().icmp(IntCC::Equal, status, three);
            let hyb = self.b.create_block();
            self.b.ins().brif(is3, hyb, &[], susb, &[]);
            self.b.seal_block(hyb);
            self.b.switch_to_block(hyb);
            // Host yield: results are in the window; this activation is the innermost.
            self.load_results(base, nr);
            let slot = self.spill();
            let (k, f, p, s) = (
                self.ic(I32, SUSPEND_YIELD as i64),
                self.ic(I32, self.fidx as i64),
                self.ic(I32, self.idx as i64 + 1),
                self.ic(I32, self.nregs as i64),
            );
            self.b.ins().call(self.h.save_inner, &[self.ctx, k, f, p, slot, s]);
            let one = self.ic(I32, 1);
            self.b.ins().return_(&[one]);
        } else {
            self.b.ins().jump(susb, &[]);
        }
        self.b.seal_block(susb);
        self.b.switch_to_block(susb);
        let slot = self.spill();
        let (f, p, s, bs) = (
            self.ic(I32, self.fidx as i64),
            self.ic(I32, self.idx as i64 + 1),
            self.ic(I32, self.nregs as i64),
            self.ic(I32, base as i64),
        );
        self.b.ins().call(self.h.save_frame, &[self.ctx, f, p, slot, s, bs]);
        let one = self.ic(I32, 1);
        self.b.ins().return_(&[one]);
    }

    fn store_window(&mut self, base: u32, np: usize) -> Value {
        for k in 0..np {
            let v = self.get(base + k as u32);
            self.b.ins().stack_store(I64, v, self.callslot, (k * 8) as i32);
        }
        self.b.ins().stack_addr(I64, self.callslot, 0)
    }

    /// Depth and register-stack checks of a call into a module function, then accounting.
    fn enter_checks(&mut self, size: Value /* I64 */, d: Value, ru: Value) {
        let d64 = self.b.ins().uextend(I64, d);
        let one = self.ic(I64, 1);
        let nd = self.b.ins().iadd(d64, one);
        let md = self.ctx_load(I32, JitCtx::OFF_MAX_DEPTH);
        let md64 = self.b.ins().uextend(I64, md);
        let bad = self.b.ins().icmp(IntCC::UnsignedGreaterThan, nd, md64);
        self.guard(bad, TRAP_STACK_OVERFLOW);
        let nru = self.b.ins().iadd(ru, size);
        let mr = self.ctx_load(I64, JitCtx::OFF_MAX_REGS);
        let bad = self.b.ins().icmp(IntCC::UnsignedGreaterThan, nru, mr);
        self.guard(bad, TRAP_STACK_OVERFLOW);
        let nd32 = self.b.ins().ireduce(I32, nd);
        self.ctx_store(nd32, JitCtx::OFF_DEPTH);
        self.ctx_store(nru, JitCtx::OFF_REGS_USED);
    }

    fn call_direct(&mut self, g: u32, base: u32) {
        let info = self.infos[g as usize];
        let (np, nr) = (info.np as usize, info.nr as usize);
        let is_import = g < self.nimports;
        let next = self.block_at(self.idx + 1);
        let site = self.b.create_block();
        let mode = self.b.append_block_param(site, I32);
        let zero = self.ic(I32, 0);
        self.b.ins().jump(site, &[zero.into()]);
        self.b.switch_to_block(site);
        if !is_import {
            self.sites.push((self.idx + 1, site));
        }
        let restore = if is_import {
            None
        } else {
            let d = self.ctx_load(I32, JitCtx::OFF_DEPTH);
            let ru = self.ctx_load(I64, JitCtx::OFF_REGS_USED);
            let size = self.ic(I64, info.size as i64);
            self.enter_checks(size, d, ru);
            Some((d, ru))
        };
        let win = self.store_window(base, np);
        let callee = self.callees[&g];
        let call = self.b.ins().call(callee, &[self.ctx, win, mode]);
        let status = self.b.inst_results(call)[0];
        self.reload_mem();
        self.finish_call(status, base, nr, is_import, restore, next);
    }

    fn call_indirect(&mut self, type_idx: u32, areg: u32, base: u32) {
        let t = self.vm.data().types[type_idx as usize].clone();
        let (np, nr) = (t.params.len(), t.results.len());
        let want = self.type_canon[type_idx as usize];
        let next = self.block_at(self.idx + 1);
        let site = self.b.create_block();
        let mode = self.b.append_block_param(site, I32);
        let zero = self.ic(I32, 0);
        self.b.ins().jump(site, &[zero.into()]);
        self.b.switch_to_block(site);
        self.sites.push((self.idx + 1, site));
        // Table lookup.
        let r = self.rd(areg, I32);
        let idx = self.b.ins().uextend(I64, r);
        let tl = self.ctx_load(I64, JitCtx::OFF_TABLE_LEN);
        let oob = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, tl);
        self.guard(oob, TRAP_UNDEF_ELEM);
        let tbl = self.ctx_load(I64, JitCtx::OFF_TABLE);
        let esz = self.ic(I64, TableEntry::SIZE);
        let off = self.b.ins().imul(idx, esz);
        let ent = self.b.ins().iadd(tbl, off);
        let code = self.b.ins().load(I64, MemFlagsData::new(), ent, TableEntry::OFF_CODE);
        let null = self.b.ins().icmp_imm_u(IntCC::Equal, code, 0);
        self.guard(null, TRAP_UNDEF_ELEM);
        let canon = self.b.ins().load(I32, MemFlagsData::new(), ent, TableEntry::OFF_CANON);
        let wantv = self.ic(I32, want as i64);
        let mism = self.b.ins().icmp(IntCC::NotEqual, canon, wantv);
        self.guard(mism, TRAP_TYPE_MISMATCH);
        let is_host = self.b.ins().load(I32, MemFlagsData::new(), ent, TableEntry::OFF_IS_HOST);
        let d = self.ctx_load(I32, JitCtx::OFF_DEPTH);
        let ru = self.ctx_load(I64, JitCtx::OFF_REGS_USED);
        let (callb, mp) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_host, callb, &[], mp, &[]);
        self.b.seal_block(mp);
        self.b.switch_to_block(mp);
        let sz32 = self.b.ins().load(I32, MemFlagsData::new(), ent, TableEntry::OFF_SIZE);
        let sz = self.b.ins().uextend(I64, sz32);
        self.enter_checks(sz, d, ru);
        self.b.ins().jump(callb, &[]);
        self.b.seal_block(callb);
        self.b.switch_to_block(callb);
        let win = self.store_window(base, np);
        let s = self.b.import_signature(self.sig_fn.clone());
        let call = self.b.ins().call_indirect(s, code, &[self.ctx, win, mode]);
        let status = self.b.inst_results(call)[0];
        self.reload_mem();
        self.finish_call(status, base, nr, true, Some((d, ru)), next);
    }

    // ---- instructions ----

    fn inst(&mut self, f: &efevm::Function, i: Inst, idx: u32) -> Result<(), JitError> {
        use Op::*;
        match i.op {
            Nop => {}
            Unreachable => self.emit_trap(TRAP_UNREACHABLE),
            Yield => {
                let slot = self.spill();
                let (k, fv, p, s) = (
                    self.ic(I32, SUSPEND_YIELD as i64),
                    self.ic(I32, self.fidx as i64),
                    self.ic(I32, idx as i64 + 1),
                    self.ic(I32, self.nregs as i64),
                );
                self.b.ins().call(self.h.save_inner, &[self.ctx, k, fv, p, slot, s]);
                let one = self.ic(I32, 1);
                self.b.ins().return_(&[one]);
            }
            Ret => {
                for k in 0..self.nr {
                    let v = self.get(i.a + k as u32);
                    self.b.ins().store(MemFlagsData::new(), v, self.window, (k * 8) as i32);
                }
                let z = self.ic(I32, 0);
                self.b.ins().return_(&[z]);
            }
            Jmp => {
                let t = self.block_at(i.a);
                self.b.ins().jump(t, &[]);
            }
            BrIf | BrIfz => {
                let c = self.rd(i.a, I32);
                let (t, n) = (self.block_at(i.b), self.block_at(idx + 1));
                if i.op == BrIf {
                    self.b.ins().brif(c, t, &[], n, &[]);
                } else {
                    self.b.ins().brif(c, n, &[], t, &[]);
                }
            }
            BrTable => {
                let tab = &f.br_tables[i.b as usize];
                let c = self.rd(i.a, I32);
                let mut sw = Switch::new();
                for (k, l) in tab.targets.iter().enumerate() {
                    sw.set_entry(k as u128, self.block_at(*l));
                }
                let def = self.block_at(tab.default);
                sw.emit(&mut self.b, c, def);
            }
            Call => self.call_direct(i.a, i.b),
            CallIndirect => self.call_indirect(i.a, i.b, i.c),
            ConstI32 | ConstF32 => {
                let v = self.ic(I64, i.b as i64);
                self.set(i.a, v);
            }
            ConstI64 | ConstF64 => {
                let v = self.ic(I64, i.const_i64_value() as i64);
                self.set(i.a, v);
            }
            Mov => {
                let v = self.get(i.b);
                self.set(i.a, v);
            }
            Select => {
                let (x, y) = i.select_regs();
                let c = self.rd(i.b, I32);
                let cb = self.b.ins().icmp_imm_u(IntCC::NotEqual, c, 0);
                let (xv, yv) = (self.get(x), self.get(y));
                let v = self.b.ins().select(cb, xv, yv);
                self.set(i.a, v);
            }
            GlobalGet => {
                let g = self.ctx_load(I64, JitCtx::OFF_GLOBALS);
                let v = self.b.ins().load(I64, mf(), g, (i.b * 8) as i32);
                self.set(i.a, v);
            }
            GlobalSet => {
                let g = self.ctx_load(I64, JitCtx::OFF_GLOBALS);
                let v = self.get(i.b);
                self.b.ins().store(mf(), v, g, (i.a * 8) as i32);
            }
            MemorySize => {
                let len = self.b.use_var(self.mem_len);
                let s = self.ic(I64, 16);
                let v = self.b.ins().ushr(len, s);
                self.set(i.a, v);
            }
            MemoryGrow => {
                let d = self.get(i.b);
                let call = self.b.ins().call(self.h.grow, &[self.ctx, d]);
                let r = self.b.inst_results(call)[0];
                self.set(i.a, r);
                self.reload_mem();
            }
            MemoryCopy | MemoryFill => {
                let (a, b, c) = (self.get(i.a), self.get(i.b), self.get(i.c));
                let h = if i.op == MemoryCopy { self.h.copy } else { self.h.fill };
                let call = self.b.ins().call(h, &[self.ctx, a, b, c]);
                let st = self.b.inst_results(call)[0];
                self.mem_status_guard(st);
            }

            // ---- i32 ----
            I32Add => self.bin(i, I32, |b, x, y| b.ins().iadd(x, y)),
            I32Sub => self.bin(i, I32, |b, x, y| b.ins().isub(x, y)),
            I32Mul => self.bin(i, I32, |b, x, y| b.ins().imul(x, y)),
            I32DivS => self.div_rem(i, I32, true, false),
            I32DivU => self.div_rem(i, I32, false, false),
            I32RemS => self.div_rem(i, I32, true, true),
            I32RemU => self.div_rem(i, I32, false, true),
            I32And => self.bin(i, I32, |b, x, y| b.ins().band(x, y)),
            I32Or => self.bin(i, I32, |b, x, y| b.ins().bor(x, y)),
            I32Xor => self.bin(i, I32, |b, x, y| b.ins().bxor(x, y)),
            I32Shl => self.bin(i, I32, |b, x, y| b.ins().ishl(x, y)),
            I32ShrS => self.bin(i, I32, |b, x, y| b.ins().sshr(x, y)),
            I32ShrU => self.bin(i, I32, |b, x, y| b.ins().ushr(x, y)),
            I32Rotl => self.bin(i, I32, |b, x, y| b.ins().rotl(x, y)),
            I32Rotr => self.bin(i, I32, |b, x, y| b.ins().rotr(x, y)),
            I32Clz => self.un(i, I32, |b, x| b.ins().clz(x)),
            I32Ctz => self.un(i, I32, |b, x| b.ins().ctz(x)),
            I32Popcnt => self.un(i, I32, |b, x| b.ins().popcnt(x)),
            I32Eqz => self.un(i, I32, |b, x| b.ins().icmp_imm_u(IntCC::Equal, x, 0)),
            I32Eq => self.icmp(i, I32, IntCC::Equal),
            I32Ne => self.icmp(i, I32, IntCC::NotEqual),
            I32LtS => self.icmp(i, I32, IntCC::SignedLessThan),
            I32LtU => self.icmp(i, I32, IntCC::UnsignedLessThan),
            I32GtS => self.icmp(i, I32, IntCC::SignedGreaterThan),
            I32GtU => self.icmp(i, I32, IntCC::UnsignedGreaterThan),
            I32LeS => self.icmp(i, I32, IntCC::SignedLessThanOrEqual),
            I32LeU => self.icmp(i, I32, IntCC::UnsignedLessThanOrEqual),
            I32GeS => self.icmp(i, I32, IntCC::SignedGreaterThanOrEqual),
            I32GeU => self.icmp(i, I32, IntCC::UnsignedGreaterThanOrEqual),

            // ---- i64 ----
            I64Add => self.bin(i, I64, |b, x, y| b.ins().iadd(x, y)),
            I64Sub => self.bin(i, I64, |b, x, y| b.ins().isub(x, y)),
            I64Mul => self.bin(i, I64, |b, x, y| b.ins().imul(x, y)),
            I64DivS => self.div_rem(i, I64, true, false),
            I64DivU => self.div_rem(i, I64, false, false),
            I64RemS => self.div_rem(i, I64, true, true),
            I64RemU => self.div_rem(i, I64, false, true),
            I64And => self.bin(i, I64, |b, x, y| b.ins().band(x, y)),
            I64Or => self.bin(i, I64, |b, x, y| b.ins().bor(x, y)),
            I64Xor => self.bin(i, I64, |b, x, y| b.ins().bxor(x, y)),
            I64Shl => self.bin(i, I64, |b, x, y| b.ins().ishl(x, y)),
            I64ShrS => self.bin(i, I64, |b, x, y| b.ins().sshr(x, y)),
            I64ShrU => self.bin(i, I64, |b, x, y| b.ins().ushr(x, y)),
            I64Rotl => self.bin(i, I64, |b, x, y| b.ins().rotl(x, y)),
            I64Rotr => self.bin(i, I64, |b, x, y| b.ins().rotr(x, y)),
            I64Clz => self.un(i, I64, |b, x| b.ins().clz(x)),
            I64Ctz => self.un(i, I64, |b, x| b.ins().ctz(x)),
            I64Popcnt => self.un(i, I64, |b, x| b.ins().popcnt(x)),
            I64Eqz => self.un(i, I64, |b, x| b.ins().icmp_imm_u(IntCC::Equal, x, 0)),
            I64Eq => self.icmp(i, I64, IntCC::Equal),
            I64Ne => self.icmp(i, I64, IntCC::NotEqual),
            I64LtS => self.icmp(i, I64, IntCC::SignedLessThan),
            I64LtU => self.icmp(i, I64, IntCC::UnsignedLessThan),
            I64GtS => self.icmp(i, I64, IntCC::SignedGreaterThan),
            I64GtU => self.icmp(i, I64, IntCC::UnsignedGreaterThan),
            I64LeS => self.icmp(i, I64, IntCC::SignedLessThanOrEqual),
            I64LeU => self.icmp(i, I64, IntCC::UnsignedLessThanOrEqual),
            I64GeS => self.icmp(i, I64, IntCC::SignedGreaterThanOrEqual),
            I64GeU => self.icmp(i, I64, IntCC::UnsignedGreaterThanOrEqual),

            // ---- loads / stores ----
            I32Load | F32Load => self.load(i, 4, |b, p| b.ins().load(I32, mf(), p, 0)),
            I64Load | F64Load => self.load(i, 8, |b, p| b.ins().load(I64, mf(), p, 0)),
            I32Load8S => self.load(i, 1, |b, p| b.ins().sload8(I32, mf(), p, 0)),
            I32Load8U => self.load(i, 1, |b, p| b.ins().uload8(I32, mf(), p, 0)),
            I32Load16S => self.load(i, 2, |b, p| b.ins().sload16(I32, mf(), p, 0)),
            I32Load16U => self.load(i, 2, |b, p| b.ins().uload16(I32, mf(), p, 0)),
            I64Load8S => self.load(i, 1, |b, p| b.ins().sload8(I64, mf(), p, 0)),
            I64Load8U => self.load(i, 1, |b, p| b.ins().uload8(I64, mf(), p, 0)),
            I64Load16S => self.load(i, 2, |b, p| b.ins().sload16(I64, mf(), p, 0)),
            I64Load16U => self.load(i, 2, |b, p| b.ins().uload16(I64, mf(), p, 0)),
            I64Load32S => self.load(i, 4, |b, p| b.ins().sload32(mf(), p, 0)),
            I64Load32U => self.load(i, 4, |b, p| b.ins().uload32(mf(), p, 0)),
            I32Store | F32Store | I64Store32 => self.store(i, 4, |b, v, p| {
                b.ins().istore32(mf(), v, p, 0);
            }),
            I64Store | F64Store => self.store(i, 8, |b, v, p| {
                b.ins().store(mf(), v, p, 0);
            }),
            I32Store8 | I64Store8 => self.store(i, 1, |b, v, p| {
                b.ins().istore8(mf(), v, p, 0);
            }),
            I32Store16 | I64Store16 => self.store(i, 2, |b, v, p| {
                b.ins().istore16(mf(), v, p, 0);
            }),

            // ---- f32 ----
            F32Add => self.bin(i, F32, |b, x, y| b.ins().fadd(x, y)),
            F32Sub => self.bin(i, F32, |b, x, y| b.ins().fsub(x, y)),
            F32Mul => self.bin(i, F32, |b, x, y| b.ins().fmul(x, y)),
            F32Div => self.bin(i, F32, |b, x, y| b.ins().fdiv(x, y)),
            F32Min => self.bin(i, F32, |b, x, y| b.ins().fmin(x, y)),
            F32Max => self.bin(i, F32, |b, x, y| b.ins().fmax(x, y)),
            F32Copysign => self.bin(i, F32, |b, x, y| b.ins().fcopysign(x, y)),
            F32Abs => {
                let x = self.get(i.b);
                let m = self.ic(I64, 0x7FFF_FFFF);
                let v = self.b.ins().band(x, m);
                self.set(i.a, v);
            }
            F32Neg => {
                let x = self.get(i.b);
                let m = self.ic(I64, 0x8000_0000);
                let v = self.b.ins().bxor(x, m);
                let m2 = self.ic(I64, 0xFFFF_FFFF);
                let v = self.b.ins().band(v, m2);
                self.set(i.a, v);
            }
            F32Sqrt => self.un(i, F32, |b, x| b.ins().sqrt(x)),
            F32Ceil => self.un(i, F32, |b, x| b.ins().ceil(x)),
            F32Floor => self.un(i, F32, |b, x| b.ins().floor(x)),
            F32Trunc => self.un(i, F32, |b, x| b.ins().trunc(x)),
            F32Nearest => self.un(i, F32, |b, x| b.ins().nearest(x)),
            F32Eq => self.fcmp(i, F32, FloatCC::Equal),
            F32Ne => self.fcmp(i, F32, FloatCC::NotEqual),
            F32Lt => self.fcmp(i, F32, FloatCC::LessThan),
            F32Gt => self.fcmp(i, F32, FloatCC::GreaterThan),
            F32Le => self.fcmp(i, F32, FloatCC::LessThanOrEqual),
            F32Ge => self.fcmp(i, F32, FloatCC::GreaterThanOrEqual),

            // ---- f64 ----
            F64Add => self.bin(i, F64, |b, x, y| b.ins().fadd(x, y)),
            F64Sub => self.bin(i, F64, |b, x, y| b.ins().fsub(x, y)),
            F64Mul => self.bin(i, F64, |b, x, y| b.ins().fmul(x, y)),
            F64Div => self.bin(i, F64, |b, x, y| b.ins().fdiv(x, y)),
            F64Min => self.bin(i, F64, |b, x, y| b.ins().fmin(x, y)),
            F64Max => self.bin(i, F64, |b, x, y| b.ins().fmax(x, y)),
            F64Copysign => self.bin(i, F64, |b, x, y| b.ins().fcopysign(x, y)),
            F64Abs => {
                let x = self.get(i.b);
                let m = self.ic(I64, 0x7FFF_FFFF_FFFF_FFFF);
                let v = self.b.ins().band(x, m);
                self.set(i.a, v);
            }
            F64Neg => {
                let x = self.get(i.b);
                let m = self.ic(I64, i64::MIN);
                let v = self.b.ins().bxor(x, m);
                self.set(i.a, v);
            }
            F64Sqrt => self.un(i, F64, |b, x| b.ins().sqrt(x)),
            F64Ceil => self.un(i, F64, |b, x| b.ins().ceil(x)),
            F64Floor => self.un(i, F64, |b, x| b.ins().floor(x)),
            F64Trunc => self.un(i, F64, |b, x| b.ins().trunc(x)),
            F64Nearest => self.un(i, F64, |b, x| b.ins().nearest(x)),
            F64Eq => self.fcmp(i, F64, FloatCC::Equal),
            F64Ne => self.fcmp(i, F64, FloatCC::NotEqual),
            F64Lt => self.fcmp(i, F64, FloatCC::LessThan),
            F64Gt => self.fcmp(i, F64, FloatCC::GreaterThan),
            F64Le => self.fcmp(i, F64, FloatCC::LessThanOrEqual),
            F64Ge => self.fcmp(i, F64, FloatCC::GreaterThanOrEqual),

            // ---- width conversions ----
            I32WrapI64 => {
                let x = self.get(i.b);
                let m = self.ic(I64, 0xFFFF_FFFF);
                let v = self.b.ins().band(x, m);
                self.set(i.a, v);
            }
            I64ExtendI32S => self.un(i, I32, |b, x| b.ins().sextend(I64, x)),
            I64ExtendI32U => self.un(i, I32, |b, x| b.ins().uextend(I64, x)),
            I32Extend8S => self.un(i, I32, |b, x| {
                let y = b.ins().ireduce(types::I8, x);
                b.ins().sextend(I32, y)
            }),
            I32Extend16S => self.un(i, I32, |b, x| {
                let y = b.ins().ireduce(types::I16, x);
                b.ins().sextend(I32, y)
            }),
            I64Extend8S => self.un(i, I64, |b, x| {
                let y = b.ins().ireduce(types::I8, x);
                b.ins().sextend(I64, y)
            }),
            I64Extend16S => self.un(i, I64, |b, x| {
                let y = b.ins().ireduce(types::I16, x);
                b.ins().sextend(I64, y)
            }),
            I64Extend32S => self.un(i, I64, |b, x| {
                let y = b.ins().ireduce(I32, x);
                b.ins().sextend(I64, y)
            }),

            // ---- trunc ----
            I32TruncF32S => self.trunc_trap(i, F32, I32, true),
            I32TruncF32U => self.trunc_trap(i, F32, I32, false),
            I32TruncF64S => self.trunc_trap(i, F64, I32, true),
            I32TruncF64U => self.trunc_trap(i, F64, I32, false),
            I64TruncF32S => self.trunc_trap(i, F32, I64, true),
            I64TruncF32U => self.trunc_trap(i, F32, I64, false),
            I64TruncF64S => self.trunc_trap(i, F64, I64, true),
            I64TruncF64U => self.trunc_trap(i, F64, I64, false),
            I32TruncSatF32S => self.trunc_sat(i, F32, I32, true),
            I32TruncSatF32U => self.trunc_sat(i, F32, I32, false),
            I32TruncSatF64S => self.trunc_sat(i, F64, I32, true),
            I32TruncSatF64U => self.trunc_sat(i, F64, I32, false),
            I64TruncSatF32S => self.trunc_sat(i, F32, I64, true),
            I64TruncSatF32U => self.trunc_sat(i, F32, I64, false),
            I64TruncSatF64S => self.trunc_sat(i, F64, I64, true),
            I64TruncSatF64U => self.trunc_sat(i, F64, I64, false),

            // ---- convert / demote / promote / reinterpret ----
            F32ConvertI32S => self.un(i, I32, |b, x| b.ins().fcvt_from_sint(F32, x)),
            F32ConvertI32U => self.un(i, I32, |b, x| b.ins().fcvt_from_uint(F32, x)),
            F32ConvertI64S => self.un(i, I64, |b, x| b.ins().fcvt_from_sint(F32, x)),
            F32ConvertI64U => self.un(i, I64, |b, x| b.ins().fcvt_from_uint(F32, x)),
            F64ConvertI32S => self.un(i, I32, |b, x| b.ins().fcvt_from_sint(F64, x)),
            F64ConvertI32U => self.un(i, I32, |b, x| b.ins().fcvt_from_uint(F64, x)),
            F64ConvertI64S => self.un(i, I64, |b, x| b.ins().fcvt_from_sint(F64, x)),
            F64ConvertI64U => self.un(i, I64, |b, x| b.ins().fcvt_from_uint(F64, x)),
            F32DemoteF64 => self.un(i, F64, |b, x| b.ins().fdemote(F32, x)),
            F64PromoteF32 => self.un(i, F32, |b, x| b.ins().fpromote(F64, x)),
            I32ReinterpretF32 | F32ReinterpretI32 => {
                let x = self.get(i.b);
                let m = self.ic(I64, 0xFFFF_FFFF);
                let v = self.b.ins().band(x, m);
                self.set(i.a, v);
            }
            I64ReinterpretF64 | F64ReinterpretI64 => {
                let v = self.get(i.b);
                self.set(i.a, v);
            }
        }
        Ok(())
    }
}
