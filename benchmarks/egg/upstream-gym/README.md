# Upstream extraction-gym adapter

This benchmark-only crate includes two unmodified source files from
[extraction-gym](https://github.com/egraphs-good/extraction-gym/tree/903ba0f818b50608fe20ae9e0f03c35cb27bc50a),
commit `903ba0f818b50608fe20ae9e0f03c35cb27bc50a`:

| File | Upstream name | Implementation history |
| --- | --- | --- |
| `src/greedy_dag.rs` | `greedy-dag` | Introduced by Max Willsey in `95e8a74`; later changes by Trevor Hansen. |
| `src/faster_greedy_dag.rs` | `faster-greedy-dag` | Introduced by Trevor Hansen in `ee68161`; later changes by Trevor Hansen and Oliver Flatt. |

These are related sharing-aware greedy extractors, not independent algorithm
families. Both estimate candidate DAG costs using unions of reachable classes.
`faster-greedy-dag` uses a work queue and avoids unnecessary set work. The
`global-greedy-dag` implementation is excluded because its registration is
disabled in the pinned upstream benchmark.

The adapter supplies the upstream types and `choose` operation needed by these
files, and exposes their complete class-to-node choices. The benchmark recovers
the choices reachable from its requested roots and verifies them independently.
The algorithms themselves are unchanged, including `greedy-dag`'s iteration
logging. The original pinned `egraph-serialize` dependency is retained; input
conversion happens outside the timed calls. These dependencies are separate
from the Eggstract library.

The full upstream MIT notice is in [LICENSE](LICENSE). SHA-256 digests:

```text
c10f6e7b1a3921174914147938ae353623b75c9034d55bc3186612c28ed60d78  src/greedy_dag.rs
b2c3b600d8643ef25c8843e19edc7b52d569d1ac95c333176d294259fdee3236  src/faster_greedy_dag.rs
33ec94d67f8eff8e107cbd61c6af1502c6e414c5ec70923cf47515f015233969  LICENSE
```
