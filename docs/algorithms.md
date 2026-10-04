# Algorithms

## Shared DAG extraction

An e-node has an owner class, ordered child classes and a nonnegative cost.
The caller requests one or more root classes. A solution chooses one e-node per
reachable class, includes its dependencies, and has no selected dependency cycle.
Its DAG cost is the sum of reachable selected e-node costs, each counted once.
Repeated roots and repeated operands do not add DAG charges.

The graph may contain cycles. Unsupported cycles do not produce values. An
e-graph's equalities are trusted input; this library does not prove them.

### Dense tree seed

A Dijkstra-style hypergraph worklist finalizes a class after all operands of a
candidate have been finalized. Scores add child tree costs with multiplicity.
Finalization order makes the selected dependency graph acyclic, including when
costs are zero. Parent adjacency is stored in dense arrays. The returned cost is
recomputed as DAG cost; a minimum tree-cost selection need not minimize DAG cost.

### DAG search

The native heuristic retains the fixed direct-search configuration from the
research implementation:

1. Build the tree seed and its rooted reference counts.
2. Reconstruct candidates using demand estimates, with up to 32 rounds and four
   local descent sweeps per round.
3. Evaluate replacements by charging newly activated dependencies and crediting
   dependencies whose last reference disappears. Reject cycles before committing.
4. Prune equivalent alternatives, use SCCs to restrict cycle checks, and attempt
   coordinated support opening and retirement.
5. Run dependency repair when earlier phases improved the seed.

Only improving candidates replace the best solution. Exact path bounds can stop
search when they match a candidate; the public native result conservatively
reports feasibility. Search has fixed limits on some replacement neighborhoods,
so stopping does not establish a local or global optimum. The time limit is
cooperative and includes search setup; mandatory seeding and final verification
can exceed it. There is no platform-specific instruction counter or environment
variable that changes the algorithm.

Costs are `f64`. The seed saturates overflowing tree scores; DAG cost must remain
finite. Summation and heuristic comparisons use floating-point arithmetic.
Use the integer CP-SAT backend for solver-reported exact optimization within its
documented numerical domain.

### Independent verification

The verifier takes the original graph, explicit requested roots, and selected
node IDs. It independently checks one representative per class, complete root
coverage, dependencies, acyclicity and absence of unreachable selections, then
recomputes DAG cost. A solution's own root declarations or reported cost never
replace the trusted request. Traversal is iterative to support deep DAGs.

### SCC CP-SAT

The optional Python model selects one node for each active class and activates
all selected dependencies. Integer ranks decrease along selected dependencies
within each strongly connected component. Every cycle lies inside one SCC, so
these rank constraints enforce acyclicity without ranking unrelated components.
The objective sums selected node costs. Hints are checked before use. See
[exact.md](exact.md) for bounds, interruption and numeric contracts.

## Memory-bounded extraction

The [resident module](resident.md) optimizes a different object: an execution
trace, potentially using different representatives of the same class at
different times. Execution costs are charged repeatedly. Its contextual deletion
criterion cannot be substituted for sum-once DAG preprocessing.

## Scope

The package retains the algorithms with useful measured behavior or independently
checked mathematical substance. Benchmark orchestration, dataset downloads,
visualization, experiment registries, unsuccessful heuristic variants, and
research logs are excluded. More elaborate preprocessing and alternative global
solvers can be evaluated separately before becoming supported API.
