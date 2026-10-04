#!/usr/bin/env python3
"""Plot the fixed synthetic suite: --raw results.json --output results.

Writes SVG, PNG, and summary JSON. Aggregates require every measured repetition
of every configured case; failed or missing runs never create a smaller cohort.
Saved witnesses must be checked separately with verify_results.py.
"""

import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path
import statistics
import sys


METHODS = {
    "egg-tree": ("egg tree", "#737d8c"),
    "egg-lp": ("egg LP", "#ce8725"),
    "gym-greedy-dag": ("Gym greedy DAG", "#659b6e"),
    "gym-faster-greedy-dag": ("Gym faster greedy DAG", "#8467b1"),
    "eggstract-dag": ("Eggstract DAG", "#2776b8"),
}
KNOWN_METHODS = {*METHODS, "eggstract-tree"}
FAMILIES = {"boolean": "Boolean", "polynomial": "Polynomial"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def positive_number(value):
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value > 0
    )


def geometric_mean(values):
    return math.exp(statistics.mean(math.log(value) for value in values))


def summarize(report):
    require(report["schema_version"] == 1, "unsupported report schema")
    require(type(report["complete"]) is bool, "complete must be a boolean")
    config = report["config"]
    methods = config["methods"]
    require(isinstance(methods, list) and len(methods) == len(set(methods)),
            "config.methods must be a list without duplicates")
    require(set(METHODS) <= set(methods) <= KNOWN_METHODS,
            "config.methods must contain the five chart methods; only eggstract-tree is optional")
    require(config["warmups"] == 1, "expected one warmup, numbered zero")
    repetitions = config["measured_repetitions"]
    require(type(repetitions) is int and repetitions > 0,
            "measured_repetitions must be a positive integer")
    require(type(config["smoke"]) is bool, "smoke must be a boolean")
    for field in ("families", "sizes", "seeds"):
        values = config[field]
        require(isinstance(values, list) and values and len(values) == len(set(values)),
                f"config.{field} must be a nonempty list without duplicates")
    require(set(config["families"]) <= set(FAMILIES), "unknown benchmark family")
    require(all(type(n) is int and n > 0 for n in config["sizes"]), "invalid size")
    require(all(type(n) is int and n >= 0 for n in config["seeds"]), "invalid seed")
    grid = list(itertools.product(config["families"], config["sizes"], config["seeds"]))
    if config["smoke"]:
        grid = grid[:1]
    expected = {f"{family}/{size}/{seed}": (family, size, seed) for family, size, seed in grid}
    families = [family for family in FAMILIES if any(row[0] == family for row in grid)]
    issues = []
    cases = {}
    generation_failures = {}
    for failure in report["generation_failures"]:
        case_id = failure["id"]
        require(case_id in expected, f"unexpected generation failure: {case_id}")
        require(case_id not in generation_failures, f"duplicate generation failure: {case_id}")
        generation_failures[case_id] = failure
        issues.append({"case": case_id, "phase": "generation", "error": failure["error"]})
    for case in report["cases"]:
        case_id = case["id"]
        require(case_id in expected, f"unexpected case: {case_id}")
        require(case_id not in cases, f"duplicate case: {case_id}")
        require(case_id not in generation_failures, f"case also has a generation failure: {case_id}")
        require((case["family"], case["size"], case["seed"]) == expected[case_id],
                f"case metadata disagrees with id: {case_id}")
        require(case["requested_roots"], f"expected nonempty roots: {case_id}")
        indexed = {}
        for run in case["measurements"]:
            method, repetition = run["method"], run["repetition"]
            require(method in methods, f"unconfigured method: {method}")
            require(type(repetition) is int and 0 <= repetition <= repetitions,
                    f"unexpected repetition in {case_id}: {repetition}")
            require(type(run["warmup"]) is bool and run["warmup"] == (repetition == 0),
                    f"inconsistent warmup in {case_id}: {method}/{repetition}")
            key = method, repetition
            require(key not in indexed, f"duplicate measurement in {case_id}: {key}")
            indexed[key] = run
        cases[case_id] = indexed

    case_metrics = {}
    for case_id in expected:
        if case_id not in cases:
            if case_id not in generation_failures:
                issues.append({"case": case_id, "phase": "generation", "error": "missing case"})
            case_metrics[case_id] = {method: None for method in methods}
            continue
        case_metrics[case_id] = {}
        for method in methods:
            costs, elapsed = [], []
            for repetition in range(repetitions + 1):
                run = cases[case_id].get((method, repetition))
                error = None
                if run is None:
                    error = "missing measurement"
                elif run["status"] != "feasible":
                    error = f"{run['status']}: {run.get('error') or 'no detail supplied'}"
                elif run.get("error") is not None:
                    error = f"feasible measurement has an error: {run['error']}"
                elif not positive_number(run["cost"]):
                    error = f"invalid cost: {run['cost']!r}"
                elif not positive_number(run["elapsed_ns"]):
                    error = f"invalid elapsed_ns: {run['elapsed_ns']!r}"
                if error:
                    issues.append({"case": case_id, "method": method,
                                   "phase": "warmup" if repetition == 0 else "measured",
                                   "repetition": repetition, "error": error})
                elif repetition:
                    costs.append(run["cost"])
                    elapsed.append(run["elapsed_ns"] / 1_000_000)
            case_metrics[case_id][method] = (
                {"median_cost": statistics.median(costs),
                 "median_elapsed_ms": statistics.median(elapsed)}
                if len(costs) == repetitions else None
            )

    aggregates = {}
    for family in families:
        ids = [case_id for case_id, row in expected.items() if row[0] == family]
        baseline_complete = all(case_metrics[case_id]["egg-tree"] is not None for case_id in ids)
        aggregates[family] = {}
        for method in methods:
            available = [case_metrics[case_id][method] for case_id in ids]
            count = sum(value is not None for value in available)
            complete = count == len(ids)
            aggregates[family][method] = {
                "expected_cases": len(ids),
                "complete_cases": count,
                "cost_ratio": geometric_mean([
                    case_metrics[case_id][method]["median_cost"]
                    / case_metrics[case_id]["egg-tree"]["median_cost"] for case_id in ids
                ]) if complete and baseline_complete else None,
                "elapsed_ms": geometric_mean([value["median_elapsed_ms"] for value in available])
                if complete else None,
                "baseline_complete": baseline_complete,
            }
    return {
        "schema_version": 1,
        "run_complete": report["complete"],
        "smoke": config["smoke"],
        "config": config,
        "plotted_methods": list(METHODS),
        "expected_cases": len(expected),
        "observed_cases": len(cases),
        "aggregation": {
            "cost_ratio": "geometric mean of per-case median cost divided by egg-tree per-case median cost",
            "elapsed_ms": "geometric mean of per-case median elapsed milliseconds",
            "warmups": "excluded from aggregates; all failures and missing warmups reported",
            "missing_data": "no aggregate unless all measured repetitions of all configured family cases are valid",
        },
        "families": aggregates,
        "cases": case_metrics,
        "issues": issues,
    }


def plot(summary, output):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.patches import Patch
    from matplotlib.ticker import FuncFormatter, LogLocator, NullLocator

    plt.rcParams.update({
        "font.family": "DejaVu Sans", "font.size": 10,
        "axes.spines.top": False, "axes.spines.right": False,
        "svg.hashsalt": "eggstract-extractor-comparison", "svg.fonttype": "none",
    })
    fig, axes = plt.subplots(1, 2, figsize=(12, 5.7))
    fig.subplots_adjust(left=0.075, right=0.97, bottom=0.30, top=0.75, wspace=0.27)
    flags = []
    if summary["smoke"]:
        flags.append("SMOKE RUN")
    if not summary["run_complete"]:
        flags.append("RUN INCOMPLETE")
    for method in summary["config"]["methods"]:
        runs = [issue for issue in summary["issues"]
                if issue.get("method") == method and issue["phase"] == "measured"]
        failed = {issue["case"] for issue in runs if issue["error"] != "missing measurement"}
        missing = {issue["case"] for issue in runs if issue["error"] == "missing measurement"}
        label = METHODS.get(method, ("Eggstract seed", None))[0]
        if failed:
            flags.append(f"{label}: {len(failed)}/{summary['expected_cases']} cases failed")
        if missing:
            flags.append(f"{label}: {len(missing)}/{summary['expected_cases']} cases incomplete")
    title = "Extraction on synthetic e-graphs"
    if flags:
        title += "  ·  " + " / ".join(flags)
    fig.suptitle(title, x=0.075, y=0.97, ha="left", fontsize=15, fontweight="bold")
    config = summary["config"]
    fig.text(0.075, 0.915,
             f"{summary['expected_cases']} configured cases · one warmup + "
             f"{config['measured_repetitions']} measured repetitions · lower is better",
             fontsize=10, color="#46515c")
    fig.legend(handles=[Patch(color=color, label=label) for label, color in METHODS.values()],
               loc="upper left", bbox_to_anchor=(0.066, 0.879), ncol=5,
               frameon=False, columnspacing=1.4, handlelength=1.2)
    families = list(summary["families"])
    width = 0.14
    for axis, metric in zip(axes, ("cost_ratio", "elapsed_ms")):
        axis.set_axisbelow(True)
        axis.grid(axis="y", alpha=0.2)
        axis.set_xlim(-0.55, len(families) - 0.45)
        axis.set_xticks(range(len(families)), [
            f"{FAMILIES[family]}\n({summary['families'][family]['egg-tree']['expected_cases']} cases)"
            for family in families
        ])
        for method_index, (method, (_, color)) in enumerate(METHODS.items()):
            for family_index, family in enumerate(families):
                x = family_index + (method_index - 2) * width
                item = summary["families"][family][method]
                value = item[metric]
                if value is None:
                    axis.plot(x, 0.10, "x", color=color, markersize=8, markeredgewidth=2,
                              transform=axis.get_xaxis_transform())
                    text = f"{item['complete_cases']}/{item['expected_cases']}"
                    if metric == "cost_ratio" and not item["baseline_complete"]:
                        text += "*"
                    axis.text(x, 0.02, text, ha="center", fontsize=8, color=color,
                              transform=axis.get_xaxis_transform())
                elif metric == "cost_ratio":
                    axis.bar(x, value, width=width * 0.87, color=color)
                    axis.annotate(f"{value:.3f}", (x, value), xytext=(0, 4 + 12 * (method_index % 2)),
                                  textcoords="offset points", ha="center", fontsize=8)
                else:
                    axis.plot(x, value, "o", color=color, markersize=7)
                    axis.annotate(f"{value:.3g}", (x, value), xytext=(0, 8),
                                  textcoords="offset points", ha="center", fontsize=8)
        if metric == "cost_ratio":
            axis.set_title("Shared DAG cost relative to egg tree", loc="left", fontsize=11, pad=12)
            axis.set_ylabel("Geometric mean cost ratio")
            axis.axhline(1, color="#52606d", linestyle="--", linewidth=1)
            values = [entry["cost_ratio"] for family in summary["families"].values()
                      for method, entry in family.items() if method in METHODS and entry["cost_ratio"] is not None]
            axis.set_ylim(0, max([1, *values]) * 1.22)
        else:
            axis.set_title("Measured extraction time", loc="left", fontsize=11, pad=12)
            axis.set_ylabel("Geometric mean milliseconds · log scale")
            axis.set_yscale("log")
            axis.yaxis.set_major_locator(LogLocator(base=10, subs=(1, 2, 5), numticks=8))
            axis.yaxis.set_major_formatter(FuncFormatter(lambda value, _: f"{value:g}"))
            axis.yaxis.set_minor_locator(NullLocator())
            values = [entry["elapsed_ms"] for family in summary["families"].values()
                      for method, entry in family.items() if method in METHODS and entry["elapsed_ms"] is not None]
            if values:
                factor = max(1.8, (max(values) / min(values)) ** 0.13)
                axis.set_ylim(min(values) / factor, max(values) * factor)
            else:
                axis.set_ylim(0.01, 1000)
    counts = {phase: sum(issue["phase"] == phase for issue in summary["issues"])
              for phase in ("generation", "warmup", "measured")}
    notes = [
        "Per case: median of measured runs. Per family: geometric mean of cost ratios and of median elapsed times.",
        "Timing includes extractor setup, search, result recovery and verification; excludes graph generation, export and file I/O.",
        f"Eggstract: {config['native_seconds']:g} s cooperative search budget. egg LP: {config['lp_solver_seconds']:g} s solver limit; elapsed time may exceed either limit.",
    ]
    if summary["issues"]:
        notes.append(f"Generation failures/missing: {counts['generation']}. Failed/missing runs: {counts['warmup']} warmups, "
                     f"{counts['measured']} measured. × = unavailable; n/N = complete cases."
                     + (" * = baseline incomplete." if any(
                         not family["egg-tree"]["baseline_complete"]
                         for family in summary["families"].values()) else ""))
    else:
        notes.append("All configured cases and repetitions present and feasible. Feasibility does not establish LP optimality.")
    for index, note in enumerate(notes):
        fig.text(0.075, 0.185 - index * 0.034, note, fontsize=8, color="#46515c")
    fig.savefig(output.with_suffix(".svg"), metadata={"Date": None})
    svg = output.with_suffix(".svg")
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n")
    fig.savefig(output.with_suffix(".png"), dpi=180, metadata={"Software": "Matplotlib"})
    plt.close(fig)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", required=True, type=Path, help="benchmark JSON report")
    parser.add_argument("--output", required=True, type=Path, help="output prefix (or .svg/.png path)")
    args = parser.parse_args()
    raw = args.raw.read_bytes()
    try:
        summary = summarize(json.loads(raw))
    except (KeyError, TypeError, ValueError, OverflowError) as error:
        parser.error(f"invalid report: {error}")
    summary["raw_file"] = args.raw.name
    summary["raw_sha256"] = hashlib.sha256(raw).hexdigest()
    output = args.output
    if output.suffix in (".svg", ".png", ".json"):
        output = output.with_suffix("")
    output.parent.mkdir(parents=True, exist_ok=True)
    summary_path = output.with_suffix(".summary.json")
    require(args.raw.resolve() not in {summary_path.resolve(), output.with_suffix(".svg").resolve(),
                                      output.with_suffix(".png").resolve()}, "output would overwrite raw input")
    summary_path.write_text(json.dumps(summary, indent=2, allow_nan=False) + "\n")
    plot(summary, output)
    print(f"Wrote {output.with_suffix('.svg')}, {output.with_suffix('.png')}, {summary_path}")
    for issue in summary["issues"]:
        print(json.dumps(issue), file=sys.stderr)
    if not summary["run_complete"] or summary["issues"]:
        print("Incomplete or failed runs are explicitly marked; see summary JSON.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
