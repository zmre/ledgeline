//! Build script: guarantee the embedded-SPA folder exists.
//!
//! The binary embeds the built SvelteKit SPA from `web/build` via `rust-embed`.
//! That folder is a build artifact (git-ignored) and is absent on a fresh
//! checkout / CI, where `rust-embed` would otherwise fail to compile with
//! "folder does not exist". To keep `cargo build` working WITHOUT the SPA, we
//! create the folder and drop in a placeholder `index.html` when it is missing.
//! The real SPA is produced by `bun run build` (→ `web/build/`) and, when
//! present, is what actually gets embedded.
//!
//! Build order for a real single binary:
//!   1. `cd web && bun run build`   (writes `web/build/`)
//!   2. `cargo build --release`     (embeds `web/build/` into the binary)

use std::path::Path;

const PLACEHOLDER_INDEX: &str = "<!doctype html>\n\
<html lang=\"en\">\n\
<head><meta charset=\"utf-8\"><title>Ledgeline</title></head>\n\
<body>\n\
  <h1>Ledgeline SPA not built</h1>\n\
  <p>The web UI has not been built yet. Run <code>bun run build</code> (or\n\
     <code>vite build</code>) inside <code>web/</code>, then rebuild the binary.</p>\n\
</body>\n\
</html>\n";

fn main() {
    // `CARGO_MANIFEST_DIR` is always set by cargo for build scripts.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let build_dir = Path::new(&manifest_dir).join("../../web/build");

    if let Err(err) = std::fs::create_dir_all(&build_dir) {
        println!(
            "cargo:warning=ledgeline: could not create {}: {err}",
            build_dir.display()
        );
    }

    let index = build_dir.join("index.html");
    if !index.exists()
        && let Err(err) = std::fs::write(&index, PLACEHOLDER_INDEX)
    {
        println!("cargo:warning=ledgeline: could not write placeholder index.html: {err}");
    }

    // Re-run (and, for release, re-embed) when the SPA build output changes.
    println!("cargo:rerun-if-changed=../../web/build");
    println!("cargo:rerun-if-changed=../../web/build/index.html");
}
