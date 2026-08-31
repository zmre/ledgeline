//! Write the deterministic synthetic performance corpus to disk.
//!
//! ```text
//! # v1 (the baseline corpus), at the given transaction counts
//! cargo run --release -p ledgeline-core --example gen_journal -- 5000 50000 200000
//!
//! # the large-repo fixture: 200 commodities, 75 accounts, 15 years
//! cargo run --release -p ledgeline-core --example gen_journal -- v2
//!
//! # ...and its mesh-price-graph twin
//! cargo run --release -p ledgeline-core --example gen_journal -- v2fx
//! ```
//!
//! With no arguments it writes the default v1 bench sizes (see
//! `corpus::bench_sizes`). Files land in `target/perf/` — gitignored, and named
//! with the shape's version and label so a shape change invalidates the cache.
//! Output is byte-identical across runs and machines; verify with
//! `hledger -f <file> check -s`.

#[path = "../benches/corpus.rs"]
mod corpus;

use corpus::Shape;

/// Resolve one command-line argument to a shape: a bare number is a v1
/// transaction count, a name selects one of the large fixtures.
fn shape_for(arg: &str) -> Shape {
    match arg {
        "v2" => corpus::V2,
        "v2fx" => corpus::V2_FX,
        _ => corpus::V1
            .with_txns(arg.parse().unwrap_or_else(|e| {
                panic!("not a transaction count or shape name: {arg:?} ({e})")
            })),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let shapes: Vec<Shape> = if args.is_empty() {
        corpus::bench_sizes()
            .into_iter()
            .map(|txns| corpus::V1.with_txns(txns))
            .collect()
    } else {
        args.iter().map(|arg| shape_for(arg)).collect()
    };

    for shape in shapes {
        let path = corpus::journal_path_for(&shape);
        let dir = path.parent().expect("corpus path has a parent");
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
        let text = corpus::generate_shape(&shape);
        let bytes = text.len();
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        println!(
            "{:>7} txns  {:>12} bytes  {:>7.2} MB  {}",
            shape.txns,
            bytes,
            bytes as f64 / 1_000_000.0,
            path.display()
        );
    }
}
