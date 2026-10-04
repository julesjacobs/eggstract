import dataclasses
import itertools
import math
import random
import unittest

from eggstract.resident import (
    DeletionCertificate, Graph, Limits, Operation, Problem, Recipe, Step,
    check_deletion, check_deletion_sequence, check_trace, deletion_test, prune, solve,
)


def all_states(classes, capacity):
    return [frozenset(values) for size in range(min(len(classes), capacity) + 1)
            for values in itertools.combinations(classes, size)]


def oracle(graph, capacity, active=None):
    states = all_states(graph.classes, capacity)
    positions = {state: i for i, state in enumerate(states)}
    distances = [[math.inf] * len(states) for _ in states]
    operations = graph.operations if active is None else [graph.operations[i] for i in active]
    for i, before in enumerate(states):
        for j, after in enumerate(states):
            if after <= before:
                distances[i][j] = 0
            for operation in operations:
                if (set(operation.arguments) <= before and operation.result in after
                        and after <= before | {operation.result}):
                    distances[i][j] = min(distances[i][j], operation.cost)
    for k in range(len(states)):
        for i in range(len(states)):
            for j in range(len(states)):
                distances[i][j] = min(distances[i][j], distances[i][k] + distances[k][j])
    return {(before, after): distances[positions[before]][positions[after]]
            for before in states for after in states}


def binary_example():
    return Graph(('x', 'y', 'z', 'c', 'b', 'h', 'r'), (
        Operation('x', (), 0), Operation('y', (), 0), Operation('z', (), 0),
        Operation('c', ('x', 'y'), 1), Operation('b', ('c',), 1),
        Operation('c', ('b',), 1), Operation('h', ('b', 'z'), 1),
        Operation('r', ('c', 'h'), 1),
    ))


class ResidentTests(unittest.TestCase):
    def test_search_against_all_state_pairs(self):
        rng = random.Random(42471)
        classes = ('a', 'b', 'c')
        for case in range(45):
            operations = tuple(Operation(rng.choice(classes),
                                         tuple(rng.choice(classes) for _ in range(rng.randrange(4))),
                                         rng.randrange(4)) for _ in range(rng.randrange(7)))
            graph = Graph(classes, operations)
            for capacity in range(4):
                for (initial, required), expected in oracle(graph, capacity).items():
                    with self.subTest(case=case, capacity=capacity, initial=initial, required=required):
                        problem = Problem(graph, initial, required, capacity)
                        result = solve(problem)
                        if expected == math.inf:
                            self.assertEqual(result.status, 'infeasible')
                            self.assertIsNone(result.trace)
                        else:
                            self.assertEqual(result.status, 'optimal')
                            self.assertEqual(result.cost, expected)
                            self.assertEqual(check_trace(problem, result.trace), expected)

    def test_deletion_against_universal_contexts(self):
        classes = ('a', 'b', 'outside')
        arguments = ((), ('a',), ('b',), ('a', 'b'), ('a', 'a'))
        choices = [Operation(result, args, price) for result in ('a', 'b')
                   for args in arguments for price in (0, 1)]
        for operations in itertools.combinations_with_replacement(choices, 2):
            graph = Graph(classes, operations)
            full = [oracle(graph, capacity) for capacity in range(4)]
            for removed in (0, 1):
                remaining = (1 - removed,)
                uniform = all(full[capacity] == oracle(graph, capacity, remaining)
                              for capacity in range(4))
                result = deletion_test(graph, removed)
                self.assertEqual(result.status == 'safe', uniform, (operations, removed))
                if uniform:
                    check_deletion(graph, result.certificate)
                else:
                    self.assertEqual(result.status, 'necessary')
                    self.assertTrue(result.failed_contexts)

    def test_inverse_example_and_external_entry(self):
        graph = binary_example()
        problem = Problem(graph, frozenset(), frozenset({'r'}), 2)
        self.assertEqual(solve(problem).cost, 5)
        self.assertEqual(solve(problem, active=(0, 1, 2, 3, 4, 6, 7)).status, 'infeasible')
        self.assertEqual(solve(dataclasses.replace(problem, capacity=3)).cost, 4)
        inverse = deletion_test(graph, 5)
        self.assertEqual(inverse.status, 'necessary')
        self.assertEqual(inverse.failed_contexts, ('b',))
        entry = Graph(('input', 'output', 'outside'), (Operation('output', ('input',), 7),))
        query = Problem(entry, frozenset({'input', 'outside'}), frozenset({'output', 'outside'}), 2)
        self.assertEqual(solve(query).cost, 7)
        self.assertEqual(solve(dataclasses.replace(query, initial=frozenset())).status, 'infeasible')

    def test_zero_capacity_and_zero_cost_cycles(self):
        graph = Graph(('a', 'b'), (Operation('a', ('b',), 0), Operation('b', ('a',), 0)))
        self.assertEqual(solve(Problem(graph, frozenset(), frozenset({'a'}), 1)).status, 'infeasible')
        result = solve(Problem(graph, frozenset({'b'}), frozenset({'a'}), 1))
        self.assertEqual((result.status, result.cost), ('optimal', 0))
        self.assertEqual(solve(Problem(graph, frozenset(), frozenset(), 0)).cost, 0)
        self.assertEqual(solve(Problem(graph, frozenset(), frozenset({'a'}), 0)).status, 'infeasible')

    def test_limits_never_claim_infeasibility_or_safety(self):
        graph = Graph(('a', 'b'), (Operation('a', (), 1), Operation('b', ('a',), 1),
                                   Operation('b', (), 3)))
        problem = Problem(graph, frozenset(), frozenset({'b'}), 1)
        for limits, reason in ((Limits(max_states=1), 'state_limit'),
                               (Limits(timeout_seconds=0), 'time_limit')):
            result = solve(problem, limits=limits)
            self.assertEqual((result.status, result.reason), ('unknown', reason))
            self.assertIsNone(result.trace)
            deletion = deletion_test(graph, 2, limits=limits)
            self.assertEqual((deletion.status, deletion.reason), ('unknown', reason))
            self.assertIsNone(deletion.certificate)
            self.assertEqual(deletion.failed_contexts, ())
            reduced = prune(graph, candidates=(2,), limits=limits)
            self.assertEqual(reduced.active, (0, 1, 2))
            self.assertEqual(reduced.unknown_operations, (2,))
        self.assertEqual(solve(problem).cost, 2)

    def test_sequential_deletion_and_stable_indices(self):
        graph = Graph(('x', 'external'), (Operation('x', (), 1), Operation('x', (), 1)))
        first = deletion_test(graph, 0).certificate
        second = deletion_test(graph, 1).certificate
        check_deletion(graph, first)
        check_deletion(graph, second)
        with self.assertRaises(ValueError):
            check_deletion_sequence(graph, (first, second))
        reduced = prune(graph)
        self.assertEqual(reduced.active, (1,))
        self.assertEqual(check_deletion_sequence(graph, reduced.certificates), (1,))
        self.assertEqual(graph.classes, ('x', 'external'))
        self.assertEqual(len(graph.operations), 2)
        self.assertEqual(prune(graph, candidates=(1, 0)).active, (0,))
        with self.assertRaises(ValueError):
            deletion_test(graph, 0, active=(1,))

    def test_multi_step_replacement_and_retention_cases(self):
        graph = Graph(('a', 'b', 't', 'c'), (
            Operation('a', (), 100), Operation('b', (), 100),
            Operation('c', ('a', 'b'), 3), Operation('t', ('a',), 1),
            Operation('c', ('t', 'b'), 1),
        ))
        self.assertEqual(deletion_test(graph, 2).status, 'necessary')
        unary = Graph(('a', 't', 'c'), (
            Operation('c', ('a', 'a'), 3), Operation('t', ('a',), 1),
            Operation('c', ('t',), 1),
        ))
        result = deletion_test(unary, 0)
        self.assertEqual(result.status, 'safe')
        self.assertEqual(len(result.certificate.recipes), 1)
        check_deletion(unary, result.certificate)
        with self.assertRaises(ValueError):
            check_deletion(unary, result.certificate, active=(0, 2))
        with self.assertRaises(ValueError):
            check_deletion(unary, DeletionCertificate(0, ()))
        recipe = result.certificate.recipes[0]
        for bad in (Recipe('c', recipe.trace), Recipe('a', ()),
                    Recipe('a', (Step(0, frozenset({'c'})),))):
            with self.assertRaises(ValueError):
                check_deletion(unary, DeletionCertificate(0, (bad,)))
        with self.assertRaises(ValueError):
            check_deletion(unary, DeletionCertificate(0, (recipe, recipe)))
        cheaper = Graph(unary.classes, (Operation('c', ('a', 'a'), 1), *unary.operations[1:]))
        with self.assertRaises(ValueError):
            check_deletion(cheaper, result.certificate)

    def test_binary_recipe_coverage_and_self_result(self):
        graph = Graph(('a', 'b', 'c'), (
            Operation('c', ('a', 'b'), 2), Operation('c', ('a', 'b'), 1),
            Operation('a', ('a', 'b'), 0),
        ))
        result = deletion_test(graph, 0)
        self.assertEqual(result.status, 'safe')
        self.assertEqual({r.evicted_operand for r in result.certificate.recipes}, {'a', 'b'})
        check_deletion(graph, result.certificate)
        with self.assertRaises(ValueError):
            check_deletion(graph, DeletionCertificate(0, result.certificate.recipes[:1]))
        redundant = deletion_test(graph, 2)
        self.assertEqual(redundant.status, 'safe')
        self.assertEqual(redundant.certificate.recipes, ())
        check_deletion(graph, redundant.certificate)
        with self.assertRaises(ValueError):
            check_deletion(graph, DeletionCertificate(2, (Recipe('a', ()),)))

    def test_trace_mutations(self):
        graph = Graph(('a', 'b', 'outside'), (Operation('b', ('a',), 2),))
        problem = Problem(graph, frozenset({'a'}), frozenset({'b'}), 1)
        self.assertEqual(check_trace(problem, (Step(0, frozenset({'b'})),)), 2)
        mutations = ((), (Step(0, frozenset()),), (Step(0, frozenset({'a', 'b'})),),
                     (Step(0, frozenset({'outside'})),), (Step(None, frozenset({'b'})),),
                     (Step(1, frozenset({'b'})),), (Step(0, frozenset({'undeclared'})),),
                     (Step(None, frozenset()), Step(0, frozenset({'b'}))))
        for trace in mutations:
            with self.assertRaises(ValueError):
                check_trace(problem, trace)
        with self.assertRaises(ValueError):
            check_trace(problem, (Step(0, frozenset({'b'})),), active=())

    def test_validation_and_immutability(self):
        arguments = ['x']
        operations = [Operation('y', arguments, 1)]
        graph = Graph(['x', 'y'], operations)
        arguments.clear()
        operations.clear()
        self.assertEqual(graph.operations[0].arguments, ('x',))
        with self.assertRaises(dataclasses.FrozenInstanceError):
            graph.classes = ()
        for cost in (-1, 1.5, math.inf, math.nan, True):
            with self.assertRaises(ValueError):
                Operation('x', (), cost)
        with self.assertRaises(ValueError):
            Graph(('x', 'x'), ())
        with self.assertRaises(ValueError):
            Graph(('x',), (Operation('x', ('missing',), 0),))
        with self.assertRaises(ValueError):
            Problem(graph, frozenset({'outside'}), frozenset(), 1)
        with self.assertRaises(ValueError):
            Problem(graph, frozenset({'x', 'y'}), frozenset(), 1)
        for capacity in (-1, True, 1.5):
            with self.assertRaises(ValueError):
                Problem(graph, frozenset(), frozenset(), capacity)
        for invalid in (0, -1, True, 1.5):
            with self.assertRaises(ValueError):
                Limits(max_states=invalid)
        for invalid in (-1, True, math.inf, math.nan, 10**1000):
            with self.assertRaises(ValueError):
                Limits(timeout_seconds=invalid)
        with self.assertRaises(ValueError):
            solve(Problem(graph, frozenset(), frozenset(), 1), active=(0, 0))


if __name__ == '__main__':
    unittest.main()
