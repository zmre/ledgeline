# Playbook: making a Nix binary cache actually useful to other people

Project-agnostic. Copy this into any flake repo that wants
`nix run github:you/project` to download rather than compile. Substitute your
own cache name for `<CACHE>` and project name for `<PROJ>`.

Written 2026-08-04 against Cachix + GitHub Actions. Pricing and free-tier terms
move; re-check anything with a number in it.

---

## The two jobs a cache does, and why conflating them breaks it

A binary cache is serving two audiences with opposite characteristics:

| | CI dependency layer | Consumer release artifacts |
| --- | --- | --- |
| Size | large (often GBs) | small (tens of MB) |
| Churn | every lockfile/toolchain bump | only on release |
| Downloads | every CI run — always warm | rare — often cold for weeks |
| Value if lost | a slow CI run | **your install instructions are a lie** |

Cachix's free tier garbage-collects **least-recently-used**. Under LRU the
frequently-touched dependency layer survives and the rarely-downloaded release
artifact is evicted first — the exact inverse of what you want. If you take one
thing from this document: **LRU will delete precisely the thing you care about,
and it will look like your CI push is broken when it isn't.**

---

## Step 1 — Work out what consumers actually resolve to

This is the step everyone gets wrong. `nix build .#<PROJ>` in CI often builds
something *no user ever installs*. Enumerate it explicitly:

| User command | Flake attribute |
| --- | --- |
| `nix build github:you/proj` | `packages.<system>.default` |
| `nix profile install github:you/proj` | `packages.<system>.default` |
| `nix run github:you/proj` | `apps.<system>.default` (**often a different derivation**) |
| `nix develop` | `devShells.<system>.default` |

Check per-system — a flake that overrides `default` on darwin (an `.app`
bundle, say) has a completely different graph there than on Linux. Print the
truth rather than trusting the flake source:

```sh
for s in x86_64-linux aarch64-darwin; do
  nix eval --raw ".#packages.$s.default.outPath"; echo
  nix eval --raw ".#apps.$s.default.program"; echo
done
```

### Gotcha: a closure is not "everything reachable from the attribute"

Nix computes runtime references by **scanning build outputs for store-path
strings**. If a wrapper `cp`s a binary into place rather than symlinking it,
the original derivation's store path is *not* a reference, and pushing the
wrapper does **not** push the binary. Verify, don't assume:

```sh
outer=$(nix eval --raw .#someWrapper.outPath)
inner=$(nix eval --raw .#theBinary.outPath)
nix path-info -r "$outer" | grep -qF "$inner" \
  && echo "in closure — pushing the wrapper is enough" \
  || echo "NOT in closure — must push it separately"
```

Anything that fails that check needs to be named in its own right when you push
and pin.

---

## Step 2 — Build the consumer artifacts in CI

You cannot cache what you never built. If `packages.default` differs by
platform, add a platform-guarded build step:

```yaml
      - name: Build the real distributable
        if: runner.os == 'macOS'
        run: nix build -L --no-link .#macDist
```

`--no-link` keeps a previously-created `result` symlink pointing where later
steps expect it.

---

## Step 3 — Push, filtering out what you don't need

```yaml
      - uses: cachix/cachix-action@v17
        with:
          name: <CACHE>
          authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}
          pushFilter: '-(source|vendor)$'
```

The action's post-job daemon uploads everything the job built, so this belongs
on **every** job, not just the build one.

- `pushFilter` is a regex over store-path names. Source trees and vendor dirs
  are pure quota waste: consumers substituting a finished output never need
  them. Caveat from the docs: a filtered path is still uploaded if it is part
  of another path's closure.
- **You do not need to filter nixpkgs.** Cachix has skipped paths available
  from `cache.nixos.org` by default since 2020 (configurable upstream list
  since Dec 2025). If your cache is full, it is full of *your* build products.

An explicit push step is worth keeping alongside the daemon so intent is
reviewable, and so it can be scoped to `push` events only:

```yaml
      - name: Push to Cachix
        if: github.event_name == 'push'
        env:
          CACHIX_AUTH_TOKEN: ${{ secrets.CACHIX_AUTH_TOKEN }}
        run: |
          if [ -z "${CACHIX_AUTH_TOKEN:-}" ]; then
            echo "No CACHIX_AUTH_TOKEN (fork or unset) — skipping cache push."
            exit 0
          fi
          attrs=(.#<PROJ>)
          if [ "$RUNNER_OS" = "macOS" ]; then
            attrs+=(.#macDist .#theBinary)   # per the closure check in Step 1
          fi
          nix build --json "${attrs[@]}" \
            | jq -r '.[].outputs | to_entries[].value' \
            | cachix push <CACHE>
```

Always guard on the token being non-empty. Secrets are absent on fork PRs, and
without the guard every outside contributor's PR goes red. Note the `if`
block rather than `[ ... ] && attrs+=(...)`: under `bash -e` a trailing `&&`
whose test fails makes the whole step exit non-zero.

---

## Step 4 — Pin the consumer artifacts (the actual fix)

```yaml
      - name: Pin release artifacts (exempt from GC)
        if: github.event_name == 'push'
        env:
          CACHIX_AUTH_TOKEN: ${{ secrets.CACHIX_AUTH_TOKEN }}
        run: |
          if [ -z "${CACHIX_AUTH_TOKEN:-}" ]; then
            echo "No CACHIX_AUTH_TOKEN — skipping pins."
            exit 0
          fi
          sys=$(nix eval --raw --impure --expr builtins.currentSystem)
          cachix pin <CACHE> "<PROJ>-$sys" \
            "$(nix build --no-link --print-out-paths .#<PROJ>)" --keep-revisions 2
```

```
cachix pin CACHE-NAME PIN-NAME STORE-PATH [--keep-days N | --keep-revisions N | --keep-forever]
```

- **Pinned paths are exempt from garbage collection.** This is the whole trick.
- A pin name is the *stable identity* of "current release artifact for this
  system"; Cachix retains `--keep-revisions` of each, so re-pinning the same
  name on every push keeps a short history rather than accumulating forever.
- Prefix pin names with the project if the cache is shared across repos.
- Derive `$sys` instead of hardcoding — runner architectures change.
- **Do not pin the dependency layer.** It is the biggest thing you push, it
  churns constantly, CI keeps it warm by itself, and losing it costs one slow
  build. Let LRU have it. Pinning it defeats the point.

---

## Step 5 — Tell consumers how to opt in

Pushing is only half of it. Nix **ignores substituters declared in a flake's
`nixConfig` unless the flake is trusted**, so a stranger running your install
command builds from source and never knows why.

In the README, either use `--accept-flake-config`:

```sh
nix run --accept-flake-config github:you/project
```

or give them the permanent form for `~/.config/nix/nix.conf`:

```
extra-substituters = https://<CACHE>.cachix.org
extra-trusted-public-keys = <CACHE>.cachix.org-1:AAAA...
```

**The caveat that has to be in the README:** `substituters` is a *trusted*
setting. On a multi-user install, a user not in `trusted-users` gets
`ignoring untrusted flake configuration setting 'extra-substituters'` and
builds from source regardless of which method they used.

A cache is plain signed HTTP — **the Nix implementation is irrelevant**.
Determinate Nix, upstream Nix, Lix and nix-darwin all consume it identically.
There is no reason to push to multiple caches "to support different machines".
(The lone exception is FlakeHub Cache, which requires `determinate-nixd login`
— which is also why it's a poor choice for public distribution.)

---

## Step 6 — Verify from outside, without building

Store paths are content-addressed, so evaluating a revision on *any* machine
reproduces exactly what CI built. That makes coverage checkable in seconds:

```sh
REV=github:you/project/<commit-sha>
for s in x86_64-linux aarch64-darwin; do
  p=$(nix eval --raw "$REV#packages.$s.default.outPath")
  code=$(curl -s -o /dev/null -w '%{http_code}' \
    "https://<CACHE>.cachix.org/$(basename "$p" | cut -d- -f1).narinfo")
  echo "$code  $s  $(basename "$p")"
done
```

`200` = a consumer gets a download. `404` = they compile.

To sweep an entire dependency graph (catches an unpushed dep layer):

```sh
nix derivation show -r "$REV#packages.x86_64-linux.default" > /tmp/drvs.json
jq -r '.derivations | to_entries[] | select(.key|test("<PROJ>")) | .value.outputs.out.path' /tmp/drvs.json \
  | sort -u | while read -r o; do
      echo "$(curl -s -o /dev/null -w '%{http_code}' \
        "https://<CACHE>.cachix.org/$(basename "$o" | cut -d- -f1).narinfo")  $(basename "$o")"
    done
```

### Reading the result

A `404` for a path CI demonstrably pushed means **eviction, not push failure**.
Before touching the workflow, confirm push is working by grepping a CI log for
`copying path '...' from 'https://<CACHE>.cachix.org'` (a pull proves the
round-trip) and for `Nothing to push - all store paths are already on Cachix`
(proves auth). If both appear, the workflow is fine and the problem is
retention — check the cache's usage/GC page.

---

## When to move off the free tier

Try pins + `pushFilter` first; for a small project that is usually the end of
it. If you still run out:

| Option | Free tier | Public consumers? | Notes |
| --- | --- | --- | --- |
| Second Cachix cache for CI churn, public cache as its *upstream* | — | ✅ | Avoids double storage. Confirm with Cachix whether 5 GB is per-cache or per-account — **unverified**. |
| Garnix | $0, 1,500 CI min/mo, no published cap, ~3mo retention | ✅ no auth for public repos | CI + cache in one. Darwin is **opt-in** — default build set is Linux-only. |
| Cloudflare R2 | 10 GB, $0 egress | ✅ | ~$0/mo at small scale. Needs a custom domain (`r2.dev` rate-limits), and you must upload `nix-cache-info` yourself — `nix copy` writes only `StoreDir`. You own GC. |
| Attic (self-hosted) | free software | ✅ | Still `0.1.0`, no releases ever, patchy maintenance. Real ops burden. |
| FlakeHub Cache | none | ⚠️ requires login | Breaks anonymous `nix run`. |
| `cache-nix-action` / `magic-nix-cache` | GHA cache, 10 GB/repo | ❌ **impossible** | Needs a per-job token; no anonymous endpoint. CI-only by construction. GitHub Free cannot buy past 10 GB. |

---

## Checklist

- [ ] Enumerated what `nix build` / `nix run` / `nix profile install` resolve to, **per system**
- [ ] Closure-checked any wrapper that `cp`s rather than symlinks
- [ ] CI builds those attributes (platform-guarded where they differ)
- [ ] `pushFilter` on every `cachix-action` block
- [ ] Token-absent guard on every push/pin step (fork PRs)
- [ ] Consumer artifacts pinned; dependency layer **not** pinned
- [ ] README documents `--accept-flake-config` **and** the `trusted-users` caveat
- [ ] Verified from outside with the `narinfo` check after the first green run
