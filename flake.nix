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
        #    substituted in and is lint-clean (`plutil -lint`).
        #
        #    DE-NIXING (the `for lib` loop) — the bundle has to launch on a Mac
        #    with no /nix/store, and dyld refuses to start a binary whose load
        #    commands name paths that do not exist there. The lone non-system
        #    dylib in the link is libiconv, and it is PHANTOM: nixpkgs' darwin
        #    stdenv appends `-liconv` to every link, but this binary imports zero
        #    iconv symbols — asserted from `nm -u` in the loop below, not assumed,
        #    since that premise is what makes the rewrite legal. Retargeting it to
        #    /usr/lib/libiconv.2.dylib is therefore a correction, not a hack —
        #    and that path needs no file on macOS 11+, it resolves out of the
        #    dyld shared cache. The store path is read back OUT of the binary
        #    instead of interpolated from `${pkgs.libiconv}` so it survives every
        #    nixpkgs bump (the hash changes), and — the real point — so the `*)`
        #    branch can FAIL the build on any other store dylib. A future one may
        #    be a genuine dependency that no system path can stand in for;
        #    shipping it silently would yield a bundle that dies on a user's Mac,
        #    so it must be vendored into Contents/Frameworks with an @rpath
        #    install name, never blanket-rewritten. `install_name_tool`
        #    invalidates the linker's ad-hoc signature and arm64 macOS will not
        #    exec a Mach-O whose signature is broken, so re-signing is
        #    load-bearing rather than cosmetic. Ad-hoc signing is still NOT
        #    Developer ID signing + notarization: a publicly distributed build
        #    needs that separate work, and until it lands Gatekeeper will still
        #    complain about a downloaded copy.
        macApp = pkgs.runCommand "ledgeline-app" {
          # A bare darwin `runCommand` has NONE of these: cctools supplies
          # install_name_tool + otool + nm, sigtool supplies codesign.
          nativeBuildInputs = [ pkgs.darwin.cctools pkgs.darwin.sigtool ];
        } ''
          app="$out/Applications/Ledgeline.app"
          bin="$app/Contents/MacOS/ledgeline"
          mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
          cp ${ledgelineWithSpa}/bin/ledgeline "$bin"
          chmod u+w "$bin"
          substitute ${./assets/Info.plist.in} "$app/Contents/Info.plist" \
            --subst-var-by version "${version}"
          cp ${ledgelineIcns} "$app/Contents/Resources/ledgeline.icns"

          # `for`, NOT `otool | while read`: a piped while-loop body runs in a
          # SUBSHELL, so the `exit 1` below would kill only the subshell and the
          # build would go green with the offending dylib still linked.
          # `NR > 1` drops otool's header line — it is the binary's own path,
          # which is itself under /nix/store while the build runs.
          for lib in $(otool -L "$bin" | awk 'NR > 1 && index($1, "/nix/store") == 1 { print $1 }'); do
            case "$lib" in
              */libiconv*.dylib)
                # The retarget is sound ONLY because the link is phantom, and the
                # case pattern above matches a PATH, not that premise — so the
                # premise is asserted here instead of trusted. A future crate that
                # genuinely calls iconv lands in this same branch and would be
                # rewritten just as silently, and whether that breaks depends on
                # which libiconv nixpkgs happens to ship. Today's pin is APPLE's
                # libiconv-113, which exports the same unprefixed
                # `_iconv_open`/`_iconv_close`/`_iconv` as /usr/lib/libiconv.2.dylib,
                # so real imports would survive the swap by luck. GNU libiconv —
                # what `pkgs.libiconv` resolves to elsewhere, and what a nixpkgs
                # bump could put here — exports those entry points as
                # `_libiconv_open`/`_libiconv_close`/`_libiconv` instead. Send real
                # GNU-prefixed imports at Apple's dylib and nothing complains at
                # build time; dyld rejects the bundle for missing symbols at LAUNCH,
                # on a user's Mac. Nothing in this file would notice the flip, hence
                # the check. `nm` output goes to a FILE rather than a pipe into grep
                # so a missing or failing `nm` aborts (set -e) instead of making the
                # assertion vacuously pass — a pipe would also mask nm's status
                # under `set -o pipefail` + grep's early exit. The pattern is
                # CASE-SENSITIVE and anchored on the leading underscore: `grep -i
                # iconv` matches AppKit's `_NSImageNameIconViewTemplate` and would
                # fail every build. `(lib)?` is load-bearing — `_iconv` does not
                # occur as a substring of `_libiconv_open`. Deliberately NOT
                # anchored with `^`: cctools `nm -u` prints bare symbols, other
                # toolchains prefix them with whitespace and `U`.
                nm -u "$bin" > "$TMPDIR/undefined-symbols.txt"
                iconvSyms=$(grep -E '_(lib)?iconv' "$TMPDIR/undefined-symbols.txt" || true)
                if [ -n "$iconvSyms" ]; then
                  echo "ERROR: Ledgeline.app imports real iconv symbols:" >&2
                  echo "$iconvSyms" | sed 's/^/         /' >&2
                  echo "  $lib may NOT be retargeted at /usr/lib/libiconv.2.dylib." >&2
                  echo "  That rewrite is only valid while NOTHING imports iconv." >&2
                  echo "  It is not an ABI-compatible substitution: if pkgs.libiconv" >&2
                  echo "  is (or bumps to) GNU libiconv, it exports _libiconv_open/" >&2
                  echo "  _libiconv_close/_libiconv while Apple's system dylib" >&2
                  echo "  exports the unprefixed _iconv_open/_iconv_close/_iconv." >&2
                  echo "  The build would still go green and the app would die in" >&2
                  echo "  dyld with missing symbols the first time a user opens it." >&2
                  echo "  Vendor the dylib into Contents/Frameworks with an @rpath" >&2
                  echo "  install name instead of retargeting it." >&2
                  exit 1
                fi
                install_name_tool -change "$lib" /usr/lib/libiconv.2.dylib "$bin"
                ;;
              *)
                echo "ERROR: Ledgeline.app would ship a Nix-store dependency:" >&2
                echo "         $lib" >&2
                echo "  Only the phantom libiconv link may be retargeted at a" >&2
                echo "  system path. This one may be REAL, and a bundle carrying" >&2
                echo "  it cannot launch on a Mac without Nix. Vendor it into" >&2
                echo "  Contents/Frameworks with an @rpath install name, or drop" >&2
                echo "  the dependency." >&2
                exit 1
                ;;
            esac
          done

          # install_name_tool just broke the ad-hoc signature the linker gave it.
          codesign -f -s - "$bin"

          # The guarantee, asserted rather than assumed.
          if otool -L "$bin" | tail -n +2 | grep -q /nix/store; then
            echo "ERROR: Nix store paths survived de-nixing:" >&2
            otool -L "$bin" >&2
            exit 1
          fi
        '';

        # 6. Combined darwin install: the `Applications/Ledgeline.app` bundle PLUS
        #    a `bin/ledgeline` that is a SYMLINK INTO the bundle
        #    (`Contents/MacOS/ledgeline`) rather than a second, standalone copy of
        #    the binary. Launching the CLI symlink resolves (via `realpath`) to a
        #    path inside `Ledgeline.app/Contents/MacOS/`, so macOS locates the
        #    bundle's Info.plist + icon and shows the real app icon in the Dock
        #    even when the binary is started from a terminal. Both entry points are
        #    thus the one real-SPA binary embedded in the bundle. A bare `nix build`
        #    (or a profile / home-manager install) still puts BOTH on the system —
        #    the CLI on PATH via `bin/`, and the app where nix-darwin /
        #    home-manager's app linking picks it up via `Applications/`. The
        #    `bin/ledgeline` link is relative so it keeps resolving into the bundle
        #    within whatever prefix the output is installed under.
        macDist = pkgs.symlinkJoin {
          name = "ledgeline-${version}";
          paths = [ macApp ];
          postBuild = ''
            mkdir -p "$out/bin"
            ln -s ../Applications/Ledgeline.app/Contents/MacOS/ledgeline "$out/bin/ledgeline"
          '';
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

        # `nix run .` → the REAL app. On darwin `apps.default` runs
        # `ledgelineWithSpa` — the binary with the ACTUAL SvelteKit UI baked in —
        # so `nix run github:zmre/ledgeline -- ~/finance/2026.journal` opens the
        # real GUI on that journal. On non-darwin it runs `ledgeline` (the
        # PLACEHOLDER-SPA binary): the real-SPA path pulls in the `spaNodeModules`
        # fixed-output derivation whose `outputHash` is PER-PLATFORM and is
        # currently pinned for aarch64-darwin only — the Linux hash can only be
        # produced by building on Linux, so a real-SPA `nix run` on Linux is a
        # documented follow-up (promote spaNodeModules/spaBuild/ledgelineWithSpa
        # to all systems with a per-system outputHash; see docs/development.md).
        # Nix laziness means the `else` branch keeps `ledgelineWithSpa` from ever
        # being forced on Linux, so the darwin-only FOD hash never trips a Linux
        # eval/build. Only `apps.default` changes here — packages.default /
        # .#ledgeline / checks / macApp are untouched.
        apps.default = flake-utils.lib.mkApp {
          drv = if pkgs.stdenv.isDarwin then ledgelineWithSpa else ledgeline;
        };

        # Dev shell — preserved from the pre-crane flake. Every tool the team and
        # the SPA tests depend on stays available; only crane's inputs are new.
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain # Rust engine: crates/ledgeline-{core,server}
            cargo-audit # RUSTSEC advisory scan of Cargo.lock (SEC-14; see the `audit` CI job)
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

        # Minimal shell for the `audit` CI job (SEC-14): cargo-audit ALONE.
        # `devShells.default` would drag in the Rust toolchain, hledger,
        # hledger-web and the Playwright browser bundle — hundreds of MB that a
        # Cargo.lock scan has no use for, and which no other CI job currently
        # builds (they all go through `nix build .#…`). Pinned to the same
        # nixpkgs input as everything else, so CI and `nix develop` agree on the
        # cargo-audit version.
        #
        # NOTE this is deliberately NOT a `checks.` derivation: cargo-audit
        # fetches the RUSTSEC advisory DB from GitHub at run time and the Nix
        # build sandbox has no network, so an audit can only ever run in a
        # shell, never in a build.
        devShells.audit = pkgs.mkShell {
          nativeBuildInputs = [ pkgs.cargo-audit ];
        };
      });
}
