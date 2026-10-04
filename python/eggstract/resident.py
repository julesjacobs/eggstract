"""Exact small-instance search in the unit-slot, arbitrary-overwrite model.

Costs are charged per execution. Operations are assumed pure, total, and
semantically equivalent within each result class. This is a different objective
from sum-once DAG extraction. See docs/resident.md for the model and provenance.
"""

from dataclasses import dataclass
import heapq
import math
import time
from typing import Iterable, Literal


def _integer(value: object, name: str, minimum: int = 0) -> None:
    if type(value) is not int or value < minimum:
        raise ValueError(f"{name} must be an integer >= {minimum}")


def _names(values: Iterable[str], name: str) -> tuple[str, ...]:
    if isinstance(values, str):
        raise ValueError(f"{name} must be a collection of class names")
    values = tuple(values)
    if any(type(value) is not str for value in values):
        raise ValueError(f"{name} must contain strings")
    return values


@dataclass(frozen=True)
class Operation:
    result: str
    arguments: tuple[str, ...]
    cost: int

    def __post_init__(self) -> None:
        if type(self.result) is not str:
            raise ValueError("result must be a class name")
        object.__setattr__(self, "arguments", _names(self.arguments, "arguments"))
        _integer(self.cost, "cost")


@dataclass(frozen=True)
class Graph:
    classes: tuple[str, ...]
    operations: tuple[Operation, ...]

    def __post_init__(self) -> None:
        classes = _names(self.classes, "classes")
        operations = tuple(self.operations)
        if len(set(classes)) != len(classes):
            raise ValueError("duplicate class names")
        universe = set(classes)
        for operation in operations:
            if not isinstance(operation, Operation):
                raise ValueError("operations must contain Operation objects")
            if operation.result not in universe or not set(operation.arguments) <= universe:
                raise ValueError("operation refers to an undeclared class")
        object.__setattr__(self, "classes", classes)
        object.__setattr__(self, "operations", operations)


@dataclass(frozen=True)
class Problem:
    graph: Graph
    initial: frozenset[str]
    required: frozenset[str]
    capacity: int

    def __post_init__(self) -> None:
        if not isinstance(self.graph, Graph):
            raise ValueError("graph must be a Graph")
        _integer(self.capacity, "capacity")
        for field in ("initial", "required"):
            values = frozenset(_names(getattr(self, field), field))
            if not values <= set(self.graph.classes):
                raise ValueError(f"{field} contains an undeclared class")
            object.__setattr__(self, field, values)
        if len(self.initial) > self.capacity:
            raise ValueError("initial residents exceed capacity")


@dataclass(frozen=True)
class Limits:
    max_states: int = 100_000
    timeout_seconds: float | None = None

    def __post_init__(self) -> None:
        _integer(self.max_states, "max_states", 1)
        seconds = self.timeout_seconds
        if seconds is not None:
            try:
                valid = type(seconds) in (int, float) and math.isfinite(seconds) and seconds >= 0
            except OverflowError:
                valid = False
            if not valid:
                raise ValueError("timeout_seconds must be finite and nonnegative")


@dataclass(frozen=True)
class Step:
    operation: int | None
    residents: frozenset[str]

    def __post_init__(self) -> None:
        if self.operation is not None:
            _integer(self.operation, "operation")
        object.__setattr__(self, "residents", frozenset(_names(self.residents, "residents")))


Trace = tuple[Step, ...]


@dataclass(frozen=True)
class SearchResult:
    status: Literal["optimal", "infeasible", "unknown"]
    cost: int | None
    trace: Trace | None
    explored_states: int
    reason: str | None = None


@dataclass(frozen=True)
class Recipe:
    evicted_operand: str | None
    trace: Trace

    def __post_init__(self) -> None:
        if self.evicted_operand is not None and type(self.evicted_operand) is not str:
            raise ValueError("evicted_operand must be a class name or None")
        object.__setattr__(self, "trace", tuple(self.trace))


@dataclass(frozen=True)
class DeletionCertificate:
    operation: int
    recipes: tuple[Recipe, ...]

    def __post_init__(self) -> None:
        _integer(self.operation, "operation")
        object.__setattr__(self, "recipes", tuple(self.recipes))


@dataclass(frozen=True)
class DeletionResult:
    status: Literal["safe", "necessary", "unknown"]
    certificate: DeletionCertificate | None
    failed_contexts: tuple[str | None, ...]
    explored_states: int
    reason: str | None = None


@dataclass(frozen=True)
class PruneResult:
    active: tuple[int, ...]
    certificates: tuple[DeletionCertificate, ...]
    unknown_operations: tuple[int, ...]


def _active(graph: Graph, active: Iterable[int] | None) -> tuple[int, ...]:
    values = tuple(range(len(graph.operations))) if active is None else tuple(active)
    for operation in values:
        _integer(operation, "operation")
        if operation >= len(graph.operations):
            raise ValueError("operation index out of range")
    if len(set(values)) != len(values):
        raise ValueError("duplicate operation indices")
    return tuple(sorted(values))


def check_trace(problem: Problem, trace: Iterable[Step], *, active: Iterable[int] | None = None) -> int:
    """Check feasibility and recompute cost, independently of the search engine."""
    allowed = set(_active(problem.graph, active))
    live = set(problem.initial)
    price = 0
    universe = set(problem.graph.classes)
    for step in trace:
        if not isinstance(step, Step):
            raise ValueError("trace must contain Step objects")
        after = set(step.residents)
        if not after <= universe or len(after) > problem.capacity:
            raise ValueError("invalid resident state")
        if step.operation is None:
            if not after <= live:
                raise ValueError("discard introduces a value")
        else:
            if step.operation not in allowed:
                raise ValueError("unavailable operation")
            operation = problem.graph.operations[step.operation]
            if not set(operation.arguments) <= live:
                raise ValueError("missing operand")
            if operation.result not in after or not after <= live | {operation.result}:
                raise ValueError("invalid operation post-state")
            price += operation.cost
        live = after
    if not problem.required <= live:
        raise ValueError("required values are missing")
    return price


def _mask(values: Iterable[str], bits: dict[str, int]) -> int:
    return sum(bits[value] for value in set(values))


def _search(graph: Graph, initial: int, goals: tuple[int, ...], capacity: int,
            active: tuple[int, ...], limits: Limits, budget: int | None = None):
    bits = {name: 1 << i for i, name in enumerate(graph.classes)}
    operations = [(i, bits[graph.operations[i].result],
                   _mask(graph.operations[i].arguments, bits), graph.operations[i].cost)
                  for i in active]
    distances = {initial: 0}
    previous = {}
    pending = [(0, initial)]
    found = {}
    remaining = set(goals)
    explored = 0
    deadline = None if limits.timeout_seconds is None else time.monotonic() + limits.timeout_seconds
    reason = None
    while pending and remaining:
        if deadline is not None and time.monotonic() >= deadline:
            reason = "time_limit"
            break
        price, state = heapq.heappop(pending)
        if distances[state] != price:
            continue
        explored += 1
        for goal in tuple(remaining):
            if goal & ~state == 0:
                found[goal] = state
                remaining.remove(goal)
        if not remaining:
            break

        def transitions():
            residents = state
            while residents:
                value = residents & -residents
                residents -= value
                yield state ^ value, 0, None
            for index, head, args, cost in operations:
                if args & ~state or head & state:
                    continue
                enlarged = state | head
                if enlarged.bit_count() <= capacity:
                    yield enlarged, cost, index
                else:
                    residents = state
                    while residents:
                        value = residents & -residents
                        residents -= value
                        yield enlarged ^ value, cost, index

        for after, extra, operation in transitions():
            if deadline is not None and time.monotonic() >= deadline:
                reason = "time_limit"
                break
            candidate = price + extra
            if budget is not None and candidate > budget:
                continue
            if candidate >= distances.get(after, math.inf):
                continue
            if after not in distances and len(distances) >= limits.max_states:
                reason = "state_limit"
                break
            distances[after] = candidate
            previous[after] = state, operation
            heapq.heappush(pending, (candidate, after))
        if reason is not None:
            break
    return distances, previous, found, explored, reason


def _trace(graph: Graph, previous: dict, initial: int, final: int) -> Trace:
    steps = []
    while final != initial:
        before, operation = previous[final]
        residents = frozenset(c for i, c in enumerate(graph.classes) if final & (1 << i))
        steps.append(Step(operation, residents))
        final = before
    return tuple(reversed(steps))


def solve(problem: Problem, *, active: Iterable[int] | None = None,
          limits: Limits = Limits()) -> SearchResult:
    """Return a solver-reported optimum, infeasibility, or a resource interruption.

    A returned trace is independently checked for feasibility and cost. No
    independent optimality certificate is provided by this experimental module.
    """
    allowed = _active(problem.graph, active)
    if problem.required <= problem.initial:
        return SearchResult("optimal", 0, (), 0)
    if len(problem.required) > problem.capacity:
        return SearchResult("infeasible", None, None, 0)
    bits = {name: 1 << i for i, name in enumerate(problem.graph.classes)}
    initial, goal = _mask(problem.initial, bits), _mask(problem.required, bits)
    distances, previous, found, explored, reason = _search(
        problem.graph, initial, (goal,), problem.capacity, allowed, limits)
    if goal in found:
        trace = _trace(problem.graph, previous, initial, found[goal])
        cost = distances[found[goal]]
        if check_trace(problem, trace, active=allowed) != cost:
            raise RuntimeError("search trace cost mismatch")
        return SearchResult("optimal", cost, trace, explored)
    return SearchResult("unknown" if reason else "infeasible", None, None, explored, reason)


def deletion_test(graph: Graph, operation: int, *, active: Iterable[int] | None = None,
                  limits: Limits = Limits()) -> DeletionResult:
    """Test preservation of every valid initial/required/capacity query.

    'necessary' supplies a failed local context, relative to the active graph.
    It does not mean the operation is needed for one particular root query.
    """
    allowed = _active(graph, active)
    _integer(operation, "operation")
    if operation not in allowed:
        raise ValueError("candidate operation is not active")
    candidate = graph.operations[operation]
    arguments = set(candidate.arguments)
    if candidate.result in arguments:
        return DeletionResult("safe", DeletionCertificate(operation, ()), (), 0)
    evictions = tuple(sorted(arguments)) if arguments else (None,)
    bits = {name: 1 << i for i, name in enumerate(graph.classes)}
    initial = _mask(arguments, bits)
    goals = tuple(_mask((arguments - {d}) | {candidate.result}, bits) for d in evictions)
    distances, previous, found, explored, reason = _search(
        graph, initial, goals, max(1, len(arguments)),
        tuple(i for i in allowed if i != operation), limits, candidate.cost)
    failed = tuple(d for d, goal in zip(evictions, goals) if goal not in found)
    if failed:
        return DeletionResult("unknown" if reason else "necessary", None,
                              () if reason else failed, explored, reason)
    recipes = tuple(Recipe(d, _trace(graph, previous, initial, found[goal]))
                    for d, goal in zip(evictions, goals))
    certificate = DeletionCertificate(operation, recipes)
    check_deletion(graph, certificate, active=allowed)
    return DeletionResult("safe", certificate, (), explored)


def check_deletion(graph: Graph, certificate: DeletionCertificate, *,
                   active: Iterable[int] | None = None) -> None:
    """Replay every required replacement recipe without invoking search."""
    allowed = _active(graph, active)
    if not isinstance(certificate, DeletionCertificate):
        raise ValueError("invalid deletion certificate")
    _integer(certificate.operation, "operation")
    if certificate.operation not in allowed:
        raise ValueError("candidate operation is not active")
    candidate = graph.operations[certificate.operation]
    arguments = set(candidate.arguments)
    if candidate.result in arguments:
        if certificate.recipes:
            raise ValueError("an already-resident result needs no recipes")
        return
    expected = arguments or {None}
    seen = set()
    remaining = tuple(i for i in allowed if i != certificate.operation)
    for recipe in certificate.recipes:
        if not isinstance(recipe, Recipe):
            raise ValueError("invalid replacement recipe")
        evicted = recipe.evicted_operand
        if evicted not in expected or evicted in seen:
            raise ValueError("invalid or duplicate retention case")
        seen.add(evicted)
        problem = Problem(graph, frozenset(arguments),
                          frozenset((arguments - {evicted}) | {candidate.result}),
                          max(1, len(arguments)))
        if check_trace(problem, recipe.trace, active=remaining) > candidate.cost:
            raise ValueError("replacement exceeds candidate cost")
    if seen != expected:
        raise ValueError("missing retention case")


def check_deletion_sequence(graph: Graph, certificates: Iterable[DeletionCertificate], *,
                            active: Iterable[int] | None = None) -> tuple[int, ...]:
    """Check certificates in order; recipes may not use earlier deletions."""
    remaining = set(_active(graph, active))
    for certificate in certificates:
        check_deletion(graph, certificate, active=remaining)
        remaining.remove(certificate.operation)
    return tuple(sorted(remaining))


def prune(graph: Graph, *, active: Iterable[int] | None = None,
          candidates: Iterable[int] | None = None, limits: Limits = Limits()) -> PruneResult:
    """Try each candidate once in input order, with limits applied per candidate."""
    remaining = set(_active(graph, active))
    order = tuple(sorted(remaining)) if candidates is None else tuple(candidates)
    _active(graph, order)
    if not set(order) <= remaining:
        raise ValueError("candidate operation is not active")
    certificates = []
    unknown = []
    for operation in order:
        result = deletion_test(graph, operation, active=remaining, limits=limits)
        if result.status == "safe":
            check_deletion(graph, result.certificate, active=remaining)
            certificates.append(result.certificate)
            remaining.remove(operation)
        elif result.status == "unknown":
            unknown.append(operation)
    return PruneResult(tuple(sorted(remaining)), tuple(certificates), tuple(unknown))
