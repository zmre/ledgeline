#!/usr/bin/env bash
# The single owner of "what version is Ledgeline".
#
#   scripts/release.sh current            print the version
#   scripts/release.sh check [vX.Y.Z]     assert every reference agrees
#                                         (and, given a tag, that it matches)
#   scripts/release.sh set X.Y.Z          rewrite every reference
#   scripts/release.sh set X.Y.Z --commit ...and commit + tag it
#
# # Why this exists
#
# The version is not stored once. `[workspace.package] version` is the source of
# truth, but three other places have to agree with it, and each drifts silently
# in its own way:
#
#   * `[workspace.dependencies] ledgeline-core.version` — REQUIRED for
#     `cargo publish` (a path dependency with no version is rejected), and cargo
#     has no `version.workspace` inheritance for it. A stale value here does not
#     break any local build, because `path` wins inside the workspace. It breaks
#     the publish, at the end of a release, after the DMG is already out.
#   * `Cargo.lock` — derived, but only if something re-resolves it. A bumped
#     Cargo.toml with a stale lock makes `--locked` builds fail in CI.
#   * `web/package.json` — invisible to the Rust build entirely, which is why it
#     was already sitting at 0.0.1 against a workspace at 0.1.0 when this script
#     was written.
#
# `check` is wired into CI so drift cannot land, and into the release workflow so
# a tag cannot ship against a tree that disagrees with it.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

CARGO_TOML="Cargo.toml"
PACKAGE_JSON="web/package.json"

die() { echo "error: $*" >&2; exit 1; }

# --- readers -----------------------------------------------------------------
# Each reader prints one version, or nothing if the line it owns has gone
# missing. A missing value is reported as a mismatch rather than crashing, so
# `check` explains what happened instead of dying on an empty variable.

# `^version = "..."` matches ONLY the `[workspace.package]` entry: every other
# version in this file is an inline table value (`serde = { version = "1" }`) or
# a bare string (`ropey = "1.6"`), neither of which starts a line with `version`.
read_workspace_version() {
    sed -n -E 's/^version = "([^"]+)"$/\1/p' "$CARGO_TOML" | head -1
}

read_core_dep_version() {
    sed -n -E 's/^ledgeline-core = \{ path = "crates\/ledgeline-core", version = "([^"]+)" \}$/\1/p' \
        "$CARGO_TOML" | head -1
}

# The FIRST `"version":` in package.json is the package's own; anything later
# would be inside a nested object. `head -1` pins that.
read_package_json_version() {
    sed -n -E 's/^[[:space:]]*"version": "([^"]+)",?$/\1/p' "$PACKAGE_JSON" | head -1
}

# Cargo.lock records both workspace members. They must agree with each other and
# with the workspace version; printing them separately lets `check` say which.
read_lock_version() {
    awk -v crate="$1" '
        $0 == "name = \"" crate "\"" { found = 1; next }
        found && /^version = / { gsub(/[",]/, "", $3); print $3; exit }
    ' Cargo.lock
}

# --- commands ----------------------------------------------------------------

cmd_current() {
    local version
    version=$(read_workspace_version)
    [ -n "$version" ] || die "could not read [workspace.package] version from $CARGO_TOML"
    printf '%s\n' "$version"
}

cmd_check() {
    local expected_tag="${1:-}"
    local version failures=0
    version=$(read_workspace_version)
    [ -n "$version" ] || die "could not read [workspace.package] version from $CARGO_TOML"

    echo "workspace version: $version"

    check_one() {
        local label="$1" actual="$2"
        if [ -z "$actual" ]; then
            echo "  MISSING  $label — could not find the version line at all" >&2
            failures=$((failures + 1))
        elif [ "$actual" != "$version" ]; then
            echo "  DRIFT    $label = $actual (expected $version)" >&2
            failures=$((failures + 1))
        else
            echo "  ok       $label = $actual"
        fi
    }

    check_one "Cargo.toml [workspace.dependencies] ledgeline-core" "$(read_core_dep_version)"
    check_one "web/package.json" "$(read_package_json_version)"
    check_one "Cargo.lock ledgeline-core" "$(read_lock_version ledgeline-core)"
    check_one "Cargo.lock ledgeline" "$(read_lock_version ledgeline)"

    if [ -n "$expected_tag" ]; then
        if [ "$expected_tag" != "v$version" ]; then
            echo "  DRIFT    git tag $expected_tag (expected v$version)" >&2
            failures=$((failures + 1))
        else
            echo "  ok       git tag $expected_tag"
        fi
    fi

    if [ "$failures" -gt 0 ]; then
        echo >&2
        echo "$failures version reference(s) disagree with $CARGO_TOML." >&2
        echo "Run: scripts/release.sh set $version" >&2
        exit 1
    fi
    echo "All version references agree."
}

cmd_set() {
    local version="${1:-}" commit="${2:-}"
    [ -n "$version" ] || die "usage: release.sh set X.Y.Z [--commit]"

    # Semver, and deliberately strict: a tag like `v1.2` or `1.2.3-rc.1 ` would
    # sail through a looser pattern and produce an Info.plist Apple rejects.
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
        || die "'$version' is not a semver version (expected X.Y.Z or X.Y.Z-pre)"

    if [ -n "$commit" ] && [ "$commit" != "--commit" ]; then
        die "unknown argument '$commit' (did you mean --commit?)"
    fi

    # Only enforced for --commit: rewriting files in a dirty tree is a normal
    # thing to do while preparing a release, but committing one is not.
    #
    # "Dirty" DELIBERATELY EXCLUDES the three files this script owns. They are
    # the ones `set X.Y.Z` just wrote, and the documented flow is:
    #
    #     scripts/release.sh set 0.2.0            # writes them, stops
    #     git diff                                # review
    #     scripts/release.sh set 0.2.0 --commit   # commit + tag
    #
    # A blanket `git status --porcelain` check makes that flow IMPOSSIBLE — step
    # one dirties the tree and step three then refuses to run — which is exactly
    # what it did until this was fixed. The guard is meant to catch unrelated
    # work in the tree, not the work it was just asked to do.
    #
    # `-uno` so an untracked scratch file cannot block a release either: only
    # the three paths below are ever committed, so nothing untracked can ride
    # along in the release commit.
    if [ "$commit" = "--commit" ]; then
        local foreign
        foreign=$(git status --porcelain -uno -- \
            ":(exclude)$CARGO_TOML" \
            ":(exclude)Cargo.lock" \
            ":(exclude)$PACKAGE_JSON")
        if [ -n "$foreign" ]; then
            echo "$foreign" >&2
            die "working tree has changes outside the version files (above);
  commit or stash them before 'set $version --commit'"
        fi
    fi

    local previous
    previous=$(read_workspace_version)
    echo "Bumping $previous → $version"

    # `sed -i` is not portable (GNU wants no argument, BSD/macOS wants one), and
    # this repo is developed on macOS and built on Linux. perl -i behaves the
    # same on both.
    perl -pi -e "s/^version = \"\Q$previous\E\"\$/version = \"$version\"/" "$CARGO_TOML"
    perl -pi -e "s/^(ledgeline-core = \{ path = \"crates\/ledgeline-core\", version = )\"[^\"]*\"( \})\$/\${1}\"$version\"\${2}/" "$CARGO_TOML"
    # `unless \$done` so only the package's own version is touched, never a
    # `"version"` key nested deeper in the file.
    perl -pi -e 'BEGIN { our $done = 0 } if (!$done && s/^(\s*"version": ")[^"]*(",?)$/${1}'"$version"'${2}/) { $done = 1 }' "$PACKAGE_JSON"

    # Re-resolve so Cargo.lock's own ledgeline-* entries follow. `--workspace`
    # touches only the local members, so this never silently bumps a third-party
    # dependency as a side effect of a version bump — which is exactly the kind
    # of thing that turns a release into a debugging session.
    echo "Refreshing Cargo.lock…"
    cargo update --workspace --offline --quiet \
        || cargo update --workspace --quiet \
        || die "cargo update failed; is the dev shell active? (nix develop path:.)"

    echo
    cmd_check

    if [ "$commit" = "--commit" ]; then
        echo
        git add "$CARGO_TOML" Cargo.lock "$PACKAGE_JSON"

        # IDEMPOTENT, because the tag is the half that matters and it used to be
        # the half that got lost. If the bump was already committed — by a
        # previous run, or by hand — `git commit` exits 1 with "nothing to
        # commit, working tree clean", and under `set -euo pipefail` that killed
        # the script BEFORE `git tag`. The message reads like success, so the
        # first sign of trouble was `git push` answering "src refspec v0.2.0
        # does not match any".
        #
        # The version files are staged and `check` has already passed, so an
        # empty diff here means HEAD's tree ALREADY carries this version —
        # tagging it is correct, not a workaround.
        if git diff --cached --quiet; then
            echo "Version files already committed at $version — nothing new to commit."
        else
            git commit -m "Release $version"
        fi

        # Tagging is likewise re-runnable, but only when the existing tag agrees
        # with HEAD. A tag pointing somewhere else is a genuine ambiguity — the
        # tag is what the release workflow builds from — so say so and stop
        # rather than moving it silently.
        if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
            local existing head
            # `^{commit}` PEELS the ref. These tags are annotated (see below),
            # so a bare `git rev-parse refs/tags/v0.2.0` returns the tag OBJECT's
            # sha, which never equals a commit sha — making every re-run look
            # like a tag pointing somewhere else.
            existing=$(git rev-parse "refs/tags/v$version^{commit}")
            head=$(git rev-parse "HEAD^{commit}")
            if [ "$existing" = "$head" ]; then
                echo "Tag v$version already exists and points at HEAD."
            else
                die "tag v$version already exists but points at ${existing:0:12}, not HEAD (${head:0:12}).
  Delete it with 'git tag -d v$version' if it is wrong, or check out the
  commit it names. Refusing to move a release tag."
            fi
        else
            # `-m`, always. A BARE `git tag v0.2.0` is lightweight only until
            # someone has `tag.gpgsign = true` in their git config — which this
            # project's maintainer does — and a signed tag is an annotated tag,
            # which needs a message. With no `-m` git opens an editor, and from
            # a script that is:
            #
            #     fatal: no tag message?
            #
            # An annotated tag is the better artifact anyway: it records who
            # tagged and when, `git describe` prefers it, and `git tag -v` can
            # check the signature of what was released.
            git tag -m "Ledgeline $version" "v$version"
        fi

        echo
        echo "Committed and tagged v$version. Nothing has been pushed."
        echo "Push it — which is what starts the release build — with:"
        echo "    git push origin $(git rev-parse --abbrev-ref HEAD) v$version"
    else
        echo
        echo "Files updated. Nothing committed."
        echo "Review with 'git diff', then:"
        echo "    scripts/release.sh set $version --commit"
    fi
}

case "${1:-}" in
    current) cmd_current ;;
    check)   shift; cmd_check "${1:-}" ;;
    set)     shift; cmd_set "${1:-}" "${2:-}" ;;
    *)
        cat >&2 <<'USAGE'
usage: scripts/release.sh <command>

  current              print the workspace version
  check [vX.Y.Z]       assert every version reference agrees; with a tag
                       argument, also assert the tag matches
  set X.Y.Z            rewrite every version reference
  set X.Y.Z --commit   ...and commit + tag (does not push)
USAGE
        exit 2
        ;;
esac
