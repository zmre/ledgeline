#!/usr/bin/env bash
# Type-check (or build) the iOS staticlib: `ledgeline-ffi --features mobile`
# for aarch64-apple-ios and aarch64-apple-ios-sim.
#
# Usage:
#   scripts/ios-check.sh            cargo check, both targets
#   scripts/ios-check.sh build      cargo build --release, both targets
#                                   (target/<triple>/release/libledgeline_ffi.a)
#
# Environment:
#   LEDGELINE_XCODE_DEVELOPER_DIR   Xcode's Developer dir
#                                   (default: /Applications/Xcode.app/Contents/Developer)
#
# Needs macOS with Xcode (for the iOS SDKs) and the dev shell's iOS std targets
# (`rustToolchainDev` in flake.nix); re-execs inside `nix develop` if they are
# missing.
#
# Why the shims (ported from mbr's apple/ios/build.sh): `aws-lc-sys` (rustls'
# crypto, via reqwest) compiles C with cc-rs, which runs
# `xcrun --sdk iphoneos --show-sdk-path`. The nix shell points DEVELOPER_DIR and
# SDKROOT at a nix *macOS* SDK, so nix's xcrun cannot answer and the target C
# compiler would get macOS flags. Only the TARGET side is routed to Xcode;
# build scripts and proc macros keep building for the host with nix's
# toolchain. Never export Xcode's DEVELOPER_DIR/SDKROOT globally: that breaks
# nix's host linker.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MODE="${1:-check}"
XCODE_DEV="${LEDGELINE_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

case "$MODE" in
    check) CARGO_ARGS=(check -p ledgeline-ffi --features mobile) ;;
    build) CARGO_ARGS=(build -p ledgeline-ffi --features mobile --release) ;;
    *)
        echo "usage: $0 [check|build]" >&2
        exit 2
        ;;
esac

has_ios_targets() {
    local sysroot
    sysroot="$(rustc --print sysroot 2>/dev/null)" || return 1
    [[ -d "$sysroot/lib/rustlib/aarch64-apple-ios" && -d "$sysroot/lib/rustlib/aarch64-apple-ios-sim" ]]
}

if ! has_ios_targets; then
    if [[ -n "${LEDGELINE_IOS_REEXEC:-}" ]]; then
        echo "error: iOS rust targets missing even inside nix develop (see rustToolchainDev in flake.nix)" >&2
        exit 1
    fi
    echo "iOS rust targets not in this shell; re-running inside nix develop..."
    export LEDGELINE_IOS_REEXEC=1
    exec nix develop "$PROJECT_ROOT" -c "$0" "$@"
fi

if [[ ! -d "$XCODE_DEV" ]]; then
    echo "error: Xcode not found at $XCODE_DEV (set LEDGELINE_XCODE_DEVELOPER_DIR)" >&2
    exit 1
fi

cd "$PROJECT_ROOT"

# A fixed path, not mktemp: the target linker's path is part of cargo's
# fingerprint, so a fresh directory per run would rebuild every crate.
SHIM_DIR="$PROJECT_ROOT/target/ios-shims"
mkdir -p "$SHIM_DIR"

# xcrun that always asks Xcode.
cat >"$SHIM_DIR/xcrun" <<SHIM
#!/bin/sh
DEVELOPER_DIR="$XCODE_DEV" exec /usr/bin/xcrun "\$@"
SHIM
chmod +x "$SHIM_DIR/xcrun"

for target in "${TARGETS[@]}"; do
    case "$target" in
        *-sim) sdk=iphonesimulator ;;
        *) sdk=iphoneos ;;
    esac
    sdk_path="$(DEVELOPER_DIR="$XCODE_DEV" /usr/bin/xcrun --sdk "$sdk" --show-sdk-path)"
    triple_env="${target//-/_}"

    # Apple clang turns the shell's MACOSX_DEPLOYMENT_TARGET into
    # -mmacos-version-min, which clashes with the iOS triple, and the nix
    # SDKROOT is a macOS SDK. The target compiler gets neither.
    cat >"$SHIM_DIR/clang-$target" <<SHIM
#!/bin/sh
unset MACOSX_DEPLOYMENT_TARGET
export SDKROOT="$sdk_path"
DEVELOPER_DIR="$XCODE_DEV" exec /usr/bin/clang "\$@"
SHIM
    chmod +x "$SHIM_DIR/clang-$target"

    echo "== cargo ${CARGO_ARGS[*]} --target $target"
    (
        export PATH="$SHIM_DIR:$PATH"
        export "CC_${triple_env}=$SHIM_DIR/clang-$target"
        export "AR_${triple_env}=/usr/bin/ar"
        export "CARGO_TARGET_$(echo "$triple_env" | tr '[:lower:]' '[:upper:]')_LINKER=$SHIM_DIR/clang-$target"
        export IPHONEOS_DEPLOYMENT_TARGET=17.0
        # The nix shell's RUSTFLAGS carry macOS link args that do not exist on
        # iOS. With --target they only ever applied to target artifacts.
        unset RUSTFLAGS
        # The nix cc wrapper that links host build scripts refuses a DEVELOPER_DIR
        # pointing at Xcode; drop one the caller may have exported.
        if [[ "${DEVELOPER_DIR:-}" == /Applications/* ]]; then
            env -u DEVELOPER_DIR cargo "${CARGO_ARGS[@]}" --target "$target"
        else
            cargo "${CARGO_ARGS[@]}" --target "$target"
        fi
    )
done
