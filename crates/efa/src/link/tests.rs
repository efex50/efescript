//! Linker tests with hand-written units (no `.efa`).

use efevm::api::{ApiType, FieldDesc, Prim, StructDesc};
use efevm::{FuncType, FunctionBuilder, Op, ValType};

use super::*;

/// A unit whose declarations are given; every function body is `ret r0` with enough registers.
struct TestUnit {
    file: String,
    ns: String,
    decls: Decls,
    resolved: Resolved,
    /// Calls `(callee unit, callee name)` emitted at the start of the first function.
    calls: Vec<(usize, String)>,
}

fn unit(file: &str, ns: &str, decls: Decls) -> TestUnit {
    TestUnit { file: file.into(), ns: ns.into(), decls, resolved: Resolved::default(), calls: Vec::new() }
}

impl Unit for TestUnit {
    fn file(&self) -> &str {
        &self.file
    }
    fn namespace(&self) -> &str {
        &self.ns
    }
    fn declare(&mut self, _prior: &Declared<'_>, _diags: &mut Vec<Diagnostic>) -> Decls {
        self.decls.clone()
    }
    fn resolve(&mut self, _cx: &ResolveCx<'_>, _diags: &mut Vec<Diagnostic>) -> Resolved {
        let mut r = self.resolved.clone();
        r.globals.resize(self.decls.globals.len(), 0);
        r
    }
    fn define(&mut self, cx: &mut DefineCx<'_>, diags: &mut Vec<Diagnostic>) {
        let indices = cx.syms.unit(cx.unit).map(|u| u.funcs.clone()).unwrap_or_default();
        for (k, f) in self.decls.funcs.iter().enumerate() {
            let mut fb = FunctionBuilder::new(4);
            let mut spans = Vec::new();
            if k == 0 {
                for (u, name) in &self.calls {
                    match cx.syms.lookup(cx.unit, *u, Space::Func, name) {
                        Ok(Symbol::Func { index, .. }) => {
                            fb.loc(cx.file_id, 1, 1);
                            fb.call(index, 0);
                            spans.push(Span::new(1, 1, 1));
                        }
                        Ok(_) => {}
                        Err(e) => diags.push(e.diagnostic(self, Span::new(1, 1, 1), "function", name)),
                    }
                }
            }
            fb.loc(cx.file_id, 2, 1);
            fb.op0(Op::Nop);
            fb.ret(0);
            spans.push(Span::new(2, 1, 1));
            spans.push(Span::new(2, 1, 1));
            cx.define(indices[k], fb, spans, f.span);
            cx.reg_name(indices[k], 0, "a");
        }
    }
}

fn ft(p: &[ValType], r: &[ValType]) -> FuncType {
    FuncType::new(p.iter().copied(), r.iter().copied())
}

fn func(name: &str, public: bool, export: bool) -> FuncDecl {
    FuncDecl {
        name: name.into(),
        ty: ft(&[], &[]),
        public,
        export: export.then(|| name.to_string()),
        span: Span::new(3, 1, 1),
    }
}

fn import(ns: &str, name: &str, ty: FuncType) -> ImportDecl {
    ImportDecl { namespace: ns.into(), name: name.into(), ty, span: Span::new(1, 1, 1) }
}

fn data(name: &str, size: u64) -> DataDecl {
    DataDecl { name: name.into(), size, align: 8, public: true, span: Span::new(5, 1, 1) }
}

fn go(units: &mut [TestUnit]) -> Result<efevm::Module, Vec<Diagnostic>> {
    let mut refs: Vec<&mut dyn Unit> = units.iter_mut().map(|u| u as &mut dyn Unit).collect();
    link(&mut refs, &LinkOptions::default())
}

fn lay(units: &mut [TestUnit]) -> (SymbolMap, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut refs: Vec<&mut dyn Unit> = units.iter_mut().map(|u| u as &mut dyn Unit).collect();
    let decls = declare_all(&mut refs, &mut diags);
    drop(refs);
    let views: Vec<&dyn Unit> = units.iter().map(|u| u as &dyn Unit).collect();
    let syms = layout::layout(&views, decls, &mut diags);
    (syms, diags)
}

#[test]
fn layout_numbers_everything_in_unit_order() {
    let pr = ft(&[ValType::I64, ValType::I64], &[]);
    let a = Decls {
        imports: vec![import("std", "println", pr.clone())],
        funcs: vec![func("main", true, true), func("helper", false, false)],
        data: vec![data("msg", 3)],
        globals: vec![GlobalDecl { name: "g".into(), ty: ValType::I64, mutable: true, public: false, export: None, span: Span::new(4, 1, 1) }],
        ..Decls::default()
    };
    let b = Decls {
        imports: vec![import("std", "print_i64", ft(&[ValType::I64], &[])), import("std", "println", pr)],
        funcs: vec![func("square", true, false)],
        data: vec![data("buf", 100), data("x", 1)],
        globals: vec![GlobalDecl { name: "h".into(), ty: ValType::I32, mutable: false, public: true, export: None, span: Span::new(4, 1, 1) }],
        ..Decls::default()
    };
    let (syms, diags) = lay(&mut [unit("a.efa", "", a), unit("b.efa", "b", b)]);
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(syms.imports.len(), 2);
    assert_eq!(syms.units[1].imports, vec![1, 0]);
    assert_eq!(syms.units[0].funcs, vec![2, 3]);
    assert_eq!(syms.units[1].funcs, vec![4]);
    assert_eq!(syms.units[1].globals, vec![1]);
    assert_eq!(syms.units[0].data, vec![1024]);
    assert_eq!(syms.units[1].data, vec![1032, 1136]);
    assert_eq!(syms.data_end, 1137);
    assert_eq!(syms.memory, (1, None));
    assert_eq!(syms.scratch, None);
}

#[test]
fn lookup_respects_visibility() {
    let a = Decls { funcs: vec![func("main", false, false)], ..Decls::default() };
    let b = Decls {
        funcs: vec![func("open", true, false), func("closed", false, false)],
        data: vec![DataDecl { public: false, ..data("d", 4) }],
        structs: vec![StructDecl {
            desc: StructDesc::new("S", 8, 8).field(FieldDesc::new("v", 0, Prim::U64, 1)),
            public: true,
            export: false,
            span: Span::new(7, 1, 1),
        }],
        ..Decls::default()
    };
    let (syms, _) = lay(&mut [unit("a.efa", "", a), unit("b.efa", "b", b)]);
    assert!(matches!(syms.lookup(0, 1, Space::Func, "open"), Ok(Symbol::Func { index: 1, .. })));
    assert_eq!(
        syms.lookup(0, 1, Space::Func, "closed"),
        Err(LookupError::Private { file: "b.efa".into(), span: Span::new(3, 1, 1) })
    );
    // Within the unit, private items are found.
    assert!(syms.lookup(1, 1, Space::Func, "closed").is_ok());
    assert!(matches!(syms.lookup(0, 1, Space::Value, "d.len"), Err(LookupError::Private { .. })));
    assert_eq!(syms.lookup(0, 1, Space::Value, "S.size"), Ok(Symbol::Value(ConstValue::Int(8))));
    assert_eq!(syms.lookup(0, 1, Space::Value, "S.v"), Ok(Symbol::Value(ConstValue::Int(0))));
    assert_eq!(syms.lookup(0, 1, Space::Func, "nothing"), Err(LookupError::Unknown));
}

#[test]
fn link_emits_names_lines_and_cross_unit_calls() {
    let a = Decls { funcs: vec![func("main", false, true)], ..Decls::default() };
    let b = Decls { funcs: vec![func("square", true, false)], ..Decls::default() };
    let mut ua = unit("a.efa", "", a);
    ua.calls.push((1, "square".into()));
    let m = go(&mut [ua, unit("util/b.efa", "util.b", b)]).expect("links");
    let d = m.data();
    assert_eq!(d.names.funcs, vec![(0, "main".to_string()), (1, "util.b.square".to_string())]);
    assert_eq!(d.names.regs, vec![(0, 0, "a".to_string()), (1, 0, "a".to_string())]);
    assert_eq!(d.lines.files, vec!["a.efa".to_string(), "util/b.efa".to_string()]);
    assert_eq!(d.functions[0].code[0].op, Op::Call);
    assert!(d.interface.is_none());
    assert_eq!(m.export_func("main"), Some(0));
}

#[test]
fn private_call_is_an_error_with_note() {
    let a = Decls { funcs: vec![func("main", false, false)], ..Decls::default() };
    let b = Decls { funcs: vec![func("hidden", false, false)], ..Decls::default() };
    let mut ua = unit("a.efa", "", a);
    ua.calls.push((1, "hidden".into()));
    let d = go(&mut [ua, unit("b.efa", "b", b)]).unwrap_err();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].message, "`hidden` is private to `b.efa`");
    assert_eq!(d[0].note.as_deref(), Some("declared at b.efa:3; mark it `pub` to use it from other files"));
}

#[test]
fn program_wide_checks() {
    let a = Decls {
        funcs: vec![func("run", false, true)],
        table: Some(Span::new(2, 1, 1)),
        imports: vec![import("std", "println", ft(&[ValType::I64, ValType::I64], &[]))],
        ..Decls::default()
    };
    let b = Decls {
        funcs: vec![func("run", false, true)],
        table: Some(Span::new(6, 1, 1)),
        memory: Some(MemoryDecl { min: Some(1), max: None, span: Span::new(8, 1, 1) }),
        scratch: Some(ScratchDecl { size: 8, span: Span::new(9, 1, 1) }),
        imports: vec![import("std", "println", ft(&[ValType::I64], &[]))],
        ..Decls::default()
    };
    let (_, d) = lay(&mut [unit("a.efa", "", a), unit("b.efa", "b", b)]);
    let msgs: Vec<(&str, Option<&str>)> = d.iter().map(|x| (x.message.as_str(), x.note.as_deref())).collect();
    assert!(msgs.contains(&("duplicate export name `run`", Some("first defined in a.efa:3"))), "{msgs:?}");
    assert!(msgs.contains(&("duplicate `table` declaration", Some("first defined in a.efa:2"))));
    assert!(msgs.contains(&("`memory` may only appear in the root file `a.efa`", None)));
    assert!(msgs.contains(&("`scratch` may only appear in the root file `a.efa`", None)));
    assert!(msgs.iter().any(|(m, _)| m.starts_with("import `std.println` is declared as (i64) here")));
    assert!(d.iter().all(|x| x.file == "b.efa"));
}

#[test]
fn memory_includes_the_scratch_region() {
    let a = Decls {
        funcs: vec![FuncDecl { ty: ft(&[ValType::I64, ValType::I64], &[]), ..func("give", false, false) }],
        data: vec![data("hello", 5)],
        provides: vec![ProvideDecl {
            name: "give".into(),
            params: vec![ProvideParam { name: "s".into(), ty: ProvideType::Api(ApiType::Str), span: Span::new(1, 1, 1) }],
            results: vec![],
            doc: "d".into(),
            target: SymRef { unit: 0, name: "give".into(), span: Span::new(1, 1, 1) },
            span: Span::new(1, 1, 1),
        }],
        scratch: Some(ScratchDecl { size: 70000, span: Span::new(2, 1, 1) }),
        ..Decls::default()
    };
    let (syms, d) = lay(&mut [unit("a.efa", "", a.clone())]);
    assert!(d.is_empty());
    assert_eq!(syms.scratch, Some((1032, 70000)));
    assert_eq!(syms.memory, (2, None));
    let m = go(&mut [unit("a.efa", "", a)]).expect("links");
    let iface = m.data().interface.as_ref().unwrap();
    assert_eq!(iface.scratch, Some((1032, 70000)));
    assert_eq!(iface.provides[0].func, 0);
}

#[test]
fn interface_orders_own_structs_then_copies_and_checks_lowering() {
    let own = StructDesc::new("Own", 4, 4).field(FieldDesc::new("a", 0, Prim::I32, 1));
    let mut copy = StructDesc::new("Pos", 8, 4).field(FieldDesc::new("x", 0, Prim::F32, 2));
    copy.owner = "engine".into();
    let p = |name: &str, ty: ProvideType| ProvideParam { name: name.into(), ty, span: Span::new(1, 1, 1) };
    let a = Decls {
        funcs: vec![
            FuncDecl { ty: ft(&[ValType::I64, ValType::I64], &[]), ..func("f", false, false) },
            FuncDecl { ty: ft(&[ValType::I32], &[]), ..func("g", false, false) },
        ],
        structs: vec![StructDecl { desc: own, public: false, export: true, span: Span::new(4, 1, 1) }],
        provides: vec![
            ProvideDecl {
                name: "f".into(),
                params: vec![
                    p("p", ProvideType::IndexStruct(copy.clone())),
                    p("o", ProvideType::Struct(SymRef { unit: 0, name: "Own".into(), span: Span::new(1, 1, 1) })),
                ],
                results: vec![],
                doc: "d".into(),
                target: SymRef { unit: 0, name: "f".into(), span: Span::new(1, 1, 1) },
                span: Span::new(1, 1, 1),
            },
            ProvideDecl {
                name: "g".into(),
                params: vec![p("x", ProvideType::Api(ApiType::I64))],
                results: vec![],
                doc: "d".into(),
                target: SymRef { unit: 0, name: "g".into(), span: Span::new(2, 1, 1) },
                span: Span::new(2, 1, 1),
            },
        ],
        ..Decls::default()
    };
    let mut units = [unit("a.efa", "", a.clone())];
    let (syms, _) = lay(&mut units);
    let views: Vec<&dyn Unit> = units.iter().map(|u| u as &dyn Unit).collect();
    let mut d = Vec::new();
    let iface = layout::interface(&views, &syms, &mut d).unwrap();
    let names: Vec<&str> = iface.structs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Own", "Pos"]);
    assert_eq!(iface.scratch, Some((1024, 4096)));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].message, "provide `g` lowers to `(i64)`, but its function `g` has `(i32)`");
}
