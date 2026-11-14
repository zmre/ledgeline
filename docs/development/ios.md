---
title: The iOS app
description: "Building, running and installing the iOS/iPadOS shell in apple/ios: the build script, the simulator, a device, and the gotchas."
order: 15
tags: [development]
---

# The iOS app

`apple/ios/` is a SwiftUI shell around the same SPA the desktop app embeds
(plan 42). The Rust core (`crates/ledgeline-ffi`, `mobile` feature) runs
in-process, linked as a static library. The app's web view loads
`ledgeline://localhost/`, and a URL scheme handler passes every request to the
core's router. There's no socket and no port. The Swift ↔ Rust contract is in
`crates/ledgeline-ffi/README.md`; the page ↔ Swift bridge is in plan 42.

You need macOS with Xcode 26, the Nix dev shell (it carries the iOS Rust
targets), and `bun`.

## Commands

| Command | What it does |
|---|---|
| `just ios-build` | Builds the SPA, the release staticlibs, the Swift bindings, the xcframework and the simulator app. |
| `just ios-run` | Does the same, then boots the "iPhone 17" simulator, installs the app and launches it. |
| `just ios-test` | Runs the Swift unit tests on the simulator. |
| `just ios-device` | Builds a signed Release app for a device. |
| `just ios-install` | Installs the last device build on `$LEDGELINE_IOS_DEVICE` and launches it. |

Each recipe calls `apple/ios/build.sh <mode>`, and the modes are `rust`, `build`,
`run`, `device`, `install` and `test`. The script's header lists its
environment variables. The ones you'll use most:

- `LEDGELINE_IOS_DEVICE=<udid>`: the phone to build for and install on. Get
  the UDID from `xcrun devicectl list devices`. A development profile needs a
  registered device, and naming the phone lets Xcode register it.
- `LEDGELINE_SIM_DEVICE`: a simulator other than "iPhone 17".
- `LEDGELINE_IOS_SKIP_SPA=1`: reuses `web/build` as it is, which saves the SPA
  build when only Swift changed.

## How the build fits together

1. `bun run build` in `web/`. `crates/ledgeline-server/build.rs` mirrors
   `web/build` into the crate's `spa/`.
2. `scripts/ios-check.sh build` cross-compiles `ledgeline-ffi` for
   `aarch64-apple-ios` and `aarch64-apple-ios-sim`. It always uses the
   **release** profile, whatever the Swift configuration, because rust-embed
   bakes the SPA into release builds only. A debug staticlib would read
   `spa/` from disk at run time, and that path doesn't exist on a phone. The
   script's shims send only the target-side C compiles to Xcode's SDK.
3. The crate's `uniffi-bindgen` bin (feature `bindgen`) writes the Swift
   bindings to `apple/ios/Generated/`, which is checked in. It replaces
   files only when they change.
4. `xcodebuild -create-xcframework` produces `LedgelineFFI.xcframework`.
5. XcodeGen produces `Ledgeline.xcodeproj` from `project.yml`. It comes from
   `nix run nixpkgs#xcodegen` unless one is on `PATH`.
6. `xcodebuild` runs with the Nix shell scrubbed from its environment
   (`env -i`, `DEVELOPER_DIR` set to Xcode's).

The project file, the xcframework and `target/ios-*` are build products and
aren't committed.

## Gotchas

- The Nix shell sets `DEVELOPER_DIR` to a Nix macOS SDK. Never export Xcode's
  `DEVELOPER_DIR` or `SDKROOT` globally, because that breaks Nix's host
  linker. Run `xcrun` and `xcodebuild` for iOS with
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.
- A global `*.sw*` ignore pattern would hide the Swift sources. The repo's
  `.gitignore` re-includes `apple/ios/**/*.swift`.
- `simctl` can't tap. In Debug builds,
  `SIMCTL_CHILD_LEDGELINE_OPEN_SAMPLE=1 xcrun simctl launch booted
  com.zmre.ledgeline.ios` opens the sample ledger at launch, which helps
  with screenshots.
- App logs go to stderr under `simctl launch --console-pty` and to the
  unified log (subsystem `com.zmre.ledgeline.ios`). Logging never writes
  journal contents.
