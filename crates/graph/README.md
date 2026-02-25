# apex-graph

Supply-chain knowledge graph — adjacency representation, PageRank, risk propagation, connected components, and shortest-path.

## Responsibilities

- **Adjacency graph** (`adjacency.rs`): `AdjacencyGraph` — adds typed directed edges, queries neighbours, and runs graph algorithms.
  - `add_edge(source, target, weight)` — build the graph incrementally.
  - `pagerank(damping, iterations)` — iterative PageRank; returns `HashMap<String, f64>`.
  - `propagate_risk(seeds, hops)` — BFS risk propagation from seed nodes; attenuates by `1/depth` per hop.
  - `connected_components()` — Union-Find implementation; returns labelled component map.
  - `shortest_path(from, to)` — BFS on unweighted adjacency; returns `Option<Vec<String>>`.

## Performance

Criterion benchmarks live in `benches/adjacency_bench.rs`:

```bash
cargo bench -p apex-graph
```

Benchmark scenarios:
| Benchmark | Graph sizes |
|-----------|------------|
| `bench_pagerank` | 110 / 210 / 420 nodes |
| `bench_propagate_risk` | 210 nodes, 1/3/5 hops |
| `bench_connected_components` | chain 50/100/200 nodes |
| `bench_shortest_path` | chain 50/100/200 nodes |

## Key invariants

- Node IDs are arbitrary `String` values.
- Weights default to `1.0`; PageRank uses them for transition probability weighting.
- `propagate_risk` seeds are raw node IDs; the returned map contains all reachable nodes with their attenuated scores.
