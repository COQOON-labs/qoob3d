//! Build graph of the declared parts (AG-PJ-02, AG-PJ-04): names are unique
//! across the project, dependencies must exist and must not form a cycle.
//! The build order puts every part after its dependencies.

use std::collections::HashMap;
use std::path::PathBuf;

use super::ProjectError;

/// A part as declared by `part("Name", #{ depends: [...] }, ...)` (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartDecl {
    pub name: String,
    /// Declaring file, relative to the project root.
    pub file: PathBuf,
    pub depends: Vec<String>,
}

/// A validated, acyclic set of parts.
#[derive(Debug, Clone)]
pub struct Graph {
    parts: Vec<PartDecl>,
    /// Indices of the dependencies of each part.
    deps: Vec<Vec<usize>>,
    /// Indices into `parts`, dependencies first.
    order: Vec<usize>,
}

impl Graph {
    /// Checks the declarations and computes the build order.
    ///
    /// # Errors
    /// Duplicate name, unknown dependency or dependency cycle.
    pub fn new(parts: Vec<PartDecl>) -> Result<Self, ProjectError> {
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (i, part) in parts.iter().enumerate() {
            if let Some(&first) = index.get(part.name.as_str()) {
                return Err(ProjectError::DuplicatePart {
                    name: part.name.clone(),
                    first: parts[first].file.clone(),
                    second: part.file.clone(),
                });
            }
            index.insert(part.name.as_str(), i);
        }
        let mut deps = Vec::with_capacity(parts.len());
        for part in &parts {
            let resolved: Result<Vec<usize>, _> = part
                .depends
                .iter()
                .map(|d| {
                    index.get(d.as_str()).copied().ok_or_else(|| ProjectError::UnknownDependency {
                        part: part.name.clone(),
                        dependency: d.clone(),
                    })
                })
                .collect();
            deps.push(resolved?);
        }
        let order = topological_order(&deps).map_err(|cycle| ProjectError::Cycle {
            cycle: cycle.into_iter().map(|i| parts[i].name.clone()).collect(),
        })?;
        Ok(Self { parts, deps, order })
    }

    /// Part names in build order: every part after its dependencies.
    #[must_use]
    pub fn order(&self) -> Vec<&str> {
        self.order.iter().map(|&i| self.parts[i].name.as_str()).collect()
    }

    /// Parts that depend on `name` directly or indirectly, in build order;
    /// they have to be rebuilt when `name` changes.
    #[must_use]
    pub fn dependents(&self, name: &str) -> Vec<&str> {
        let Some(changed) = self.parts.iter().position(|p| p.name == name) else {
            return Vec::new();
        };
        let mut affected = vec![false; self.parts.len()];
        // In build order every dependency comes first, so one pass suffices.
        for &i in &self.order {
            affected[i] = self.deps[i].iter().any(|&d| d == changed || affected[d]);
        }
        self.order.iter().filter(|&&i| affected[i]).map(|&i| self.parts[i].name.as_str()).collect()
    }
}

/// Kahn's algorithm over `deps[i]` (the dependencies of node `i`),
/// deterministic for the same input. On a cycle, returns its nodes in
/// dependency direction, the first node repeated at the end.
fn topological_order(deps: &[Vec<usize>]) -> Result<Vec<usize>, Vec<usize>> {
    let mut missing: Vec<usize> = deps.iter().map(Vec::len).collect();
    let mut dependents = vec![Vec::new(); deps.len()];
    for (i, ds) in deps.iter().enumerate() {
        for &d in ds {
            dependents[d].push(i);
        }
    }
    let mut order: Vec<usize> = (0..deps.len()).filter(|&i| missing[i] == 0).collect();
    let mut next = 0;
    while let Some(&i) = order.get(next) {
        next += 1;
        for &j in &dependents[i] {
            missing[j] -= 1;
            if missing[j] == 0 {
                order.push(j);
            }
        }
    }
    let Some(start) = (0..deps.len()).find(|&i| missing[i] > 0) else {
        return Ok(order);
    };
    // Every unfinished node waits for an unfinished dependency, so following
    // those must revisit a node; the path from that node on is the cycle.
    let mut path = vec![start];
    loop {
        let last = path[path.len() - 1];
        let d = *deps[last]
            .iter()
            .find(|&&d| missing[d] > 0)
            .expect("an unfinished node waits for an unfinished dependency");
        if let Some(at) = path.iter().position(|&p| p == d) {
            let mut cycle = path.split_off(at);
            cycle.push(d);
            return Err(cycle);
        }
        path.push(d);
    }
}
