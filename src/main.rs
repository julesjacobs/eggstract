use clap::{Parser, Subcommand, ValueEnum};
use eggstract::{extract, tree_seed, verify, ClassId, EGraph, NodeId, Options};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "Extract a shared acyclic DAG from an e-graph")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Extract using the graph's roots, or explicit --root overrides.
    Extract {
        input: PathBuf,
        #[arg(long, value_enum, default_value_t = Algorithm::Dag)]
        algorithm: Algorithm,
        #[arg(long, default_value_t = 1000)]
        time_limit_ms: u64,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long)]
        root: Vec<String>,
    },
    /// Verify selected nodes against the input graph's trusted roots.
    Verify {
        input: PathBuf,
        solution: PathBuf,
        #[arg(long)]
        root: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Algorithm {
    Dag,
    Tree,
}

fn roots(graph: &EGraph, overrides: Vec<String>) -> Vec<ClassId> {
    if overrides.is_empty() {
        graph.root_eclasses.clone()
    } else {
        overrides.into_iter().map(ClassId::from).collect()
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let output = match Args::parse().command {
        Command::Extract {
            input,
            algorithm,
            time_limit_ms,
            seed,
            root,
        } => {
            let graph: EGraph = serde_json::from_slice(&fs::read(input)?)?;
            let roots = roots(&graph, root);
            let result = match algorithm {
                Algorithm::Tree => tree_seed(&graph, &roots)?,
                Algorithm::Dag => extract(
                    &graph,
                    &roots,
                    &Options {
                        time_limit: Duration::from_millis(time_limit_ms),
                        seed,
                    },
                )?,
            };
            let cost = verify(&graph, &roots, &result.selected)?;
            json!({
                "selected_enodes": result.selected,
                "root_eclasses": roots,
                "cost": cost,
                "status": "feasible",
            })
        }
        Command::Verify {
            input,
            solution,
            root,
        } => {
            let graph: EGraph = serde_json::from_slice(&fs::read(input)?)?;
            let roots = roots(&graph, root);
            let candidate: serde_json::Value = serde_json::from_slice(&fs::read(solution)?)?;
            if let Some(declared) = candidate.get("root_eclasses") {
                let declared: Vec<ClassId> = serde_json::from_value(declared.clone())?;
                let expected: std::collections::BTreeSet<_> = roots.iter().collect();
                if declared.iter().collect::<std::collections::BTreeSet<_>>() != expected {
                    return Err("solution roots differ from requested roots".into());
                }
            }
            let selected: Vec<NodeId> = serde_json::from_value(
                candidate
                    .get("selected_enodes")
                    .ok_or("missing selected_enodes")?
                    .clone(),
            )?;
            let cost = verify(&graph, &roots, &selected)?;
            json!({ "valid": true, "cost": cost })
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eggstract: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
