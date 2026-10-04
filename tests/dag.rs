use eggstract::{extract, tree_seed, verify, ClassId, EGraph, Error, NodeId, Options};
use egraph_serialize::{Cost, Node};
use std::time::Duration;

fn graph(nodes: &[(&str, &str, f64, &[&str])]) -> EGraph {
    let mut graph = EGraph::default();
    for &(id, class, cost, children) in nodes {
        graph.add_node(
            id,
            Node {
                op: id.to_owned(),
                eclass: ClassId::from(class.to_owned()),
                cost: Cost::new(cost).unwrap(),
                children: children
                    .iter()
                    .map(|id| NodeId::from((*id).to_owned()))
                    .collect(),
                subsumed: false,
            },
        );
    }
    graph
}

fn roots(ids: &[&str]) -> Vec<ClassId> {
    ids.iter()
        .map(|id| ClassId::from((*id).to_owned()))
        .collect()
}

fn selected(ids: &[&str]) -> Vec<NodeId> {
    ids.iter()
        .map(|id| NodeId::from((*id).to_owned()))
        .collect()
}

#[test]
fn sharing_improves_tree_seed() {
    let input: EGraph = serde_json::from_str(include_str!("../examples/sharing.json")).unwrap();
    let seed = tree_seed(&input, &input.root_eclasses).unwrap();
    let result = extract(&input, &input.root_eclasses, &Options::default()).unwrap();
    assert_eq!(seed.cost, 8.0);
    assert_eq!(result.cost, 6.0);
    assert_eq!(
        verify(&input, &input.root_eclasses, &result.selected).unwrap(),
        6.0
    );
}

#[test]
fn trusted_roots_override_embedded_roots() {
    let mut input = graph(&[("r", "R", 100.0, &[]), ("l", "L", 1.0, &[])]);
    input.root_eclasses = roots(&["L"]);
    assert!(verify(&input, &roots(&["R"]), &selected(&["l"])).is_err());
    assert_eq!(
        extract(&input, &roots(&["R"]), &Options::default())
            .unwrap()
            .cost,
        100.0
    );
    assert_eq!(tree_seed(&input, &roots(&["R"])).unwrap().cost, 100.0);
}

#[test]
fn repeated_operands_and_roots_are_charged_once() {
    let input = graph(&[("r", "R", 0.5, &["x", "x"]), ("x", "X", 0.25, &[])]);
    let requested = roots(&["R", "R", "X"]);
    assert_eq!(tree_seed(&input, &requested).unwrap().cost, 0.75);
    assert_eq!(
        extract(&input, &requested, &Options::default())
            .unwrap()
            .cost,
        0.75
    );
}

#[test]
fn child_ids_refer_to_classes_not_forced_representatives() {
    let input = graph(&[
        ("r", "R", 0.0, &["expensive"]),
        ("expensive", "X", 100.0, &[]),
        ("cheap", "X", 1.0, &[]),
    ]);
    let result = tree_seed(&input, &roots(&["R"])).unwrap();
    assert_eq!(result.cost, 1.0);
    assert!(result.selected.contains(&NodeId::from("cheap".to_owned())));
}

#[test]
fn malformed_and_unreachable_selections_are_rejected() {
    let input = graph(&[
        ("r", "R", 1.0, &["x"]),
        ("x", "X", 2.0, &[]),
        ("y", "X", 3.0, &[]),
        ("extra", "E", 0.0, &[]),
    ]);
    for choice in [
        vec![],
        vec!["r"],
        vec!["r", "x", "y"],
        vec!["r", "x", "x"],
        vec!["r", "x", "extra"],
        vec!["missing"],
    ] {
        assert!(
            verify(&input, &roots(&["R"]), &selected(&choice)).is_err(),
            "{choice:?}"
        );
    }
    assert_eq!(
        verify(&input, &roots(&["R"]), &selected(&["r", "x"])).unwrap(),
        3.0
    );
    assert_eq!(verify(&input, &[], &[]), Err(Error::NoRoots));
    assert!(matches!(
        tree_seed(&input, &roots(&["missing"])),
        Err(Error::UnknownRoot(_))
    ));
}

#[test]
fn unsupported_cycle_is_infeasible_and_cyclic_selection_is_rejected() {
    let input = graph(&[("a", "A", 0.0, &["b"]), ("b", "B", 0.0, &["a"])]);
    assert!(matches!(
        tree_seed(&input, &roots(&["A"])),
        Err(Error::Infeasible(_))
    ));
    assert!(matches!(
        extract(&input, &roots(&["A"]), &Options::default()),
        Err(Error::Infeasible(_))
    ));
    assert!(verify(&input, &roots(&["A"]), &selected(&["a", "b"])).is_err());
}

#[test]
fn zero_cost_cycle_with_grounded_alternative_is_supported() {
    let input = graph(&[
        ("a", "A", 0.0, &["b"]),
        ("b", "B", 0.0, &["a"]),
        ("leaf", "B", 0.0, &[]),
    ]);
    let result = extract(&input, &roots(&["A"]), &Options::default()).unwrap();
    assert_eq!(result.cost, 0.0);
    assert_eq!(
        verify(&input, &roots(&["A"]), &result.selected).unwrap(),
        0.0
    );
}

#[test]
fn invalid_costs_and_missing_children_are_rejected() {
    for bad in [-1.0, f64::INFINITY, f64::NEG_INFINITY] {
        let input = graph(&[("r", "R", 1.0, &[]), ("bad", "B", bad, &[])]);
        assert!(matches!(
            tree_seed(&input, &roots(&["R"])),
            Err(Error::InvalidGraph(_))
        ));
        assert!(matches!(
            verify(&input, &roots(&["R"]), &selected(&["r"])),
            Err(Error::InvalidGraph(_))
        ));
    }
    let input = graph(&[("r", "R", 1.0, &["missing"])]);
    assert!(matches!(
        extract(&input, &roots(&["R"]), &Options::default()),
        Err(Error::InvalidGraph(_))
    ));
}

#[test]
fn dag_overflow_is_an_error_but_tree_duplication_can_saturate() {
    let input = graph(&[("r", "R", f64::MAX, &["x"]), ("x", "X", f64::MAX, &[])]);
    assert!(matches!(
        tree_seed(&input, &roots(&["R"])),
        Err(Error::CostOverflow)
    ));
    let shared = graph(&[("r", "R", 0.0, &["x", "x"]), ("x", "X", f64::MAX, &[])]);
    assert_eq!(tree_seed(&shared, &roots(&["R"])).unwrap().cost, f64::MAX);
}

#[test]
fn zero_budget_keeps_a_verified_seed() {
    let input: EGraph = serde_json::from_str(include_str!("../examples/sharing.json")).unwrap();
    let seed = tree_seed(&input, &input.root_eclasses).unwrap();
    let result = extract(
        &input,
        &input.root_eclasses,
        &Options {
            time_limit: Duration::ZERO,
            seed: 42,
        },
    )
    .unwrap();
    assert_eq!(result.selected, seed.selected);
    assert_eq!(result.cost, seed.cost);
}

#[test]
fn deep_dag_uses_iterative_traversal() {
    let mut input = EGraph::default();
    for i in 0..20_000 {
        let id = i.to_string();
        input.add_node(
            id.clone(),
            Node {
                op: "f".into(),
                eclass: ClassId::from(id),
                cost: Cost::new(1.0).unwrap(),
                children: if i == 0 {
                    vec![]
                } else {
                    vec![NodeId::from((i - 1).to_string())]
                },
                subsumed: false,
            },
        );
    }
    let requested = roots(&["19999"]);
    let result = tree_seed(&input, &requested).unwrap();
    assert_eq!(result.cost, 20_000.0);
    assert_eq!(
        verify(&input, &requested, &result.selected).unwrap(),
        20_000.0
    );
}

#[test]
fn generated_small_graphs_match_an_exhaustive_feasibility_oracle() {
    let mut random = 173u64;
    for _ in 0..80 {
        let mut next = || {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random
        };
        let mut input = EGraph::default();
        for c in 0..4 {
            for alternative in 0..2 {
                input.add_node(
                    format!("{c}-{alternative}"),
                    Node {
                        op: "f".into(),
                        eclass: ClassId::from(c.to_string()),
                        cost: Cost::new((next() % 8) as f64).unwrap(),
                        children: if alternative == 0 {
                            vec![]
                        } else {
                            (0..next() % 3)
                                .map(|_| NodeId::from(format!("{}-0", next() % 4)))
                                .collect()
                        },
                        subsumed: false,
                    },
                );
            }
        }
        let requested = roots(&["0", "1"]);
        let ids: Vec<_> = input.nodes.keys().cloned().collect();
        let mut optimum = f64::INFINITY;
        for mask in 0..1u32 << ids.len() {
            let choice: Vec<_> = ids
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, id)| id.clone())
                .collect();
            if let Ok(cost) = verify(&input, &requested, &choice) {
                optimum = optimum.min(cost);
            }
        }
        let seed = tree_seed(&input, &requested).unwrap();
        let result = extract(&input, &requested, &Options::default()).unwrap();
        assert!(result.cost >= optimum);
        assert!(result.cost <= seed.cost);
        assert_eq!(
            verify(&input, &requested, &result.selected).unwrap(),
            result.cost
        );
    }
}
