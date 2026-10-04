import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from eggstract.exact import MAX_TOTAL_COST, solve, verify


def graph(nodes, roots=None):
    return {"nodes": nodes, "root_eclasses": roots or []}


def node(cls, cost=1, children=()):
    return {"eclass": cls, "op": "test", "cost": cost, "children": list(children)}


def exhaustive(data, roots):
    nodes = data["nodes"]
    classes = {}
    for ident, item in nodes.items():
        classes.setdefault(item["eclass"], []).append(ident)
    best, witness = None, None
    for alternatives in itertools.product(*classes.values()):
        choice = dict(zip(classes, alternatives))
        active, visiting = set(), set()

        def visit(cls):
            if cls in visiting:
                return False
            if cls in active:
                return True
            visiting.add(cls)
            for child in nodes[choice[cls]]["children"]:
                if not visit(nodes[child]["eclass"]):
                    return False
            visiting.remove(cls)
            active.add(cls)
            return True

        if all(visit(root) for root in roots):
            selected = sorted(choice[cls] for cls in active)
            cost = sum(nodes[ident]["cost"] for ident in selected)
            if best is None or cost < best:
                best, witness = cost, selected
    return best, witness


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.data = graph({"leaf": node("L", 2), "root": node("R", 3, ["leaf", "leaf"])})

    def test_shared_cost_and_explicit_roots(self):
        self.data["root_eclasses"] = ["L"]
        self.assertEqual(verify(self.data, ["R", "L", "R"], ["root", "leaf"]), 5)
        with self.assertRaisesRegex(ValueError, "omits required"):
            verify(self.data, ["R"], ["leaf"])

    def test_invalid_costs(self):
        for bad in [-1, 0.5, float("nan"), float("inf"), True, "2", None, MAX_TOTAL_COST + 1]:
            with self.subTest(cost=bad), self.assertRaises(ValueError):
                solve(graph({"n": node("C", bad)}), ["C"], time_limit=0)
        with self.assertRaisesRegex(ValueError, "sum"):
            solve(graph({"a": node("A", MAX_TOTAL_COST), "b": node("B", 1)}), ["A"], time_limit=0)
        self.assertEqual(verify(graph({"a": node("A", MAX_TOTAL_COST)}), ["A"], ["a"]), MAX_TOTAL_COST)
        self.assertEqual(verify(graph({"a": node("A", 2.0)}), ["A"], ["a"]), 2)

    def test_invalid_graphs_and_options(self):
        cases = [({}, ["R"]), (self.data, []), (self.data, "R"), (self.data, ["missing"]),
                 (graph({"a": node("A", children=["missing"])}), ["A"]),
                 (graph({"a": {"eclass": 1}}), ["A"]),
                 (graph({"a": {"eclass": "A", "children": "a"}}), ["A"])]
        for data, roots in cases:
            with self.subTest(data=data, roots=roots), self.assertRaises(ValueError):
                solve(data, roots, time_limit=0)
        for time in [-1, float("inf"), float("nan"), 10**1000, True, "1"]:
            with self.subTest(time=time), self.assertRaises(ValueError):
                solve(self.data, ["R"], time_limit=time)
        for workers in [0, -1, 1.5, True, 2**31, 10**1000]:
            with self.subTest(workers=workers), self.assertRaises(ValueError):
                solve(self.data, ["R"], time_limit=0, workers=workers)

    def test_hint_validation(self):
        data = graph({**self.data["nodes"], "other": node("O"), "alt": node("L", 4)})
        invalid = [["leaf"], ["root", "leaf", "other"], ["root", "leaf", "alt"],
                   ["root", "leaf", "leaf"], ["root", "missing"],
                   {"selected_enodes": ["root", "leaf"], "root_eclasses": ["L"]},
                   {}, "root"]
        for hint in invalid:
            with self.subTest(hint=hint), self.assertRaises(ValueError):
                solve(data, ["R"], hint=hint, time_limit=0)
        cyclic = graph({"a": node("A", 0, ["b"]), "b": node("B", 0, ["a"])})
        with self.assertRaisesRegex(ValueError, "cycle"):
            solve(cyclic, ["A"], hint=["a", "b"], time_limit=0)

    def test_zero_time_preserves_verified_hint(self):
        empty = solve(self.data, ["R"], time_limit=0)
        self.assertEqual(empty.solver_status, "unknown")
        self.assertFalse(empty.feasible)
        self.assertIsNone(empty.cost)
        self.assertIsNone(empty.lower_bound)
        hint = {"selected_enodes": ["root", "leaf"], "root_eclasses": ["R"], "cost": -999}
        result = solve(self.data, ["R"], time_limit=0, hint=hint)
        self.assertEqual((result.solver_status, result.source, result.cost), ("unknown", "hint", 5))
        self.assertTrue(result.feasible)

    def test_import_and_verification_need_only_stdlib(self):
        env = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1]))
        command = "from eggstract.exact import verify; import sys; assert 'ortools' not in sys.modules; assert verify({'nodes': {'n': {'eclass': 'R', 'cost': 2}}}, ['R'], ['n']) == 2"
        subprocess.run([sys.executable, "-S", "-c", command], env=env, check=True)

    def test_missing_solver_is_explicit(self):
        original_import = __import__

        def import_without_solver(name, *args, **kwargs):
            if name.startswith("ortools"):
                raise ImportError("solver unavailable")
            return original_import(name, *args, **kwargs)

        with patch("builtins.__import__", side_effect=import_without_solver):
            with self.assertRaisesRegex(RuntimeError, "optional ortools"):
                solve(self.data, ["R"])

    def test_cli_zero_time_and_validation(self):
        env = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1]))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "graph.json").write_text(json.dumps(self.data))
            (path / "hint.json").write_text(json.dumps({"selected_enodes": ["root", "leaf"]}))
            command = [sys.executable, "-m", "eggstract.exact", str(path / "graph.json"),
                       "--root", "R", "--hint", str(path / "hint.json"), "--time-limit", "0"]
            output = subprocess.run(command, env=env, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(output.stdout)["cost"], 5)
            command[command.index("R")] = "missing"
            output = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(output.returncode, 2)
            self.assertIn("error", json.loads(output.stdout))


@unittest.skipUnless(importlib.util.find_spec("ortools"), "optional OR-Tools is not installed")
class SolverTests(unittest.TestCase):
    def test_exhaustive_small_graphs(self):
        rng = random.Random(8517)
        for case in range(36):
            nodes = {}
            for cls in range(4):
                for alternative in range(2):
                    children = [f"{rng.randrange(4)}a" for _ in range(rng.randrange(3))]
                    nodes[f"{cls}{'ab'[alternative]}"] = node(str(cls), rng.randrange(5), children)
            data = graph(nodes)
            roots = ["0", "2"] if case % 2 else ["0"]
            optimum, witness = exhaustive(data, roots)
            with self.subTest(case=case):
                for hint in [None, witness] if witness else [None]:
                    result = solve(data, roots, time_limit=5, hint=hint)
                    if optimum is None:
                        self.assertEqual(result.solver_status, "infeasible")
                        self.assertFalse(result.feasible)
                        self.assertIsNone(result.cost)
                    else:
                        self.assertEqual(result.solver_status, "optimal")
                        self.assertTrue(result.feasible)
                        self.assertEqual(result.cost, optimum)
                        self.assertEqual(result.lower_bound, optimum)
                        self.assertEqual(verify(data, roots, result.selected_enodes), optimum)

    def test_sharing_multiroot_and_ungrounded_cycles(self):
        data = graph({"x": node("X", 5), "a": node("A", 1, ["x", "x"]),
                      "b": node("B", 1, ["x"]), "unused": node("U", 0)})
        result = solve(data, ["A", "B"])
        self.assertEqual(result.cost, 7)
        self.assertNotIn("unused", result.selected_enodes)
        cyclic = graph({"a": node("A", 0, ["b"]), "b": node("B", 0, ["a"])})
        self.assertEqual(solve(cyclic, ["A"]).solver_status, "infeasible")
        cyclic["nodes"]["base"] = node("A", 3)
        self.assertEqual(solve(cyclic, ["A", "B"]).cost, 3)

    def test_hint_is_incumbent_not_a_fixed_choice(self):
        data = graph({"cheap": node("R", 2), "dear": node("R", 9)})
        result = solve(data, ["R"], hint=["dear"])
        self.assertEqual((result.solver_status, result.cost, result.source), ("optimal", 2, "solver"))
        result = solve(data, ["R"], hint=["dear"], time_limit=1e-9)
        self.assertTrue(result.feasible)
        self.assertLessEqual(result.cost, 9)
        self.assertEqual(verify(data, ["R"], result.selected_enodes), result.cost)

    def test_largest_supported_cost(self):
        data = graph({"n": node("R", MAX_TOTAL_COST)})
        result = solve(data, ["R"])
        self.assertEqual(result.solver_status, "optimal")
        self.assertEqual(result.cost, MAX_TOTAL_COST)


if __name__ == "__main__":
    unittest.main()
