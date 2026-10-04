use eggstract::{extract, tree_seed, verify, EGraph, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph: EGraph = serde_json::from_str(include_str!("sharing.json"))?;
    let roots = &graph.root_eclasses;
    let seed = tree_seed(&graph, roots)?;
    let result = extract(&graph, roots, &Options::default())?;
    println!("tree seed: {}, DAG extraction: {}", seed.cost, result.cost);
    assert_eq!(verify(&graph, roots, &result.selected)?, result.cost);
    Ok(())
}
