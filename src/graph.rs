use crate::{ClassId, EGraph, Error};
use rustc_hash::FxHashMap;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub(super) const NONE: usize = usize::MAX;

pub(super) struct FastGraph<'a> {
    pub graph: &'a EGraph,
    pub class_ids: Vec<&'a ClassId>,
    pub node_class: Vec<usize>,
    pub costs: Vec<f64>,
    pub child_offsets: Vec<usize>,
    pub children: Vec<usize>,
    pub parent_offsets: Vec<usize>,
    pub parents: Vec<usize>,
    pub roots: Vec<usize>,
    pub class_sizes: Vec<usize>,
}

#[derive(Clone, Copy)]
struct Candidate {
    cost: f64,
    node: usize,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.node == other.node
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl<'a> FastGraph<'a> {
    pub fn new(graph: &'a EGraph, roots: &[ClassId]) -> Result<Self, Error> {
        let n = graph.nodes.len();
        let mut class_ids = Vec::new();
        let mut classes = FxHashMap::default();
        let mut node_ids = FxHashMap::with_capacity_and_hasher(n, Default::default());
        let mut node_class = Vec::with_capacity(n);
        let mut costs = Vec::with_capacity(n);
        let mut class_sizes = Vec::new();
        for (i, (id, node)) in graph.nodes.iter().enumerate() {
            let cost = node.cost.into_inner();
            if !cost.is_finite() || cost < 0.0 {
                return Err(Error::InvalidGraph(format!(
                    "node {id} requires finite nonnegative cost"
                )));
            }
            let c = *classes.entry(&node.eclass).or_insert_with(|| {
                class_ids.push(&node.eclass);
                class_sizes.push(0);
                class_ids.len() - 1
            });
            class_sizes[c] += 1;
            node_ids.insert(id, i);
            node_class.push(c);
            costs.push(cost);
        }
        let roots = roots
            .iter()
            .map(|root| {
                classes
                    .get(root)
                    .copied()
                    .ok_or_else(|| Error::UnknownRoot(root.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if roots.is_empty() {
            return Err(Error::NoRoots);
        }
        let edges: usize = graph.nodes.values().map(|node| node.children.len()).sum();
        let mut children = Vec::with_capacity(edges);
        let mut child_offsets = Vec::with_capacity(n + 1);
        let mut parent_offsets = vec![0; class_ids.len() + 1];
        for node in graph.nodes.values() {
            child_offsets.push(children.len());
            for id in &node.children {
                let child = node_ids
                    .get(id)
                    .ok_or_else(|| Error::InvalidGraph(format!("missing child {id}")))?;
                let c = node_class[*child];
                children.push(c);
                parent_offsets[c + 1] += 1;
            }
        }
        child_offsets.push(children.len());
        for c in 0..class_ids.len() {
            parent_offsets[c + 1] += parent_offsets[c];
        }
        let mut next = parent_offsets[..class_ids.len()].to_vec();
        let mut parents = vec![0; edges];
        for node in 0..n {
            for &c in &children[child_offsets[node]..child_offsets[node + 1]] {
                parents[next[c]] = node;
                next[c] += 1;
            }
        }
        Ok(Self {
            graph,
            class_ids,
            node_class,
            costs,
            child_offsets,
            children,
            parent_offsets,
            parents,
            roots,
            class_sizes,
        })
    }

    #[inline]
    pub fn children(&self, node: usize) -> &[usize] {
        &self.children[self.child_offsets[node]..self.child_offsets[node + 1]]
    }

    #[inline]
    pub fn parents(&self, class: usize) -> &[usize] {
        &self.parents[self.parent_offsets[class]..self.parent_offsets[class + 1]]
    }

    pub fn seed(&self) -> Result<Vec<usize>, Error> {
        let mut remaining: Vec<_> = self.child_offsets.windows(2).map(|w| w[1] - w[0]).collect();
        let mut totals = self.costs.clone();
        let mut choices = vec![NONE; self.class_ids.len()];
        let mut root = vec![false; self.class_ids.len()];
        let mut roots_remaining = 0;
        for &c in &self.roots {
            if !root[c] {
                roots_remaining += 1;
                root[c] = true;
            }
        }
        let mut initial = Vec::new();
        for (node, &count) in remaining.iter().enumerate() {
            if count == 0 {
                let candidate = Candidate {
                    node,
                    cost: totals[node],
                };
                initial.push(candidate);
            }
        }
        let mut pending = BinaryHeap::from(initial);
        while let Some(Candidate { node, cost }) = pending.pop() {
            let c = self.node_class[node];
            if choices[c] != NONE {
                continue;
            }
            choices[c] = node;
            if root[c] {
                roots_remaining -= 1;
                if roots_remaining == 0 {
                    return Ok(choices);
                }
            }
            for &parent in self.parents(c) {
                if choices[self.node_class[parent]] != NONE {
                    continue;
                }
                remaining[parent] -= 1;
                // Saturation preserves a finite candidate when tree duplication overflows.
                totals[parent] = (totals[parent] + cost).min(f64::MAX);
                if remaining[parent] == 0 {
                    let candidate = Candidate {
                        node: parent,
                        cost: totals[parent],
                    };
                    pending.push(candidate);
                }
            }
        }
        let root = self.roots.iter().find(|&&c| choices[c] == NONE).unwrap();
        Err(Error::Infeasible(self.class_ids[*root].clone()))
    }
}
