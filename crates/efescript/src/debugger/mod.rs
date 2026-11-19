//! Line-oriented debugger library for the efevm bytecode VM (design: e3-design.md section 8).

use std::collections::HashMap;
use std::io::{BufRead, Result as IoResult, Write};

use efevm::api::ApiIndex;
use efevm::op::{Op, Shape};
use efevm::trap::{Exit, Trap};
use efevm::{ExportKind, FiberId, Instance, Limits, LineEntry, Module};

/// A breakpoint in a module.
#[derive(Clone, Debug)]
struct Breakpoint {
    id: u32,
    func: u32,
    inst: u32,
    desc: String,
}

/// A line-oriented debugger for efevm bytecode execution.
pub struct Debugger<H: 'static> {
    instance: Instance<H>,
    fiber: FiberId,
    module: Module,
    index: Option<ApiIndex>,
    sources: HashMap<String, String>,
    breakpoints: Vec<Breakpoint>,
    next_bp_id: u32,
    ended: bool,
}

impl<H: 'static> Debugger<H> {
    /// Links `module` against `imports` and spawns export `entry` with no args.
    pub fn new(
        module: Module,
        imports: &efevm::Imports<H>,
        entry: &str,
        index: Option<ApiIndex>,
    ) -> Result<Self, String> {
        let mut instance = Instance::new(&module, imports, Limits::default())
            .map_err(|e| e.to_string())?;
        let fiber = instance.spawn(entry, &[])
            .map_err(|e| e.to_string())?;
        Ok(Debugger {
            instance,
            fiber,
            module,
            index,
            sources: HashMap::new(),
            breakpoints: Vec::new(),
            next_bp_id: 1,
            ended: false,
        })
    }

    /// Optional source text for `list` and line breakpoints, keyed by the file name in the lines section.
    pub fn add_source(&mut self, file: &str, text: &str) {
        self.sources.insert(file.to_string(), text.to_string());
    }

    /// Access the underlying instance.
    pub fn instance(&self) -> &Instance<H> {
        &self.instance
    }

    /// Mutably access the underlying instance.
    pub fn instance_mut(&mut self) -> &mut Instance<H> {
        &mut self.instance
    }

    /// The paused fiber being debugged.
    pub fn fiber(&self) -> FiberId {
        self.fiber
    }

    /// The module being debugged.
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Whether the program has ended (completed or trapped).
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Executes one debugger command line, writing output to `out`. Returns false after `quit` or when the program has ended.
    pub fn command(
        &mut self,
        line: &str,
        host: &mut H,
        out: &mut dyn Write,
    ) -> IoResult<bool> {
        if self.ended {
            writeln!(out, "program has ended")?;
            return Ok(false);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(true);
        }

        let (cmd, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (trimmed, ""),
        };

        match cmd {
            "break" | "b" => {
                self.cmd_break(rest, out)?;
                Ok(true)
            }
            "delete" | "d" => {
                self.cmd_delete(rest, out)?;
                Ok(true)
            }
            "step" | "s" => self.cmd_step(host, out),
            "next" | "n" => self.cmd_next(host, out),
            "continue" | "c" => self.cmd_continue(host, out),
            "regs" => {
                self.cmd_regs(out)?;
                Ok(true)
            }
            "mem" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                self.cmd_mem(&parts, out)?;
                Ok(true)
            }
            "bt" => {
                self.cmd_bt(out)?;
                Ok(true)
            }
            "list" | "l" => {
                self.cmd_list(out)?;
                Ok(true)
            }
            "where" | "w" => {
                self.cmd_where(out)?;
                Ok(true)
            }
            "quit" | "q" => Ok(false),
            "help" | "h" | "?" => {
                self.cmd_help(out)?;
                Ok(true)
            }
            other => {
                writeln!(out, "unknown command `{other}`. Type 'help' for available commands.")?;
                Ok(true)
            }
        }
    }

    /// Reads commands from `input` until quit/EOF/program end, printing a `(efdb) ` prompt before each.
    pub fn run_script(
        &mut self,
        host: &mut H,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> IoResult<()> {
        while !self.ended {
            write!(out, "(efdb) ")?;
            out.flush()?;
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !self.command(trimmed, host, out)? {
                break;
            }
        }
        Ok(())
    }

    fn step_one_instruction(&mut self, host: &mut H) -> Exit {
        self.instance.step(self.fiber, host)
    }

    fn cmd_step(&mut self, host: &mut H, out: &mut dyn Write) -> IoResult<bool> {
        if self.ended {
            writeln!(out, "program has ended")?;
            return Ok(false);
        }
        let exit = self.step_one_instruction(host);
        match exit {
            Exit::Done => {
                self.ended = true;
                self.print_program_finished(out)?;
                Ok(false)
            }
            Exit::Trap(trap) => {
                self.ended = true;
                self.print_trap(&trap, out)?;
                Ok(false)
            }
            Exit::OutOfBudget | Exit::Yield => {
                self.print_stop_location(out)?;
                Ok(true)
            }
        }
    }

    fn cmd_continue(&mut self, host: &mut H, out: &mut dyn Write) -> IoResult<bool> {
        if self.ended {
            writeln!(out, "program has ended")?;
            return Ok(false);
        }
        let mut steps: u64 = 0;
        const MAX_STEPS: u64 = 100_000_000;
        while steps < MAX_STEPS {
            steps += 1;
            let exit = self.step_one_instruction(host);
            match exit {
                Exit::Done => {
                    self.ended = true;
                    self.print_program_finished(out)?;
                    return Ok(false);
                }
                Exit::Trap(trap) => {
                    self.ended = true;
                    self.print_trap(&trap, out)?;
                    return Ok(false);
                }
                Exit::OutOfBudget | Exit::Yield => {
                    if let Some(loc) = self.instance.fiber_location(self.fiber) {
                        if self.is_breakpoint(loc) {
                            self.print_stop_location(out)?;
                            return Ok(true);
                        }
                    }
                }
            }
        }
        writeln!(out, "step limit reached ({MAX_STEPS} steps)")?;
        self.print_stop_location(out)?;
        Ok(true)
    }

    fn cmd_next(&mut self, host: &mut H, out: &mut dyn Write) -> IoResult<bool> {
        if self.ended {
            writeln!(out, "program has ended")?;
            return Ok(false);
        }
        let initial_depth = self.instance.fiber_backtrace(self.fiber).len();
        let exit = self.step_one_instruction(host);
        match exit {
            Exit::Done => {
                self.ended = true;
                self.print_program_finished(out)?;
                return Ok(false);
            }
            Exit::Trap(trap) => {
                self.ended = true;
                self.print_trap(&trap, out)?;
                return Ok(false);
            }
            Exit::OutOfBudget | Exit::Yield => {}
        }

        let mut steps: u64 = 1;
        const MAX_STEPS: u64 = 100_000_000;
        while steps < MAX_STEPS {
            let depth = self.instance.fiber_backtrace(self.fiber).len();
            if depth <= initial_depth {
                break;
            }
            if let Some(loc) = self.instance.fiber_location(self.fiber) {
                if self.is_breakpoint(loc) {
                    break;
                }
            }
            steps += 1;
            let exit = self.step_one_instruction(host);
            match exit {
                Exit::Done => {
                    self.ended = true;
                    self.print_program_finished(out)?;
                    return Ok(false);
                }
                Exit::Trap(trap) => {
                    self.ended = true;
                    self.print_trap(&trap, out)?;
                    return Ok(false);
                }
                Exit::OutOfBudget | Exit::Yield => {}
            }
        }
        self.print_stop_location(out)?;
        Ok(true)
    }

    fn cmd_regs(&self, out: &mut dyn Write) -> IoResult<()> {
        let Some(regs) = self.instance.fiber_registers(self.fiber) else {
            writeln!(out, "no registers available")?;
            return Ok(());
        };
        let (func, _) = self.instance.fiber_location(self.fiber).unwrap_or((0, 0));
        let names = &self.module.data().names.regs;
        for (i, &val) in regs.iter().enumerate() {
            let reg_name = names
                .iter()
                .find(|(f, r, _)| *f == func && *r == i as u32)
                .map(|(_, _, n)| n.as_str());
            if let Some(name) = reg_name {
                writeln!(out, "r{i} ({name}) = {val} (0x{val:x})")?;
            } else {
                writeln!(out, "r{i} = {val} (0x{val:x})")?;
            }
        }
        Ok(())
    }

    fn cmd_mem(&self, args: &[&str], out: &mut dyn Write) -> IoResult<()> {
        if args.len() < 2 {
            writeln!(out, "usage: mem <addr> <len>")?;
            return Ok(());
        }
        let Some(addr) = parse_u64(args[0]) else {
            writeln!(out, "invalid address: {}", args[0])?;
            return Ok(());
        };
        let Some(len) = parse_u64(args[1]) else {
            writeln!(out, "invalid length: {}", args[1])?;
            return Ok(());
        };
        if len == 0 {
            return Ok(());
        }
        if len > 65536 * 1024 {
            writeln!(out, "length too large: {len}")?;
            return Ok(());
        }
        let mut buf = vec![0u8; len as usize];
        if self.instance.memory().read(addr, &mut buf).is_err() {
            writeln!(out, "error: out of bounds memory access")?;
            return Ok(());
        }

        for (chunk_idx, chunk) in buf.chunks(16).enumerate() {
            let line_addr = addr + (chunk_idx * 16) as u64;
            let mut hex_parts = Vec::with_capacity(chunk.len());
            let mut ascii_part = String::with_capacity(chunk.len());
            for &b in chunk {
                hex_parts.push(format!("{b:02x}"));
                if (0x20..=0x7e).contains(&b) {
                    ascii_part.push(b as char);
                } else {
                    ascii_part.push('.');
                }
            }
            let hex_str = hex_parts.join(" ");
            writeln!(out, "{line_addr:08x}  {hex_str:<47}  |{ascii_part}|")?;
        }
        Ok(())
    }

    fn cmd_bt(&self, out: &mut dyn Write) -> IoResult<()> {
        let mut trace = self.instance.fiber_backtrace(self.fiber);
        if trace.is_empty() {
            if let Some(bt) = self.instance.backtrace(self.fiber) {
                trace = bt.to_vec();
            }
        }
        if trace.is_empty() {
            writeln!(out, "no backtrace available")?;
            return Ok(());
        }
        for (i, &(func, inst)) in trace.iter().enumerate() {
            let name = self.func_name(func);
            if let Some((file, line, col)) = self.lookup_line(func, inst) {
                writeln!(out, "#{i}  {name} at {file}:{line}:{col}")?;
            } else {
                writeln!(out, "#{i}  {name} (inst {inst})")?;
            }
        }
        Ok(())
    }

    fn cmd_list(&self, out: &mut dyn Write) -> IoResult<()> {
        let Some((func, inst)) = self.instance.fiber_location(self.fiber) else {
            writeln!(out, "program not paused")?;
            return Ok(());
        };
        let loc = self.lookup_line(func, inst);
        if let Some((file, cur_line, _)) = loc {
            // Another file of a multi-file program is read from disk when it was not added.
            let found: Option<String> = self.sources.get(&file).cloned().or_else(|| {
                std::fs::metadata(&file)
                    .ok()
                    .filter(|m| m.is_file() && m.len() <= 16 * 1024 * 1024)
                    .and_then(|_| std::fs::read_to_string(&file).ok())
            }).or_else(|| {
                let tail = |long: &str, short: &str| long.strip_suffix(short).is_some_and(|h| h.ends_with('/'));
                self.sources.iter().find_map(|(k, v)| {
                    if tail(k, &file) || tail(&file, k) {
                        Some(v.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(src) = found.as_deref() {
                let lines: Vec<&str> = src.lines().collect();
                if !lines.is_empty() {
                    let cur = cur_line as usize;
                    let start = cur.saturating_sub(2).max(1);
                    let end = (start + 4).min(lines.len());
                    let start = if end - start < 4 && start > 1 {
                        end.saturating_sub(4).max(1)
                    } else {
                        start
                    };
                    for ln in start..=end {
                        let text = lines.get(ln - 1).copied().unwrap_or("");
                        let marker = if ln == cur { ">" } else { " " };
                        writeln!(out, "{marker} {ln:>4} | {text}")?;
                    }
                    return Ok(());
                }
            }
        }

        let inst_text = self.disasm_inst(func, inst);
        writeln!(out, "{inst_text}")?;
        Ok(())
    }

    fn cmd_where(&self, out: &mut dyn Write) -> IoResult<()> {
        if self.ended {
            writeln!(out, "program has ended")?;
            return Ok(());
        }
        self.print_stop_location(out)
    }

    fn cmd_break(&mut self, arg: &str, out: &mut dyn Write) -> IoResult<()> {
        let arg = arg.trim();
        if arg.is_empty() {
            writeln!(out, "usage: break <line|file:line|func>")?;
            return Ok(());
        }

        if let Some((file, line_str)) = arg.split_once(':') {
            if let Ok(line) = line_str.trim().parse::<u32>() {
                if let Some((func, inst, desc)) = self.resolve_line_breakpoint(Some(file.trim()), line) {
                    let id = self.next_bp_id;
                    self.next_bp_id += 1;
                    self.breakpoints.push(Breakpoint { id, func, inst, desc: desc.clone() });
                    writeln!(out, "breakpoint {id} at {desc}")?;
                    return Ok(());
                } else {
                    writeln!(out, "cannot resolve breakpoint at {arg}")?;
                    return Ok(());
                }
            }
        }

        if let Ok(line) = arg.parse::<u32>() {
            if let Some((func, inst, desc)) = self.resolve_line_breakpoint(None, line) {
                let id = self.next_bp_id;
                self.next_bp_id += 1;
                self.breakpoints.push(Breakpoint { id, func, inst, desc: desc.clone() });
                writeln!(out, "breakpoint {id} at {desc}")?;
                return Ok(());
            } else {
                writeln!(out, "cannot resolve breakpoint at {arg}")?;
                return Ok(());
            }
        }

        if let Some((func, inst, desc)) = self.resolve_func_breakpoint(arg) {
            let id = self.next_bp_id;
            self.next_bp_id += 1;
            self.breakpoints.push(Breakpoint { id, func, inst, desc: desc.clone() });
            writeln!(out, "breakpoint {id} at {desc}")?;
            return Ok(());
        }

        writeln!(out, "cannot resolve breakpoint at `{arg}`")?;
        Ok(())
    }

    fn cmd_delete(&mut self, arg: &str, out: &mut dyn Write) -> IoResult<()> {
        let arg = arg.trim();
        if arg.is_empty() {
            writeln!(out, "usage: delete <id>")?;
            return Ok(());
        }
        let Ok(id) = arg.parse::<u32>() else {
            writeln!(out, "invalid breakpoint number: {arg}")?;
            return Ok(());
        };
        if let Some(pos) = self.breakpoints.iter().position(|b| b.id == id) {
            self.breakpoints.remove(pos);
            writeln!(out, "deleted breakpoint {id}")?;
        } else {
            writeln!(out, "no breakpoint {id}")?;
        }
        Ok(())
    }

    fn cmd_help(&self, out: &mut dyn Write) -> IoResult<()> {
        writeln!(out, "Available commands:")?;
        writeln!(out, "  break, b <LINE|FILE:LINE|FUNC>  Set a breakpoint")?;
        writeln!(out, "  delete, d <N>                   Delete breakpoint N")?;
        writeln!(out, "  step, s                         Step one instruction")?;
        writeln!(out, "  next, n                         Step over call")?;
        writeln!(out, "  continue, c                     Continue execution")?;
        writeln!(out, "  regs                            Show registers of current frame")?;
        writeln!(out, "  mem <ADDR> <LEN>                Hex dump memory")?;
        writeln!(out, "  bt                              Print backtrace")?;
        writeln!(out, "  list, l                         Show source lines around current line")?;
        writeln!(out, "  where, w                        Print current location and next instruction")?;
        writeln!(out, "  quit, q                         Exit debugger")?;
        writeln!(out, "  help, h                         Show this help")?;
        Ok(())
    }

    /// Prints `func_name at file:line:col` and the next instruction.
    fn print_stop_location(&self, out: &mut dyn Write) -> IoResult<()> {
        if let Some((func, inst)) = self.instance.fiber_location(self.fiber) {
            let func_name = self.func_name(func);
            if let Some((file, line, col)) = self.lookup_line(func, inst) {
                writeln!(out, "{func_name} at {file}:{line}:{col}")?;
            } else {
                writeln!(out, "{func_name} at inst {inst}")?;
            }
            let inst_str = self.disasm_inst(func, inst);
            writeln!(out, "    {inst_str}")?;
        }
        Ok(())
    }

    fn print_program_finished(&self, out: &mut dyn Write) -> IoResult<()> {
        if let Some(res) = self.instance.results(self.fiber) {
            if res.is_empty() {
                writeln!(out, "program finished")?;
            } else {
                let res_strs: Vec<String> = res.iter().map(|v| v.to_string()).collect();
                writeln!(out, "program finished: {}", res_strs.join(", "))?;
            }
        } else {
            writeln!(out, "program finished")?;
        }
        Ok(())
    }

    fn print_trap(&self, trap: &Trap, out: &mut dyn Write) -> IoResult<()> {
        writeln!(out, "trap: {trap}")?;
        Ok(())
    }

    fn is_breakpoint(&self, loc: (u32, u32)) -> bool {
        self.breakpoints
            .iter()
            .any(|b| b.func == loc.0 && b.inst == loc.1)
    }

    fn resolve_line_breakpoint(
        &self,
        file_opt: Option<&str>,
        line: u32,
    ) -> Option<(u32, u32, String)> {
        let lines = &self.module.data().lines;
        // The file list holds every file of a multi-file program: an exact name wins, then a
        // name that is the tail of a recorded path (or the other way round) at a `/` boundary.
        let file_idx = file_opt.and_then(|file_name| {
            let tail = |long: &str, short: &str| long.strip_suffix(short).is_some_and(|h| h.ends_with('/'));
            lines
                .files
                .iter()
                .position(|f| f == file_name)
                .or_else(|| lines.files.iter().position(|f| tail(f, file_name)))
                .or_else(|| lines.files.iter().position(|f| tail(file_name, f)))
        });

        if file_opt.is_some() && file_idx.is_none() {
            return None;
        }

        // Try current function first if no file was specified
        if file_opt.is_none() {
            if let Some((cur_func, _)) = self.instance.fiber_location(self.fiber) {
                if let Some(fl) = lines.funcs.iter().find(|f| f.func == cur_func) {
                    let mut matching: Vec<&LineEntry> = fl
                        .entries
                        .iter()
                        .filter(|e| e.line == line)
                        .collect();
                    if !matching.is_empty() {
                        matching.sort_by_key(|e| e.inst);
                        let first = matching[0];
                        let func_name = self.func_name(fl.func);
                        let file_name = lines
                            .files
                            .get(first.file as usize)
                            .cloned()
                            .unwrap_or_default();
                        let desc = format!("{func_name} ({file_name}:{line})");
                        return Some((fl.func, first.inst, desc));
                    }
                }
            }
        }

        // Search across all functions
        for fl in &lines.funcs {
            let mut matching: Vec<&LineEntry> = fl
                .entries
                .iter()
                .filter(|e| {
                    file_idx.map_or(true, |idx| e.file as usize == idx) && e.line == line
                })
                .collect();
            if !matching.is_empty() {
                matching.sort_by_key(|e| e.inst);
                let first_entry = matching[0];
                let func_name = self.func_name(fl.func);
                let file_name = lines
                    .files
                    .get(first_entry.file as usize)
                    .cloned()
                    .unwrap_or_default();
                let desc = format!("{func_name} ({file_name}:{line})");
                return Some((fl.func, first_entry.inst, desc));
            }
        }
        None
    }

    fn resolve_func_breakpoint(&self, name: &str) -> Option<(u32, u32, String)> {
        for exp in &self.module.data().exports {
            if exp.name == name {
                if let ExportKind::Func(f) = exp.kind {
                    let desc = format!("{name} (inst 0)");
                    return Some((f, 0, desc));
                }
            }
        }
        for (f, fname) in &self.module.data().names.funcs {
            if fname == name {
                let desc = format!("{name} (inst 0)");
                return Some((*f, 0, desc));
            }
        }
        None
    }

    /// Looks up function name from module debug names, exports, or imports.
    pub fn func_name(&self, func: u32) -> String {
        self.module
            .data()
            .names
            .funcs
            .iter()
            .find(|(i, _)| *i == func)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| {
                let nimports = self.module.data().imports.len();
                if (func as usize) < nimports {
                    let imp = &self.module.data().imports[func as usize];
                    format!("{}.{}", imp.namespace, imp.name)
                } else {
                    format!("f{func}")
                }
            })
    }

    /// `(file, line, column)` of the line entry with greatest instruction index <= `inst`.
    pub fn lookup_line(&self, func: u32, inst: u32) -> Option<(String, u32, u32)> {
        let lines = &self.module.data().lines;
        let fl = lines.funcs.iter().find(|f| f.func == func)?;
        let e = fl.entries.iter().filter(|e| e.inst <= inst).max_by_key(|e| e.inst)?;
        let file = lines.files.get(e.file as usize)?.clone();
        Some((file, e.line, e.column))
    }

    /// Formats the instruction at `(func, inst_idx)`.
    pub fn disasm_inst(&self, func: u32, inst_idx: u32) -> String {
        let nimports = self.module.data().imports.len();
        if (func as usize) < nimports {
            return self.func_name(func);
        }
        let f_idx = func as usize - nimports;
        let Some(f) = self.module.data().functions.get(f_idx) else {
            return format!("inst {inst_idx}");
        };
        let Some(inst) = f.code.get(inst_idx as usize) else {
            return format!("inst {inst_idx}");
        };

        if inst.op == Op::Call {
            let target = inst.a;
            if (target as usize) < nimports {
                let imp = &self.module.data().imports[target as usize];
                if let Some(index) = &self.index {
                    if let Some(host_fn) = index.function(&imp.namespace, &imp.name) {
                        let mut cur_reg = inst.b;
                        let mut param_strs = Vec::new();
                        for p in &host_fn.params {
                            let slots = p.ty.lower().len() as u32;
                            if slots == 1 {
                                param_strs.push(format!("{} = r{cur_reg}", p.name));
                            } else if slots == 2 {
                                param_strs.push(format!("{} = r{}..r{}", p.name, cur_reg, cur_reg + 1));
                            } else {
                                param_strs.push(format!("{} = r{}..r{}", p.name, cur_reg, cur_reg + slots - 1));
                            }
                            cur_reg += slots;
                        }
                        return format!("call {}.{}({})", imp.namespace, imp.name, param_strs.join(", "));
                    }
                }
                return format!("call {}.{}", imp.namespace, imp.name);
            } else {
                let name = self.func_name(target);
                return format!("call {name}, r{}", inst.b);
            }
        }

        let m = inst.op.name();
        match inst.op.shape() {
            Shape::None => m.to_string(),
            Shape::D => format!("{m} r{}", inst.a),
            Shape::DA => format!("{m} r{}, r{}", inst.a, inst.b),
            Shape::DAB | Shape::ABC => format!("{m} r{}, r{}, r{}", inst.a, inst.b, inst.c),
            Shape::ConstI32 => format!("{m} r{}, {}", inst.a, inst.b as i32),
            Shape::ConstI64 => format!("{m} r{}, {}", inst.a, inst.const_i64_value() as i64),
            Shape::ConstF32 => format!("{m} r{}, {:?}", inst.a, f32::from_bits(inst.b)),
            Shape::ConstF64 => format!("{m} r{}, {:?}", inst.a, f64::from_bits(inst.const_i64_value())),
            Shape::Select => {
                let (ra, rb) = inst.select_regs();
                format!("{m} r{}, r{}, r{ra}, r{rb}", inst.a, inst.b)
            }
            Shape::Load | Shape::Store => format!("{m} r{}, r{}, {}", inst.a, inst.b, inst.c),
            Shape::GlobalGet => format!("{m} r{}, g{}", inst.a, inst.b),
            Shape::GlobalSet => format!("{m} g{}, r{}", inst.a, inst.b),
            Shape::Jmp => format!("{m} {}", inst.a),
            Shape::Br => format!("{m} r{}, {}", inst.a, inst.b),
            Shape::BrTable => format!("{m} r{}, br_table_{}", inst.a, inst.b),
            Shape::Call => {
                let name = self.func_name(inst.a);
                format!("{m} {name}, r{}", inst.b)
            }
            Shape::CallIndirect => format!("{m} type_{}, r{}, r{}", inst.a, inst.b, inst.c),
            Shape::Ret => {
                let ty = self.module.data().func_type(func);
                if ty.map_or(true, |t| t.results.is_empty()) {
                    "ret".to_string()
                } else {
                    format!("{m} r{}", inst.a)
                }
            }
        }
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}
