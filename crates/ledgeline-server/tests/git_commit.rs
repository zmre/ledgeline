//! `git.rs` against real repositories.
//!
//! Every test here builds a throwaway repository in a `tempfile` directory,
//! `git init`s it, and drives the real binary. Nothing is mocked, because
//! everything worth testing in that module is a claim about what git *actually*
//! does — and several of those claims turned out to be wrong when checked
//! against git 2.55 rather than against its manual.
//!
//! # Why these run by default instead of behind an opt-in
//!
//! The rules-renderer suite is gated behind `LEDGELINE_HLEDGER_RENDER_CHECK`
//! because hledger is a specialist tool that most machines do not have, so
//! running it by default would break `cargo test` for contributors.
//!
//! git is not in that category. It is present on every machine that can clone
//! this repository, it is in the dev shell, and CI cannot check the code out
//! without it. More to the point, the property this file exists to defend —
//! *an import never touches a byte of your unrelated work* — is one where a
//! test that silently does not run is worse than no test at all, and an opt-in
//! variable nobody exports is exactly that.
//!
//! So these **skip if absent** rather than opting in: [`git_available`] gates
//! each test, and a machine with no git prints a notice and passes. `cargo test`
//! stays hermetic in the sense that matters (no network, no user state, no
//! installed toolchain assumed), while the machines that can run these — which
//! is all of them, in practice — do.
//!
//! # Isolation from the developer's own git config
//!
//! `git init` in a temp directory still reads `~/.gitconfig` and
//! `/etc/gitconfig`, so a contributor with `commit.gpgsign = true`, a global
//! `core.hooksPath`, or a global `core.excludesFile` would see failures nobody
//! else does. Rather than mutating the process environment — `set_var` is
//! `unsafe` in edition 2024 and races with every other test thread — each
//! repository neutralises those settings in its **own local config**, which
//! takes precedence. See [`TestRepo::init`].

// `git.rs` is a private module of the server crate, so an integration test
// cannot reach it through the public API. Compiling the file directly is the
// only way to test it before the WP-11 import routes exist to expose it, and it
// works because the module is deliberately self-contained: std and `thiserror`,
// no `crate::` references at all. That self-containment is worth preserving.
#[path = "../src/git.rs"]
mod git;

use git::{FileState, GitError, GitStatus, Repo, git_available};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Announce a skip once per test rather than failing on a machine with no git.
macro_rules! require_git {
    () => {
        if !git_available() {
            eprintln!("skipping: no `git` on PATH");
            return;
        }
    };
}

/// A throwaway repository that deletes itself when the test ends.
struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    /// An initialised repository with a committable identity and every
    /// developer-machine variable pinned locally.
    ///
    /// The `git config` calls are not boilerplate. Local config beats global,
    /// so each one closes a way a contributor's own `~/.gitconfig` could change
    /// the outcome: signing would demand a passphrase nobody can type, a global
    /// `core.hooksPath` would hide the hook the hook test installs, and a global
    /// `core.excludesFile` would make files ignored that these tests expect to
    /// be committable.
    fn init() -> Self {
        let repo = Self {
            dir: TempDir::new().expect("temp dir"),
        };
        repo.git(&["init", "--quiet"]);
        let hooks = repo.path().join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        for setting in [
            ["user.name", "Ledgeline Test"],
            ["user.email", "test@ledgeline.invalid"],
            ["commit.gpgsign", "false"],
            ["tag.gpgsign", "false"],
            ["core.excludesFile", ""],
            ["core.hooksPath", &hooks.to_string_lossy()],
        ] {
            repo.git(&["config", setting[0], setting[1]]);
        }
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn at(&self, name: &str) -> PathBuf {
        self.path().join(name)
    }

    /// Write a file, creating parent directories as needed.
    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.at(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(&path, contents).expect("write file");
        path
    }

    /// Run git in this repository, asserting success. Test scaffolding only —
    /// the code under test never shells out this way.
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Stage and commit everything, for building a starting state.
    fn commit_all(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--message", message]);
    }

    /// `git status --porcelain` as raw lines — the independent oracle these
    /// tests assert against, rather than trusting the module's own parse.
    fn porcelain(&self) -> Vec<String> {
        self.git(&["status", "--porcelain"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The paths touched by `HEAD`.
    fn head_files(&self) -> Vec<String> {
        self.git(&["show", "--name-only", "--format=", "HEAD"])
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn head_subject(&self) -> String {
        self.git(&["log", "-1", "--format=%s"]).trim().to_string()
    }

    fn commit_count(&self) -> usize {
        self.git(&["rev-list", "--count", "HEAD"])
            .trim()
            .parse()
            .unwrap_or(0)
    }

    /// Install an executable hook under the repository's pinned hooks path.
    fn hook(&self, name: &str, script: &str) {
        let path = self.path().join(".git").join("hooks").join(name);
        std::fs::write(&path, script).expect("write hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod hook");
        }
    }

    fn discover(&self) -> Repo {
        Repo::discover(self.path()).expect("a freshly initialised repo is discoverable")
    }
}

/// The state the module reports for one path.
fn state_of(status: &GitStatus, name: &str) -> FileState {
    status
        .files
        .iter()
        .find(|(path, _)| path == name)
        .map(|(_, state)| *state)
        .unwrap_or_else(|| panic!("{name} missing from {:?}", status.files))
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

/// The toplevel is found from anywhere beneath it, and from a path that does
/// not exist yet — which is the normal case for an import's CSV destination.
#[test]
fn discover_finds_the_toplevel_from_anywhere_inside() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("a/b/c/deep.journal", "; deep\n");
    repo.commit_all("init");

    let from_root = Repo::discover(repo.path()).expect("from the root");
    let from_nested_dir = Repo::discover(&repo.at("a/b/c")).expect("from a nested directory");
    let from_nested_file =
        Repo::discover(&repo.at("a/b/c/deep.journal")).expect("from a nested file");
    let from_future_file = Repo::discover(&repo.at("a/b/c/not-written-yet.csv"))
        .expect("from a path that does not exist yet");

    assert_eq!(from_root, from_nested_dir);
    assert_eq!(from_root, from_nested_file);
    assert_eq!(from_root, from_future_file);

    // And it really is *this* repository, not some ancestor that happens to be
    // one. Asserted behaviourally: the toplevel is the directory git named.
    assert_eq!(
        from_root.toplevel(),
        std::fs::canonicalize(repo.path())
            .expect("canonicalize")
            .as_path()
    );
}

/// Outside any repository there is no safety net, and that is not an error.
#[test]
fn discover_returns_none_outside_a_repository() {
    require_git!();
    let bare_dir = TempDir::new().expect("temp dir");
    // A temp dir has no `.git` and (unlike a directory under this source tree)
    // no repository above it either.
    assert_eq!(Repo::discover(bare_dir.path()), None);
    assert_eq!(Repo::discover(&bare_dir.path().join("nothing.csv")), None);
    assert_eq!(
        Repo::discover(Path::new("/this/path/does/not/exist/at/all")),
        None
    );
}

/// A bare repository has no work tree, so there is nothing to commit *into*.
#[test]
fn discover_declines_a_bare_repository() {
    require_git!();
    let dir = TempDir::new().expect("temp dir");
    let output = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .current_dir(dir.path())
        .output()
        .expect("git init --bare");
    assert!(output.status.success());
    assert_eq!(Repo::discover(dir.path()), None);
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// All four states, in one repository, distinguished correctly.
#[test]
fn status_classifies_every_state() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("clean.journal", "; unchanged\n");
    repo.write("dirty.journal", "; original\n");
    repo.write(".gitignore", "ignored.csv\n");
    repo.commit_all("init");

    repo.write("dirty.journal", "; original\n; edited\n");
    repo.write("fresh.csv", "date,amount\n");
    repo.write("ignored.csv", "date,amount\n");

    let status = repo
        .discover()
        .status(&[
            &repo.at("clean.journal"),
            &repo.at("dirty.journal"),
            &repo.at("fresh.csv"),
            &repo.at("ignored.csv"),
        ])
        .expect("status");

    assert!(status.available);
    assert_eq!(state_of(&status, "clean.journal"), FileState::Clean);
    assert_eq!(state_of(&status, "dirty.journal"), FileState::Modified);
    assert_eq!(state_of(&status, "fresh.csv"), FileState::Untracked);
    assert_eq!(state_of(&status, "ignored.csv"), FileState::Ignored);

    // `dirty` is exactly the blocking subset: an untracked CSV is expected and
    // must never stand between a user and their import.
    assert_eq!(status.dirty, vec!["dirty.journal".to_string()]);
}

/// A change that exists only in the index still blocks: there is no committed
/// state to revert to, which is the entire premise of the safety net.
#[test]
fn a_staged_but_uncommitted_target_is_modified() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; one\n");
    repo.commit_all("init");
    repo.write("main.journal", "; one\n; two\n");
    repo.git(&["add", "--", "main.journal"]);

    let status = repo
        .discover()
        .status(&[&repo.at("main.journal")])
        .expect("status");
    assert_eq!(state_of(&status, "main.journal"), FileState::Modified);
    assert_eq!(status.dirty, vec!["main.journal".to_string()]);
}

/// A gitignored destination is reported as such and never force-added — the
/// user put it in `.gitignore` on purpose.
#[test]
fn a_gitignored_target_is_reported_and_never_added() {
    require_git!();
    let repo = TestRepo::init();
    repo.write(".gitignore", "statements/\nsecret.csv\n");
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("secret.csv", "date,amount\n");
    // The harder case: the file itself is not named, its *directory* is.
    repo.write("statements/january.csv", "date,amount\n");

    let git = repo.discover();
    let status = git
        .status(&[&repo.at("secret.csv"), &repo.at("statements/january.csv")])
        .expect("status");
    assert_eq!(state_of(&status, "secret.csv"), FileState::Ignored);
    assert_eq!(
        state_of(&status, "statements/january.csv"),
        FileState::Ignored,
        "a file inside an ignored DIRECTORY is still ignored; git reports the \
         directory, not the file"
    );
    assert!(status.dirty.is_empty(), "ignored files do not block");

    // Committing them is refused rather than forced.
    let error = git
        .commit(
            &[&repo.at("secret.csv"), &repo.at("statements/january.csv")],
            "ledgeline: import",
        )
        .expect_err("an all-ignored commit is refused");
    assert!(
        matches!(&error, GitError::NothingToCommit { ignored } if ignored.len() == 2),
        "{error:?}"
    );
    assert_eq!(repo.commit_count(), 1, "no commit was created");
    assert!(
        !repo.git(&["ls-files"]).contains("secret.csv"),
        "the ignored file must not have been added"
    );
}

/// An ignored path alongside a real one degrades: the real one is committed,
/// the ignored one is left alone.
#[test]
fn an_ignored_target_does_not_sink_the_rest_of_the_commit() {
    require_git!();
    let repo = TestRepo::init();
    repo.write(".gitignore", "secret.csv\n");
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("main.journal", "; journal\n; imported\n");
    repo.write("secret.csv", "date,amount\n");

    let staged = repo
        .discover()
        .commit(
            &[&repo.at("secret.csv"), &repo.at("main.journal")],
            "ledgeline: import 3 transactions",
        )
        .expect("the non-ignored path still commits");

    // What comes back is what was STAGED, not what was asked for. A caller that
    // reported the request back to a user would be claiming a commit of a file
    // git never took — which is exactly what the import result panel does with
    // this list.
    assert_eq!(
        staged,
        vec!["main.journal".to_string()],
        "the ignored path must not be reported as committed"
    );
    assert_eq!(repo.head_files(), vec!["main.journal".to_string()]);
    assert!(
        !repo.git(&["ls-files"]).contains("secret.csv"),
        "the ignored file is still untracked"
    );
}

/// Asking about no paths must not turn into asking about the whole repository.
#[test]
fn an_empty_path_list_never_scans_the_repository() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("unrelated.txt", "in progress\n");

    let status = repo.discover().status(&[]).expect("status of nothing");
    assert!(status.available);
    assert!(
        status.files.is_empty() && status.dirty.is_empty(),
        "an empty request must not report the repository's other work: {status:?}"
    );
}

/// A path outside the repository is refused, and the error names only the file.
#[test]
fn a_target_outside_the_repository_is_refused_without_echoing_its_path() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    let elsewhere = TempDir::new().expect("temp dir");
    let outsider = elsewhere.path().join("somebody-elses.journal");
    std::fs::write(&outsider, "; not ours\n").expect("write");

    let error = repo
        .discover()
        .status(&[&outsider])
        .expect_err("outside the repo");
    assert!(matches!(&error, GitError::Outside { .. }), "{error:?}");

    let rendered = error.to_string();
    assert!(rendered.contains("somebody-elses.journal"), "{rendered}");
    assert!(
        !rendered.contains(&elsewhere.path().to_string_lossy().into_owned()),
        "the containing directory must not be disclosed: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// commit — the property this whole module exists for
// ---------------------------------------------------------------------------

/// **The critical test.** A repository with unrelated work in progress, both
/// unstaged and already staged. Commit our two targets. Everything else must be
/// exactly as it was: still dirty, still staged, still uncommitted, and absent
/// from the commit we made.
///
/// This is the assertion that would fail the instant somebody reached for
/// `git add -A`, `git add .` or `commit -a`.
#[test]
fn committing_our_targets_leaves_unrelated_work_untouched() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.write("notes.md", "notes\n");
    repo.write("budget.txt", "budget\n");
    repo.commit_all("init");

    // Unrelated work in progress, in all three shapes a user can have it.
    repo.write("notes.md", "notes\nhalf-written thought\n");
    repo.write("budget.txt", "budget\nstaged rework\n");
    repo.git(&["add", "--", "budget.txt"]);
    repo.write("scratch.md", "brand new, not ours\n");

    // Our targets: an edited journal and a CSV that did not exist before.
    repo.write("main.journal", "; journal\n2026-08-12 Coffee\n");
    repo.write("statement.csv", "date,amount\n2026-08-12,4.50\n");

    let before = repo.porcelain();
    repo.discover()
        .commit(
            &[&repo.at("statement.csv"), &repo.at("main.journal")],
            "ledgeline: import 1 transaction from statement.csv",
        )
        .expect("commit");

    // The commit contains exactly our two paths.
    let mut committed = repo.head_files();
    committed.sort();
    assert_eq!(
        committed,
        vec!["main.journal".to_string(), "statement.csv".to_string()]
    );
    assert_eq!(
        repo.head_subject(),
        "ledgeline: import 1 transaction from statement.csv"
    );

    // And every piece of unrelated work is exactly where it was left.
    let after = repo.porcelain();
    let survivors: Vec<&String> = after
        .iter()
        .filter(|line| !line.contains("main.journal") && !line.contains("statement.csv"))
        .collect();
    assert!(
        survivors.iter().any(|line| line.as_str() == " M notes.md"),
        "the unstaged edit must still be dirty: {after:?}"
    );
    assert!(
        survivors
            .iter()
            .any(|line| line.as_str() == "M  budget.txt"),
        "the STAGED edit must still be staged and uncommitted: {after:?}"
    );
    assert!(
        survivors
            .iter()
            .any(|line| line.as_str() == "?? scratch.md"),
        "the unrelated new file must still be untracked: {after:?}"
    );

    // Stated once more as a whole: the only lines that changed are ours.
    let expected: Vec<&String> = before
        .iter()
        .filter(|line| !line.contains("main.journal") && !line.contains("statement.csv"))
        .collect();
    assert_eq!(survivors, expected, "before {before:?} / after {after:?}");

    // The staged rework exists only in the index. Its content must appear
    // nowhere in history — neither in `HEAD` nor in any other commit.
    assert_eq!(repo.git(&["show", "HEAD:budget.txt"]), "budget\n");
    assert_eq!(
        repo.git(&["log", "--all", "--format=%H", "-S", "staged rework"]),
        "",
        "the staged content must never have entered history"
    );
}

/// Pathspecs are arguments, not shell words: a file named `-f` is a file, and a
/// file with a space in its name is one file.
#[test]
fn awkward_filenames_are_paths_and_not_options() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("seed.txt", "seed\n");
    repo.commit_all("init");

    // A leading dash, a space, and a pathspec metacharacter.
    let dashed = repo.write("-f", "looks like a flag\n");
    let spaced = repo.write("bank statement 2026.csv", "date,amount\n");
    let globbed = repo.write("statement[2026].csv", "date,amount\n");
    let decoy = repo.write("statement2026.csv", "must not be swept in\n");

    let git = repo.discover();
    let status = git
        .status(&[&dashed, &spaced, &globbed])
        .expect("status of awkward names");
    assert_eq!(state_of(&status, "-f"), FileState::Untracked);
    assert_eq!(
        state_of(&status, "bank statement 2026.csv"),
        FileState::Untracked
    );
    assert_eq!(
        state_of(&status, "statement[2026].csv"),
        FileState::Untracked
    );

    git.commit(&[&dashed, &spaced, &globbed], "ledgeline: awkward names")
        .expect("commit");

    let mut committed = repo.head_files();
    committed.sort();
    assert_eq!(
        committed,
        vec![
            "-f".to_string(),
            "bank statement 2026.csv".to_string(),
            "statement[2026].csv".to_string(),
        ]
    );
    // `--literal-pathspecs` earning its place: the bracketed name is one file,
    // not a character class that also matches the decoy.
    assert!(
        repo.porcelain()
            .iter()
            .any(|line| line.contains("statement2026.csv")),
        "the decoy must still be untracked"
    );
    assert!(decoy.exists());
}

/// The same path twice is one path — not two status rows, and not two
/// pathspecs handed to `git add`.
#[test]
fn a_repeated_target_is_reported_and_staged_once() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("seed.txt", "seed\n");
    repo.commit_all("init");
    let journal = repo.write("main.journal", "; journal\n");

    let status = repo
        .discover()
        .status(&[&journal, &journal])
        .expect("status");
    assert_eq!(status.files.len(), 1, "{:?}", status.files);
}

/// A commit message is required. Refused here rather than letting git try to
/// open an editor into a GUI that has no terminal.
#[test]
fn an_empty_message_is_refused_before_git_runs() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("seed.txt", "seed\n");
    repo.commit_all("init");
    let journal = repo.write("main.journal", "; journal\n");

    for message in ["", "   ", "\n\t "] {
        assert_eq!(
            repo.discover().commit(&[&journal], message),
            Err(GitError::EmptyMessage)
        );
    }
    assert_eq!(repo.commit_count(), 1);
}

/// No paths must never become "commit everything". This is the sharpest footgun
/// in the module: `git commit -m msg --` with an empty pathspec list is a full
/// commit of the index.
#[test]
fn committing_no_paths_is_refused_rather_than_committing_everything() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("seed.txt", "seed\n");
    repo.commit_all("init");
    repo.write("staged-by-someone-else.txt", "not ours\n");
    repo.git(&["add", "--", "staged-by-someone-else.txt"]);

    let error = repo
        .discover()
        .commit(&[], "ledgeline: import")
        .expect_err("an empty path list is refused");
    assert!(
        matches!(error, GitError::NothingToCommit { .. }),
        "{error:?}"
    );
    assert_eq!(repo.commit_count(), 1, "nothing may have been committed");
    assert!(
        repo.porcelain()
            .iter()
            .any(|line| line == "A  staged-by-someone-else.txt"),
        "somebody else's staged file is still staged: {:?}",
        repo.porcelain()
    );
}

// ---------------------------------------------------------------------------
// failure surfaces
// ---------------------------------------------------------------------------

/// A pre-commit hook that refuses must reach the user with its own words. The
/// hook is the only thing that can explain why the commit was rejected, so
/// swallowing its stderr — or bypassing it with `--no-verify` — would be a bug.
#[test]
fn a_rejecting_pre_commit_hook_surfaces_its_stderr() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("main.journal", "; journal\n; imported\n");
    repo.hook(
        "pre-commit",
        "#!/bin/sh\necho 'policy: journals are reviewed before commit' >&2\nexit 1\n",
    );

    let error = repo
        .discover()
        .commit(&[&repo.at("main.journal")], "ledgeline: import")
        .expect_err("the hook refuses");

    let rendered = error.to_string();
    assert!(
        matches!(&error, GitError::Failed { command, .. } if command == "commit"),
        "{error:?}"
    );
    assert!(
        rendered.contains("policy: journals are reviewed before commit"),
        "the hook's own message must survive: {rendered}"
    );

    // Reported and stopped. Nothing was rolled back — the journal on disk is
    // still the imported one, because by this point it was correctly written.
    assert_eq!(repo.commit_count(), 1);
    assert_eq!(
        std::fs::read_to_string(repo.at("main.journal")).expect("read"),
        "; journal\n; imported\n"
    );
}

/// A hook's message may quote absolute paths. Ours is the one prefix we can
/// recognise, and it is stripped so the result panel shows a repo-relative
/// path instead of the user's home directory.
#[test]
fn the_repository_path_is_stripped_out_of_git_output() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("main.journal", "; journal\n; imported\n");
    // A hook that prints its own absolute path, as real hooks routinely do.
    repo.hook(
        "pre-commit",
        "#!/bin/sh\necho \"refused by $0\" >&2\nexit 1\n",
    );

    let rendered = repo
        .discover()
        .commit(&[&repo.at("main.journal")], "ledgeline: import")
        .expect_err("the hook refuses")
        .to_string();

    let toplevel = std::fs::canonicalize(repo.path()).expect("canonicalize");
    assert!(
        !rendered.contains(&toplevel.to_string_lossy().into_owned()),
        "the repository path must not be echoed: {rendered}"
    );
    assert!(
        rendered.contains(".git/hooks/pre-commit"),
        "but the hook must still be identifiable: {rendered}"
    );
}

/// git that cannot work out who is committing must say so, not hang.
///
/// Reaching this state is fiddlier than it looks: a merely *missing*
/// `user.email` does **not** fail — git synthesises `username@hostname` and
/// commits. It refuses only when it can derive nothing at all, which is what
/// empty local values produce. Local config also shadows the contributor's own
/// `~/.gitconfig`, so this is reproducible on any machine.
#[test]
fn an_unusable_author_identity_reports_instead_of_hanging() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("main.journal", "; journal\n; imported\n");
    repo.git(&["config", "user.name", ""]);
    repo.git(&["config", "user.email", ""]);

    let started = Instant::now();
    let error = repo
        .discover()
        .commit(&[&repo.at("main.journal")], "ledgeline: import")
        .expect_err("no usable identity");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "it must fail, not block on input"
    );

    let rendered = error.to_string();
    assert!(matches!(&error, GitError::Failed { .. }), "{error:?}");
    assert!(
        rendered.to_lowercase().contains("identity")
            || rendered.contains("Please tell me who you are"),
        "the message must be actionable: {rendered}"
    );
    assert_eq!(repo.commit_count(), 1);
}

/// The wall-clock timeout, exercised through the same choke point every real
/// invocation uses. A hook that sleeps stands in for the case that motivated
/// the timeout: a GPG pinentry dialog nobody answers, which would otherwise
/// hang the desktop GUI for as long as the app runs.
///
/// The assertion that matters is not just the error — it is that we returned
/// *long before the hook would have finished*.
#[test]
fn a_hanging_git_is_killed_and_reported() {
    require_git!();
    let repo = TestRepo::init();
    repo.write("main.journal", "; journal\n");
    repo.commit_all("init");
    repo.write("main.journal", "; journal\n; imported\n");
    repo.hook("pre-commit", "#!/bin/sh\nsleep 120\n");
    repo.git(&["add", "--", "main.journal"]);

    // The real commit invocation, built by the module itself — only the budget
    // is shortened, so this is the production code path under a test clock.
    let args = git::commit_args("ledgeline: import", &["main.journal".to_string()]);

    let started = Instant::now();
    let error = git::run(
        Some(repo.path()),
        "commit",
        &args,
        Duration::from_millis(750),
    )
    .expect_err("the sleeping hook must time out");
    let elapsed = started.elapsed();

    assert!(
        matches!(&error, GitError::Timeout { command, .. } if command == "commit"),
        "{error:?}"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "returned after {elapsed:?}; the 120s hook was clearly waited on"
    );
    assert_eq!(repo.commit_count(), 1, "the killed commit created nothing");
}

// ---------------------------------------------------------------------------
// several repositories at once
// ---------------------------------------------------------------------------

/// The CSV destination and the journal may live in different repositories, or
/// one may be outside version control entirely. Each target resolves on its
/// own, and committing in one repository has no effect on the other.
#[test]
fn targets_in_different_repositories_are_handled_independently() {
    require_git!();
    let journals = TestRepo::init();
    journals.write("main.journal", "; journal\n");
    journals.commit_all("init");

    let statements = TestRepo::init();
    statements.write("README", "bank exports\n");
    statements.commit_all("init");

    let unversioned = TempDir::new().expect("temp dir");
    let loose = unversioned.path().join("loose.csv");
    std::fs::write(&loose, "date,amount\n").expect("write");

    journals.write("main.journal", "; journal\n2026-08-12 Coffee\n");
    let csv = statements.write("statement.csv", "date,amount\n2026-08-12,4.50\n");

    // Resolution is per target, and the two repositories are distinct.
    let journal_repo = Repo::discover(&journals.at("main.journal")).expect("journal repo");
    let statement_repo = Repo::discover(&csv).expect("statement repo");
    assert_ne!(journal_repo, statement_repo);
    // The unversioned target has no repository at all: skipped, not failed.
    assert_eq!(Repo::discover(&loose), None);

    journal_repo
        .commit(&[&journals.at("main.journal")], "ledgeline: import")
        .expect("commit the journal");
    statement_repo
        .commit(&[&csv], "ledgeline: import")
        .expect("commit the statement");

    assert_eq!(journals.head_files(), vec!["main.journal".to_string()]);
    assert_eq!(statements.head_files(), vec!["statement.csv".to_string()]);
    assert_eq!(journals.commit_count(), 2);
    assert_eq!(statements.commit_count(), 2);

    // A target belonging to the other repository is refused, not silently
    // committed into the wrong history.
    let error = journal_repo
        .status(&[&csv])
        .expect_err("the CSV is not in the journal repo");
    assert!(matches!(error, GitError::Outside { .. }), "{error:?}");
}

/// A repository whose only commit is the one we are about to make — no `HEAD`
/// yet. Nothing in the module may assume history exists.
#[test]
fn a_repository_with_no_commits_yet_still_works() {
    require_git!();
    let repo = TestRepo::init();
    let journal = repo.write("main.journal", "; brand new\n");
    let csv = repo.write("statement.csv", "date,amount\n");

    let git = repo.discover();
    let status = git
        .status(&[&journal, &csv])
        .expect("status on unborn HEAD");
    assert_eq!(state_of(&status, "main.journal"), FileState::Untracked);
    assert!(status.dirty.is_empty());

    git.commit(&[&journal, &csv], "ledgeline: first import")
        .expect("commit on an unborn HEAD");

    let mut committed = repo.head_files();
    committed.sort();
    assert_eq!(
        committed,
        vec!["main.journal".to_string(), "statement.csv".to_string()]
    );
}

/// `git_available` agrees with reality, and `GitStatus::unavailable` is the
/// shape a caller reports when discovery found nothing.
#[test]
fn availability_is_reported_honestly() {
    assert_eq!(
        git_available(),
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success())
    );

    let none = GitStatus::unavailable();
    assert!(!none.available);
    assert!(none.files.is_empty());
}
