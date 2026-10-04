#!/usr/bin/env python3
"""Check saved benchmark witnesses without calling any extractor or Rust verifier."""
import argparse
import hashlib
import json
import math
from pathlib import Path


def check_witness(graph, roots, selected):
    nodes = graph['nodes']
    choices = {}
    for nid in selected:
        assert nid in nodes, f'unknown selected node: {nid}'
        cid = nodes[nid]['eclass']
        assert cid not in choices, f'multiple choices in class: {cid}'
        choices[cid] = nid
    state = {}
    for root in roots:
        stack = [(root, False)]
        while stack:
            cid, leaving = stack.pop()
            if leaving:
                state[cid] = 2
                continue
            assert state.get(cid) != 1, f'cycle through {cid}'
            if state.get(cid) == 2:
                continue
            assert cid in choices, f'missing reachable class: {cid}'
            state[cid] = 1
            stack.append((cid, True))
            for child in nodes[choices[cid]]['children']:
                stack.append((nodes[child]['eclass'], False))
    assert set(state) == set(choices), 'unreachable selected nodes'
    return math.fsum(nodes[nid]['cost'] for nid in selected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    args = parser.parse_args()
    report = json.loads(args.report.read_text())
    assert report['complete'], 'incomplete report'
    assert not report['generation_failures'], report['generation_failures']
    ids = [case['id'] for case in report['cases']]
    assert len(ids) == len(set(ids)), 'duplicate cases'
    if not report['config']['smoke']:
        expected = {
            f'{family}/{size}/{seed}'
            for family in report['config']['families']
            for size in report['config']['sizes']
            for seed in report['config']['seeds']
        }
        assert set(ids) == expected, 'incomplete parameter grid'
    checked = 0
    failed = 0
    for case in report['cases']:
        raw = (args.report.parent / case['graph_file']).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == case['graph_sha256'], case['id']
        graph = json.loads(raw)
        assert len(graph['nodes']) == case['nodes']
        assert graph['root_eclasses'] == case['requested_roots']
        assert all(node['cost'] == 1 for node in graph['nodes'].values())
        seen = set()
        methods = set()
        for run in case['measurements']:
            key = run['method'], run['repetition']
            assert key not in seen, f'duplicate measurement: {key}'
            seen.add(key)
            methods.add(run['method'])
            assert run['warmup'] == (run['repetition'] == 0)
            assert run['elapsed_ns'] >= 0
            if run['status'] != 'feasible':
                assert run['cost'] is None
                failed += 1
                continue
            cost = check_witness(graph, case['requested_roots'], run['selected_enodes'])
            assert cost == run['cost'], (case['id'], key, cost, run['cost'])
            checked += 1
        assert methods == set(report['config']['methods']), f'incomplete methods: {methods}'
        assert seen == {(method, repetition) for method in methods for repetition in range(report['config']['measured_repetitions'] + 1)}
    print(f'Checked {checked} feasible witnesses in {len(ids)} cases; {failed} failed attempts retained.')


if __name__ == '__main__':
    main()
