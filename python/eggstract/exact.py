"""Optional CP-SAT extraction for nonnegative integer DAG costs."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import json
import math
from pathlib import Path
from typing import Any, Sequence


MAX_TOTAL_COST = 2**53 - 1


@dataclass(frozen=True)
class Result:
    solver_status: str
    feasible: bool
    selected_enodes: tuple[str, ...]
    root_eclasses: tuple[str, ...]
    cost: int | None
    lower_bound: float | None
    source: str | None


@dataclass
class _Graph:
    classes: dict[str, list[str]]
    owner: dict[str, str]
    children: dict[str, set[str]]
    costs: dict[str, int]
    roots: tuple[str, ...]


def _ids(value: Any, label: str) -> list[str]:
    if not isinstance(value, (list, tuple)) or any(not isinstance(x, str) for x in value):
        raise ValueError(f"{label} must be a list of string IDs")
    return list(value)


def _graph(data: dict[str, Any], roots: Sequence[str]) -> _Graph:
    requested = tuple(dict.fromkeys(_ids(roots, "roots")))
    if not requested:
        raise ValueError("explicit roots must be nonempty")
    nodes = data.get("nodes") if isinstance(data, dict) else None
    if not isinstance(nodes, dict):
        raise ValueError("graph must contain a nodes object")
    classes: dict[str, list[str]] = {}
    owner, costs, references = {}, {}, {}
    total = 0
    for node_id, node in nodes.items():
        if not isinstance(node_id, str) or not isinstance(node, dict):
            raise ValueError("nodes must map string IDs to objects")
        cls = node.get("eclass")
        if not isinstance(cls, str):
            raise ValueError(f"node {node_id!r} must have a string eclass")
        cost = node.get("cost", 1)
        if isinstance(cost, bool) or not isinstance(cost, (int, float)):
            raise ValueError(f"node {node_id!r} cost must be a nonnegative integer")
        if isinstance(cost, float) and (not math.isfinite(cost) or not cost.is_integer()):
            raise ValueError(f"node {node_id!r} cost must be finite and integral")
        if cost < 0 or cost > MAX_TOTAL_COST:
            raise ValueError(f"node {node_id!r} cost is outside the supported range")
        total += int(cost)
        if total > MAX_TOTAL_COST:
            raise ValueError(f"sum of all node costs must be <= {MAX_TOTAL_COST}")
        owner[node_id], costs[node_id] = cls, int(cost)
        references[node_id] = _ids(node.get("children", []), f"children of {node_id!r}")
        classes.setdefault(cls, []).append(node_id)
    children = {}
    for node, refs in references.items():
        if any(child not in owner for child in refs):
            raise ValueError(f"node {node!r} references a missing child node")
        children[node] = {owner[child] for child in refs}
    if any(root not in classes for root in requested):
        raise ValueError("a requested root e-class does not exist")
    return _Graph(classes, owner, children, costs, requested)


def _verify(graph: _Graph, selected: Sequence[str]) -> int:
    selected = _ids(selected, "selected_enodes")
    if len(set(selected)) != len(selected):
        raise ValueError("selection contains duplicate node IDs")
    chosen = {}
    for node in selected:
        if node not in graph.owner:
            raise ValueError(f"selected node {node!r} does not exist")
        cls = graph.owner[node]
        if cls in chosen:
            raise ValueError(f"selection has multiple nodes in e-class {cls!r}")
        chosen[cls] = node
    reachable, pending = set(), list(graph.roots)
    while pending:
        cls = pending.pop()
        if cls in reachable:
            continue
        if cls not in chosen:
            raise ValueError(f"selection omits required e-class {cls!r}")
        reachable.add(cls)
        pending.extend(graph.children[chosen[cls]])
    if reachable != chosen.keys():
        raise ValueError("selection contains nodes unreachable from the requested roots")
    incoming = dict.fromkeys(chosen, 0)
    for node in chosen.values():
        for child in graph.children[node]:
            incoming[child] += 1
    ready = [cls for cls, degree in incoming.items() if degree == 0]
    count = 0
    while ready:
        cls = ready.pop()
        count += 1
        for child in graph.children[chosen[cls]]:
            incoming[child] -= 1
            if incoming[child] == 0:
                ready.append(child)
    if count != len(chosen):
        raise ValueError("selection contains a dependency cycle")
    return sum(graph.costs[node] for node in selected)


def verify(graph: dict[str, Any], roots: Sequence[str], selected_enodes: Sequence[str]) -> int:
    """Check rooted DAG feasibility and return exact cost; does not prove optimality."""
    return _verify(_graph(graph, roots), selected_enodes)


def _components(adjacency: dict[str, set[str]]) -> tuple[dict[str, int], list[int]]:
    reverse = {v: [] for v in adjacency}
    for v, children in adjacency.items():
        for child in children:
            reverse[child].append(v)
    seen, order = set(), []
    for root in adjacency:
        if root in seen:
            continue
        seen.add(root)
        stack = [(root, iter(sorted(adjacency[root])))]
        while stack:
            v, children = stack[-1]
            child = next(children, None)
            if child is None:
                order.append(v)
                stack.pop()
            elif child not in seen:
                seen.add(child)
                stack.append((child, iter(sorted(adjacency[child]))))
    component, sizes = {}, []
    for root in reversed(order):
        if root in component:
            continue
        component[root] = len(sizes)
        stack, size = [root], 0
        while stack:
            v = stack.pop()
            size += 1
            for child in reverse[v]:
                if child not in component:
                    component[child] = len(sizes)
                    stack.append(child)
        sizes.append(size)
    return component, sizes


def solve(
    graph: dict[str, Any], roots: Sequence[str], *, time_limit: float = 10.0,
    workers: int = 1, hint: dict[str, Any] | Sequence[str] | None = None,
) -> Result:
    """Solve with optional OR-Tools; time_limit bounds solver search in seconds.

    All returned incumbents are independently verified. Bounds and solver_status
    are solver reports. A verified hint survives an interrupted search.
    """
    if isinstance(time_limit, bool) or not isinstance(time_limit, (int, float)):
        raise ValueError("time_limit must be a finite nonnegative number")
    try:
        time_limit = float(time_limit)
    except OverflowError as error:
        raise ValueError("time_limit must be a finite nonnegative number") from error
    if not math.isfinite(time_limit) or time_limit < 0:
        raise ValueError("time_limit must be a finite nonnegative number")
    if isinstance(workers, bool) or not isinstance(workers, int) or not 1 <= workers <= 2**31 - 1:
        raise ValueError("workers must be an integer in [1, 2**31 - 1]")
    data = _graph(graph, roots)
    selected: tuple[str, ...] = ()
    cost, source = None, None
    if hint is not None:
        if isinstance(hint, dict):
            if "root_eclasses" in hint and set(_ids(hint["root_eclasses"], "hint roots")) != set(data.roots):
                raise ValueError("hint roots differ from requested roots")
            hint = hint.get("selected_enodes")
        cost = _verify(data, hint)
        selected, source = tuple(sorted(hint)), "hint"
    if time_limit == 0:
        return Result("unknown", cost is not None, selected, data.roots, cost, None, source)
    try:
        from ortools.sat.python import cp_model
    except ImportError as error:
        raise RuntimeError("exact extraction requires the optional ortools package") from error

    model = cp_model.CpModel()
    x = {node: model.new_bool_var(f"node_{i}") for i, node in enumerate(data.owner)}
    y = {cls: model.new_bool_var(f"class_{i}") for i, cls in enumerate(data.classes)}
    adjacency = {cls: set() for cls in data.classes}
    for node, children in data.children.items():
        adjacency[data.owner[node]].update(children)
    component, sizes = _components(adjacency)
    ranks = {
        cls: model.new_int_var(0, sizes[component[cls]] - 1, f"rank_{i}")
        for i, cls in enumerate(data.classes)
    }
    parents = {cls: [] for cls in data.classes}
    for cls, nodes in data.classes.items():
        model.add(sum(x[node] for node in nodes) == y[cls])
    for root in data.roots:
        model.add(y[root] == 1)
    for node, children in data.children.items():
        cls = data.owner[node]
        for child in sorted(children):
            model.add(y[child] >= x[node])
            parents[child].append(x[node])
            if child == cls:
                model.add(x[node] == 0)
            elif component[child] == component[cls]:
                model.add(ranks[child] >= ranks[cls] + 1).only_enforce_if(x[node])
    for cls in data.classes:
        if cls not in data.roots:
            model.add(y[cls] <= sum(parents[cls]))
    objective = sum(data.costs[node] * var for node, var in x.items())
    model.minimize(objective)
    if cost is not None:
        model.add(objective <= cost)
        chosen = set(selected)
        active = {data.owner[node] for node in selected}
        for node, var in x.items():
            model.add_hint(var, int(node in chosen))
        for cls, var in y.items():
            model.add_hint(var, int(cls in active))
        hint_edges = {data.owner[node]: data.children[node] for node in selected}
        incoming = dict.fromkeys(active, 0)
        for children in hint_edges.values():
            for child in children:
                incoming[child] += 1
        ready = sorted(cls for cls, degree in incoming.items() if degree == 0)
        next_rank = dict.fromkeys(range(len(sizes)), 0)
        hint_ranks = dict.fromkeys(data.classes, 0)
        while ready:
            cls = ready.pop()
            hint_ranks[cls] = next_rank[component[cls]]
            next_rank[component[cls]] += 1
            for child in sorted(hint_edges[cls]):
                incoming[child] -= 1
                if incoming[child] == 0:
                    ready.append(child)
        for cls, var in ranks.items():
            model.add_hint(var, hint_ranks[cls])
    solver = cp_model.CpSolver()
    solver.parameters.max_time_in_seconds = float(time_limit)
    solver.parameters.num_search_workers = workers
    status = solver.solve(model)
    name = solver.status_name(status).lower()
    if status == cp_model.MODEL_INVALID:
        raise RuntimeError(f"CP-SAT rejected the model: {solver.solution_info()}")
    if status == cp_model.INFEASIBLE and cost is not None:
        raise RuntimeError("CP-SAT reported infeasibility despite a verified hint")
    bound = None
    if status in (cp_model.OPTIMAL, cp_model.FEASIBLE, cp_model.UNKNOWN):
        value = solver.best_objective_bound
        if math.isfinite(value):
            bound = value
    if status in (cp_model.OPTIMAL, cp_model.FEASIBLE):
        candidate = tuple(sorted(node for node, var in x.items() if solver.boolean_value(var)))
        candidate_cost = _verify(data, candidate)
        if candidate_cost != solver.objective_value:
            raise RuntimeError("verified cost differs from CP-SAT's reported objective")
        if cost is not None and candidate_cost > cost:
            raise RuntimeError("CP-SAT returned a solution worse than its hint bound")
        selected, cost, source = candidate, candidate_cost, "solver"
    return Result(name, cost is not None, selected, data.roots, cost, bound, source)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--root", action="append", required=True)
    parser.add_argument("--time-limit", type=float, default=10.0)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--hint", type=Path)
    args = parser.parse_args(argv)
    try:
        graph = json.loads(args.input.read_text())
        hint = json.loads(args.hint.read_text()) if args.hint else None
        result = solve(graph, args.root, time_limit=args.time_limit, workers=args.workers, hint=hint)
    except (OSError, ValueError, RuntimeError) as error:
        print(json.dumps({"error": str(error)}))
        return 2
    print(json.dumps(asdict(result), allow_nan=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
