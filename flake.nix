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

        # --- Linux EGL: keep WebKit's web process from aborting -----------------
        # WebKitGTK 2.52 calls `eglGetDisplay` while constructing a page and
        # `CRASH()`es outright when it gets EGL_NO_DISPLAY, so a blank window and
        # a `WebKitWebProcess` SIGABRT is what a non-NixOS host gets today.
        # libglvnd looks for an EGL vendor ICD in the three directories below, in
        # order. On NixOS the first exists; on Arch (or any other distro) only
        # the third does, and the vendor it names cannot be dlopen'd from a Nix
        # process — `/usr/lib/libEGL_mesa.so.0` needs the host's `libgallium`,
        # which is built against a different glibc and so is not (and must not
        # be) on a Nix binary's search path. glvnd therefore finds no vendor at
        # all and the web process dies before the first paint.
        #
        # No environment variable avoids this. Both WEBKIT_DISABLE_DMABUF_RENDERER
        # and WEBKIT_DISABLE_COMPOSITING_MODE were tried upstream; the abort is in
        # `initializePlatformDisplayIfNeeded`, ahead of either switch.
        #
        # The fix is to ship nixpkgs' own Mesa as a LAST-RESORT vendor: appended
        # to the search path, never substituted for it, so a working host driver
        # still wins wherever there is one. That is what nixGL does, minus the
        # extra tool. `--set-default` then `--suffix` is the whole trick (see
        # `linuxDist`): unset → the platform defaults go in first and the store
        # path lands after them; already set → the user's value is kept and the
        # store path still lands last. Both variables are colon-separated lists.
        #
        # GBM_BACKENDS_PATH matters more than it looks: without it Mesa logs
        #   MESA-LOADER: failed to open dri: /run/opengl-driver/lib/gbm/dri_gbm.so
        # which is not fatal but drops WebKit off the DMA-BUF path onto a slower
        # one. The ordering rule is load-bearing here too — on a NixOS box with a
        # proprietary driver the host's gbm backend is the only correct one.
        #
        # THE COST IS REAL: `pkgs.mesa`'s closure is ~1.0 GiB (most of it LLVM,
        # which every Gallium driver links) on a host whose store has no Mesa
        # already; on NixOS it is close to free. It is therefore paid ONLY by the
        # wrapped `linuxDist` below — `ledgeline` itself stays bare, so `checks`,
        # CI and any release artefact are untouched by this.
        glvndVendorDefaults =
          "/run/opengl-driver/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d";
        mesaEglVendorDir = "${pkgs.mesa}/share/glvnd/egl_vendor.d";
        mesaDriDir = "${pkgs.mesa}/lib/dri";
        mesaGbmDir = "${pkgs.mesa}/lib/gbm";
        gbmBackendsDefault = "/run/opengl-driver/lib/gbm";

        # Imports shell out to `hledger`, which must therefore be findable at
        # RUN time. `hledger::resolve` tries, in order: the user's preference,
        # $LEDGELINE_HLEDGER, this baked path, then $PATH.
        #
        # Baking it is what makes a Nix install zero-config, and the case it
        # really exists for is macOS: an app bundle launched from Finder does
        # NOT inherit the shell's PATH, so a user with hledger installed and
        # working in their terminal would still get "hledger was not found" from
        # Ledgeline.app. The preference exists for everyone else.
        #
        # The cost is honest and worth knowing: this puts hledger's ~158 MiB
        # closure into the binary's runtime closure. To opt out, set this to
        # `null` — resolution then falls through to $PATH and the preference,
        # and the UI's "set hledger path" banner covers the rest.
        hledgerPath = "${pkgs.hledger}/bin/hledger";

        # Applied ONLY to the two outputs that produce a runnable binary. It is
        # deliberately kept out of `commonArgs`: `cargoArtifacts` is the cached
        # third-party dependency layer, and threading a store path through it
        # would invalidate that cache on every hledger bump for no benefit —
        # nothing in the dependency layer reads this.
        hledgerEnv = lib.optionalAttrs (hledgerPath != null) {
          LEDGELINE_HLEDGER_PATH = hledgerPath;
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
        ledgeline = craneLib.buildPackage (commonArgs // hledgerEnv // {
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
        #
        #    A STALE PIN HERE IS INVISIBLE UNTIL THE CACHE DROPS THE PATH. A
        #    fixed-output derivation's store path comes from its hash, not its
        #    inputs, so while the old path is substitutable from Cachix nothing
        #    ever runs the builder and nothing compares. Adding a dev dependency
        #    and forgetting this line therefore passes CI indefinitely and then
        #    fails, on an unrelated commit, whenever the eviction happens — which
        #    is exactly how it went: the `@testing-library/svelte` + `jsdom` bump
        #    landed weeks before the build that reported it.
        # See `outputHash` below for why this is keyed by system.
        # Re-pinned for the toolchain bump two commits back (svelte 5.56.9,
        # vite 8.2.2, playwright 1.61.1 and the rest): the FOD hash covers the
        # resolved `node_modules`, so any dependency change invalidates it.
        #
        # An FOD hash can only be produced ON the platform it describes, and the
        # bump was made on aarch64-darwin. THE x86_64-linux HASH BELOW IS STALE
        # and its first build will fail with a hash mismatch that names the
        # correct value — put that value here. See docs/development.md.
        spaNodeModulesHashes = {
          aarch64-darwin = "sha256-2Ubynne5AlCQkD/dcMWL2UE96Pzy41LgSntwBgUtW/k=";
          x86_64-linux = "sha256-+ibyfS34nA4G/W1CvJQ0cA3LRkrdHvAkRZlwdLsGlXY=";
        };

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
          # PER-SYSTEM by necessity: the tree contains platform-specific native
          # binaries (esbuild, rollup, @tailwindcss/oxide), so the recursive
          # hash differs on every system and can only be produced by building
          # there. A system with no entry gets a build-time error naming itself,
          # rather than a confusing hash mismatch against someone else's platform.
          outputHash = spaNodeModulesHashes.${system} or (throw
            "ledgeline: no spaNodeModules outputHash pinned for ${system}. Build it there and add the hash to `spaNodeModulesHashes` in flake.nix.");
        };

        # 2. The static SPA (`web/build`). Pure/offline: reuses the pinned
        #    node_modules, runs `svelte-kit sync`, then `vite build`
        #    (adapter-static → a client-only bundle with an index.html fallback).
        spaBuild = pkgs.stdenv.mkDerivation {
          pname = "ledgeline-spa";
          inherit version;
          src = ./web;
          # nodejs is here for `patchShebangs` below, not to run the build (bun
          # does that) — it is what the rewritten shebangs point AT.
          nativeBuildInputs = [ pkgs.bun pkgs.nodejs_22 ];
          dontConfigure = true;
          buildPhase = ''
            export HOME="$TMPDIR"
            cp -R ${spaNodeModules}/node_modules ./node_modules
            chmod -R u+w node_modules
            # The `.bin` shims npm/bun generate carry `#!/usr/bin/env node`, and
            # the LINUX build sandbox has no /usr/bin/env — so `svelte-kit` and
            # `vite` died with "bad interpreter". The darwin sandbox does have
            # it, which is the only reason this ever worked there. Repoint them
            # at the nodejs in this build. Done on the COPY, never on the
            # fixed-output `spaNodeModules` itself, whose contents must stay
            # exactly what the pinned hash covers.
            #
            # The WHOLE tree, not just `.bin`: the entries there are SYMLINKS
            # into the packages and `patchShebangs` only rewrites regular files,
            # so pointing it at `.bin` alone reported success and changed
            # nothing.
            patchShebangs node_modules
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
        ledgelineWithSpa = craneLib.buildPackage (commonArgs // hledgerEnv // {
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

        # 5. The `hledger` the DMG SHIPS, so a downloaded Ledgeline can import
        #    without the user installing anything.
        #
        #    WHY BUNDLE AT ALL: imports shell out to hledger, and neither of the
        #    other ways to find one survives distribution. The baked
        #    `LEDGELINE_HLEDGER_PATH` is a `/nix/store/…` path that does not
        #    exist on a user's Mac, and `$PATH` inside a Finder-launched `.app`
        #    is launchd's, not the one their shell exports — so a user with
        #    Homebrew hledger working in their terminal still gets "hledger was
        #    not found". `hledger::sibling_hledger` (resolution step 4) is what
        #    finds this copy.
        #
        #    WHY THE UPSTREAM RELEASE BINARY rather than `pkgs.hledger`: the Nix
        #    one links five non-system dylibs (libz, libncursesw, libiconv,
        #    libgmp, libffi) out of the store, so shipping it would mean
        #    vendoring all five into Contents/Frameworks with @rpath install
        #    names — the very de-nixing dance `macApp` does below, five times
        #    over, across every nixpkgs bump. The binary hledger's own project
        #    publishes links ONLY `/usr/lib/*` and system frameworks, verified by
        #    the assertion below rather than assumed. It is also 83 MiB against
        #    the Nix build's 158 MiB closure.
        #
        #    The asset is per-system so a local `nix build .#macApp` still works
        #    on an Intel Mac; the RELEASE workflow only ever builds aarch64.
        #    Pinned by version + hash, so this is reproducible and offline after
        #    the first fetch — a `fetchurl` is a fixed-output derivation, and the
        #    hash is over the tarball, not over a moving "latest".
        bundledHledgerVersion = "1.52.1";
        hledgerAsset = {
          aarch64-darwin = {
            file = "hledger-mac-arm64.tar.gz";
            hash = "sha256-zS/aAeiz5f3TEsTKyi5VxT8+lmdHtcokWDiTH5xKRQ8=";
          };
          x86_64-darwin = {
            file = "hledger-mac-x64.tar.gz";
            hash = "sha256-FulUy7eS/CS0JxRt9jvthNnUSQ97titR2CRyAxfdMDc=";
          };
        }.${system} or null;

        bundledHledger = pkgs.runCommand "hledger-bundled-${bundledHledgerVersion}" {
          src = pkgs.fetchurl {
            url = "https://github.com/simonmichael/hledger/releases/download/"
              + "${bundledHledgerVersion}/${hledgerAsset.file}";
            inherit (hledgerAsset) hash;
          };
          nativeBuildInputs = [ pkgs.darwin.cctools ];
        } ''
          mkdir -p "$out/bin"
          # The archive holds hledger, hledger-ui and hledger-web plus man/info
          # pages. We run exactly one of them, so extract exactly one — the other
          # two are ~50 MiB of download the user would carry for nothing.
          tar xzf "$src" -C "$TMPDIR" hledger
          install -m 0555 "$TMPDIR/hledger" "$out/bin/hledger"

          # The premise this whole choice rests on, asserted rather than trusted:
          # every library it names must resolve on a stock Mac. If hledger's
          # release build ever starts linking something else, this fails HERE —
          # in the build that introduced it — instead of in dyld on a user's
          # machine after they have downloaded the DMG. `for`, not a piped
          # `while read`: a piped loop body runs in a subshell where `exit 1`
          # would kill only the subshell and let the build go green.
          for lib in $(otool -L "$out/bin/hledger" | awk 'NR > 1 { print $1 }'); do
            case "$lib" in
              /usr/lib/*|/System/Library/*) ;;
              *)
                echo "ERROR: bundled hledger ${bundledHledgerVersion} links a non-system library:" >&2
                echo "         $lib" >&2
                echo "  Ledgeline.app ships this binary to Macs with no Nix and no" >&2
                echo "  Homebrew, so every load command has to resolve from the" >&2
                echo "  base OS. Vendor the library into Contents/Frameworks with" >&2
                echo "  an @rpath install name, or go back to a build that does" >&2
                echo "  not need it." >&2
                exit 1
                ;;
            esac
          done
        '';

        # 6. Assemble Ledgeline.app in the STANDARD nix-darwin app layout:
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

          # The bundled hledger, as a SIBLING of our binary in Contents/MacOS/ —
          # which is precisely where `hledger::sibling_hledger` looks. Not
          # Contents/Resources: `codesign --deep` and notarization both expect
          # executable code under MacOS/, and Resources/ is for data.
          cp ${bundledHledger}/bin/hledger "$app/Contents/MacOS/hledger"
          chmod u+w "$app/Contents/MacOS/hledger"

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

          # The guarantee, asserted rather than assumed. Covers the bundled
          # hledger too: `bundledHledger` already made the same assertion, but
          # this is the check that speaks for the ARTIFACT rather than for one of
          # its inputs, so it is the one that keeps holding if the bundle later
          # grows a third Mach-O.
          for macho in "$bin" "$app/Contents/MacOS/hledger"; do
            if otool -L "$macho" | tail -n +2 | grep -q /nix/store; then
              echo "ERROR: Nix store paths survived de-nixing in $macho:" >&2
              otool -L "$macho" >&2
              exit 1
            fi
          done
        '';

        # 7. Combined darwin install: the `Applications/Ledgeline.app` bundle PLUS
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

        # --- Linux desktop entry + MIME type -----------------------------------
        # Without an XDG desktop entry a launcher has only the bare binary to go
        # on, cannot tell a GUI program from a command-line one, and runs it
        # through a terminal — which is where the "two windows when launched from
        # a launcher" report comes from. `Terminal=false` is the line that fixes
        # it. Nothing here is a code change; Linux has no console-subsystem flag.
        #
        # `MimeType` only does something if some type actually maps to a journal
        # file, and shared-mime-info ships none, so we declare one. Installed to
        # share/mime/packages/, which is where `update-desktop-database` /
        # `update-mime-database` pick it up when the package lands in a profile.
        # `sub-class-of text/plain` keeps journals opening in a text editor for
        # everyone who has not chosen Ledgeline.
        ledgelineMimeXml = pkgs.writeText "ledgeline-mime.xml" ''
          <?xml version="1.0" encoding="UTF-8"?>
          <mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
            <mime-type type="text/x-hledger-journal">
              <comment>hledger journal</comment>
              <sub-class-of type="text/plain"/>
              <glob pattern="*.journal"/>
              <glob pattern="*.hledger"/>
              <glob pattern="*.ledger"/>
            </mime-type>
          </mime-info>
        '';

        # `@out@` becomes the store path in `linuxDist`: an absolute Exec works
        # whether or not the install put `bin/` on $PATH.
        #
        # `StartupWMClass` is what Hyprland/sway/GNOME match a window back to its
        # launcher with. tao sets the app_id to the binary name; MEASURED with
        # `hyprctl clients -j`, which reports `"class": "ledgeline"`.
        #
        # NO `Path=` here, deliberately. mbr needs one because its path argument
        # defaults to the working directory and a launcher's cwd is arbitrary.
        # Ledgeline's `journal` argument is optional and `resolve_journal` falls
        # back to $LEDGELINE_FIXTURE → the most-recently-opened journal that
        # still exists → a relative dev fixture, so a bare launch reopens the
        # user's last journal regardless of cwd and no `Path=` would improve the
        # final fallback anyway.
        #
        # Exactly ONE main category (`Office`); `Finance` is an additional
        # category. Two main categories would list the app twice in menus.
        ledgelineDesktopItem = pkgs.writeText "ledgeline.desktop.in" ''
          [Desktop Entry]
          Type=Application
          Version=1.0
          Name=Ledgeline
          GenericName=Accounting
          Comment=Local hledger GUI — reports, journal editing and CSV imports
          Exec=@out@/bin/ledgeline %f
          Icon=ledgeline
          Terminal=false
          StartupWMClass=ledgeline
          Categories=Office;Finance;
          MimeType=text/x-hledger-journal;
          Keywords=hledger;ledger;accounting;journal;finance;bookkeeping;
        '';

        # --- Linux desktop install (`.#linuxDist`, and `default` on Linux) ------
        # The runnable Linux artefact, and the counterpart of `macDist`: it wraps
        # `ledgelineWithSpa` — the binary with the REAL SvelteKit UI embedded —
        # with the EGL/GBM search-path suffixes documented above, so the WebKit
        # web process finds a loadable vendor ICD and stops aborting on a
        # non-NixOS host.
        #
        # `ledgelineWithSpa`, NOT `ledgeline`: `.#ledgeline` embeds the CI
        # PLACEHOLDER SPA (web/build is absent in the crane sandbox), so a
        # `nix run` off it opened a window reading "Ledgeline SPA not built".
        # That was fine while the real-SPA path was darwin-only, and stopped
        # being fine once this became the Linux install path.
        #
        # This is deliberately a SEPARATE output rather than a `postInstall` on
        # `ledgeline`. `ledgeline` is what `checks` and CI build and what a
        # release artefact should ship: keeping it bare keeps ~1 GiB of Mesa AND
        # the whole bun/SPA build out of the CI closure, while everyone who
        # installs the app gets the wrapper. `nix run .` and `nix profile
        # install` both resolve here.
        #
        # runCommand, not symlinkJoin: the wrapper must REPLACE `bin/ledgeline`,
        # and a symlinkJoin of the unwrapped package would collide with it.
        linuxDist = pkgs.runCommand "ledgeline-${version}"
          {
            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
              pkgs.imagemagick
              pkgs.desktop-file-utils
            ];
            meta = ledgeline.meta;
          } ''
          mkdir -p "$out/bin"
          makeBinaryWrapper ${ledgelineWithSpa}/bin/ledgeline "$out/bin/ledgeline" \
            --set-default __EGL_VENDOR_LIBRARY_DIRS "${glvndVendorDefaults}" \
            --suffix      __EGL_VENDOR_LIBRARY_DIRS : "${mesaEglVendorDir}" \
            --suffix      LIBGL_DRIVERS_PATH        : "${mesaDriDir}" \
            --set-default GBM_BACKENDS_PATH         "${gbmBackendsDefault}" \
            --suffix      GBM_BACKENDS_PATH         : "${mesaGbmDir}"

          # assets/ledgeline.png is 2048². Downsize into each hicolor slot rather
          # than dropping the original into one and claiming a size it is not;
          # 48x48 is the slot menus actually require.
          for s in 32 48 64 128 256 512; do
            dir="$out/share/icons/hicolor/''${s}x''${s}/apps"
            mkdir -p "$dir"
            magick ${./assets/ledgeline.png} -resize "''${s}x''${s}" "$dir/ledgeline.png"
          done

          mkdir -p "$out/share/mime/packages"
          cp ${ledgelineMimeXml} "$out/share/mime/packages/ledgeline.xml"

          mkdir -p "$out/share/applications"
          substitute ${ledgelineDesktopItem} "$out/share/applications/ledgeline.desktop" \
            --subst-var out

          # Catches a bad Categories= list, a missing trailing semicolon and the
          # rest of the entry's sharp edges at BUILD time rather than as an app
          # that silently never appears in a menu.
          desktop-file-validate "$out/share/applications/ledgeline.desktop"
        '';
      in
      {
        # Buildable outputs. `nix build .#ledgeline` proves the GUI deps resolve
        # (webkitgtk on Linux, system WebKit on macOS); the checks reuse the
        # cached dependency layer.
        packages = {
          inherit ledgeline clippy fmt tests;
          default = ledgeline;
        }
        # Linux-only: the wrapped desktop install, plus the SPA-in-Nix pieces it
        # is built from. `default` is OVERRIDDEN to it so `nix build` /
        # `nix profile install` give a `bin/ledgeline` that (a) embeds the REAL
        # SvelteKit UI and (b) has a WebKit web process that can actually create
        # an EGL display off NixOS. `.#ledgeline` stays the bare,
        # placeholder-SPA binary on every system (CI + releases).
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          inherit linuxDist spaNodeModules spaBuild ledgelineWithSpa;
          default = linuxDist;
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
          inherit macApp macDist spaNodeModules spaBuild ledgelineWithSpa ledgelineIcns bundledHledger;
          default = macDist;
        };

        # `nix flake check` runs all of these; CI invokes them individually
        # (`nix build .#{fmt,clippy,tests,ledgeline}`) — the bare attr resolves to
        # the current system automatically.
        checks = {
          inherit ledgeline clippy fmt tests;
        };

        # `nix run .` → the REAL app on BOTH platforms, so
        # `nix run github:zmre/ledgeline -- ~/finance/2026.journal` opens the real
        # GUI on that journal anywhere. darwin runs `ledgelineWithSpa` directly;
        # Linux runs `linuxDist`, which wraps that same real-SPA binary with the
        # EGL/GBM suffixes — an unwrapped `nix run` aborts in the WebKit web
        # process on any host without /run/opengl-driver.
        #
        # Linux used to run the PLACEHOLDER-SPA `ledgeline` here, because the
        # `spaNodeModules` FOD hash was pinned for aarch64-darwin only. It is now
        # keyed by system (see `spaNodeModulesHashes`), so both platforms get the
        # actual SvelteKit UI.
        #
        # `name` is passed EXPLICITLY and is load-bearing. `mkApp` defaults it to
        # `drv.pname or drv.name`, and only the crane outputs carry a `pname` —
        # `linuxDist` is a `runCommand`, which has just `name =
        # "ledgeline-${version}"`. Leaving it to infer therefore pointed
        # `nix run` at `bin/ledgeline-0.1.0`, a path that does not exist, and it
        # failed for everyone on Linux while `nix build` / `nix profile install`
        # stayed fine. Nothing in `nix flake check` builds an app's program path,
        # so this is not caught by the gate; say the name out loud instead.
        apps.default = flake-utils.lib.mkApp {
          drv = if pkgs.stdenv.isDarwin then ledgelineWithSpa else linuxDist;
          name = "ledgeline";
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
            playwright-driver.browsers # browsers for playwright e2e (version must match web/package.json @playwright/test — asserted in shellHook)
          ];

          # Desktop GUI (wry/tao) native deps — the SAME list the package builds
          # against, rather than a hand-maintained subset. The subset that used
          # to be here was missing `xdotool`, so `cargo build` in this shell died
          # at link time with `unable to find library -lxdo` (tao links libxdo)
          # and the GUI could only ever be built through `nix build`. Sharing the
          # one list is what stops the two drifting apart again.
          # macOS uses the system WKWebView, so this is empty there.
          inherit buildInputs;

          shellHook = ''
            export LEDGELINE_FIXTURE="$PWD/fixtures/sample.journal"
            export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            ${lib.optionalString pkgs.stdenv.isLinux ''
            # `cargo run` in this shell links the SAME Nix WebKitGTK as the
            # packaged binary and hits the same EGL abort, so the dev shell needs
            # the same last-resort Mesa vendor that `linuxDist` wraps in. The
            # `''${VAR:-…}` / `''${VAR:+…}` forms reproduce the wrapper's
            # `--set-default` + `--suffix` ordering: a value the user (or nixGL)
            # already exported is kept, and the store path is still appended
            # LAST so a working host driver keeps winning.
            export __EGL_VENDOR_LIBRARY_DIRS="''${__EGL_VENDOR_LIBRARY_DIRS:-${glvndVendorDefaults}}:${mesaEglVendorDir}"
            export LIBGL_DRIVERS_PATH="''${LIBGL_DRIVERS_PATH:+''${LIBGL_DRIVERS_PATH}:}${mesaDriDir}"
            export GBM_BACKENDS_PATH="''${GBM_BACKENDS_PATH:-${gbmBackendsDefault}}:${mesaGbmDir}"
            ''}

            # The e2e BROWSERS come from nixpkgs (above) while the RUNNER comes
            # from web/package.json, and Playwright refuses to launch a browser
            # build its runner does not expect — so these two have to move
            # together. `@playwright/test` is pinned EXACTLY (no caret) for that
            # reason.
            #
            # This is asserted rather than merely documented because the failure
            # is remote from the cause and reads like a broken checkout: a
            # `nix flake update` bumps playwright-driver, and every one of the 65
            # e2e specs then fails in about a millisecond with "Executable
            # doesn't exist at /nix/store/…", telling you to run
            # `npx playwright install` — which is the one thing that must not be
            # done here. Exactly that happened on 2026-08-20 (driver 1.60.0 →
            # 1.61.1). CI never sees it: the e2e job installs its own browsers
            # with `bunx playwright install`, so this is a dev-shell-only trap.
            #
            # A warning, not an error: a mismatched shell is still perfectly good
            # for everything that is not e2e, and refusing to open would be worse
            # than saying so.
            if [ -f web/package.json ]; then
              pinnedPlaywright=$(sed -n -E 's/.*"@playwright\/test": "([^"]+)".*/\1/p' web/package.json | head -1)
              if [ -n "$pinnedPlaywright" ] \
                 && [ "$pinnedPlaywright" != "${pkgs.playwright-driver.version}" ]; then
                echo "WARNING: Playwright version drift — e2e will fail."
                echo "  web/package.json @playwright/test : $pinnedPlaywright"
                echo "  nixpkgs playwright-driver         : ${pkgs.playwright-driver.version}"
                echo "  Fix: set @playwright/test to ${pkgs.playwright-driver.version} in web/package.json,"
                echo "       run 'bun install', then re-pin spaNodeModules' outputHash in flake.nix."
              fi
            fi

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

        # Minimal shell for the `spa-audit` CI job: bun ALONE, for exactly the
        # reason `devShells.audit` above is minimal — `devShells.default` would
        # drag in the Rust toolchain, hledger, hledger-web and the Playwright
        # browser bundle, none of which a lockfile scan has any use for.
        #
        # Also deliberately NOT a `checks.` derivation, same as the Rust audit:
        # `bun audit` queries the GitHub advisory API over the network at run
        # time and the Nix build sandbox has none, so it can only run in a
        # shell. It needs no `bun install` — it reads web/package.json and
        # web/bun.lock directly, with no node_modules present.
        devShells.spaAudit = pkgs.mkShell {
          nativeBuildInputs = [ pkgs.bun ];
        };
      });
}
