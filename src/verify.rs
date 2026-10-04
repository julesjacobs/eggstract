use crate::{ClassId, EGraph, Error, NodeId};
use std::collections::{HashMap, HashSet};

/// Independently check closure, one choice per class, trusted roots, rooted
/// reachability and acyclicity, then sum each selected node once. This checker
/// uses an iterative traversal, including on deeply nested inputs.
///
/// The graph's embedded `root_eclasses` are ignored. Unknown, duplicate and
/// unreachable selected nodes are rejected. Every input node must have finite
/// nonnegative cost and existing child IDs, including unselected alternatives.
pub fn verify(graph: &EGraph, roots: &[ClassId], selected: &[NodeId]) -> Result<f64, Error> {
    if roots.is_empty() {
        return Err(Error::NoRoots);
    }
    let mut classes = HashSet::new();
    for (id, node) in &graph.nodes {
        let cost = node.cost.into_inner();
        if !cost.is_finite() || cost < 0.0 {
            return Err(Error::InvalidGraph(format!(
                "node {id} requires finite nonnegative cost"
            )));
        }
        classes.insert(&node.eclass);
        for child in &node.children {
            if !graph.nodes.contains_key(child) {
                return Err(Error::InvalidGraph(format!(
                    "node {id} has missing child {child}"
                )));
            }
        }
    }
    for root in roots {
        if !classes.contains(root) {
            return Err(Error::UnknownRoot(root.clone()));
        }
    }
    let mut choices = HashMap::new();
    let mut cost = 0.0;
    for id in selected {
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| Error::InvalidSelection(format!("unknown node {id}")))?;
        if choices.insert(&node.eclass, (id, node)).is_some() {
            return Err(Error::InvalidSelection(format!(
                "multiple selected nodes in class {}",
                node.eclass
            )));
        }
        cost += node.cost.into_inner();
    }
    let mut colors = HashMap::new();
    let mut reached = 0;
    for root in roots {
        let mut stack = vec![(root, false)];
        while let Some((class, leaving)) = stack.pop() {
            if leaving {
                colors.insert(class, 2u8);
                continue;
            }
            match colors.get(class).copied().unwrap_or(0) {
                2 => continue,
                1 => {
                    return Err(Error::InvalidSelection(format!(
                        "cycle through class {class}"
                    )))
                }
                _ => (),
            }
            let (_, node) = choices.get(class).ok_or_else(|| {
                Error::InvalidSelection(format!("missing choice for class {class}"))
            })?;
            colors.insert(class, 1);
            reached += 1;
            stack.push((class, true));
            for child in node.children.iter().rev() {
                stack.push((&graph.nodes[child].eclass, false));
            }
        }
    }
    if reached != selected.len() {
        return Err(Error::InvalidSelection(
            "selected nodes are unreachable from requested roots".into(),
        ));
    }
    if !cost.is_finite() {
        return Err(Error::CostOverflow);
    }
    Ok(cost)
}
