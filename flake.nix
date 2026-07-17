{
  description = "Ledgeline — a modern web GUI for hledger";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        # Rust toolchain for the journal engine (crates/); pinned in rust-toolchain.toml.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain # Rust engine: crates/ledgeline-{core,server}
            nodejs_22 # runtime for vite/svelte tooling
            bun # package manager + script runner
            hledger # CLI: golden fixture generation, journal validation, differential oracle
            hledger-web # JSON API server for local dev + e2e + wire-parity oracle
            just # task runner (see justfile)
            playwright-driver.browsers # browsers for playwright e2e (version must match web/package.json @playwright/test)
          ];

          shellHook = ''
            export LEDGELINE_FIXTURE="$PWD/fixtures/sample.journal"
            export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            echo "ledgeline dev shell: node $(node --version), bun $(bun --version), $(hledger --version | head -1), $(rustc --version)"
          '';
        };
      });
}
