//! Build script: keep the embedded-SPA folder (`spa/`) present and current
//! when the `server` feature is on (the CLI alone embeds nothing).
//!
//! The same three cases as Ledgeline's (`crates/ledgeline-server/build.rs`,
//! which explains why the SPA is mirrored into the crate rather than embedded
//! from `invoicer/web/build` directly):
//!
//! 1. `../../web/build/index.html` exists (`just build-invoicer` ran): mirror
//!    it into `spa/`.
//! 2. Otherwise, a `spa/index.html` already there is left alone.
//! 3. Otherwise, write a placeholder, because `rust-embed` fails to compile
//!    against a missing folder.

use std::path::{Path, PathBuf};

const PLACEHOLDER_INDEX: &str = "<!doctype html>\n\
<html lang=\"en\">\n\
<head><meta charset=\"utf-8\"><title>Ledgeline Invoicer</title></head>\n\
<body>\n\
  <h1>Ledgeline Invoicer SPA not built</h1>\n\
  <p>Run <code>just build-invoicer</code> (or <code>bun run build</code> in\n\
     <code>invoicer/web/</code>), then rebuild the binary.</p>\n\
</body>\n\
</html>\n";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_SERVER").is_none() {
        return;
    }
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
    let spa_dir = manifest_dir.join("spa");
    let web_build = manifest_dir.join("../../web/build");
    if manifest_dir.join("../../web").is_dir() {
        println!("cargo:rerun-if-changed=../../web/build");
    }

    if web_build.join("index.html").is_file() {
        if let Err(err) = mirror(&web_build, &spa_dir) {
            println!(
                "cargo:warning=ledgeline-invoicer: could not sync invoicer/web/build into {}: {err}",
                spa_dir.display()
            );
        }
        return;
    }
    if spa_dir.join("index.html").is_file() {
        return;
    }
    if let Err(err) = std::fs::create_dir_all(&spa_dir)
        .and_then(|()| std::fs::write(spa_dir.join("index.html"), PLACEHOLDER_INDEX))
    {
        println!("cargo:warning=ledgeline-invoicer: could not write the placeholder SPA: {err}");
    }
}

/// Replace `dest` with a copy of `src`: a mirror, not an overlay, so old
/// content-hashed chunks never pile up in the binary.
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
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
