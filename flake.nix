{
  description = "Ledgeline — a modern web GUI for hledger";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            nodejs_22 # runtime for vite/svelte tooling
            bun # package manager + script runner
            hledger # CLI: golden fixture generation, journal validation
            hledger-web # JSON API server for local dev + e2e
            just # task runner (see justfile)
            # playwright-driver.browsers  # uncomment when e2e lands (WP-09)
          ];

          shellHook = ''
            export LEDGELINE_FIXTURE="$PWD/fixtures/sample.journal"
            # Playwright-on-nix (enable together with playwright-driver.browsers above):
            # export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
            # export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            echo "ledgeline dev shell: node $(node --version), bun $(bun --version), $(hledger --version | head -1)"
          '';
        };
      });
}
