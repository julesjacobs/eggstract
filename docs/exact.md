# Optional exact extraction

The Python backend uses Google OR-Tools CP-SAT to minimize shared DAG cost. It
selects one node per active e-class and uses topological ranks inside each SCC
of the possible dependency graph to exclude cycles. It does not prune alternatives.

```python
from eggstract.exact import solve

result = solve(graph, ["root-class"], time_limit=10.0, workers=1)
print(result.solver_status, result.cost, result.lower_bound)
```

Install the optional `ortools` package to run the solver. Importing this module
and calling `verify` require only Python's standard library.

The input is an egraph-serialize JSON object. Node `children` contain **node IDs**;
their e-classes determine the dependencies. Explicit, nonempty roots are required;
the graph's `root_eclasses` field does not override them. Costs must be finite
nonnegative integers, and the sum of all serialized node costs must be at most
`2**53 - 1`. Integer-valued JSON floats are accepted. Missing costs default to one.
There is no rounding or scaling. Negative, fractional, malformed, or out-of-range
inputs raise `ValueError`.

`feasible` means the returned selection passed an independent check for requested
roots, exactly one node per active class, complete dependencies, acyclicity, and
absence of unrooted nodes. `cost` is its exact integer cost, counting shared nodes
once. `verify(graph, roots, selected_enodes)` performs that check without a solver
and returns the cost; it does not certify optimality.

`solver_status` and `lower_bound` are CP-SAT reports. An `optimal` status reports
solver optimality for this integer model; no independently replayable optimality
certificate is produced. An `unknown` status can accompany a feasible retained
hint. Missing incumbents have `feasible=False` and `cost=None`. `source` is
`"solver"`, `"hint"`, or `None`.

Pass `hint` as a list of selected node IDs or a dictionary with `selected_enodes`.
If the dictionary declares `root_eclasses`, they must match the requested roots.
Hints are independently checked; their reported cost and status are ignored.
A valid hint supplies a cost upper bound and remains available if search times out.

`time_limit` is the solver-search allowance in seconds; graph validation and model
construction occur before it. Zero skips search and returns the verified hint, if
provided. `workers` is a positive int32 controlling solver parallelism; multiple workers can change the
search trajectory and CPU consumption.

```sh
python -m eggstract.exact graph.json --root root-class --time-limit 10
python -m eggstract.exact graph.json --root root-class --hint seed.json
```

The CLI prints one JSON result. Invalid input or a missing solver dependency
produces a JSON error and exit code 2. Add `--root` for each requested output.
The backend does not invoke the Rust CLI or depend on its runtime.

The formulation is adapted from this project's earlier native CP-SAT driver.
OR-Tools is the solver; this backend is not a reproduction of e-boost's pipeline.
