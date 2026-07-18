{
  description = "Ledgeline — a modern web GUI for hledger";

  # Binary caches. `cache.nixos.org` and `nix-community` are public and give an
  # immediate pull benefit. `ledgeline.cachix.org` is OURS: it does not exist
  # until you create it (see docs/development.md → "Cachix setup"). The key below
  # is a PLACEHOLDER — after `cachix use ledgeline` prints the real public key,
  # paste it here, and add the `CACHIX_AUTH_TOKEN` repo secret so CI can push.
  nixConfig = {
    extra-substituters = [
      "https://cache.nixos.org"
      "https://nix-community.cachix.org"
      "https://ledgeline.cachix.org"
    ];
    extra-trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      # PLACEHOLDER — replace with the real key from `cachix use ledgeline`.
      "ledgeline.cachix.org-1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        inherit (pkgs) lib;

        # Rust toolchain for the journal engine (crates/); pinned in rust-toolchain.toml.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        # Crane, driven by our pinned toolchain. This is what gives us the cached
        # dependency layer (`cargoArtifacts`) reused across every check + the build.
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Single source of truth for the version (virtual workspace → workspace.package).
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        # Cleaned source for the workspace crates. Besides the Cargo/Rust files,
        # the integration tests read the repo `fixtures/` tree at RUNTIME (via
        # `CARGO_MANIFEST_DIR/../../fixtures` + `canonicalize()`), so `fixtures/`
        # must survive the source filter or `cargoTest` fails to find them.
        # `web/build` is deliberately excluded — see `spaPlaceholder` below.
        src = lib.cleanSourceWith {
          src = ./.;
          name = "ledgeline-source";
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*/fixtures(/.*)?" path != null);
        };

        # `crates/ledgeline-server/src/spa.rs` embeds the built SvelteKit SPA from
        # `web/build` via `#[derive(RustEmbed)]`. That folder is a git-ignored build
        # artifact and is ABSENT in the Nix sandbox (we do NOT build the SPA in Nix —
        # out of scope). rust-embed fails to COMPILE when the folder is missing, so
        # before every cargo phase we drop in a placeholder `index.html`. Nix/CI
        # binaries therefore embed a placeholder SPA — that is fine: CI proves the
        # Rust compiles/links + tests pass. The real single binary is produced
        # locally with `cd web && bun run build` then `cargo build --release`
        # (see docs/development.md). This mirrors what `build.rs` does on a bare
        # checkout, but does it here too so the crane sandbox never depends on it.
        spaPlaceholder = ''
          mkdir -p web/build
          [ -e web/build/index.html ] || printf '%s\n' \
            '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Ledgeline</title></head><body><h1>Ledgeline SPA not built (Nix/CI placeholder)</h1></body></html>' \
            > web/build/index.html
        '';

        nativeBuildInputs = [ pkgs.pkg-config ]
          ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.apple-sdk ];

        # Desktop GUI (wry/tao/muda/rfd, default-on `gui` feature) native deps.
        # macOS links the system WKWebView via the Apple SDK (nothing extra here).
        # Linux needs the full webkitgtk/gtk/soup stack that wry/tao link against.
        buildInputs = lib.optionals pkgs.stdenv.isLinux (with pkgs; [
          webkitgtk_4_1
          gtk3
          libsoup_3
          glib
          cairo
          pango
          gdk-pixbuf
          atk
          xdotool # provides libxdo, needed by tao
        ]);

        # Args shared by the dependency layer, the binary, and every check.
        commonArgs = {
          inherit src version nativeBuildInputs buildInputs;
          pname = "ledgeline";
          strictDeps = true;
          preBuild = spaPlaceholder;
        };

        # THE CACHING WIN: build only the workspace's third-party dependencies
        # (incl. the whole wry/tao GUI stack) from a dummy source. Source-only
        # changes reuse this layer verbatim, so rebuilds/retests skip recompiling
        # every dependency. Every output below inherits `cargoArtifacts`.
        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
          src = craneLib.cleanCargoSource ./.;
        });

        # The workspace binary (`ledgeline` = axum server + wry/tao GUI). Tests run
        # in the `tests` check, so skip them here.
        ledgeline = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false;
          meta = {
            description = "Ledgeline — local hledger GUI (axum server + wry/tao webview) with embedded SPA";
            homepage = "https://github.com/zmre/ledgeline";
            license = lib.licenses.mit;
            mainProgram = "ledgeline";
            platforms = lib.platforms.unix;
          };
        });

        clippy = craneLib.cargoClippy (commonArgs // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- -D warnings";
        });

        tests = craneLib.cargoTest (commonArgs // {
          inherit cargoArtifacts;
        });

        fmt = craneLib.cargoFmt {
          inherit src version;
          pname = "ledgeline";
        };
      in
      {
        # Buildable outputs. `nix build .#ledgeline` proves the GUI deps resolve
        # (webkitgtk on Linux, system WebKit on macOS); the checks reuse the
        # cached dependency layer.
        packages = {
          inherit ledgeline clippy fmt tests;
          default = ledgeline;
        };

        # `nix flake check` runs all of these; CI invokes them individually
        # (`nix build .#{fmt,clippy,tests,ledgeline}`) — the bare attr resolves to
        # the current system automatically.
        checks = {
          inherit ledgeline clippy fmt tests;
        };

        apps.default = flake-utils.lib.mkApp { drv = ledgeline; };

        # Dev shell — preserved from the pre-crane flake. Every tool the team and
        # the SPA tests depend on stays available; only crane's inputs are new.
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain # Rust engine: crates/ledgeline-{core,server}
            pkg-config # locates the Linux GUI libs below (no-op on macOS)
            nodejs_22 # runtime for vite/svelte tooling
            bun # package manager + script runner
            hledger # CLI: golden fixture generation, journal validation, differential oracle
            hledger-web # JSON API server for local dev + e2e + wire-parity oracle
            just # task runner (see justfile)
            playwright-driver.browsers # browsers for playwright e2e (version must match web/package.json @playwright/test)
          ];

          # Desktop GUI (wry/tao) native deps. Linux links webkitgtk/gtk/libsoup;
          # macOS uses the system WKWebView, so nothing extra is needed there.
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
            webkitgtk_4_1
            gtk3
            libsoup_3
          ]);

          shellHook = ''
            export LEDGELINE_FIXTURE="$PWD/fixtures/sample.journal"
            export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            echo "ledgeline dev shell: node $(node --version), bun $(bun --version), $(hledger --version | head -1), $(rustc --version)"
          '';
        };
      });
}
