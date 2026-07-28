//! Write the deterministic synthetic performance corpus to disk.
//!
//! ```text
//! cargo run --release -p ledgeline-core --example gen_journal -- 5000 50000 200000
//! ```
//!
//! With no arguments it writes the default bench sizes (see
//! `corpus::bench_sizes`). Files land in `target/perf/` — gitignored, and named
//! with the generator's `CORPUS_VERSION` so a shape change invalidates the
//! cache. Output is byte-identical across runs and machines; verify with
//! `hledger -f <file> check`.

#[path = "../benches/corpus.rs"]
mod corpus;

fn main() {
    let sizes: Vec<usize> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            corpus::bench_sizes()
        } else {
            args.iter()
                .map(|a| {
                    a.parse()
                        .unwrap_or_else(|e| panic!("not a transaction count: {a:?} ({e})"))
                })
                .collect()
        }
    };

    for txns in sizes {
        let path = corpus::journal_path(txns);
        let dir = path.parent().expect("corpus path has a parent");
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
        let text = corpus::generate(txns);
        let bytes = text.len();
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        println!(
            "{:>7} txns  {:>12} bytes  {:>7.2} MB  {}",
            txns,
            bytes,
            bytes as f64 / 1_000_000.0,
            path.display()
        );
    }
}
