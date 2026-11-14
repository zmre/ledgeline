//! UniFFI's Swift-bindings generator, pinned to this crate's uniffi version.
//! `apple/ios/build.sh` runs it after building the staticlib:
//!
//! ```text
//! cargo run -p ledgeline-ffi --features bindgen --bin uniffi-bindgen -- \
//!     generate --library target/aarch64-apple-ios/release/libledgeline_ffi.a \
//!     --language swift --out-dir apple/ios/Generated
//! ```

fn main() {
    uniffi::uniffi_bindgen_main();
}
