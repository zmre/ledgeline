//! `ledgeline-invoicer` — Ledgeline Invoicer's binary (`invoicer/ARCHITECTURE.md`
//! §2.2, §6.4).
//!
//! Scaffolding only (invoicer PLAN 0d): `--version` and `--help`. The CLI
//! commands arrive in PR 1c, the GUI in Phase 3.

use clap::Parser;

/// The command line. `about` is the package description, so `--help` opens by
/// naming the app.
#[derive(Parser, Debug)]
#[command(name = "ledgeline-invoicer", version, about, long_about = None)]
struct Cli {}

fn main() {
    let Cli {} = Cli::parse();
}
