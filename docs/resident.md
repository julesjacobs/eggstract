# Experimental bounded-resident extraction

`eggstract.resident` is a dependency-free Python reference implementation for
small instances. It minimizes execution cost while limiting the number of
resident values. Ordinary DAG extraction in the Rust library uses a separate
objective: each selected node is charged once. Resident extraction charges
every execution and may use different producers of a class at different times.

```python
from eggstract.resident import Graph, Limits, Operation, Problem, check_trace, solve

graph = Graph(
    classes=("input", "temporary", "result", "outside"),
    operations=(
        Operation("temporary", ("input",), 1),
        Operation("result", ("temporary",), 1),
        Operation("result", ("input",), 3),
    ),
)
problem = Problem(
    graph,
    initial=frozenset({"input", "outside"}),
    required=frozenset({"result", "outside"}),
    capacity=2,
)
result = solve(problem, limits=Limits(max_states=10_000, timeout_seconds=1.0))
if result.status == "optimal":
    assert result.cost == check_trace(problem, result.trace) == 2
```

The class universe is explicit and remains fixed when operations are disabled.
It may contain external values with no producer. Initial residents are supplied
by the caller; they cannot be reloaded unless an operation supplies them.
Operations and classes are immutable. Operation indices are positions in the
original `Graph.operations` tuple and remain stable during pruning. Argument
order and repeated arguments are preserved; repeated arguments occupy one slot.

## Execution contract

Operations are pure, deterministic, total, and have nonnegative integer costs.
The caller is responsible for the semantic equivalence of all producers of a
class. Every value occupies one slot. All arguments must be resident before an
operation executes. Its result can overwrite any resident value, including an
operand; no extra transient result slot is needed. Free discarding is allowed.
There are no spills, traps, effects, register classes, fixed operand aliases,
variable-width values, scratch registers, or overlapping execution costs.

A `Step(operation, residents)` gives an operation index and the complete
resident set afterward. `operation=None` denotes a free discard. The checker
verifies operands, capacity, legal post-states, active operations, final required
values, and total execution cost. It accepts arbitrary valid initial states.

`solve(problem, active=None, limits=Limits())` uses forward Dijkstra search.
`active` optionally restricts the original operation indices. Search generates
free discards and executions with maximal retention. Subsequent free discards
give every smaller legal post-state. Executions whose result is already present
can be replaced by free discards because costs are nonnegative.

Search returns one of:

- `optimal`: a solver-reported optimum, with a checked trace and exact cost;
- `infeasible`: the finite state space was exhausted, or the required set exceeds
  capacity;
- `unknown`: a state or time limit interrupted search, with a reason and no
  claimed cost or trace.

An already-satisfied query returns cost zero without search. An initial set
larger than capacity is invalid input. `check_trace` independently establishes
feasibility and cost; it does **not** establish optimality. This module does not
emit independent optimality certificates.

The default limit is 100,000 discovered states. `timeout_seconds` is an optional
cooperatively checked search deadline; input validation and graph preparation
are outside that deadline. Limits are not operating-system memory or process
limits. The unconstrained state count is
`sum(comb(number_of_classes, i), i=0..capacity)`, capped at the class count. Use
small graphs and capacities. Python integers provide exact costs without a
fixed-width overflow boundary.

## Certified contextual deletion

For an operation with distinct arguments A, result c outside A, and cost w,
deletion preserves every valid initial/required/capacity query precisely when
the graph without that operation can produce `(A - {d}) | {c}` from A at cost
at most w, using `len(A)` slots, for every d in A. All queries share one bounded
search. A nullary operation uses the query from empty to `{c}` with one slot.
An operation whose result is already an argument is always dispensable.

```python
from eggstract.resident import check_deletion_sequence, deletion_test, prune

decision = deletion_test(graph, 2)
assert decision.status == "safe"

reduced = prune(graph, candidates=(2,))
assert reduced.active == (0, 1)
assert check_deletion_sequence(graph, reduced.certificates) == reduced.active
```

`deletion_test` returns `safe` with a `DeletionCertificate`, `necessary` with
the failed local retention cases, or `unknown` after interruption. `necessary`
means that some context needs the operation; it need not be used by a particular
root query. A `None` failed retention case denotes a nullary operation.

A certificate contains a `Recipe(evicted_operand, trace)` for each required
case. `check_deletion(graph, certificate, active=...)` replays these recipes,
rejects use of the candidate or unavailable operations, and checks cost and
capacity. It does not run a search. For already-resident results the certificate
has no recipes.

`prune` checks candidates once in the supplied order, deleting only operations
whose recipes pass the independent checker. It returns active indices, the
ordered certificates, and operations left undecided by resource limits. Limits
apply **per candidate**. `check_deletion_sequence` verifies the whole sequence
against the original graph, rejecting dependencies on earlier deletions. A
recipe may use an operation deleted later. Independently safe deletions must
not be applied as an unchecked parallel batch: duplicate producers can each
justify deleting the other.

## Proof, tests, and provenance

The deletion proof uses a frame argument. A recipe can preserve an outside set
P by unioning P into every resident state. If the original execution drops an
argument d, frame its corresponding recipe with the retained outside values;
the original pre-state provides enough slots. If it retains every argument,
also frame d; the original post-state provides the extra slot. Each replacement
restores the required post-state without increasing cost or capacity. Necessity
follows by using each tight-capacity query as a surrounding context. Sequential
deletions compose these replacements.

This implementation was written from the operational model and theorem audited
in the original Eggstract research on 2026-10-04. The supplied theorem document
had SHA256 `c60dba3be5997ff9406c3a49f7b968cc4aa84d105cd3124801084e000c6bc10d`.
That audit found no counterexample in 6,175 deletion decisions and 4,157,020
context comparisons. It also independently checked 30 separation cases and 36
spill schedules. Those are historical audit results, not counts from this
package's tests or a mechanized proof.

The tests here independently construct complete state graphs, including all
legal post-states and redundant-result executions. Floyd–Warshall provides
all-pairs costs to compare with search and the deletion criterion. Tests also
cover external entry values, zero costs, repeated arguments, certificate
mutations, sequential deletion, and resource interruptions.

Reverse rematerialization is prior work: Bahi and Eisenbeis,
[Register Reverse Rematerialization](https://inria.hal.science/inria-00607323/document)
(2011). Universal cost-preserving sequence replacement has precedent in Holte,
Alkhazraji, and Wehrle,
[A Generalization of Sleep Sets Based on Operator Sequence Redundancy](https://doi.org/10.1609/aaai.v29i1.9641)
(2015). Historical novelty of the precise arity-local deletion criterion is
unresolved. Neither practical generated-code benefits nor machine-level speedups
are established by this reference implementation.
