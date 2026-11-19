//! Pure dependency resolution over a set of candidate packages (E8, design D8 and
//! section 9.1). No I/O: the engine calls this before anything is instantiated.
//!
//! A dependency is satisfied by a candidate of the set with that id, or by an entry of
//! `loaded`. A candidate whose id is already loaded, or that shares its id with another
//! candidate, is a [`ResolveProblem::DuplicateId`]. Problems are reported per candidate,
//! the first one found in this priority: duplicate id, missing or mismatched dependency,
//! cycle, failed dependency. The load order is a topological sort with ties broken by id.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::version::{Version, VersionReq};

/// A dependency on another package: its id and the version range it accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub req: VersionReq,
}

/// A package offered for loading, with its version and declared dependencies.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub version: &'a Version,
    pub dependencies: &'a [Dependency],
}

/// Why a candidate cannot load. Requirements are kept as text, for messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveProblem {
    /// The id is shared with another candidate, or is already loaded.
    DuplicateId { id: String },
    /// No candidate or loaded package has the id `dep`.
    Missing { dep: String, req: String },
    /// A package `dep` exists but its version `found` is outside `req`.
    Version { dep: String, req: String, found: String },
    /// The candidate is on a dependency cycle; `path` starts and ends with its own id.
    Cycle { path: Vec<String> },
    /// A dependency `dep` is a candidate that failed for any reason.
    DependencyFailed { dep: String },
}

impl fmt::Display for ResolveProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveProblem::DuplicateId { id } => write!(f, "duplicate id `{id}`"),
            ResolveProblem::Missing { dep, req } => {
                write!(f, "dependency `{dep}` ({req}) is missing")
            }
            ResolveProblem::Version { dep, req, found } => {
                write!(f, "dependency `{dep}` requires {req}, found {found}")
            }
            ResolveProblem::Cycle { path } => write!(f, "cycle: {}", path.join(" -> ")),
            ResolveProblem::DependencyFailed { dep } => write!(f, "dependency `{dep}` failed"),
        }
    }
}

/// The outcome of resolving a set of candidates.
///
/// `order` lists the indices of the candidates that can load, dependencies first. Each
/// index appears in at most one of `order` and `problems`. `problems` is sorted by index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    pub order: Vec<usize>,
    pub problems: Vec<(usize, ResolveProblem)>,
}

/// Resolves `candidates` against the already `loaded` packages `(id, version)`.
pub fn resolve(candidates: &[Candidate<'_>], loaded: &[(&str, &Version)]) -> Resolution {
    let n = candidates.len();
    let mut by_id: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, c) in candidates.iter().enumerate() {
        by_id.entry(c.id).or_default().push(i);
    }
    let is_loaded = |id: &str| loaded.iter().any(|(l, _)| *l == id);

    // 1. Duplicate ids, then missing and mismatched dependencies.
    let mut problem: Vec<Option<ResolveProblem>> = vec![None; n];
    for (i, c) in candidates.iter().enumerate() {
        let shared = by_id.get(c.id).is_some_and(|ix| ix.len() > 1);
        if shared || is_loaded(c.id) {
            problem[i] = Some(ResolveProblem::DuplicateId { id: c.id.to_owned() });
            continue;
        }
        problem[i] = c
            .dependencies
            .iter()
            .find_map(|dep| check_dep(dep, candidates, &by_id, loaded));
    }

    // Dependency edges between candidates: only to a unique candidate of the set that is
    // not already loaded (a loaded package satisfies the dependency by itself).
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, c) in candidates.iter().enumerate() {
        for dep in c.dependencies {
            if is_loaded(dep.id.as_str()) {
                continue;
            }
            if let Some([j]) = by_id.get(dep.id.as_str()).map(Vec::as_slice) {
                if !edges[i].contains(j) {
                    edges[i].push(*j);
                }
            }
        }
    }

    // 2. Cycles, for candidates without a problem of their own.
    for i in 0..n {
        if problem[i].is_none() {
            if let Some(cycle) = cycle_through(i, &edges) {
                let path = cycle.iter().map(|&j| candidates[j].id.to_owned()).collect();
                problem[i] = Some(ResolveProblem::Cycle { path });
            }
        }
    }

    // 3. Failed dependencies, transitively. Each round uses the failures of the previous
    // round only, so the reported dependency does not depend on candidate order.
    loop {
        let failed: Vec<bool> = problem.iter().map(Option::is_some).collect();
        let mut newly = Vec::new();
        for (i, c) in candidates.iter().enumerate() {
            if failed[i] {
                continue;
            }
            let hit = c.dependencies.iter().find(|dep| {
                !is_loaded(dep.id.as_str())
                    && by_id
                        .get(dep.id.as_str())
                        .is_some_and(|ix| ix.iter().any(|&j| failed[j]))
            });
            if let Some(dep) = hit {
                newly.push((i, dep.id.clone()));
            }
        }
        if newly.is_empty() {
            break;
        }
        for (i, dep) in newly {
            problem[i] = Some(ResolveProblem::DependencyFailed { dep });
        }
    }

    // 4. Topological order of the candidates without a problem; ties by id.
    let ok: Vec<bool> = problem.iter().map(Option::is_none).collect();
    let mut indegree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in (0..n).filter(|&i| ok[i]) {
        indegree[i] = edges[i].len();
        for &j in &edges[i] {
            dependents[j].push(i);
        }
    }
    let mut ready: BTreeSet<(&str, usize)> = (0..n)
        .filter(|&i| ok[i] && indegree[i] == 0)
        .map(|i| (candidates[i].id, i))
        .collect();
    let mut order = Vec::new();
    while let Some((_, i)) = ready.pop_first() {
        order.push(i);
        for &r in &dependents[i] {
            indegree[r] -= 1;
            if indegree[r] == 0 {
                ready.insert((candidates[r].id, r));
            }
        }
    }
    debug_assert_eq!(order.len(), ok.iter().filter(|&&b| b).count());

    let problems = problem
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| (i, p)))
        .collect();
    Resolution { order, problems }
}

/// The first problem of one dependency that is not a failed candidate: missing, or a
/// version outside the requirement. `None` if it is satisfied, or if its provider in the
/// set is ambiguous (duplicate ids), which the failed-dependency pass reports.
fn check_dep(
    dep: &Dependency,
    candidates: &[Candidate<'_>],
    by_id: &BTreeMap<&str, Vec<usize>>,
    loaded: &[(&str, &Version)],
) -> Option<ResolveProblem> {
    let id = dep.id.as_str();
    let loaded_versions: Vec<&Version> = loaded
        .iter()
        .filter(|(l, _)| *l == id)
        .map(|(_, v)| *v)
        .collect();
    if let Some(first) = loaded_versions.first() {
        if loaded_versions.iter().any(|v| dep.req.matches(v)) {
            return None;
        }
        return Some(ResolveProblem::Version {
            dep: id.to_owned(),
            req: dep.req.to_string(),
            found: first.to_string(),
        });
    }
    match by_id.get(id) {
        None => Some(ResolveProblem::Missing { dep: id.to_owned(), req: dep.req.to_string() }),
        Some(ix) if ix.len() == 1 => {
            let found = candidates[ix[0]].version;
            if dep.req.matches(found) {
                None
            } else {
                Some(ResolveProblem::Version {
                    dep: id.to_owned(),
                    req: dep.req.to_string(),
                    found: found.to_string(),
                })
            }
        }
        Some(_) => None,
    }
}

/// The shortest cycle through `start`, as indices `start, ..., start`, found by a
/// breadth-first search that follows dependencies in declaration order.
fn cycle_through(start: usize, edges: &[Vec<usize>]) -> Option<Vec<usize>> {
    let n = edges.len();
    let mut parent = vec![usize::MAX; n];
    let mut visited = vec![false; n];
    let mut queue = VecDeque::from([start]);
    let mut closing = None;
    'search: while let Some(u) = queue.pop_front() {
        for &v in &edges[u] {
            if v == start {
                closing = Some(u);
                break 'search;
            }
            if !visited[v] {
                visited[v] = true;
                parent[v] = u;
                queue.push_back(v);
            }
        }
    }
    let last = closing?;
    let mut path = vec![last];
    let mut cur = last;
    while cur != start {
        cur = parent[cur];
        path.push(cur);
    }
    path.reverse();
    path.push(start);
    Some(path)
}
