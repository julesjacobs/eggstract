//! Acyclic, sum-once DAG extraction from e-graphs.
//!
//! The caller supplies the required roots. Each selected class has one selected
//! node, and each selected node is charged once, including shared dependencies.
//! Inputs may contain cycles; returned selections cannot.

mod compact;
mod exact_path;
mod graph;
mod verify;

pub use egraph_serialize::{ClassId, EGraph, NodeId};
use std::time::{Duration, Instant};
pub use verify::verify;

/// Search settings. The deadline is cooperative: indexing, tree seeding and
/// final verification always finish, and can exceed `time_limit`.
#[derive(Clone, Debug)]
pub struct Options {
    pub time_limit: Duration,
    /// Determines the order in which local alternatives are considered.
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            time_limit: Duration::from_secs(1),
            seed: 0,
        }
    }
}

/// An independently verified selection, with its recomputed sum-once cost.
/// Neither extraction routine promises a globally optimal DAG.
#[derive(Clone, Debug)]
pub struct Extraction {
    pub selected: Vec<NodeId>,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    NoRoots,
    UnknownRoot(ClassId),
    InvalidGraph(String),
    InvalidSelection(String),
    /// A required class has no finite acyclic derivation.
    Infeasible(ClassId),
    /// The sum of selected costs cannot be represented as a finite `f64`.
    CostOverflow,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRoots => write!(f, "at least one root is required"),
            Self::UnknownRoot(root) => write!(f, "unknown root {root}"),
            Self::InvalidGraph(message) => write!(f, "invalid graph: {message}"),
            Self::InvalidSelection(message) => write!(f, "invalid selection: {message}"),
            Self::Infeasible(root) => write!(f, "root {root} has no acyclic derivation"),
            Self::CostOverflow => write!(f, "selected costs overflow f64"),
        }
    }
}
impl std::error::Error for Error {}

/// Find an acyclic DAG using demand reconstruction, local descent, support
/// exchange and dependency repair. All node costs must be finite and nonnegative.
/// A zero time limit returns the tree seed. Duplicate roots are allowed.
///
/// Costs and heuristic comparisons use binary64 arithmetic. Final cost is
/// independently recomputed, but search quality is subject to rounding.
pub fn extract(graph: &EGraph, roots: &[ClassId], options: &Options) -> Result<Extraction, Error> {
    let start = Instant::now();
    let indexed = graph::FastGraph::new(graph, roots)?;
    let choices = compact::extract(&indexed, options, start)?;
    checked_result(graph, roots, &indexed, &choices)
}

/// Build a dense Dijkstra tree-cost seed and charge shared nodes only once.
/// Tree-cost addition saturates if tree duplication overflows; the resulting
/// DAG cost must still fit in a finite `f64`.
pub fn tree_seed(graph: &EGraph, roots: &[ClassId]) -> Result<Extraction, Error> {
    let indexed = graph::FastGraph::new(graph, roots)?;
    let choices = indexed.seed()?;
    let choices = compact::rooted_choices(&indexed, &choices)?;
    checked_result(graph, roots, &indexed, &choices)
}

fn checked_result(
    graph: &EGraph,
    roots: &[ClassId],
    indexed: &graph::FastGraph<'_>,
    choices: &[usize],
) -> Result<Extraction, Error> {
    let selected: Vec<_> = choices
        .iter()
        .filter(|&&n| n != graph::NONE)
        .map(|&n| indexed.graph.nodes.get_index(n).unwrap().0.clone())
        .collect();
    let cost = verify(graph, roots, &selected)?;
    Ok(Extraction { selected, cost })
}
