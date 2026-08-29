//! Build script: keep the embedded-SPA folder (`spa/`) present and current.
//!
//! # Why `spa/` and not `../../web/build`
//!
//! `spa.rs` embeds `$CARGO_MANIFEST_DIR/spa` via `rust-embed`. Pointing
//! rust-embed at `../../web/build` directly — which is what this crate used to
//! do — works fine inside the workspace and makes the crate **unpublishable**:
//! `cargo package` refuses to include files outside the package root, so the
//! `.crate` would ship with no UI at all and `cargo install ledgeline-server`
//! would serve the placeholder page below. Owning a directory inside the crate
//! is what lets the built SPA travel with it.
//!
//! # The three cases, in order
//!
//! 1. **Workspace checkout with a built SPA** (`../../web/build/index.html`
//!    exists) — mirror it into `spa/`. This is the developer and Nix path:
//!    `bun run build` then `cargo build` embeds the real UI.
//! 2. **Published crate** (no `web/build`, but `spa/index.html` shipped inside
//!    the `.crate`) — leave it strictly alone. Clobbering it here is precisely
//!    how `cargo install` would silently serve a placeholder to every user.
//! 3. **Neither** (fresh checkout, or CI before the SPA is built) — write a
//!    placeholder, because `rust-embed` fails to COMPILE against a missing
//!    folder and every `cargo check` in the repo would stop working.
//!
//! Build order for a real single binary:
//!
//! 1. `cd web && bun run build` — writes `web/build/`
//! 2. `cargo build --release` — this script mirrors it into `spa/`, which
//!    rust-embed then bakes into the binary
//!
//! # A note on debug builds
//!
//! `rust-embed` reads from disk at RUN time in debug builds (it only bakes the
//! bytes in for release, absent the `debug-embed` feature). So a debug binary
//! serves whatever is in `spa/` at the moment of the request, and `spa/` is
//! refreshed by this script rather than by `vite`. That is a change from reading
//! `web/build` live, and it is immaterial in practice: `just dev` serves the SPA
//! from vite on its own port, and the Playwright suite uses `bun run preview` —
//! neither goes through the embedded copy.

use std::path::{Path, PathBuf};

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
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
    let spa_dir = manifest_dir.join("spa");
    let web_build = manifest_dir.join("../../web/build");

    // Only meaningful inside the workspace. Emitting it unconditionally would
    // name a path that does not exist in a published crate, which cargo treats
    // as "always re-run" — so every single build of an installed copy would run
    // this script again to do nothing.
    if manifest_dir.join("../../web").is_dir() {
        println!("cargo:rerun-if-changed=../../web/build");
    }

    // `hledger::resolve` reads this through `option_env!`, which cargo does NOT
    // track on its own: without this line, flipping the variable leaves a stale
    // binary that still points at the previous hledger and gives no hint why.
    // Unset is the normal case (a plain `cargo build`), and resolution then
    // falls through to the sibling binary and `$PATH`.
    println!("cargo:rerun-if-env-changed=LEDGELINE_HLEDGER_PATH");

    // Case 1: a built SPA in the workspace wins over anything already in `spa/`.
    if web_build.join("index.html").is_file() {
        if let Err(err) = mirror(&web_build, &spa_dir) {
            println!(
                "cargo:warning=ledgeline: could not sync web/build into {}: {err}",
                spa_dir.display()
            );
        }
        return;
    }

    // Case 2: a published crate ships its SPA. Never touch it.
    if spa_dir.join("index.html").is_file() {
        return;
    }

    // Case 3: nothing to embed — make rust-embed compile.
    if let Err(err) = std::fs::create_dir_all(&spa_dir) {
        println!(
            "cargo:warning=ledgeline: could not create {}: {err}",
            spa_dir.display()
        );
        return;
    }
    if let Err(err) = std::fs::write(spa_dir.join("index.html"), PLACEHOLDER_INDEX) {
        println!("cargo:warning=ledgeline: could not write placeholder index.html: {err}");
    }
}

/// Replace `dest` with a copy of `src`.
///
/// The removal is what makes this a MIRROR rather than an overlay, and it is
/// load-bearing: SvelteKit emits content-hashed filenames
/// (`entry/app.CYy4IZL6.js`), so overwriting in place would leave every previous
/// build's chunks sitting in `spa/` and rust-embed would bake all of them into
/// the binary. The result is a binary that grows monotonically with the number
/// of times you have ever built the SPA, which is not a thing anyone would think
/// to look for.
fn mirror(src: &Path, dest: &Path) -> std::io::Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    copy_dir(src, dest)
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        // `file_type()` does not follow symlinks, so a symlinked directory falls
        // to the `copy` branch and is dereferenced there. The SvelteKit output
        // contains neither, but the distinction keeps this from recursing into
        // something unbounded if that ever changes.
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
