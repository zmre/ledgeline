//! Peak-RSS harness for PERF-1 — criterion measures time, not memory, and the
//! 3.4 GB figure is the whole point of that finding.
//!
//! Builds exactly what `ledgeline-server`'s `Snapshot::from_journal` holds
//! resident for one journal, stops at the requested `--stage`, and keeps
//! everything alive until the process exits. Run it under an external peak-RSS
//! reporter and diff the stages to attribute the growth:
//!
//! ```text
//! nix develop -c cargo build --release -p ledgeline-core --example load_rss
//!
//! for stage in text parse clone value snapshot; do
//!   /usr/bin/time -l ./target/release/examples/load_rss 200000 --stage $stage \
//!     2>&1 | grep -E 'stage|maximum resident'
//! done
//! ```
//!
//! On macOS `/usr/bin/time -l` prints `maximum resident set size` in BYTES; on
//! Linux use `/usr/bin/time -v` (`Maximum resident set size`, in KB).
//!
//! Sampling RSS from inside the process was tried and abandoned: `ps -o rss` now
//! requires an entitlement on macOS 26, and a `mach_task_basic_info` call would
//! mean `unsafe` plus a `libc` dependency in a crate that has neither. The
//! stage-by-stage external measurement is both simpler and more trustworthy —
//! the allocator does not return freed pages promptly, so an in-process
//! *current* RSS would over-report anyway.

#[path = "../benches/corpus.rs"]
mod corpus;

use ledgeline_core::{Journal, parse_journal, wire};
use std::hint::black_box;

/// How far to build before stopping. `text` .. `snapshot` are nested, so
/// `peak(stage) - peak(previous)` is that stage's incremental cost.
///
/// `bytes` is NOT part of that chain: it is the residency PERF-1 predicts once
/// the snapshot holds serialized bytes instead of a `Value` tree, so it skips
/// both the journal clone and the `Value`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Source text only — the floor.
    Text,
    /// + the parsed `Journal` (what `JournalEditor` alone holds).
    Parse,
    /// + `Arc::new(journal.clone())`, `ledgeline-server/src/lib.rs:65` (PERF-1b).
    Clone,
    /// + the `/transactions` `serde_json::Value` tree (PERF-1's 2,449 MB).
    Value,
    /// + the other six precomputed wire payloads: the full `Snapshot` today.
    Snapshot,
    /// Text + `Journal` + the serialized `/transactions` bytes: the PERF-1
    /// target state, for comparison against `snapshot`.
    Bytes,
}

impl Stage {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "parse" => Some(Self::Parse),
            "clone" => Some(Self::Clone),
            "value" => Some(Self::Value),
            "snapshot" => Some(Self::Snapshot),
            "bytes" => Some(Self::Bytes),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Text => "text (source only)",
            Self::Parse => "parse (+ Journal)",
            Self::Clone => "clone (+ Arc<Journal> clone)",
            Self::Value => "value (+ /transactions Value)",
            Self::Snapshot => "snapshot (+ 6 other wire Values)",
            Self::Bytes => "bytes (PERF-1 target: Journal + serialized bytes)",
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let txns: usize = args
        .first()
        .map_or(50_000, |a| a.parse().expect("first arg is a txn count"));
    let stage = args
        .iter()
        .position(|a| a == "--stage")
        .and_then(|i| args.get(i + 1))
        .map_or(Stage::Snapshot, |name| {
            Stage::parse(name).expect("--stage is one of text|parse|clone|value|snapshot")
        });

    let path = corpus::ensure_journal(txns);
    println!("corpus:  {} ({txns} txns)", path.display());
    println!("stage:   {}", stage.label());

    let text = std::fs::read_to_string(&path).expect("corpus is readable");
    println!("source:  {} bytes", text.len());
    if stage == Stage::Text {
        black_box(&text);
        return;
    }

    let journal = parse_journal(&text, &path.to_string_lossy()).expect("corpus parses");
    println!(
        "parsed:  {} transactions, {} postings, {} prices, {} account decls",
        journal.transactions.len(),
        journal
            .transactions
            .iter()
            .map(|t| t.postings.len())
            .sum::<usize>(),
        journal.prices.len(),
        journal.accounts.len(),
    );
    if stage == Stage::Parse {
        black_box((&text, &journal));
        return;
    }

    // The PERF-1 target: serialize once, straight to bytes, with no `Value` tree
    // and no journal clone anywhere in the residency.
    if stage == Stage::Bytes {
        let serialized =
            serde_json::to_vec(&wire::journal_to_transactions(&journal)).expect("serializable");
        println!(
            "wire:    /transactions serializes to {} bytes",
            serialized.len()
        );
        black_box((&text, &journal, &serialized));
        return;
    }

    let cloned: std::sync::Arc<Journal> = std::sync::Arc::new(journal.clone());
    if stage == Stage::Clone {
        black_box((&text, &journal, &cloned));
        return;
    }

    let transactions = wire::journal_to_value(&journal).expect("wire value");
    if stage == Stage::Value {
        black_box((&text, &journal, &cloned, &transactions));
        return;
    }

    let others = [
        wire::version_value(),
        wire::journal_to_accountnames_value(&journal).expect("accountnames"),
        wire::journal_to_prices_value(&journal).expect("prices"),
        wire::journal_to_commodities_value(&journal).expect("commodities"),
        wire::journal_to_accounts_value(&journal).expect("accounts"),
        wire::journal_to_diagnostics_value(&journal).expect("diagnostics"),
    ];

    // Hold everything to process exit so the OS peak reflects full residency.
    black_box((&text, &journal, &cloned, &transactions, &others));
}
