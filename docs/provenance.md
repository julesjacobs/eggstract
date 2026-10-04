# Provenance and scope

This initial release was distilled on 2026-10-04 from Jules Jacobs's Eggstract
research workspace. That workspace had no committed history. The release starts
a new history containing the selected source, tests and documentation.

The implementation was developed with AI assistance. The resident theorem and
its original Python prototype arose from a research consultation; mathematical
arguments and implementations were then checked independently. The public
resident module is a rewrite with explicit input validation, arbitrary entry
residents, search limits, stable operation indices and sequential pruning.
Correctness tests do not establish historical novelty or practical speedups.

## Source lineage

| Release component | Research source |
|---|---|
| `src/graph.rs` | Dense indexing and tree seed in `crates/extractors/src/fast_seed.rs` |
| `src/compact.rs` | Fixed `research-compact-direct` path in `crates/extractors/src/compact_search.rs` |
| `src/exact_path.rs` | Exact binary-rescaled path bounds in `crates/extractors/src/exact_path.rs` |
| `src/verify.rs` | Rewritten trusted-root DAG verifier |
| `python/eggstract/exact.py` | Simplified SCC formulation from `scripts/native_cpsat_extract.py`, with checked integer costs |
| `python/eggstract/resident.py` | Audited bounded-resident model and contextual deletion theorem |

The release removes the experiment registry, alternate search policies,
environment-driven diagnostics, instruction counters and benchmark harness.
The direct extractor retains demand reconstruction, descent, support exchange,
dependency repair and its internal path-bound shortcut. API and verification
costs differ from the research runner, so old timing numbers do not characterize
this release.

Before release, 43 existing small generated/tiny inputs were run to natural
completion at seeds 0, 1 and 7. All 129 comparisons matched the current research
implementation's cost and selected-node set. This checks the port on those
inputs; it is neither a proof of implementation equivalence nor a runtime study.

Research source SHA-256 hashes:

```text
fast_seed.rs       d6e95ea40a0f8ce1ef9a4409ab824823a0da81e69cef120528e5af488d0d3d46
compact_search.rs  ec7e960238b9f23c8bb36b43385d4526a709003e434f06880238184f89f049c2
exact_path.rs      0c17a782a91defc489905246d8a0dc777c46399d1a0e715d817b700f0757f350
verify/lib.rs      b120bacc16d72e0b7a78000694c75b0644afc0a1869a880ba8a8e2fb9742f2e8
native_cpsat_extract.py
                  486a5c44352d4f5eb92d7895d212201e3be3d1482465a8cae6827281aac97125
THEOREMS.md       c60dba3be5997ff9406c3a49f7b968cc4aa84d105cd3124801084e000c6bc10d
```

## Attribution

The earlier research included adaptations of
[extraction-gym](https://github.com/egraphs-good/extraction-gym), inspected at
commit `903ba0f818b50608fe20ae9e0f03c35cb27bc50a`. Its MIT notice, copyright 2019
Max Willsey, is retained in [THIRD_PARTY_NOTICES](../THIRD_PARTY_NOTICES).
This does not imply that the fixed direct heuristic is an upstream algorithm or
that this package reproduces extraction-gym's full implementation.

[`egraph-serialize`](https://github.com/egraphs-good/egraph-serialize) supplies the
Rust input representation. [Google OR-Tools](https://github.com/google/or-tools)
supplies the optional CP-SAT solver. These are registry dependencies, not
vendored code. The CP-SAT formulation is local; it is not the e-boost pipeline.

For memory-bounded extraction, reverse rematerialization and cost-preserving
operator-sequence replacement have prior art. See [resident.md](resident.md) for
the exact model, theorem statement, proof outline and references. Historical
novelty of the finite deletion criterion remains unresolved.

No third-party benchmark corpus, paper PDF, private consultation transcript or
research result dump is distributed with this project.
