mod export;
mod generate;

use clap::Parser;
use egg::{AstSize, Extractor, LpExtractor};
use eggstract::{Extraction, Options};
use export::Export;
use generate::Graph;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(about = "Fixed unit-cost extraction comparison against egg 0.11.0")]
struct Args {
    #[arg(long)]
    output: PathBuf,
    /// Restrict the suite to polynomial/32/0; retains warmup and five measurements.
    #[arg(long)]
    smoke: bool,
}

#[derive(Clone, Copy)]
enum Method {
    EggTree,
    EggLp,
    GymGreedy,
    GymFaster,
    NativeTree,
    NativeDag,
}
impl Method {
    const ALL: [Self; 6] = [
        Self::EggTree,
        Self::EggLp,
        Self::GymGreedy,
        Self::GymFaster,
        Self::NativeTree,
        Self::NativeDag,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::EggTree => "egg-tree",
            Self::EggLp => "egg-lp",
            Self::GymGreedy => "gym-greedy-dag",
            Self::GymFaster => "gym-faster-greedy-dag",
            Self::NativeTree => "eggstract-tree",
            Self::NativeDag => "eggstract-dag",
        }
    }
}

#[derive(Serialize)]
struct Measurement {
    method: &'static str,
    warmup: bool,
    /// Zero for warmup; measured repetitions are numbered 1 through 5.
    repetition: usize,
    order: usize,
    elapsed_ns: u64,
    status: &'static str,
    cost: Option<f64>,
    selected_enodes: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct Case {
    id: String,
    family: &'static str,
    size: usize,
    seed: u64,
    nodes: usize,
    classes: usize,
    edges: usize,
    requested_roots: Vec<String>,
    unique_roots: usize,
    graph_sha256: String,
    graph_file: String,
    generation_stop: String,
    source_nodes: usize,
    reachable_source_nodes: usize,
    measurements: Vec<Measurement>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    complete: bool,
    config: serde_json::Value,
    cases: Vec<Case>,
    generation_failures: Vec<serde_json::Value>,
}

fn extract(method: Method, input: &Graph, exported: &Export) -> Result<Extraction, String> {
    match method {
        Method::NativeTree => {
            eggstract::tree_seed(&exported.graph, &exported.roots).map_err(|e| e.to_string())
        }
        Method::NativeDag => eggstract::extract(
            &exported.graph,
            &exported.roots,
            &Options {
                time_limit: Duration::from_secs(1),
                seed: 0,
            },
        )
        .map_err(|e| e.to_string()),
        Method::EggTree => {
            let extractor = Extractor::new(input, AstSize);
            let selected = exported.tree_selection(input, &extractor)?;
            let cost = eggstract::verify(&exported.graph, &exported.roots, &selected)
                .map_err(|e| e.to_string())?;
            Ok(Extraction { selected, cost })
        }
        Method::GymGreedy | Method::GymFaster => {
            let selected = exported.gym_selection(matches!(method, Method::GymFaster))?;
            let cost = eggstract::verify(&exported.graph, &exported.roots, &selected)
                .map_err(|e| e.to_string())?;
            Ok(Extraction { selected, cost })
        }
        Method::EggLp => {
            let (expr, roots) = LpExtractor::new(input, AstSize).solve_multiple_with_timeout(
                &exported.canonical_roots,
                good_lp::coin_cbc,
                1.0,
            );
            let selected = exported.lp_selection(input, &expr, &roots)?;
            let cost = eggstract::verify(&exported.graph, &exported.roots, &selected)
                .map_err(|e| e.to_string())?;
            Ok(Extraction { selected, cost })
        }
    }
}

fn measure(
    method: Method,
    input: &Graph,
    exported: &Export,
    repetition: usize,
    order: usize,
) -> Measurement {
    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        extract(method, input, exported)
    }));
    let elapsed_ns = start.elapsed().as_nanos().try_into().unwrap_or(u64::MAX);
    let (status, cost, selected_enodes, error) = match result {
        Ok(Ok(result)) => (
            "feasible",
            Some(result.cost),
            result
                .selected
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            None,
        ),
        Ok(Err(error)) => ("error", None, Vec::new(), Some(error)),
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            ("panic", None, Vec::new(), Some(message))
        }
    };
    Measurement {
        method: method.name(),
        warmup: repetition == 0,
        repetition,
        order,
        elapsed_ns,
        status,
        cost,
        selected_enodes,
        error,
    }
}

fn checkpoint(path: &Path, report: &Report) -> Result<(), Box<dyn std::error::Error>> {
    let temp = path.with_extension("tmp.json");
    fs::write(&temp, serde_json::to_vec_pretty(report)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.output.exists() {
        return Err(format!("output already exists: {}", args.output.display()).into());
    }
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let graph_dir_name = format!(
        "{}-graphs",
        args.output
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    );
    let graph_dir = parent.join(&graph_dir_name);
    fs::create_dir(&graph_dir)?;
    let mut report = Report {
        schema_version: 1,
        complete: false,
        config: json!({
            "egg_version": "0.11.0",
            "methods": Method::ALL.map(Method::name),
            "gym_revision": "903ba0f818b50608fe20ae9e0f03c35cb27bc50a",
            "gym_setup": "untimed serialization adapter and lazy class-index construction; upstream algorithms unchanged including iteration logging", "solver": "good_lp::coin_cbc", "cost_model": "unit node cost, summed once over requested-root DAG",
            "families": ["polynomial", "boolean"], "sizes": [32,128,512], "seeds": [0,1,2,3],
            "source_pruning": "union of ancestors of four source roots before insertion and saturation; final roots canonicalized and deduplicated",
            "generation_iterations": 5, "generation_node_limit": 30000, "generation_guard_seconds": 3600,
            "native_seconds": 1.0, "native_seed": 0, "lp_solver_seconds": 1.0,
            "budget_semantics": "native cooperative extraction budget; LP solver-only cap; neither is a total wall-time cap",
            "warmups": 1, "measured_repetitions": 5, "order": "rotate methods by (case index + repetition) modulo 6",
            "timing": "fresh extractor, algorithm setup/search, result construction and verification; excludes graph generation/export, output encoding and file IO",
            "lp_status": "API does not expose solver status or bound; feasible results do not imply optimality; DFS cycle pruning restricts alternatives",
            "smoke": args.smoke, "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
            "started_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        }),
        cases: Vec::new(),
        generation_failures: Vec::new(),
    };
    checkpoint(&args.output, &report)?;
    let mut case_index = 0;
    'families: for boolean in [false, true] {
        let family = if boolean { "boolean" } else { "polynomial" };
        for size in [32, 128, 512] {
            for seed in 0..4 {
                let id = format!("{family}/{size}/{seed}");
                eprintln!("Generating {id}");
                let generated = generate::generate(boolean, size, seed).and_then(|generated| {
                    Export::new(&generated.graph, &generated.roots)
                        .map(|export| (generated, export))
                });
                let (generated, exported) = match generated {
                    Ok(generated) => generated,
                    Err(error) => {
                        report
                            .generation_failures
                            .push(json!({"id": id, "error": error}));
                        checkpoint(&args.output, &report)?;
                        case_index += 1;
                        if args.smoke {
                            break 'families;
                        }
                        continue;
                    }
                };
                let input = generated.graph;
                let graph_bytes = serde_json::to_vec(&exported.graph)?;
                let graph_file = format!("{family}-{size}-{seed}.json");
                fs::write(graph_dir.join(&graph_file), &graph_bytes)?;
                let unique_roots = exported
                    .roots
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                report.cases.push(Case {
                    id: id.clone(),
                    family,
                    size,
                    seed,
                    nodes: exported.graph.nodes.len(),
                    classes: input.number_of_classes(),
                    edges: exported
                        .graph
                        .nodes
                        .values()
                        .map(|node| node.children.len())
                        .sum(),
                    requested_roots: exported.roots.iter().map(|id| id.to_string()).collect(),
                    unique_roots,
                    graph_sha256: format!("{:x}", Sha256::digest(&graph_bytes)),
                    graph_file: format!("{graph_dir_name}/{graph_file}"),
                    generation_stop: generated.stop,
                    source_nodes: generated.source_nodes,
                    reachable_source_nodes: generated.reachable_source_nodes,
                    measurements: Vec::new(),
                });
                checkpoint(&args.output, &report)?;
                for repetition in 0..=5 {
                    for order in 0..Method::ALL.len() {
                        let method =
                            Method::ALL[(case_index + repetition + order) % Method::ALL.len()];
                        let measurement = measure(method, &input, &exported, repetition, order);
                        eprintln!(
                            "{id} {} repetition {repetition}: {}",
                            method.name(),
                            measurement.status
                        );
                        report
                            .cases
                            .last_mut()
                            .unwrap()
                            .measurements
                            .push(measurement);
                        checkpoint(&args.output, &report)?;
                    }
                }
                case_index += 1;
                if args.smoke {
                    break 'families;
                }
            }
        }
    }
    report.complete = true;
    checkpoint(&args.output, &report)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egg::SymbolLang;

    #[test]
    fn all_methods_preserve_shared_roots() {
        let mut graph = Graph::default();
        let x = graph.add(SymbolLang::leaf("x"));
        let f = graph.add(SymbolLang::new("f", vec![x, x]));
        let g = graph.add(SymbolLang::new("g", vec![x]));
        graph.rebuild();
        let export = Export::new(&graph, &[f, g]).unwrap();
        for method in Method::ALL {
            let result = extract(method, &graph, &export).unwrap();
            assert_eq!(result.cost, 3.0, "{}", method.name());
            assert_eq!(result.selected.len(), 3);
        }
    }
}
