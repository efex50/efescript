//! Review fixes for the runtime: boundary masking, trap locations, Caller::fiber, spawn errors,
//! linear-time instantiation.
mod common;
use common::reg_host;
use efevm::ValType::{F32, I32, I64};
use efevm::*;

fn ty(p: &[ValType], r: &[ValType]) -> FuncType {
    FuncType::new(p.iter().copied(), r.iter().copied())
}

fn run_done<H>(i: &mut Instance<H>, host: &mut H, f: FiberId) -> Vec<u64> {
    assert_eq!(i.resume(f, host, u64::MAX), Exit::Done);
    i.results(f).unwrap().to_vec()
}

#[test]
fn spawn_args_are_masked() {
    let mut mb = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(1);
    fb.ret(0);
    let f = mb.add_func(ty(&[I32], &[I32]), fb);
    mb.export_func("id32", f);
    let mut fb = FunctionBuilder::new(2);
    fb.ret(0);
    let g = mb.add_func(ty(&[F32, I64], &[F32, I64]), fb);
    mb.export_func("mixed", g);
    let m = mb.finish().unwrap();
    let mut i = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let a = i.spawn("id32", &[u64::MAX]).unwrap();
    assert_eq!(run_done(&mut i, &mut (), a), vec![0xFFFF_FFFF]);
    let b = i.spawn("mixed", &[u64::MAX, u64::MAX]).unwrap();
    assert_eq!(run_done(&mut i, &mut (), b), vec![0xFFFF_FFFF, u64::MAX]);
    let c = i.spawn_func(f, &[0x1234_5678_9ABC_DEF0]).unwrap();
    assert_eq!(run_done(&mut i, &mut (), c), vec![0x9ABC_DEF0]);
}

fn host_ret_module(t: ValType) -> Module {
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "h", ty(&[], &[t]));
    let mut fb = FunctionBuilder::new(1);
    fb.call(h, 0);
    fb.ret(0);
    let f = mb.add_func(ty(&[], &[t]), fb);
    mb.export_func("main", f);
    mb.finish().unwrap()
}

#[test]
fn host_results_are_masked() {
    for (t, want) in [(I32, 0xFFFF_FFFFu64), (F32, 0xFFFF_FFFF), (I64, u64::MAX)] {
        let m = host_ret_module(t);
        let mut imports = HostRegistry::<()>::new();
        reg_host(&mut imports, "env", "h", ty(&[], &[t]), |_, _, r| {
            r[0] = u64::MAX;
            Ok(HostOk::Continue)
        });
        let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
        let f = i.spawn("main", &[]).unwrap();
        assert_eq!(run_done(&mut i, &mut (), f), vec![want], "{t:?}");
    }
}

#[test]
fn set_global_is_masked() {
    let mut mb = ModuleBuilder::new();
    let g32 = mb.global(I32, true, 0);
    let g64 = mb.global(I64, true, 0);
    let gf = mb.global(F32, true, 0);
    let h = mb.import("env", "set", ty(&[], &[]));
    let mut fb = FunctionBuilder::new(1);
    fb.call(h, 0);
    fb.ret(0);
    let f = mb.add_func(ty(&[], &[]), fb);
    mb.export_func("main", f);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<()>::new();
    reg_host(&mut imports, "env", "set", ty(&[], &[]), move |cx, _, _| {
        cx.set_global(g32, u64::MAX)?;
        cx.set_global(g64, u64::MAX)?;
        cx.set_global(gf, u64::MAX - 1)?;
        Ok(HostOk::Continue)
    });
    let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    assert!(i.set_global(g32, u64::MAX));
    assert!(i.set_global(g64, u64::MAX));
    assert!(i.set_global(gf, u64::MAX));
    assert_eq!(i.global(g32), Some(0xFFFF_FFFF));
    assert_eq!(i.global(g64), Some(u64::MAX));
    assert_eq!(i.global(gf), Some(0xFFFF_FFFF));
    i.set_global(g32, 0);
    i.set_global(g64, 0);
    i.set_global(gf, 0);
    let fb = i.spawn("main", &[]).unwrap();
    run_done(&mut i, &mut (), fb);
    assert_eq!(i.global(g32), Some(0xFFFF_FFFF));
    assert_eq!(i.global(g64), Some(u64::MAX));
    assert_eq!(i.global(gf), Some(0xFFFF_FFFE));
}

#[test]
fn trap_location_and_backtrace_three_deep() {
    let mut mb = ModuleBuilder::new();
    let main = mb.declare_func(ty(&[], &[]));
    let a = mb.declare_func(ty(&[], &[]));
    let b = mb.declare_func(ty(&[], &[]));
    let c = mb.declare_func(ty(&[], &[]));
    let mut fb = FunctionBuilder::new(1);
    fb.nop();
    fb.call(a, 0);
    fb.ret(0);
    mb.define(main, fb);
    let mut fb = FunctionBuilder::new(1);
    fb.call(b, 0);
    fb.ret(0);
    mb.define(a, fb);
    let mut fb = FunctionBuilder::new(1);
    fb.nop();
    fb.nop();
    fb.call(c, 0);
    fb.ret(0);
    mb.define(b, fb);
    let mut fb = FunctionBuilder::new(1);
    fb.nop();
    fb.unreachable();
    mb.define(c, fb);
    mb.export_func("main", main);
    let m = mb.finish().unwrap();
    let mut i = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    let f = i.spawn("main", &[]).unwrap();
    assert_eq!(i.trap_location(f), None);
    assert_eq!(i.backtrace(f), None);
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Trap(Trap::Unreachable));
    assert_eq!(i.trap_location(f), Some((c, 1)));
    assert_eq!(i.backtrace(f), Some(&[(c, 1), (b, 2), (a, 0), (main, 1)][..]));
    // Stable across a repeated resume.
    assert_eq!(i.resume(f, &mut (), 1000), Exit::Trap(Trap::Unreachable));
    assert_eq!(i.trap_location(f), Some((c, 1)));
}

#[test]
fn host_trap_reports_call_instruction() {
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "boom", ty(&[], &[]));
    let mut fb = FunctionBuilder::new(1);
    fb.nop();
    fb.call(h, 0);
    fb.ret(0);
    let f = mb.add_func(ty(&[], &[]), fb);
    mb.export_func("main", f);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<()>::new();
    reg_host(&mut imports, "env", "boom", ty(&[], &[]), |_, _, _| Err(Trap::Host("x".into())));
    let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let fid = i.spawn("main", &[]).unwrap();
    assert!(matches!(i.resume(fid, &mut (), 100), Exit::Trap(Trap::Host(_))));
    assert_eq!(i.trap_location(fid), Some((f, 1)));
    assert_eq!(i.backtrace(fid), Some(&[(f, 1)][..]));
}

#[test]
fn caller_fiber_matches_spawned_id() {
    let mut mb = ModuleBuilder::new();
    let h = mb.import("env", "who", ty(&[], &[]));
    let mut fb = FunctionBuilder::new(1);
    fb.call(h, 0);
    fb.ret(0);
    let f = mb.add_func(ty(&[], &[]), fb);
    mb.export_func("main", f);
    let m = mb.finish().unwrap();
    let mut imports = HostRegistry::<Vec<FiberId>>::new();
    reg_host(&mut imports, "env", "who", ty(&[], &[]), |cx, _, _| {
        let id = cx.fiber();
        cx.host().push(id);
        Ok(HostOk::Continue)
    });
    let mut i = Instance::new(&m, &imports.imports(), Limits::default()).unwrap();
    let a = i.spawn("main", &[]).unwrap();
    let b = i.spawn("main", &[]).unwrap();
    let mut seen = Vec::new();
    assert_eq!(i.resume(b, &mut seen, 100), Exit::Done);
    assert_eq!(i.resume(a, &mut seen, 100), Exit::Done);
    assert_eq!(seen, vec![b, a]);
    assert_ne!(a, b);
}

#[test]
fn spawn_func_out_of_range() {
    let mut mb = ModuleBuilder::new();
    let mut fb = FunctionBuilder::new(0);
    fb.ret(0);
    let f = mb.add_func(ty(&[], &[]), fb);
    let m = mb.finish().unwrap();
    let mut i = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    assert!(i.spawn_func(f, &[]).is_ok());
    assert_eq!(i.spawn_func(f + 1, &[]), Err(SpawnError::NotAFunction));
    assert_eq!(i.spawn_func(u32::MAX, &[]), Err(SpawnError::NotAFunction));
}

#[test]
fn many_distinct_types_instantiate_fast() {
    let n = 60_000u32;
    let types: Vec<FuncType> = (0..n)
        .map(|k| {
            let p = (0..16).map(|b| if (k >> b) & 1 == 1 { I64 } else { I32 });
            FuncType::new(p, [])
        })
        .collect();
    let data = ModuleData { types, ..ModuleData::default() };
    let m = Module::new(data).unwrap();
    let t = std::time::Instant::now();
    let _i = Instance::new(&m, &Imports::<()>::new(), Limits::default()).unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
}
