pub use egraph_serialize::{ClassId, EGraph, NodeId};
use indexmap::IndexMap;
use ordered_float::NotNan;
use std::collections::HashMap;

#[rustfmt::skip]
mod faster_greedy_dag;
#[rustfmt::skip]
mod greedy_dag;

type Cost = NotNan<f64>;
const INFINITY: Cost = unsafe { NotNan::new_unchecked(f64::INFINITY) };

trait Extractor: Sync {
    fn extract(&self, egraph: &EGraph, roots: &[ClassId]) -> ExtractionResult;
}

#[derive(Default)]
struct ExtractionResult {
    choices: IndexMap<ClassId, NodeId>,
}

impl ExtractionResult {
    fn choose(&mut self, class_id: ClassId, node_id: NodeId) {
        self.choices.insert(class_id, node_id);
    }
}

pub fn greedy_dag(egraph: &EGraph, roots: &[ClassId]) -> IndexMap<ClassId, NodeId> {
    greedy_dag::GreedyDagExtractor
        .extract(egraph, roots)
        .choices
}

pub fn faster_greedy_dag(egraph: &EGraph, roots: &[ClassId]) -> IndexMap<ClassId, NodeId> {
    faster_greedy_dag::FasterGreedyDagExtractor
        .extract(egraph, roots)
        .choices
}
