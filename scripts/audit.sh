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
  # unlike the quick-xml pair below there is no CI failure to suppress.
  #
  # WHAT WOULD MAKE THIS ACTIONABLE: any of tao / wry / muda / gtk / gdk /
  # webkit2gtk starting to iterate a string-array GVariant (XDG portal replies,
  # GSettings string lists and GTK action targets are the plausible routes).
  # That risk arrives with a dependency bump, so re-run the grep above whenever
  # wry/tao move — not on a calendar. The real fix is upstream: wry/tao
  # migrating Linux off GTK3. gtk3-rs topped out at 0.18.2 and requires
  # glib "^0.18", so no lockfile change can reach glib 0.20 — which is also why
  # Dependabot cannot open a PR for it.
  #
  # RUSTSEC-2026-0194 / RUSTSEC-2026-0195 — quick-xml 0.39.4, two "7.5
  # high" denial-of-service issues (quadratic duplicate-attribute scan;
  # unbounded namespace-declaration allocation).
  #
  #   WHY IGNORED — unreachable at run time AND unfixable from this repo:
  #     * The only path is
  #         quick-xml -> wayland-scanner (PROC-MACRO) -> wayland-client
  #         -> rfd -> ledgeline
  #       wayland-scanner is a build-time proc macro, so quick-xml parses
  #       the vendored wayland protocol .xml files during compilation. It
  #       never sees a journal file, an HTTP request, or any other runtime
  #       input, so neither DoS is reachable by an attacker.
  #     * rfd gates its wayland deps on cfg(target_os = "linux" | the
  #       BSDs), so the crate is absent from the macOS graph entirely —
  #       `cargo tree -i quick-xml` there reports "nothing to print". It is
  #       only in Cargo.lock because the lockfile is target-agnostic.
  #     * The advisories' fix is quick-xml >= 0.41, but wayland-scanner
  #       0.31.10 requires "^0.39". No `cargo update` can reach it.
  #
  #   WHAT WOULD MAKE THIS ACTIONABLE: a wayland-scanner release that
  #   widens its quick-xml requirement past 0.39 (or an rfd bump that drops
  #   wayland-scanner). Delete the two --ignore flags then and let the
  #   lockfile take the fix. Re-check whenever this list is touched.
  nix develop -L .#audit -c cargo-audit audit \
    --ignore RUSTSEC-2026-0194 \
    --ignore RUSTSEC-2026-0195
}

spa_audit() {
  # All six are in build/test tooling, or in code paths Vite's browser
  # build drops. None reaches a user. Ledgeline ships ONE artifact out of
  # web/: the static bundle in web/build, embedded into the Rust binary by
  # rust-embed and served by our own axum server. There is no Node runtime
  # and no SvelteKit server — adapter-static emits only _app/, index.html,
  # robots.txt and an icon.
  #
  # Every claim below was CHECKED against the built bundle rather than
  # assumed, with `grep -r <marker> web/build/_app`.
  #
  #   postcss  GHSA-r28c-9q8g-f849 (high), GHSA-fxqj-rqcc-2cmp (moderate)
  #   nanoid   GHSA-28wg-ghj8-5hjv (high), GHSA-2v37-7h3g-55p8 (high)
  #     Reached only via `vite > postcss [> nanoid]` and
  #     `eslint-plugin-svelte > postcss` — build-time CSS tooling. Both
  #     "postcss" and "nanoid" appear in ZERO bundle files. The postcss
  #     issues are sourceMappingURL path traversal during a build, which
  #     only ever reads our own sources.
  #
  #   uuid  GHSA-w5hq-g745-h8pq (moderate)
  #     `exceljs > uuid`, and exceljs does ship. The advisory requires uuid
  #     v3/v5/v6 called WITH a `buf` argument; the v3/v5 namespace
  #     constants ("DNS:") and the v4 bit-twiddling are both absent from
  #     the bundle, so the vulnerable entry points are not in it.
  #
  #   cookie  GHSA-pxg6-pf52-xh8x (low)
  #     SERVER-side: kit's cookie handling. There is no server here —
  #     adapter-static emits no server bundle, and "sameSite", "Max-Age"
  #     and "httpOnly" appear in ZERO bundle files.
  #
  #   WHAT WOULD MAKE THESE ACTIONABLE: there is nothing to move to today.
  #   `bun update --latest` was tried and clears only ONE of the thirteen
  #   (13 -> 12); upstream vite/kit/eslint still pin the vulnerable
  #   transitives. Re-check whenever this list is touched, and drop any
  #   --ignore whose package has since been bumped past the advisory.
  #
  #   NEW ADVISORY WITH A PATCH RELEASE: fix the lockfile, don't add an
  #   --ignore. `bun update` re-resolves the whole tree (and rewrites
  #   package.json ranges), so for a transitive bump hand-edit the one
  #   bun.lock entry instead — version + `dist.integrity` from
  #   registry.npmjs.org/<pkg>/<ver> — then `bun install --frozen-lockfile`
  #   to prove bun accepts it, then `just audit`.
  nix develop -L .#spaAudit -c bash -c 'cd web && bun audit \
    --ignore=GHSA-r28c-9q8g-f849 \
    --ignore=GHSA-fxqj-rqcc-2cmp \
    --ignore=GHSA-28wg-ghj8-5hjv \
    --ignore=GHSA-2v37-7h3g-55p8 \
    --ignore=GHSA-w5hq-g745-h8pq \
    --ignore=GHSA-pxg6-pf52-xh8x'
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
