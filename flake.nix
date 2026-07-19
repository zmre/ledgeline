{
  description = "Ledgeline — a modern web GUI for hledger";

  # Binary caches. `cache.nixos.org` and `nix-community` are public and give an
  # immediate pull benefit. `zmre.cachix.org` is the shared cache we reuse from
  # zmre/mbr-markdown-browser (its real public key is below). CI pushes to it
  # when the `CACHIX_AUTH_TOKEN` repo secret is present (see docs/development.md →
  # "Cachix binary cache"); pulls work for everyone with no setup.
  nixConfig = {
    extra-substituters = [
      "https://cache.nixos.org"
      "https://nix-community.cachix.org"
      "https://zmre.cachix.org"
    ];
    extra-trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      "zmre.cachix.org-1:WIE1U2a16UyaUVr+Wind0JM6pEXBe43PQezdPKoDWLE="
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

        # --- macOS app bundle (`.#macApp` → Ledgeline.app) ---------------------
        # `.#ledgeline` embeds the CI PLACEHOLDER SPA (web/build is absent in the
        # sandbox). A real distributable must embed the ACTUAL SvelteKit UI, so we
        # build the SPA in Nix (bun) and feed it into a dedicated crane build.
        # This whole block is only ever forced on macOS (see `packages` below).

        # 1. node_modules for the SPA. `bun install` needs the network, so this is
        #    a fixed-output derivation: the recursive `outputHash` pins the exact
        #    dependency tree from `web/bun.lock`. `--ignore-scripts` keeps it
        #    deterministic — the SvelteKit `prepare` (`svelte-kit sync`) runs in
        #    the build below, not here; the native binaries (esbuild, rollup,
        #    @tailwindcss/oxide) are ordinary per-platform packages that land with
        #    no install script. The hash is platform-specific (it captures the
        #    aarch64-darwin native deps); re-pin it if `bun.lock` changes.
        spaNodeModules = pkgs.stdenv.mkDerivation {
          pname = "ledgeline-spa-node-modules";
          inherit version;
          src = ./web;
          nativeBuildInputs = [ pkgs.bun ];
          dontConfigure = true;
          buildPhase = ''
            export HOME="$TMPDIR"
            export BUN_INSTALL_CACHE_DIR="$TMPDIR/bun-cache"
            bun install --frozen-lockfile --no-progress --ignore-scripts
          '';
          installPhase = ''
            mkdir -p "$out"
            cp -R node_modules "$out/"
          '';
          dontFixup = true;
          outputHashMode = "recursive";
          outputHashAlgo = "sha256";
          outputHash = "sha256-pcvCnuTrfQVvT2v9i7Jnj6NgB8fUvfiMX4kcb6dmEWQ=";
        };

        # 2. The static SPA (`web/build`). Pure/offline: reuses the pinned
        #    node_modules, runs `svelte-kit sync`, then `vite build`
        #    (adapter-static → a client-only bundle with an index.html fallback).
        spaBuild = pkgs.stdenv.mkDerivation {
          pname = "ledgeline-spa";
          inherit version;
          src = ./web;
          nativeBuildInputs = [ pkgs.bun ];
          dontConfigure = true;
          buildPhase = ''
            export HOME="$TMPDIR"
            cp -R ${spaNodeModules}/node_modules ./node_modules
            chmod -R u+w node_modules
            bun run prepare
            bun run build
          '';
          installPhase = ''
            mkdir -p "$out"
            cp -R build/. "$out/"
          '';
        };

        # 3. The `ledgeline` binary with the REAL SPA baked in (rust-embed reads
        #    web/build at compile time). Reuses the cached dependency layer, so
        #    only the workspace crates recompile — now against the real UI.
        ledgelineWithSpa = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false;
          preBuild = ''
            mkdir -p web/build
            cp -R ${spaBuild}/. web/build/
          '';
          meta = ledgeline.meta;
        });

        # 4. Icon: assets/ledgeline.png (2048²) → a multi-resolution
        #    ledgeline.icns. imagemagick downsizes to each icns slot; png2icns
        #    (libicns) assembles them — no macOS `iconutil` required, so it builds
        #    in the pure Nix sandbox.
        ledgelineIcns = pkgs.runCommand "ledgeline.icns" {
          nativeBuildInputs = [ pkgs.imagemagick pkgs.libicns ];
        } ''
          for s in 16 32 48 128 256 512 1024; do
            magick ${./assets/ledgeline.png} -resize "''${s}x''${s}" "icon_''${s}.png"
          done
          png2icns "$out" icon_16.png icon_32.png icon_48.png icon_128.png \
            icon_256.png icon_512.png icon_1024.png
        '';

        # 5. Assemble Ledgeline.app in the STANDARD nix-darwin app layout:
        #    `$out/Applications/Ledgeline.app` (mirrors zmre/mbr-markdown-browser,
        #    which installs `$out/Applications/MBR.app`). `nix build .#macApp`
        #    therefore yields `result/Applications/Ledgeline.app` — the location
        #    home-manager / nix-darwin's `copyApplications` expects, and a plain
        #    drag-to-/Applications install. Info.plist gets the workspace version
        #    substituted in and is lint-clean (`plutil -lint`). NOTE: the binary
        #    still links Nix-store dylibs; a signed, relocatable release
        #    (makeBinaryWrapper + `codesign --sign -`, as in mbr's darwin bundle)
        #    is a documented follow-up — this produces the bundle structure with
        #    the real UI embedded.
        macApp = pkgs.runCommand "ledgeline-app" { } ''
          app="$out/Applications/Ledgeline.app"
          mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
          cp ${ledgelineWithSpa}/bin/ledgeline "$app/Contents/MacOS/ledgeline"
          chmod u+w "$app/Contents/MacOS/ledgeline"
          substitute ${./assets/Info.plist.in} "$app/Contents/Info.plist" \
            --subst-var-by version "${version}"
          cp ${ledgelineIcns} "$app/Contents/Resources/ledgeline.icns"
        '';

        # 6. Combined darwin install: the CLI binary (`bin/ledgeline`, real SPA)
        #    PLUS the `Applications/Ledgeline.app` bundle, joined into one output.
        #    A bare `nix build` (or a profile / home-manager install) then puts
        #    BOTH on the system — the CLI on PATH via `bin/`, and the app where
        #    nix-darwin / home-manager's app linking picks it up via
        #    `Applications/`. Both reference the same real-SPA binary.
        macDist = pkgs.symlinkJoin {
          name = "ledgeline-${version}";
          paths = [ ledgelineWithSpa macApp ];
          meta = ledgeline.meta;
        };
      in
      {
        # Buildable outputs. `nix build .#ledgeline` proves the GUI deps resolve
        # (webkitgtk on Linux, system WebKit on macOS); the checks reuse the
        # cached dependency layer.
        packages = {
          inherit ledgeline clippy fmt tests;
          default = ledgeline;
        }
        # macOS-only: the app bundle, the combined `macDist` install, and the
        # SPA-in-Nix pieces they are assembled from. Guarded so `nix flake check`
        # / builds on Linux never force the platform-specific (aarch64-darwin) SPA
        # node_modules FOD. On darwin `default` is OVERRIDDEN to `macDist` —
        # `result/bin/ledgeline` (CLI, real SPA) + `result/Applications/
        # Ledgeline.app` — so a bare `nix build` (or a profile install) puts BOTH
        # the binary on PATH and the app where nix-darwin / home-manager pick it
        # up. `.#macApp` is the app bundle alone. On Linux `default` stays the
        # headless `ledgeline` binary. `.#ledgeline` remains the binary on every
        # system (CI); `apps.default` / `nix run .` run it.
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          inherit macApp macDist spaNodeModules spaBuild ledgelineWithSpa ledgelineIcns;
          default = macDist;
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
