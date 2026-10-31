#!/usr/bin/env bash
# Advisory scans of both lockfiles: Cargo.lock (cargo-audit, RUSTSEC) and
# web/bun.lock (bun audit, GitHub advisory DB).
#
# This is the ONE place the ignore lists live. CI and the daily scheduled run
# (.github/workflows/audit.yml) and `just audit` / `just pre-push` all call it,
# so a laptop and CI cannot disagree about what is ignored.
#
# Usage: scripts/audit.sh [rust|spa|all]    (default: all)
#
# Each half runs in its own minimal dev shell (`.#audit`, `.#spaAudit`; see
# flake.nix for why they are separate from `.#default`). Both need the network:
# the advisory DBs are fetched at run time, which is also why neither can be a
# `checks.` derivation.
set -euo pipefail

cd "$(dirname "$0")/.."

rust_audit() {
  # RUSTSEC-2024-0429 (glib < 0.20, `VariantStrIter` unsoundness) is one of
  # those twelve, and it is the one Dependabot raises as a repo security alert —
  # GitHub's advisory DB ingests RustSec informational entries without the
  # unsound/vulnerability split, so it looks more urgent there than it is.
  # Investigated 2026-08-31; assessed as NOT AFFECTED, on two independent
  # grounds, both verified rather than reasoned:
  #
  #   * NO CALLER EXISTS. The only entry point is `Variant::array_iter_str()`
  #     (`VariantStrIter::new` is `pub(crate)`). Grepping the whole vendored
  #     registry for `VariantStrIter|array_iter_str` returns nothing outside
  #     glib-0.18.5 itself. tao's `Variant` is `dbus::arg::Variant`, unrelated;
  #     glib's own other uses are `g_variant_get_child_VALUE`, which is not
  #     variadic and not what the advisory is about.
  #   * NOT ON macOS AT ALL. `cargo tree -i -p glib@0.18.5 --target
  #     aarch64-apple-darwin` prints nothing; the GTK subtree is cfg-gated to
  #     Linux/BSD and appears in Cargo.lock only because lockfiles are
  #     target-agnostic.
  #
  # Note the bug is in the RUST BINDING's `unsafe` block (`&p` where `&mut p`
  # was needed), NOT in C glib — so "we link GTK3, therefore we are exposed" is
  # wrong, and is the easy mistake to make here.
  #
  # It is deliberately NOT `--ignore`d: it costs nothing to leave visible, and
  # there is no CI failure to suppress.
  #
  # WHAT WOULD MAKE THIS ACTIONABLE: any of tao / wry / muda / gtk / gdk /
  # webkit2gtk starting to iterate a string-array GVariant (XDG portal replies,
  # GSettings string lists and GTK action targets are the plausible routes).
  # That risk arrives with a dependency bump, so re-run the grep above whenever
  # wry/tao move — not on a calendar (last re-run 2026-10-30, wry 0.57.1 / tao
  # 0.37.2: still nothing). The real fix is upstream: wry/tao migrating Linux off
  # GTK3. gtk 0.19 exists as of 2026-09, but wry, tao, muda and webkit2gtk all
  # still require gtk "^0.18" (and so glib "^0.18"), so no lockfile change can
  # reach a fixed glib — which is also why Dependabot cannot open a PR for it.
  # Bumping our own direct `gtk` to 0.19 would only add a second GTK binding.
  #
  # Nothing is --ignored here. zip 8.6.0's RUSTSEC-2026-0367/0368/0369 were,
  # until calamine 0.37 moved to zip 9 (2026-10-30, zip 9.1.3).
  nix develop -L .#audit -c cargo-audit audit
}

spa_audit() {
  # Neither ignored advisory reaches a user. Ledgeline ships ONE
  # artifact out of web/: the static bundle in web/build, embedded into the Rust binary by
  # rust-embed and served by our own axum server. There is no Node runtime
  # and no SvelteKit server — adapter-static emits only _app/, index.html,
  # robots.txt and an icon.
  #
  # Every claim below was CHECKED against the built bundle rather than
  # assumed, with `grep -r <marker> web/build/_app`.
  #
  #   deepmerge  GHSA-gcqx-w48m-w2h4 (low)
  #     `@sveltejs/vite-plugin-svelte > deepmerge`, a devDependency the Vite
  #     plugin uses at build time to merge its own config objects. Nothing
  #     user-supplied is merged, and deepmerge is not in the bundle
  #     (`isMergeableObject` / `mergeObject` / `deepmerge` absent from
  #     web/build/_app, checked 2026-10-29). No fixed release exists: 4.3.1
  #     is the latest and the advisory covers >=4.2.0 <=4.3.1, which is also
  #     why `bun audit fix` has nothing to offer and main's daily audit went
  #     red on 2026-10-28.
  #
  #   minimatch  GHSA-g3hv-75wg-m249 (moderate, published 2026-10-30)
  #     DoS via globparts amplification in minimatch < 11.0.1. Three copies:
  #     10.x under eslint and typescript-eslint (devDependencies, matching
  #     our own config globs at lint time) and 3.x under
  #     `exceljs > archiver > archiver-utils > glob@7`, which globs only
  #     when archiver adds a directory from disk; Ledgeline never does, and
  #     none of it is in the bundle (`minimatch` / `GLOBSTAR` / `globparts` /
  #     `archiver-utils` / `readdir-glob` absent from web/build/_app,
  #     checked 2026-10-30). There is no 3.x/5.x/10.x backport, and an
  #     `overrides` pin to 11 was tried and BREAKS glob 7 (`glob.sync`
  #     throws reading `rawGlob.charAt`: v11 changed the parsed set glob 7
  #     walks), and bun cannot scope an override to one consumer.
  #
  #   WHAT WOULD MAKE THESE ACTIONABLE: a deepmerge release past 4.3.1
  #   (hand-edit the bun.lock entry as below and drop that --ignore), or
  #   vite-plugin-svelte dropping deepmerge; eslint/typescript-eslint moving
  #   to minimatch 11 AND exceljs/archiver moving off glob 7 (then drop the
  #   minimatch --ignore, overriding to ^11 if only one side moved).
  #   Re-check whenever this list is touched, and drop any --ignore whose
  #   package has since been bumped past the advisory (the postcss and nanoid
  #   ignores went that way on 2026-10-14, cookie GHSA-pxg6-pf52-xh8x with
  #   the SvelteKit 3 upgrade, which moved kit onto cookie 2, and uuid
  #   GHSA-w5hq-g745-h8pq on 2026-10-30 via web/package.json's `overrides`:
  #   exceljs 4.4.0, still the latest, wants uuid 8 but only calls the CJS
  #   `v4`, which uuid 11 still exports — drop that override when exceljs
  #   moves off uuid 8).
  #
  #   NEW ADVISORY WITH A PATCH RELEASE: fix the lockfile, don't add an
  #   --ignore. `bun update` re-resolves the whole tree (and rewrites
  #   package.json ranges), so for a transitive bump hand-edit the one
  #   bun.lock entry instead — version + `dist.integrity` from
  #   registry.npmjs.org/<pkg>/<ver> — then `bun install --frozen-lockfile`
  #   to prove bun accepts it, then `just audit`.
  nix develop -L .#spaAudit -c bash -c 'cd web && bun audit \
    --ignore=GHSA-gcqx-w48m-w2h4 \
    --ignore=GHSA-g3hv-75wg-m249'
}

case "${1:-all}" in
  rust) rust_audit ;;
  spa) spa_audit ;;
  all)
    # Both, even if the first fails, so one run reports everything.
    status=0
    rust_audit || status=1
    spa_audit || status=1
    exit "$status"
    ;;
  *)
    echo "usage: $0 [rust|spa|all]" >&2
    exit 2
    ;;
esac
