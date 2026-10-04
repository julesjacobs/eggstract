use egg::{rewrite as rw, Id, RecExpr, Rewrite, Runner, SymbolLang};
use std::time::Duration;

pub type Graph = egg::EGraph<SymbolLang, ()>;

pub struct Generated {
    pub graph: Graph,
    pub roots: Vec<Id>,
    pub stop: String,
    pub source_nodes: usize,
    pub reachable_source_nodes: usize,
}

fn add_reachable(graph: &mut Graph, expr: &RecExpr<SymbolLang>, roots: &[Id]) -> (Vec<Id>, usize) {
    let mut reachable = vec![false; expr.len()];
    let mut stack = roots.to_vec();
    while let Some(id) = stack.pop() {
        if std::mem::replace(&mut reachable[usize::from(id)], true) {
            continue;
        }
        stack.extend(&expr[id].children);
    }
    let mut mapping = vec![None; expr.len()];
    for (i, node) in expr.as_ref().iter().enumerate() {
        if !reachable[i] {
            continue;
        }
        let children = node
            .children
            .iter()
            .map(|id| mapping[usize::from(*id)].unwrap())
            .collect();
        mapping[i] = Some(graph.add(SymbolLang::new(node.op, children)));
    }
    (
        roots
            .iter()
            .map(|id| mapping[usize::from(*id)].unwrap())
            .collect(),
        reachable.iter().filter(|&&v| v).count(),
    )
}

struct Random(u64);
impl Random {
    fn index(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % n
    }
}

fn rules(boolean: bool) -> Vec<Rewrite<SymbolLang, ()>> {
    if boolean {
        vec![
            rw!("and-comm"; "(& ?a ?b)" => "(& ?b ?a)"),
            rw!("or-comm"; "(| ?a ?b)" => "(| ?b ?a)"),
            rw!("and-assoc"; "(& (& ?a ?b) ?c)" => "(& ?a (& ?b ?c))"),
            rw!("or-assoc"; "(| (| ?a ?b) ?c)" => "(| ?a (| ?b ?c))"),
            rw!("distribute"; "(& ?a (| ?b ?c))" => "(| (& ?a ?b) (& ?a ?c))"),
            rw!("factor"; "(| (& ?a ?b) (& ?a ?c))" => "(& ?a (| ?b ?c))"),
            rw!("absorb"; "(| ?a (& ?a ?b))" => "?a"),
            rw!("and-idempotent"; "(& ?a ?a)" => "?a"),
            rw!("or-idempotent"; "(| ?a ?a)" => "?a"),
            rw!("double-not"; "(! (! ?a))" => "?a"),
            rw!("de-morgan"; "(! (& ?a ?b))" => "(| (! ?a) (! ?b))"),
        ]
    } else {
        vec![
            rw!("add-comm"; "(+ ?a ?b)" => "(+ ?b ?a)"),
            rw!("mul-comm"; "(* ?a ?b)" => "(* ?b ?a)"),
            rw!("add-assoc"; "(+ (+ ?a ?b) ?c)" => "(+ ?a (+ ?b ?c))"),
            rw!("mul-assoc"; "(* (* ?a ?b) ?c)" => "(* ?a (* ?b ?c))"),
            rw!("distribute"; "(* ?a (+ ?b ?c))" => "(+ (* ?a ?b) (* ?a ?c))"),
            rw!("factor"; "(+ (* ?a ?b) (* ?a ?c))" => "(* ?a (+ ?b ?c))"),
            rw!("add-zero"; "(+ ?a 0)" => "?a"),
            rw!("mul-one"; "(* ?a 1)" => "?a"),
            rw!("mul-zero"; "(* ?a 0)" => "0"),
        ]
    }
}

pub fn generate(boolean: bool, size: usize, seed: u64) -> Result<Generated, String> {
    let mut random =
        Random(0x9f06e742ab519dc3 ^ (size as u64 * 97) ^ (seed * 65537) ^ boolean as u64);
    let mut expr = RecExpr::<SymbolLang>::default();
    for i in 0..16 {
        expr.add(SymbolLang::leaf(format!("x{i}")));
    }
    let mut roots = Vec::new();
    for i in 0..size {
        let len = expr.as_ref().len();
        let a = Id::from(random.index(len));
        let b = Id::from(random.index(len));
        let op = if boolean {
            ["&", "|", "!"][random.index(3)]
        } else {
            ["+", "*"][random.index(2)]
        };
        let children = if op == "!" { vec![a] } else { vec![a, b] };
        let n = expr.add(SymbolLang::new(op, children));
        if i >= size - 4 {
            roots.push(n);
        }
    }
    let mut runner = Runner::<SymbolLang, ()>::default()
        .with_iter_limit(5)
        .with_node_limit(30_000)
        .with_time_limit(Duration::from_secs(3600));
    let (roots, reachable_source_nodes) = add_reachable(&mut runner.egraph, &expr, &roots);
    runner.egraph.rebuild();
    runner = runner.run(&rules(boolean));
    if matches!(runner.stop_reason, Some(egg::StopReason::TimeLimit(_))) {
        return Err("generation reached its time guard; time-dependent input rejected".into());
    }
    let mut roots: Vec<_> = roots.iter().map(|&id| runner.egraph.find(id)).collect();
    roots.sort();
    roots.dedup();
    let stop = format!("{:?}", runner.stop_reason);
    Ok(Generated {
        graph: runner.egraph,
        roots,
        stop,
        source_nodes: expr.len(),
        reachable_source_nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_only_ancestors_of_all_requested_roots() {
        let mut expr = RecExpr::default();
        let x = expr.add(SymbolLang::leaf("x"));
        let unused = expr.add(SymbolLang::leaf("unused"));
        expr.add(SymbolLang::new("unused-f", vec![unused]));
        let f = expr.add(SymbolLang::new("f", vec![x]));
        let g = expr.add(SymbolLang::new("g", vec![x, x]));
        let mut graph = Graph::default();
        let (roots, count) = add_reachable(&mut graph, &expr, &[f, g, f]);
        graph.rebuild();
        assert_eq!(count, 3);
        assert_eq!(graph.total_number_of_nodes(), 3);
        assert_eq!(roots.len(), 3);
        assert_eq!(roots[0], roots[2]);
        assert_ne!(roots[0], roots[1]);
        assert!(graph.lookup(SymbolLang::leaf("unused")).is_none());
    }
}
