//! A mock host built from an API index: every function logs its raw arguments and returns
//! fixed values, so a translated mod can be run without the engine.
#![allow(dead_code)]

use efevm::*;

#[derive(Default)]
pub struct Mock {
    pub log: Vec<String>,
}

pub const PTR_RESULT: u64 = 0x1_2345_6789;

/// Namespaces, struct layouts, events and functions of `ix`, plus `engine.events` (version 2).
pub fn registry(ix: &ApiIndex) -> HostRegistry<Mock> {
    let mut r = HostRegistry::<Mock>::new();
    r.set_game(ix.game.clone());
    for n in &ix.namespaces {
        r.define_namespace(&n.name, n.version, &n.owner, &n.doc).unwrap();
    }
    if r.namespace("engine.events").is_none() {
        r.define_namespace("engine.events", 2, "test", "Typed script events.").unwrap();
        let log = |name: &'static str| {
            move |c: &mut Caller<'_, Mock>, a: &Args<'_>, res: &mut Results<'_>| {
                c.host().log.push(format!("engine.events.{name}{:?}", a.raw()));
                match name {
                    "count" => res.raw()[0] = 3,
                    "read" => res.raw()[0] = 0,
                    _ => {}
                }
                Ok(HostOk::Continue)
            }
        };
        r.register(HostFn::new("engine.events", "send").doc("Send.").param("ev", ApiType::I32).param("payload", ApiType::Ptr), log("send")).unwrap();
        r.register(HostFn::new("engine.events", "count").doc("Count.").param("ev", ApiType::I32).result("count", ApiType::I32), log("count")).unwrap();
        r.register(
            HostFn::new("engine.events", "read").doc("Read.").param("ev", ApiType::I32).param("index", ApiType::I32).param("out", ApiType::Ptr).result("status", ApiType::I32),
            log("read"),
        )
        .unwrap();
    }
    for t in &ix.types {
        r.register_type(t.clone()).unwrap();
    }
    for e in &ix.events {
        r.register_event(e.clone()).unwrap();
    }
    for f in &ix.functions {
        let (ns, name) = (f.namespace.clone(), f.name.clone());
        let nres = f.results.len();
        let ptr_res: Vec<bool> = f.results.iter().map(|q| q.ty == ApiType::Ptr).collect();
        r.register(f.clone(), move |c: &mut Caller<'_, Mock>, a: &Args<'_>, res: &mut Results<'_>| {
            c.host().log.push(format!("{ns}.{name}{:?}", a.raw()));
            for k in 0..nres {
                res.raw()[k] = if ptr_res[k] { PTR_RESULT } else { 0 };
            }
            Ok(HostOk::Continue)
        })
        .unwrap();
    }
    r
}
