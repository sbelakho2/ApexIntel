//! Criterion benchmarks for heavy graph operations (B268).
//!
//! Covers the three most CPU-intensive operations on `AdjacencyGraph`:
//!   - `pagerank` (power iteration)
//!   - `propagate_risk` (BFS risk propagation)
//!   - `connected_components` (undirected BFS)
//!   - `shortest_path` (BFS)
//!
//! Benchmarks use a synthetic scale-free-like graph with ~200 nodes and ~800
//! directed edges, representative of the entity graph seen in production.

use apex_graph::adjacency::AdjacencyGraph;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;

// ────────────────────────────────────────────
// Graph fixture builders
// ────────────────────────────────────────────

/// Build a dense hub-and-spoke graph with `hubs` hub nodes, each connected to
/// `spokes_per_hub` spoke nodes.  Spoke-to-hub back-edges are also added so
/// the graph has both in- and out-degree on all nodes.
fn hub_and_spoke(hubs: usize, spokes_per_hub: usize) -> AdjacencyGraph {
    let mut g = AdjacencyGraph::new();
    for h in 0..hubs {
        let hub = format!("hub_{h}");
        for s in 0..spokes_per_hub {
            let spoke = format!("spoke_{h}_{s}");
            g.add_bidi_edge(&hub, &spoke, 0.8);
        }
        // Cross-connect hubs in a ring
        let next_hub = format!("hub_{}", (h + 1) % hubs);
        g.add_edge(&hub, &next_hub, 0.5);
    }
    g
}

/// Build a random-ish linear chain of `n` nodes with occasional cross-edges.
fn chain_with_shortcuts(n: usize) -> AdjacencyGraph {
    let mut g = AdjacencyGraph::new();
    for i in 0..n {
        let a = format!("node_{i}");
        let b = format!("node_{}", i + 1);
        g.add_edge(&a, &b, 0.9);
        // Add a shortcut every 10 nodes
        if i % 10 == 0 && i + 15 < n {
            let c = format!("node_{}", i + 15);
            g.add_edge(&a, &c, 0.3);
        }
    }
    g
}

// ────────────────────────────────────────────
// Benchmark: PageRank
// ────────────────────────────────────────────

fn bench_pagerank(c: &mut Criterion) {
    let mut group = c.benchmark_group("pagerank");

    for &(hubs, spokes) in &[(5, 20), (10, 20), (20, 20)] {
        let g = hub_and_spoke(hubs, spokes);
        let node_count = g.node_count();
        group.bench_with_input(
            BenchmarkId::new("hub_and_spoke", format!("{}n", node_count)),
            &g,
            |b, g| b.iter(|| black_box(g.pagerank(black_box(30), black_box(0.85)))),
        );
    }

    group.finish();
}

// ────────────────────────────────────────────
// Benchmark: Risk Propagation
// ────────────────────────────────────────────

fn bench_propagate_risk(c: &mut Criterion) {
    let mut group = c.benchmark_group("propagate_risk");

    for &hops in &[1u8, 3, 5] {
        let g = hub_and_spoke(10, 20); // 210 nodes
        let mut initial: HashMap<String, f64> = HashMap::new();
        // Seed risk on the hub nodes
        for h in 0..10 {
            initial.insert(format!("hub_{h}"), 0.9);
        }

        group.bench_with_input(
            BenchmarkId::new("hub_and_spoke_210n", format!("hops_{hops}")),
            &(g, initial, hops),
            |b, (g, init, hops)| {
                b.iter(|| {
                    black_box(g.propagate_risk(black_box(init), black_box(*hops), black_box(0.7)))
                })
            },
        );
    }

    group.finish();
}

// ────────────────────────────────────────────
// Benchmark: Connected Components
// ────────────────────────────────────────────

fn bench_connected_components(c: &mut Criterion) {
    let mut group = c.benchmark_group("connected_components");

    for &n in &[50usize, 100, 200] {
        let g = chain_with_shortcuts(n);
        group.bench_with_input(
            BenchmarkId::new("chain_with_shortcuts", format!("{}n", n)),
            &g,
            |b, g| b.iter(|| black_box(g.connected_components())),
        );
    }

    // Fragmented graph (all isolated nodes)
    let mut isolated = AdjacencyGraph::new();
    for i in 0..100 {
        // Adding a self-edge registers the node without connecting it to others
        isolated.add_edge(&format!("iso_{i}"), &format!("iso_{i}"), 0.0);
    }
    group.bench_with_input(
        BenchmarkId::new("isolated_100n", "all_singletons"),
        &isolated,
        |b, g| b.iter(|| black_box(g.connected_components())),
    );

    group.finish();
}

// ────────────────────────────────────────────
// Benchmark: Shortest Path (BFS)
// ────────────────────────────────────────────

fn bench_shortest_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("shortest_path");

    for &n in &[50usize, 100, 200] {
        let g = chain_with_shortcuts(n);
        let src = "node_0".to_string();
        let dst = format!("node_{}", n - 1);

        group.bench_with_input(
            BenchmarkId::new("chain_with_shortcuts", format!("{}n", n)),
            &(g, src, dst),
            |b, (g, src, dst)| {
                b.iter(|| black_box(g.shortest_path(black_box(src), black_box(dst))))
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_pagerank,
    bench_propagate_risk,
    bench_connected_components,
    bench_shortest_path
);
criterion_main!(benches);
