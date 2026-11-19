//! Tests for dependency resolution (E8, design section 9.1).

use efemod::{resolve, Candidate, Dependency, Resolution, ResolveProblem, Version, VersionReq};

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

fn req(s: &str) -> VersionReq {
    s.parse().unwrap()
}

fn dep(id: &str, r: &str) -> Dependency {
    Dependency { id: id.to_owned(), req: req(r) }
}

/// An owned package, borrowed into a `Candidate` by [`candidates`].
struct Pkg {
    id: &'static str,
    version: Version,
    deps: Vec<Dependency>,
}

fn pkg(id: &'static str, version: &str, deps: Vec<Dependency>) -> Pkg {
    Pkg { id, version: v(version), deps }
}

fn candidates(pkgs: &[Pkg]) -> Vec<Candidate<'_>> {
    pkgs.iter()
        .map(|p| Candidate { id: p.id, version: &p.version, dependencies: &p.deps })
        .collect()
}

/// The ids of the loaded candidates, in load order.
fn order_ids<'a>(pkgs: &'a [Pkg], res: &Resolution) -> Vec<&'a str> {
    res.order.iter().map(|&i| pkgs[i].id).collect()
}

fn problems_of(res: &Resolution) -> Vec<(usize, ResolveProblem)> {
    res.problems.clone()
}

fn cycle(path: &[&str]) -> ResolveProblem {
    ResolveProblem::Cycle { path: path.iter().map(|s| s.to_string()).collect() }
}

#[test]
fn empty_set_resolves_to_nothing() {
    let res = resolve(&[], &[]);
    assert_eq!(res, Resolution::default());
}

#[test]
fn independent_mods_are_sorted_by_id() {
    let pkgs = [pkg("c", "1.0.0", vec![]), pkg("a", "1.0.0", vec![]), pkg("b", "1.0.0", vec![])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert_eq!(order_ids(&pkgs, &res), ["a", "b", "c"]);
    assert!(res.problems.is_empty());
}

#[test]
fn chain_loads_dependencies_first() {
    let pkgs = [
        pkg("c", "1.0.0", vec![dep("b", "^1")]),
        pkg("b", "1.0.0", vec![dep("a", "^1")]),
        pkg("a", "1.0.0", vec![]),
    ];
    let res = resolve(&candidates(&pkgs), &[]);
    assert_eq!(order_ids(&pkgs, &res), ["a", "b", "c"]);
    assert!(res.problems.is_empty());
}

#[test]
fn dependency_satisfied_by_loaded_package() {
    let pkgs = [pkg("x", "1.0.0", vec![dep("core", "^1")])];
    let core = v("1.2.0");
    let res = resolve(&candidates(&pkgs), &[("core", &core)]);
    assert_eq!(order_ids(&pkgs, &res), ["x"]);
    assert!(res.problems.is_empty());
}

#[test]
fn loaded_version_outside_requirement_is_a_version_problem() {
    let pkgs = [pkg("x", "1.0.0", vec![dep("core", "^1")])];
    let core = v("2.0.0");
    let res = resolve(&candidates(&pkgs), &[("core", &core)]);
    assert!(res.order.is_empty());
    assert_eq!(
        problems_of(&res),
        [(
            0,
            ResolveProblem::Version {
                dep: "core".into(),
                req: "^1".into(),
                found: "2.0.0".into()
            }
        )]
    );
}

#[test]
fn missing_dependency() {
    let pkgs = [pkg("a", "1.0.0", vec![dep("b", "^1.2")])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.order.is_empty());
    assert_eq!(
        problems_of(&res),
        [(0, ResolveProblem::Missing { dep: "b".into(), req: "^1.2".into() })]
    );
}

#[test]
fn version_mismatch_in_set() {
    let pkgs = [pkg("a", "1.0.0", vec![dep("b", "^2")]), pkg("b", "1.4.0", vec![])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert_eq!(order_ids(&pkgs, &res), ["b"]);
    assert_eq!(
        problems_of(&res),
        [(
            0,
            ResolveProblem::Version { dep: "b".into(), req: "^2".into(), found: "1.4.0".into() }
        )]
    );
}

#[test]
fn duplicate_ids_in_set_are_all_reported() {
    let pkgs = [pkg("a", "1.0.0", vec![]), pkg("a", "2.0.0", vec![]), pkg("b", "1.0.0", vec![])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert_eq!(order_ids(&pkgs, &res), ["b"]);
    assert_eq!(
        problems_of(&res),
        [
            (0, ResolveProblem::DuplicateId { id: "a".into() }),
            (1, ResolveProblem::DuplicateId { id: "a".into() }),
        ]
    );
}

#[test]
fn duplicate_with_loaded_package() {
    let pkgs = [pkg("a", "1.0.0", vec![])];
    let loaded_a = v("1.0.0");
    let res = resolve(&candidates(&pkgs), &[("a", &loaded_a)]);
    assert!(res.order.is_empty());
    assert_eq!(problems_of(&res), [(0, ResolveProblem::DuplicateId { id: "a".into() })]);
}

#[test]
fn two_cycle_paths() {
    let pkgs = [pkg("a", "1.0.0", vec![dep("b", "^1")]), pkg("b", "1.0.0", vec![dep("a", "^1")])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.order.is_empty());
    assert_eq!(
        problems_of(&res),
        [(0, cycle(&["a", "b", "a"])), (1, cycle(&["b", "a", "b"]))]
    );
}

#[test]
fn three_cycle_paths() {
    let pkgs = [
        pkg("a", "1.0.0", vec![dep("b", "^1")]),
        pkg("b", "1.0.0", vec![dep("c", "^1")]),
        pkg("c", "1.0.0", vec![dep("a", "^1")]),
    ];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.order.is_empty());
    assert_eq!(
        problems_of(&res),
        [
            (0, cycle(&["a", "b", "c", "a"])),
            (1, cycle(&["b", "c", "a", "b"])),
            (2, cycle(&["c", "a", "b", "c"])),
        ]
    );
}

#[test]
fn self_dependency_is_a_cycle() {
    let pkgs = [pkg("a", "1.0.0", vec![dep("a", "^1")])];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.order.is_empty());
    assert_eq!(problems_of(&res), [(0, cycle(&["a", "a"]))]);
}

#[test]
fn dependent_of_a_cycle_is_dependency_failed() {
    let pkgs = [
        pkg("a", "1.0.0", vec![dep("b", "^1")]),
        pkg("b", "1.0.0", vec![dep("a", "^1")]),
        pkg("c", "1.0.0", vec![dep("a", "^1")]),
    ];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.order.is_empty());
    assert_eq!(
        problems_of(&res),
        [
            (0, cycle(&["a", "b", "a"])),
            (1, cycle(&["b", "a", "b"])),
            (2, ResolveProblem::DependencyFailed { dep: "a".into() }),
        ]
    );
}

#[test]
fn transitive_dependency_failed() {
    let pkgs = [
        pkg("a", "1.0.0", vec![dep("z", "^1")]),
        pkg("b", "1.0.0", vec![dep("a", "^1")]),
        pkg("c", "1.0.0", vec![dep("b", "^1")]),
        pkg("d", "1.0.0", vec![]),
    ];
    let res = resolve(&candidates(&pkgs), &[]);
    assert_eq!(order_ids(&pkgs, &res), ["d"]);
    assert_eq!(
        problems_of(&res),
        [
            (0, ResolveProblem::Missing { dep: "z".into(), req: "^1".into() }),
            (1, ResolveProblem::DependencyFailed { dep: "a".into() }),
            (2, ResolveProblem::DependencyFailed { dep: "b".into() }),
        ]
    );
}

#[test]
fn diamond_loads_each_once_in_id_order() {
    let pkgs = [
        pkg("d", "1.0.0", vec![dep("c", "^1"), dep("b", "^1")]),
        pkg("c", "1.0.0", vec![dep("a", "^1")]),
        pkg("b", "1.0.0", vec![dep("a", "^1")]),
        pkg("a", "1.0.0", vec![]),
    ];
    let res = resolve(&candidates(&pkgs), &[]);
    assert!(res.problems.is_empty());
    assert_eq!(order_ids(&pkgs, &res), ["a", "b", "c", "d"]);
}

#[test]
fn order_is_independent_of_input_order() {
    let base = || {
        [
            pkg("d", "1.0.0", vec![dep("c", "^1")]),
            pkg("b", "1.0.0", vec![]),
            pkg("c", "1.0.0", vec![dep("a", "^1")]),
            pkg("a", "1.0.0", vec![]),
            pkg("e", "1.0.0", vec![dep("q", "^1")]),
        ]
    };
    let shuffled = [
        pkg("e", "1.0.0", vec![dep("q", "^1")]),
        pkg("a", "1.0.0", vec![]),
        pkg("d", "1.0.0", vec![dep("c", "^1")]),
        pkg("c", "1.0.0", vec![dep("a", "^1")]),
        pkg("b", "1.0.0", vec![]),
    ];
    let first = base();
    let res1 = resolve(&candidates(&first), &[]);
    let res2 = resolve(&candidates(&shuffled), &[]);
    assert_eq!(order_ids(&first, &res1), ["a", "b", "c", "d"]);
    assert_eq!(order_ids(&shuffled, &res2), ["a", "b", "c", "d"]);
    assert_eq!(res1.problems.len(), 1);
    assert_eq!(res2.problems.len(), 1);
    assert_eq!(res1.problems[0].1, ResolveProblem::Missing { dep: "q".into(), req: "^1".into() });
    assert_eq!(res2.problems[0].1, ResolveProblem::Missing { dep: "q".into(), req: "^1".into() });
}

#[test]
fn display_strings() {
    assert_eq!(
        ResolveProblem::Missing { dep: "b".into(), req: "^1.2".into() }.to_string(),
        "dependency `b` (^1.2) is missing"
    );
    assert_eq!(
        ResolveProblem::Version { dep: "b".into(), req: "^2".into(), found: "1.4.0".into() }
            .to_string(),
        "dependency `b` requires ^2, found 1.4.0"
    );
    assert_eq!(
        ResolveProblem::Cycle { path: vec!["a".into(), "b".into(), "a".into()] }.to_string(),
        "cycle: a -> b -> a"
    );
    assert_eq!(
        ResolveProblem::DependencyFailed { dep: "b".into() }.to_string(),
        "dependency `b` failed"
    );
    assert_eq!(
        ResolveProblem::DuplicateId { id: "a".into() }.to_string(),
        "duplicate id `a`"
    );
}
