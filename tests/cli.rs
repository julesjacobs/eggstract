#![cfg(feature = "cli")]

use serde_json::Value;
use std::process::Command;

fn cli(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eggstract"))
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn extracts_machine_readable_solutions_and_rejects_bad_requests() {
    let output = cli(&["extract", "examples/sharing.json"]);
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["cost"], 6.0);
    assert_eq!(result["root_eclasses"], serde_json::json!(["R"]));
    assert_eq!(result["status"], "feasible");
    let seed = cli(&["extract", "examples/sharing.json", "--algorithm", "tree"]);
    let seed: Value = serde_json::from_slice(&seed.stdout).unwrap();
    assert_eq!(seed["cost"], 8.0);
    assert!(
        !cli(&["extract", "examples/sharing.json", "--root", "missing"])
            .status
            .success()
    );
}

#[test]
fn verification_ignores_claimed_cost_and_binds_declared_roots() {
    let path = std::env::temp_dir().join(format!("eggstract-cli-{}.json", std::process::id()));
    std::fs::write(
        &path,
        r#"{"selected_enodes":["a","b","root"],"root_eclasses":["R"],"cost":0,"status":"optimal"}"#,
    )
    .unwrap();
    let output = cli(&["verify", "examples/sharing.json", path.to_str().unwrap()]);
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["cost"], 8.0);
    std::fs::write(
        &path,
        r#"{"selected_enodes":["a","b","root"],"root_eclasses":["A"]}"#,
    )
    .unwrap();
    let rejected = cli(&["verify", "examples/sharing.json", path.to_str().unwrap()]);
    std::fs::remove_file(path).unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("roots differ"));
}
