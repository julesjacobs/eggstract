use crate::generate::Graph;
use egg::{Id, Language, RecExpr, SymbolLang};
use eggstract::{ClassId, EGraph, NodeId};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Export {
    pub graph: EGraph,
    pub roots: Vec<ClassId>,
    pub canonical_roots: Vec<Id>,
    pub gym: upstream_gym::EGraph,
    nodes: HashMap<(Id, SymbolLang), NodeId>,
}

impl Export {
    pub fn new(input: &Graph, roots: &[Id]) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("no requested roots".into());
        }
        let mut classes: Vec<_> = input.classes().collect();
        classes.sort_by_key(|c| usize::from(c.id));
        let representatives: HashMap<_, _> = classes
            .iter()
            .map(|c| (c.id, format!("n{}_0", c.id)))
            .collect();
        let mut json_nodes = BTreeMap::new();
        let mut nodes = HashMap::new();
        for class in &classes {
            let mut alternatives = class.nodes.clone();
            alternatives.sort();
            for (i, node) in alternatives.into_iter().enumerate() {
                let node = node.map_children(|id| input.find(id));
                let id = format!("n{}_{}", class.id, i);
                json_nodes.insert(id.clone(), serde_json::json!({
                    "op": node.op.to_string(), "eclass": format!("c{}", class.id), "cost": 1.0,
                    "children": node.children.iter().map(|c| representatives[c].clone()).collect::<Vec<_>>()
                }));
                if nodes.insert((class.id, node), NodeId::from(id)).is_some() {
                    return Err("duplicate canonical node in rebuilt graph".into());
                }
            }
        }
        let canonical_roots: Vec<_> = roots.iter().map(|&id| input.find(id)).collect();
        let roots: Vec<_> = canonical_roots
            .iter()
            .map(|id| ClassId::from(format!("c{id}")))
            .collect();
        let graph = serde_json::from_value(serde_json::json!({
            "nodes": json_nodes, "root_eclasses": roots,
        }))
        .map_err(|e| format!("export failed: {e}"))?;
        let gym: upstream_gym::EGraph =
            serde_json::from_value(serde_json::to_value(&graph).map_err(|e| e.to_string())?)
                .map_err(|e| format!("upstream graph conversion failed: {e}"))?;
        gym.classes();
        Ok(Self {
            gym,
            graph,
            roots,
            canonical_roots,
            nodes,
        })
    }

    pub fn node(&self, input: &Graph, class: Id, node: &SymbolLang) -> Result<NodeId, String> {
        let node = node.clone().map_children(|id| input.find(id));
        self.nodes
            .get(&(input.find(class), node))
            .cloned()
            .ok_or_else(|| format!("output node is absent from original class {class}"))
    }

    pub fn gym_selection(&self, faster: bool) -> Result<Vec<NodeId>, String> {
        let choices = if faster {
            upstream_gym::faster_greedy_dag(&self.gym, &self.gym.root_eclasses)
        } else {
            upstream_gym::greedy_dag(&self.gym, &self.gym.root_eclasses)
        };
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = self.gym.root_eclasses.clone();
        while let Some(class) = stack.pop() {
            if !seen.insert(class.clone()) {
                continue;
            }
            let id = choices
                .get(&class)
                .ok_or_else(|| format!("gym omitted requested dependency {class}"))?;
            let node = self
                .gym
                .nodes
                .get(id)
                .ok_or_else(|| format!("gym returned unknown node {id}"))?;
            if node.eclass != class {
                return Err(format!("gym assigned node {id} to the wrong class"));
            }
            selected.push(NodeId::from(id.to_string()));
            stack.extend(
                node.children
                    .iter()
                    .map(|id| self.gym.nid_to_cid(id).clone()),
            );
        }
        Ok(selected)
    }

    pub fn tree_selection(
        &self,
        input: &Graph,
        extractor: &egg::Extractor<'_, egg::AstSize, SymbolLang, ()>,
    ) -> Result<Vec<NodeId>, String> {
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = self.canonical_roots.clone();
        while let Some(class) = stack.pop() {
            let class = input.find(class);
            if !seen.insert(class) {
                continue;
            }
            let node = extractor.find_best_node(class);
            selected.push(self.node(input, class, node)?);
            stack.extend(node.children());
        }
        Ok(selected)
    }

    pub fn lp_selection(
        &self,
        input: &Graph,
        expr: &RecExpr<SymbolLang>,
        roots: &[Id],
    ) -> Result<Vec<NodeId>, String> {
        if !expr.is_dag() {
            return Err("LP expression contains a cycle".into());
        }
        let mut classes = Vec::with_capacity(expr.len());
        let mut selected = Vec::with_capacity(expr.len());
        for node in expr.as_ref() {
            let children = node
                .children
                .iter()
                .map(|child| {
                    classes
                        .get(usize::from(*child))
                        .copied()
                        .ok_or_else(|| "LP output has a forward dependency".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut node = SymbolLang::new(node.op, children);
            let class = input
                .lookup(&mut node)
                .ok_or_else(|| "LP output node is absent from the original graph".to_string())?;
            selected.push(self.node(input, class, &node)?);
            classes.push(class);
        }
        if roots.len() != self.canonical_roots.len() {
            return Err("LP returned wrong number of roots".into());
        }
        for (&actual, &expected) in roots.iter().zip(&self.canonical_roots) {
            if classes.get(usize::from(actual)) != Some(&expected) {
                return Err("LP returned a different root class".into());
            }
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egg::{AstSize, Extractor};

    #[test]
    fn shared_multiroot_tree_round_trip() {
        let mut graph = Graph::default();
        let x = graph.add(SymbolLang::leaf("x"));
        let f = graph.add(SymbolLang::new("f", vec![x, x]));
        let g = graph.add(SymbolLang::new("g", vec![x]));
        graph.rebuild();
        let export = Export::new(&graph, &[f, g, f]).unwrap();
        let extractor = Extractor::new(&graph, AstSize);
        let selected = export.tree_selection(&graph, &extractor).unwrap();
        assert_eq!(
            eggstract::verify(&export.graph, &export.roots, &selected).unwrap(),
            3.0
        );
    }

    #[test]
    fn lp_expression_mapping_checks_original_roots_and_choices() {
        let mut graph = Graph::default();
        let x = graph.add(SymbolLang::leaf("x"));
        let f = graph.add(SymbolLang::new("f", vec![x]));
        let g = graph.add(SymbolLang::new("g", vec![x]));
        graph.rebuild();
        let export = Export::new(&graph, &[f, g]).unwrap();
        let mut expr = RecExpr::default();
        let ex = expr.add(SymbolLang::leaf("x"));
        let ef = expr.add(SymbolLang::new("f", vec![ex]));
        let eg = expr.add(SymbolLang::new("g", vec![ex]));
        let selected = export.lp_selection(&graph, &expr, &[ef, eg]).unwrap();
        assert_eq!(
            eggstract::verify(&export.graph, &export.roots, &selected).unwrap(),
            3.0
        );
        assert!(export.lp_selection(&graph, &expr, &[eg, ef]).is_err());
        assert!(export.lp_selection(&graph, &expr, &[ex, eg]).is_err());
    }

    #[test]
    fn cyclic_alternative_is_preserved_and_rejected_if_selected() {
        let mut graph = Graph::default();
        let x = graph.add(SymbolLang::leaf("x"));
        let f = graph.add(SymbolLang::new("f", vec![x]));
        graph.union(x, f);
        graph.rebuild();
        let export = Export::new(&graph, &[x]).unwrap();
        assert_eq!(export.graph.nodes.len(), 2);
        let extractor = Extractor::new(&graph, AstSize);
        let selected = export.tree_selection(&graph, &extractor).unwrap();
        assert_eq!(
            eggstract::verify(&export.graph, &export.roots, &selected).unwrap(),
            1.0
        );
        let cyclic = export
            .node(&graph, x, &SymbolLang::new("f", vec![graph.find(x)]))
            .unwrap();
        assert!(eggstract::verify(&export.graph, &export.roots, &[cyclic]).is_err());
    }
}
