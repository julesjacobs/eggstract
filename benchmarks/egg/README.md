# Extractor comparison

This is a reproducible synthetic comparison, not an application-performance or
held-out benchmark. The generator's design descends from Eggstract's development
workloads. The complete parameter grid is retained regardless of the results.

## Methods

- **egg tree:** upstream `egg::Extractor` with `AstSize`.
- **egg LP:** upstream `egg::LpExtractor` with `AstSize`, CBC, and a one-second
  solver limit. egg disables some cyclic alternatives before solving. Its API
  does not return the solver's optimality/timeout status; returned solutions are
  reported as feasible after verification, never as unrestricted optima.
- **Gym greedy DAG / Gym faster greedy DAG:** unmodified `greedy-dag` and
  `faster-greedy-dag` from [extraction-gym](https://github.com/egraphs-good/extraction-gym),
  pinned to `903ba0f818b50608fe20ae9e0f03c35cb27bc50a`. A benchmark-only
  adapter retains the original graph dependency and converts results to the
  common verifier.
- **Eggstract seed:** `eggstract::tree_seed`.
- **Eggstract DAG:** `eggstract::extract`, seed 0 and a one-second cooperative
  search limit. Initial seeding and final verification can exceed that limit.

All methods receive the same equivalence classes, alternatives and requested
roots. Each node costs one, including leaves. Output size is the number of unique
reachable selected nodes across the entire root set. egg tree minimizes tree
size, so a larger shared DAG does not contradict its intended objective.

The suite contains Boolean and polynomial rewriting, sizes 32, 128 and 512, and
generator seeds 0, 1, 2 and 3: 24 cases. Size is a generator parameter, not the
number of e-nodes after rewriting. All roots are retained, with duplicates
removed after canonicalization. Before rewriting, only the union of the input
roots' dependencies is inserted, so unrelated generated expressions do not
inflate the workload. Rewriting uses five iterations and a 30,000-node limit,
with no timing-based truncation. The generator and input hashes are included for reproducibility; no input is selected by its
measured performance.

Graph generation, rebuilding, serialization and report I/O are outside the
timer. Gym's lazy input class index is prepared before timing, matching egg's
already-built input graph. Upstream iteration logging is retained and redirected
to `/dev/null`.
Each timed call includes extractor construction, extraction, selection recovery
and independent feasibility verification. This includes Eggstract's native
indexing and built-in verifier, and the common verifier for egg's results.
Sharing across roots is counted once. Each method gets one warmup and five
measured repetitions; calls run sequentially with rotated method order.

Each case contributes its median measured cost and latency. Cost bars show the
geometric mean of per-case ratios to egg tree; latency points show the geometric
mean of per-case median elapsed times in milliseconds. Per-case values and
individual repetitions remain available in the archived `results.json`. The
Eggstract seed is an internal baseline, retained in the summary and raw results
but omitted from the chart to keep it compact. Failures are retained
and must be reported explicitly rather than silently excluded. The two limits
above have different boundaries; the chart reports actual elapsed time.

The benchmark crate is separate from the library: egg and CBC are not runtime
dependencies of Eggstract. Versions are pinned in its lockfile. The vendored
upstream files retain their MIT notice and are used only as competitors. Results
describe the measured machine and synthetic inputs, not a universal ranking or novelty
claim.

This is a practical CPU baseline comparison, not a comprehensive comparison
with research systems such as [e-boost](https://github.com/Yu-Maryland/e-boost)
and [SmoothE](https://github.com/cornell-zhang/SmoothE). It does not establish
state-of-the-art performance.

## Reproduce

Install CBC (`brew install cbc` on macOS; `apt install coinor-libcbc-dev` on
Debian/Ubuntu). On Homebrew macOS, set `LIBRARY_PATH="$(brew --prefix)/lib"`
if the linker cannot find CBC. From the repository root:

```sh
cargo test --locked --manifest-path benchmarks/egg/Cargo.toml
cargo build --release --locked --manifest-path benchmarks/egg/Cargo.toml
mkdir -p /tmp/eggstract-benchmark
benchmarks/egg/target/release/eggstract-egg-benchmark \
  --output /tmp/eggstract-benchmark/results.json > /dev/null
python3 benchmarks/egg/verify_results.py /tmp/eggstract-benchmark/results.json
python3 -m pip install matplotlib
python3 benchmarks/egg/plot.py --raw /tmp/eggstract-benchmark/results.json \
  --output /tmp/eggstract-benchmark/chart
```

Run without other benchmarks or builds competing for CPU time. The runner
refuses to overwrite an existing report. `--smoke` runs one case; the full grid
and repetition count are fixed in the source. It saves graph JSON sidecars,
input hashes, selected-node witnesses and errors, and checkpoints after every
attempt. The independent Python checker validates those witnesses and hashes.

Measured on an Apple M4 Max with 48 GiB RAM, macOS 26.6.2, Rust 1.97.1 and
CBC 2.10.13. All measured Eggstract DAG calls completed within 45 ms on this
suite; the one-second budget did not bind. See
[environment.json](environment.json) for the recorded environment.

## Recorded results

[Chart](results.svg), [per-case summary](results.summary.json), and
[raw results and graph witnesses](raw-results.tar.gz) (423 KiB compressed).
The archive contains `results.json` and `results-graphs/`; extract it into a
scratch directory and run `verify_results.py` on that report to repeat the
independent witness check. It checked 816 feasible outputs including warmups.

All five non-LP methods returned feasible outputs on all 24 cases (61–5,684
nodes). Against Gym faster greedy DAG, Eggstract DAG improved 18 cases, tied 6,
and lost none; its geometric mean cost ratio was 0.9292 and latency ratio 1.611.
Against egg tree, those ratios were 0.8900 and 8.355. These are extraction costs
and elapsed times, not generated-program performance.

The egg LP calls failed consistently in all six attempts on eight cases:
seven reported solver infeasibility, and `polynomial/512/2` panicked inside
upstream solution reconstruction at `lp_extract.rs:286`. The raw results retain
all 40 failed measurements and eight failed warmups. The original graphs are
feasible, as the independently checked outputs demonstrate. The failures occur
inside the upstream LP call before our output conversion. Their precise cause
has not been established; neither a timeout nor a restricted-model failure is
proof that the original extraction problem is infeasible. No LP aggregate is
computed on the smaller surviving subset. The plotting command returns status
1 after producing its chart and summary when failures are present.
