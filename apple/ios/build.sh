#!/usr/bin/env bash
# Build the Ledgeline iOS/iPadOS app (plan 42). Modelled on mbr's
# apple/ios/build.sh.
#
# Usage:
#   apple/ios/build.sh rust     SPA + Rust staticlibs + Swift bindings + xcframework
#   apple/ios/build.sh [build]  ...then the simulator app (Debug Swift)
#   apple/ios/build.sh run      ...then boot a simulator, install and launch it
#   apple/ios/build.sh device   ...a signed Release build for a device instead
#   apple/ios/build.sh install  install the last `device` build on
#                               $LEDGELINE_IOS_DEVICE and launch it (no build)
#   apple/ios/build.sh test     ...run the Swift unit tests (LedgelineTests) on
#                               the simulator
#
# Environment:
#   LEDGELINE_SIM_DEVICE            simulator for `run` (default: "iPhone 17")
#   LEDGELINE_IOS_DEVICE            device UDID (`xcrun devicectl list devices`):
#                                   `device` builds for it and registers it with
#                                   the team; `install` needs it
#   LEDGELINE_IOS_NO_SIGN           with `device`: compile and link, don't sign
#   LEDGELINE_IOS_SKIP_SPA          skip `bun run build` (reuse web/build as is)
#   LEDGELINE_XCODE_DEVELOPER_DIR   Xcode's Developer dir
#                                   (default: /Applications/Xcode.app/Contents/Developer)
#
# The Rust staticlib is ALWAYS a release build, whatever the Swift
# configuration: rust-embed bakes the SPA into release builds only, and a
# debug build would read `crates/ledgeline-server/spa/` from disk at run time,
# a path that does not exist on a phone.
#
# Runs inside `nix develop` (re-execs itself there if the iOS rust targets are
# missing). The cross-compile shims are scripts/ios-check.sh's; xcodebuild runs
# with the nix shell scrubbed from its environment.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODE="${1:-build}"
XCODE_DEV="${LEDGELINE_XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
BUNDLE_ID=com.zmre.ledgeline.ios
LIB=libledgeline_ffi.a
DERIVED="$PROJECT_ROOT/target/ios-derived-data"
DEVICE_APP="$DERIVED/Build/Products/Release-iphoneos/Ledgeline.app"

case "$MODE" in
    rust | build | run | device | install | test) ;;
    *)
        echo "usage: $0 [rust|build|run|device|install|test]" >&2
        exit 2
        ;;
esac

xcrun_xcode() { DEVELOPER_DIR="$XCODE_DEV" /usr/bin/xcrun "$@"; }

# `install` builds nothing: it needs no nix shell.
if [[ "$MODE" == "install" ]]; then
    if [[ -z "${LEDGELINE_IOS_DEVICE:-}" ]]; then
        echo "error: set LEDGELINE_IOS_DEVICE to the device's UDID (xcrun devicectl list devices)" >&2
        exit 2
    fi
    if [[ ! -d "$DEVICE_APP" ]]; then
        echo "error: no device build at $DEVICE_APP; run '$0 device' first" >&2
        exit 1
    fi
    xcrun_xcode devicectl device install app --device "$LEDGELINE_IOS_DEVICE" "$DEVICE_APP"
    xcrun_xcode devicectl device process launch --device "$LEDGELINE_IOS_DEVICE" "$BUNDLE_ID"
    exit 0
fi

has_ios_targets() {
    local sysroot
    sysroot="$(rustc --print sysroot 2>/dev/null)" || return 1
    [[ -d "$sysroot/lib/rustlib/aarch64-apple-ios" && -d "$sysroot/lib/rustlib/aarch64-apple-ios-sim" ]]
}

if ! has_ios_targets; then
    if [[ -n "${LEDGELINE_IOS_BUILD_REEXEC:-}" ]]; then
        echo "error: iOS rust targets missing even inside nix develop (see rustToolchainDev in flake.nix)" >&2
        exit 1
    fi
    echo "iOS rust targets not in this shell; re-running inside nix develop..."
    export LEDGELINE_IOS_BUILD_REEXEC=1
    exec nix develop "$PROJECT_ROOT" -c "$0" "$@"
fi

cd "$PROJECT_ROOT"

# Host cargo never sees an Xcode DEVELOPER_DIR: the nix cc wrapper that links
# host build scripts refuses one (see scripts/ios-check.sh).
host_cargo() {
    if [[ "${DEVELOPER_DIR:-}" == /Applications/* ]]; then
        env -u DEVELOPER_DIR cargo "$@"
    else
        cargo "$@"
    fi
}

# 1. The SPA. ledgeline-server's build.rs mirrors web/build into spa/, which
# the release staticlib embeds; without it the app serves a placeholder page.
if [[ -z "${LEDGELINE_IOS_SKIP_SPA:-}" ]]; then
    echo "Building the SPA (web/)..."
    (
        cd web
        [[ -d node_modules ]] || bun install --frozen-lockfile
        bun run build
    )
elif [[ ! -f web/build/index.html ]]; then
    echo "error: LEDGELINE_IOS_SKIP_SPA is set but web/build/index.html does not exist" >&2
    exit 1
fi

# 2. The staticlibs (release, both targets), through ios-check.sh's shims.
start=$(date +%s)
LEDGELINE_XCODE_DEVELOPER_DIR="$XCODE_DEV" "$PROJECT_ROOT/scripts/ios-check.sh" build
for target in aarch64-apple-ios aarch64-apple-ios-sim; do
    echo "  $target: $(du -h "target/$target/release/$LIB" | cut -f1)"
done
echo "  Rust: $(($(date +%s) - start))s"

# 3. Swift bindings from the device library, with the crate's own generator
# (a host binary). Written to scratch first and copied only when they differ,
# so an unchanged API leaves the checked-in files (and Xcode's incremental
# build) alone.
echo "Generating Swift bindings..."
GENERATED="$SCRIPT_DIR/Generated"
SCRATCH="$PROJECT_ROOT/target/ios-bindings"
rm -rf "$SCRATCH"
host_cargo run --quiet -p ledgeline-ffi --features bindgen --bin uniffi-bindgen -- \
    generate --library "target/aarch64-apple-ios/release/$LIB" \
    --language swift --out-dir "$SCRATCH"
mkdir -p "$GENERATED"
for file in ledgeline_ffi.swift ledgeline_ffiFFI.h ledgeline_ffiFFI.modulemap; do
    if ! cmp -s "$SCRATCH/$file" "$GENERATED/$file"; then
        cp "$SCRATCH/$file" "$GENERATED/$file"
        echo "  updated Generated/$file (commit it)"
    fi
done

# 4. The xcframework (device + simulator slices, one shared header set).
echo "Creating LedgelineFFI.xcframework..."
HEADERS="$PROJECT_ROOT/target/ios-headers"
rm -rf "$HEADERS" "$SCRIPT_DIR/LedgelineFFI.xcframework"
mkdir -p "$HEADERS"
cp "$GENERATED/ledgeline_ffiFFI.h" "$HEADERS/"
# Inside an xcframework the module map must be named module.modulemap.
cp "$GENERATED/ledgeline_ffiFFI.modulemap" "$HEADERS/module.modulemap"
xcrun_xcode xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/$LIB" -headers "$HEADERS" \
    -library "target/aarch64-apple-ios-sim/release/$LIB" -headers "$HEADERS" \
    -output "$SCRIPT_DIR/LedgelineFFI.xcframework" >/dev/null

[[ "$MODE" == "rust" ]] && exit 0

# The app's version is the workspace's.
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"

# 5. The Xcode project. XcodeGen comes from nixpkgs on demand (not installed
# globally); a global one wins when present.
cd "$SCRIPT_DIR"
echo "Generating Xcode project..."
if command -v xcodegen >/dev/null 2>&1; then
    xcodegen generate --quiet
else
    nix run nixpkgs#xcodegen -- generate --quiet
fi

# xcodebuild with the nix shell scrubbed out of its environment: it must use
# Xcode's SDKs, clang and ld, never the nix wrappers earlier on PATH.
xcodebuild_clean() {
    env -i HOME="$HOME" USER="$USER" TMPDIR="${TMPDIR:-/tmp}" \
        PATH=/usr/bin:/bin:/usr/sbin:/sbin \
        DEVELOPER_DIR="$XCODE_DEV" \
        /usr/bin/xcodebuild "$@"
}

# A fixed derived-data directory per checkout (Xcode's default is keyed by a
# hash of the project path, so every worktree would get its own anyway, but
# this one is findable).
if [[ "$MODE" == "device" ]]; then
    echo "Building for device..."
    signing=(-allowProvisioningUpdates)
    if [[ -n "${LEDGELINE_IOS_NO_SIGN:-}" ]]; then
        signing=(CODE_SIGNING_ALLOWED=NO)
    fi
    # A development profile needs at least one registered device, and the
    # generic destination never registers one: name the phone to register it.
    destination='generic/platform=iOS'
    if [[ -n "${LEDGELINE_IOS_DEVICE:-}" ]]; then
        destination="id=$LEDGELINE_IOS_DEVICE"
        [[ -z "${LEDGELINE_IOS_NO_SIGN:-}" ]] && signing+=(-allowProvisioningDeviceRegistration)
    fi
    xcodebuild_clean -project Ledgeline.xcodeproj -scheme Ledgeline \
        -configuration Release -destination "$destination" \
        -derivedDataPath "$DERIVED" "${signing[@]}" \
        MARKETING_VERSION="$VERSION" build
    echo "App: $DEVICE_APP"
    echo "Install with: LEDGELINE_IOS_DEVICE=<udid> $0 install"
    exit 0
fi

SIM="${LEDGELINE_SIM_DEVICE:-iPhone 17}"
if [[ "$MODE" == "test" ]]; then
    echo "Running the unit tests on $SIM..."
    xcodebuild_clean -project Ledgeline.xcodeproj -scheme Ledgeline \
        -configuration Debug -destination "platform=iOS Simulator,name=$SIM" \
        -derivedDataPath "$DERIVED" MARKETING_VERSION="$VERSION" test
    exit $?
fi

echo "Building for simulator..."
xcodebuild_clean -project Ledgeline.xcodeproj -scheme Ledgeline \
    -configuration Debug -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DERIVED" MARKETING_VERSION="$VERSION" build
APP="$DERIVED/Build/Products/Debug-iphonesimulator/Ledgeline.app"
echo "App: $APP"

if [[ "$MODE" == "run" ]]; then
    simctl() { xcrun_xcode simctl "$@"; }
    simctl boot "$SIM" 2>/dev/null || true
    simctl bootstatus "$SIM" -b >/dev/null
    DEVELOPER_DIR="$XCODE_DEV" open -a Simulator 2>/dev/null || true
    simctl install "$SIM" "$APP"
    echo "Launching (Ctrl-C to stop; app output follows)..."
    simctl launch --console-pty --terminate-running-process "$SIM" "$BUNDLE_ID"
fi
