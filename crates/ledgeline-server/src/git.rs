//! Commit around an import — the first and only place Ledgeline touches git.
//!
//! Version control has always been the user's business. An import is the first
//! operation that rewrites a journal **in place**, so it is the first that earns
//! a safety net: when an import goes wrong, `git diff` and `git revert` are the
//! recovery path, and those only work if the pre-import state was committed.
//! That is the whole value proposition, and it bounds the scope — this module
//! stages and commits, and does nothing else. There is no branch, no push, no
//! merge, no history rewriting, and no `reset`.
//!
//! # What a caller does with it
//!
//! ```text
//! before writing:  Repo::discover(target) -> status(&[target]) -> Modified? block.
//! after writing:   Repo::commit(&[csv, journal], "…")
//! ```
//!
//! [`Repo`] is deliberately **per-repository**. The CSV destination and the
//! journal may live in different repositories, or one may be outside version
//! control entirely, so the caller resolves each target with [`Repo::discover`],
//! groups the targets that share a [`Repo`] (the type is `Eq` for exactly this),
//! and commits each group on its own. A target that resolves to `None` is
//! reported as skipped, not treated as a failure — that is what "degrade
//! gracefully" means here, and the decision belongs to the layer that can tell
//! the user, not to this one.
//!
//! # The rules this module exists to enforce
//!
//! These are not stylistic. Each one is a way this feature could damage a
//! user's work, and each is pinned by a test in `tests/git_commit.rs`.
//!
//! - **Explicit pathspecs only.** Never `git add -A`, never `git add .`, never
//!   `commit -a`. Exactly the paths passed in are staged, and `commit --only`
//!   states that intent on the command line rather than leaving it implicit in
//!   git's pathspec behaviour. Someone with unrelated work in progress —
//!   *including work already staged in the index* — must find it exactly as
//!   they left it.
//! - **Arguments are a `Vec<OsString>`, never a shell string.** Same rule as
//!   `hledger.rs`, for the same reason. `--` terminates every pathspec list so a
//!   file named `-f` is a file, and `--literal-pathspecs` means a file named
//!   `statement[2026].csv` is that one file rather than a glob. There is no
//!   `sh -c` anywhere in this module, and there must never be.
//! - **Ignored files are reported, never force-added.** If the CSV destination
//!   is gitignored the user meant it. [`Repo::commit`] drops ignored paths
//!   before staging — not merely as a courtesy, but because `git add` on an
//!   ignored path *fails the whole invocation* while still staging its other
//!   arguments, which would leave a half-staged index behind.
//! - **`Untracked` does not block; only `Modified` does.** A brand-new CSV is
//!   expected to be untracked.
//! - **Failure is never silent and never fatal.** A rejecting pre-commit hook,
//!   a GPG passphrase prompt, an unusable author identity — each surfaces git's
//!   own output in [`GitError`]. By the time we commit, the journal is already
//!   correctly written, so a failed commit *reports and stops*. It never
//!   attempts to roll anything back; there is nothing here that could.
//! - **Every invocation has a wall-clock timeout**, because a signing prompt
//!   would otherwise hang the desktop GUI forever.
//!
//! # Paths in errors
//!
//! Every path this module puts in a [`GitStatus`] or a [`GitError`] is relative
//! to the repository toplevel, consistent with the no-disclosure rule in
//! `docs/imports.md`. git's own stderr is passed through **verbatim except for
//! one substitution**: the toplevel prefix is stripped ([`Repo::scrub`]), so a
//! hook's complaint about `/Users/someone/finances/.git/hooks/pre-commit`
//! reaches the UI as `.git/hooks/pre-commit`. Surfacing that stderr is required
//! — it is the only thing that tells a user *why* their commit was refused —
//! and redacting the one prefix we know keeps it from being a path oracle.
//!
//! # Empirical notes (git 2.55)
//!
//! Verified against the binary, not the manual, because three of these
//! contradict the obvious reading:
//!
//! - `git status --porcelain=v1 -z --ignored=matching` reports an ignored file
//!   inside an ignored **directory** as the directory (`imports/`), never as the
//!   file, and `--untracked-files=all` does not change that. Hence the
//!   directory-prefix arm in [`classify`].
//! - A **missing `user.email` does not fail.** git synthesises an identity from
//!   `username@hostname` and commits happily. It refuses only when it cannot
//!   derive one at all, which is why the test for that case empties both
//!   `user.name` and `user.email` in the repository's own config.
//! - `git commit` writes **"nothing to commit" to stdout, not stderr**, so an
//!   error built from stderr alone would be blank. [`Repo::check`] falls back to
//!   stdout.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use thiserror::Error;

/// The binary. Resolved through `PATH` by [`Command`]; never through a shell.
const GIT: &str = "git";

/// Global flags every single invocation carries.
///
/// `--no-pager` because a pager on an inherited terminal would block forever,
/// and `--literal-pathspecs` because a perfectly ordinary bank statement named
/// `statement[2026].csv` is a *pathspec glob* to git otherwise, matching either
/// nothing or the wrong thing.
const GLOBAL_FLAGS: [&str; 2] = ["--no-pager", "--literal-pathspecs"];

/// Budget for the cheap probes (`--version`, `rev-parse`). These do no real
/// work; if one has not answered in five seconds something is badly wrong.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Budget for `status` / `ls-files`. Generous enough for a large repository on
/// a cold index, short enough that the pre-import check cannot appear to hang.
pub(crate) const STATUS_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for `add` + `commit`. Deliberately the longest: a commit legitimately
/// runs the user's pre-commit hooks (linters, test suites) and may put a GPG
/// pinentry dialog in front of them, and a minute is a fair window in which to
/// type a passphrase. It is finite because the alternative — a signing prompt
/// nobody answers — is an import that never returns.
pub(crate) const COMMIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Initial gap between `try_wait` polls, doubling up to [`POLL_MAX`]. Starting
/// small keeps the common case (git exits in a few milliseconds) prompt without
/// spinning for the whole duration of a slow hook.
const POLL_START: Duration = Duration::from_millis(1);
/// Ceiling for the poll backoff.
const POLL_MAX: Duration = Duration::from_millis(20);

/// A git work tree containing at least one import target.
///
/// The `toplevel` field is **private and has no public constructor from a raw
/// path**: a `Repo` can only be obtained from [`Repo::discover`], so "this
/// directory really is a git work tree" is established once, by git itself,
/// rather than assumed at each call site. The path is stored canonicalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    toplevel: PathBuf,
}

/// One target path and what git thinks of it, relative to the toplevel. The
/// element type of [`GitStatus::files`].
type Classified = (String, FileState);

/// What git thinks of one target path.
///
/// There is no `Missing` variant: a destination that does not exist yet — the
/// CSV an import is about to write — classifies as [`Untracked`](Self::Untracked),
/// which is what it will be a moment later, and which does not block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Not known to git. Expected for a new CSV; never blocks.
    Untracked,
    /// Tracked, and identical to both the index and `HEAD`.
    Clean,
    /// Tracked with uncommitted content — in the work tree, in the index, or
    /// both. This is the one that blocks an import, because it is the state in
    /// which `git revert` could not undo what we are about to do.
    Modified,
    /// Matched by a `.gitignore`. Reported so the caller can say the file was
    /// skipped; never force-added.
    Ignored,
}

/// git's view of a set of import targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    /// `git` is on `PATH` **and** the targets sit in a work tree. False only
    /// for [`GitStatus::unavailable`], which is what a caller reports when
    /// [`Repo::discover`] found nothing.
    pub available: bool,
    /// One entry per requested path, in the order requested, deduplicated.
    /// Paths are relative to the repository toplevel.
    pub files: Vec<(String, FileState)>,
    /// The subset of `files` that blocks — exactly the [`FileState::Modified`]
    /// ones. Precomputed because it is what the caller branches on.
    pub dirty: Vec<String>,
}

impl GitStatus {
    /// The answer for targets that are not under version control at all: not an
    /// error, just nothing to do.
    pub fn unavailable() -> Self {
        Self {
            available: false,
            files: Vec::new(),
            dirty: Vec::new(),
        }
    }
}

/// Why a git invocation could not be completed.
///
/// Carries no [`std::io::Error`], so the type stays `Clone` + `Eq` and can be
/// compared in tests and held in a response body — the same trade `RulesError`
/// makes, for the same reasons. `command` is a subcommand name, never a path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GitError {
    /// No `git` on `PATH`. A caller should degrade to "no version control"
    /// rather than treat this as a failure.
    #[error("git is not installed, or is not on PATH")]
    NotFound,

    /// The invocation outlived its budget and was killed. The overwhelmingly
    /// likely cause is an interactive prompt — a GPG passphrase, a credential
    /// helper — that nobody answered.
    #[error(
        "`git {command}` was still running after {seconds}s and was stopped; \
         if this repository signs commits, check for a passphrase prompt"
    )]
    Timeout { command: String, seconds: u64 },

    /// git ran and refused. `detail` is git's own output, stripped of the
    /// toplevel prefix — a pre-commit hook's message, an identity complaint, a
    /// merge conflict.
    #[error("`git {command}` failed{}: {detail}", exit_suffix(*.code))]
    Failed {
        command: String,
        code: Option<i32>,
        detail: String,
    },

    /// git could not be started, or the wait on it failed.
    #[error("could not run `git {command}`: {detail}")]
    Unrunnable { command: String, detail: String },

    /// A target does not live under this repository's toplevel. Only the file
    /// name is quoted — enough to identify which argument was wrong, without
    /// echoing a path back.
    #[error("`{name}` is not inside this repository")]
    Outside { name: String },

    /// Nothing was left to stage: either no paths were passed at all, or every
    /// one of them was gitignored — reported rather than force-added.
    #[error("nothing to commit{}", ignored_suffix(.ignored))]
    NothingToCommit { ignored: Vec<String> },

    /// An empty commit message. Refused here rather than letting git try to
    /// open an editor nobody can see.
    #[error("a commit message may not be empty")]
    EmptyMessage,
}

/// `" (exit 1)"`, or nothing at all when the process died on a signal.
fn exit_suffix(code: Option<i32>) -> String {
    code.map(|code| format!(" (exit {code})"))
        .unwrap_or_default()
}

/// The tail of [`GitError::NothingToCommit`] that names what was skipped.
fn ignored_suffix(ignored: &[String]) -> String {
    if ignored.is_empty() {
        String::new()
    } else {
        format!(": every path is gitignored ({})", ignored.join(", "))
    }
}

/// Whether a usable `git` exists, established by running it rather than by
/// stat-ing `PATH` — an entry that is present but not executable, or a shim
/// that cannot start, is not a usable git.
///
/// Deliberately uncached. It is one process spawn on a path that already spawns
/// several, and caching it would mean a user who installs git mid-session is
/// told for the rest of that session that they have not.
pub fn git_available() -> bool {
    matches!(
        run(None, "--version", &version_args(), PROBE_TIMEOUT),
        Ok(outcome) if outcome.succeeded()
    )
}

impl Repo {
    /// The work tree containing `path`, or `None` if there is not one.
    ///
    /// `None` covers every "no safety net available" case uniformly — git is
    /// not installed, the path is outside any repository, the repository is
    /// bare, the directory does not exist. None of those is an error; they all
    /// mean the same thing to a caller.
    ///
    /// `path` need not exist. An import's CSV destination usually does not yet,
    /// so the search starts from the containing directory.
    pub fn discover(path: &Path) -> Option<Self> {
        let start = search_dir(path)?;
        let outcome = run(Some(&start), "rev-parse", &toplevel_args(), PROBE_TIMEOUT).ok()?;
        if !outcome.succeeded() {
            return None;
        }
        // Not `from_utf8_lossy`: a lossy conversion of a non-UTF-8 repository
        // root yields a path that silently is not the one git named, and every
        // `strip_prefix` against it would then be wrong. Declining is safe —
        // it just means no automatic commit for that exotic repository.
        let reported = String::from_utf8(outcome.stdout).ok()?;
        let trimmed = reported.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return None;
        }
        let toplevel = PathBuf::from(trimmed);
        Some(Self {
            toplevel: std::fs::canonicalize(&toplevel).unwrap_or(toplevel),
        })
    }

    /// The repository root. Crate-internal: callers use it to group targets by
    /// repository, and it must never reach a user-facing string — see the
    /// module's "Paths in errors".
    pub(crate) fn toplevel(&self) -> &Path {
        &self.toplevel
    }

    /// Classify each of `paths`.
    ///
    /// Two invocations, because neither alone is sufficient: `git status`
    /// reports only what *differs*, so a clean tracked file and a path git has
    /// never heard of are both absent from its output, and `git ls-files` is
    /// what tells those two apart.
    ///
    /// An empty `paths` returns an empty status without running git at all.
    /// That is not an optimisation: a pathspec-less `git status` walks the
    /// entire repository, which is emphatically not what a caller asking about
    /// zero targets meant.
    pub fn status(&self, paths: &[&Path]) -> Result<GitStatus, GitError> {
        let wanted = self.relative_paths(paths)?;
        if wanted.is_empty() {
            return Ok(GitStatus {
                available: true,
                files: Vec::new(),
                dirty: Vec::new(),
            });
        }

        let reported = self.porcelain(&wanted)?;
        let tracked = self.tracked(&wanted)?;

        let files: Vec<Classified> = wanted
            .into_iter()
            .map(|path| {
                let state = classify(&path, &reported, &tracked);
                (path, state)
            })
            .collect();
        let dirty = files
            .iter()
            .filter(|(_, state)| *state == FileState::Modified)
            .map(|(path, _)| path.clone())
            .collect();

        Ok(GitStatus {
            available: true,
            files,
            dirty,
        })
    }

    /// Stage and commit exactly `paths`, and nothing else.
    ///
    /// Gitignored paths are dropped first — see the module docs for why that is
    /// a correctness requirement and not politeness. If every path was ignored,
    /// or none were given, this is [`GitError::NothingToCommit`]; a caller
    /// reports that as "skipped", since a gitignored destination is a choice the
    /// user already made.
    ///
    /// The commit itself is `git commit --only -- <paths>`, which takes the work
    /// tree content of those paths and leaves the rest of the index — including
    /// somebody's unrelated staged work — untouched and uncommitted.
    ///
    /// Hooks are **not** bypassed. A repository that refuses this commit is
    /// telling the user something, and `--no-verify` would silence it.
    pub fn commit(&self, paths: &[&Path], message: &str) -> Result<(), GitError> {
        if message.trim().is_empty() {
            return Err(GitError::EmptyMessage);
        }

        let (ignored, stageable): (Vec<Classified>, Vec<Classified>) = self
            .status(paths)?
            .files
            .into_iter()
            .partition(|(_, state)| *state == FileState::Ignored);
        let stageable = names(stageable);
        if stageable.is_empty() {
            return Err(GitError::NothingToCommit {
                ignored: names(ignored),
            });
        }

        let added = self.exec("add", &add_args(&stageable), COMMIT_TIMEOUT)?;
        self.check("add", added)?;

        let committed = self.exec("commit", &commit_args(message, &stageable), COMMIT_TIMEOUT)?;
        self.check("commit", committed).map(|_| ())
    }

    /// `git status` over the requested paths, parsed from the NUL-delimited
    /// porcelain v1 form.
    ///
    /// `-z` rather than the line form so a filename containing a space, a quote
    /// or a newline arrives as raw bytes instead of a C-quoted string we would
    /// have to unescape.
    fn porcelain(&self, wanted: &[String]) -> Result<Vec<Classified>, GitError> {
        let outcome = self.exec("status", &status_args(wanted), STATUS_TIMEOUT)?;
        Ok(parse_porcelain(&self.check("status", outcome)?))
    }

    /// The requested paths git already tracks — the only way to tell a clean
    /// tracked file from one git has never seen, since `status` prints neither.
    fn tracked(&self, wanted: &[String]) -> Result<Vec<String>, GitError> {
        let outcome = self.exec("ls-files", &ls_files_args(wanted), STATUS_TIMEOUT)?;
        Ok(nul_fields(&self.check("ls-files", outcome)?))
    }

    /// Run git in this repository, mapping a transport failure to a redacted
    /// [`GitError`]. The exit status is *not* inspected here — [`Repo::check`]
    /// does that separately, so a caller that tolerates a non-zero exit still
    /// can.
    fn exec(&self, command: &str, args: &[OsString], timeout: Duration) -> Result<Run, GitError> {
        run(Some(&self.toplevel), command, args, timeout).map_err(|error| self.redact(error))
    }

    /// Require a zero exit, yielding stdout. On failure, build the error from
    /// git's stderr — falling back to stdout, because `git commit` reports
    /// "nothing to commit" there and an error built from stderr alone would
    /// carry no message at all.
    fn check(&self, command: &str, outcome: Run) -> Result<Vec<u8>, GitError> {
        if outcome.succeeded() {
            return Ok(outcome.stdout);
        }
        let stderr = text(&outcome.stderr);
        let detail = if stderr.trim().is_empty() {
            text(&outcome.stdout)
        } else {
            stderr
        };
        Err(self.redact(GitError::Failed {
            command: command.to_string(),
            code: outcome.code,
            detail: detail.trim().to_string(),
        }))
    }

    /// Rewrite the toplevel out of any message git produced.
    fn redact(&self, error: GitError) -> GitError {
        match error {
            GitError::Failed {
                command,
                code,
                detail,
            } => GitError::Failed {
                command,
                code,
                detail: self.scrub(detail),
            },
            GitError::Unrunnable { command, detail } => GitError::Unrunnable {
                command,
                detail: self.scrub(detail),
            },
            other => other,
        }
    }

    /// Strip the toplevel prefix from text git produced, so an absolute path
    /// inside a hook's complaint reaches the UI repo-relative. See the module's
    /// "Paths in errors" for why this is a substitution rather than a wholesale
    /// replacement of git's own words.
    fn scrub(&self, detail: String) -> String {
        let prefix = self.toplevel.to_string_lossy().into_owned();
        detail
            .replace(&format!("{prefix}/"), "")
            .replace(prefix.as_str(), ".")
    }

    /// Map each target to a toplevel-relative path, preserving the caller's
    /// order and dropping repeats.
    ///
    /// Deduplication matters beyond tidiness: a repeated path would be reported
    /// twice in [`GitStatus::files`] and passed twice to `git add`.
    fn relative_paths(&self, paths: &[&Path]) -> Result<Vec<String>, GitError> {
        paths
            .iter()
            .map(|path| self.relative(path))
            .collect::<Result<Vec<String>, GitError>>()
            .map(dedup_preserving_order)
    }

    /// One target as a toplevel-relative, forward-slashed string.
    fn relative(&self, path: &Path) -> Result<String, GitError> {
        physical(path)
            .strip_prefix(&self.toplevel)
            .map(slashed)
            .map_err(|_| GitError::Outside {
                name: file_label(path),
            })
    }
}

/// A finished git process. `stdout` stays bytes because paths in `-z` output
/// are raw and need not be UTF-8.
#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl Run {
    fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

/// **The single choke point.** Every git invocation in this module goes through
/// here, so the non-interactive environment, the piped stdio and the wall-clock
/// timeout are properties of the module rather than of each call site.
///
/// `args` is a `Vec<OsString>` handed straight to [`Command::args`]. There is no
/// shell, no string interpolation, and no quoting to get wrong. `label` is the
/// subcommand name used in errors, passed in rather than parsed back out of
/// `args` so that no error message can ever accidentally quote a pathspec.
///
/// # Why the plumbing looks like this
///
/// - **stdin is `/dev/null`** and `GIT_TERMINAL_PROMPT=0`, so git cannot block
///   reading a credential from a terminal it happened to inherit.
/// - **`GIT_EDITOR=false`** so nothing can spawn an editor into a GUI process
///   that has no terminal to show it in. It fails loudly instead of hanging.
/// - **stdout and stderr are drained by threads.** Reading them in sequence
///   after `wait()` deadlocks the moment either pipe's buffer fills, which is a
///   real prospect for a chatty pre-commit hook.
/// - **On timeout the reader threads are dropped, not joined.** `kill` reaches
///   git, but a *grandchild* it spawned — a hook, a pinentry dialog — may still
///   hold the write end of those pipes, so joining could block for exactly as
///   long as the timeout existed to prevent. Detaching them costs one idle
///   thread that ends when the grandchild does.
///
/// `cwd` is `None` only for `git --version`, which is not about any repository.
pub(crate) fn run(
    cwd: Option<&Path>,
    label: &str,
    args: &[OsString],
    timeout: Duration,
) -> Result<Run, GitError> {
    let mut command = Command::new(GIT);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "false")
        .env("GIT_PAGER", "cat")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GCM_INTERACTIVE", "never");
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GitError::NotFound
        } else {
            GitError::Unrunnable {
                command: label.to_string(),
                detail: error.to_string(),
            }
        }
    })?;

    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    match wait_with_timeout(&mut child, timeout) {
        Wait::Exited(code) => Ok(Run {
            code,
            stdout: join(stdout),
            stderr: join(stderr),
        }),
        Wait::TimedOut => Err(GitError::Timeout {
            command: label.to_string(),
            seconds: timeout.as_secs(),
        }),
        Wait::Broken(detail) => Err(GitError::Unrunnable {
            command: label.to_string(),
            detail,
        }),
    }
}

/// Outcome of waiting on a child within a budget.
enum Wait {
    /// Exited on its own; `None` means it was killed by a signal.
    Exited(Option<i32>),
    /// Outlived the budget and was killed.
    TimedOut,
    /// The wait itself failed.
    Broken(String),
}

/// Poll `child` to completion, or to `timeout` and then kill it.
///
/// A poll loop rather than a waiter thread with a channel: [`Child::wait`] needs
/// `&mut Child`, so a thread doing the waiting would hold the only handle that
/// could also `kill` it, and the timeout could never be acted upon.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Wait {
    let deadline = Instant::now() + timeout;
    let mut gap = POLL_START;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Wait::Exited(status.code()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Wait::TimedOut;
            }
            Ok(None) => {
                std::thread::sleep(gap);
                gap = (gap * 2).min(POLL_MAX);
            }
            Err(error) => {
                let _ = child.kill();
                return Wait::Broken(error.to_string());
            }
        }
    }
}

/// Read a child pipe to EOF on its own thread. A read error yields whatever was
/// read so far: partial output is more useful than none when the job is to
/// report why a commit was refused.
fn drain<R: Read + Send + 'static>(source: Option<R>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        source
            .map(|mut reader| {
                let mut buffer = Vec::new();
                let _ = reader.read_to_end(&mut buffer);
                buffer
            })
            .unwrap_or_default()
    })
}

/// Collect a drained pipe, treating a panicked reader thread as empty output.
fn join(handle: JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

/// The global flags, then the subcommand and its options, then the pathspecs.
///
/// `rest` must end with `--`, which is what makes a file named `-f` a file.
fn invocation(rest: &[&str], paths: &[String]) -> Vec<OsString> {
    GLOBAL_FLAGS
        .iter()
        .copied()
        .chain(rest.iter().copied())
        .map(OsString::from)
        .chain(paths.iter().map(OsString::from))
        .collect()
}

/// The complete argument vector of every git invocation this module makes, one
/// builder each.
///
/// They are named functions rather than inline literals so that
/// [`tests::no_invocation_can_sweep_the_whole_repository`] can lint **what the
/// module actually runs**. A test over argument lists retyped inside the test
/// would only ever prove the test right.
fn version_args() -> Vec<OsString> {
    invocation(&["--version"], &[])
}

/// See [`version_args`].
fn toplevel_args() -> Vec<OsString> {
    invocation(&["rev-parse", "--show-toplevel"], &[])
}

/// See [`version_args`]. `status.relativePaths=false` pins the output to
/// toplevel-relative regardless of the user's own config.
fn status_args(paths: &[String]) -> Vec<OsString> {
    invocation(
        &[
            "-c",
            "status.relativePaths=false",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--ignored=matching",
            "--",
        ],
        paths,
    )
}

/// See [`version_args`].
fn ls_files_args(paths: &[String]) -> Vec<OsString> {
    invocation(&["ls-files", "-z", "--"], paths)
}

/// See [`version_args`].
fn add_args(paths: &[String]) -> Vec<OsString> {
    invocation(&["add", "--"], paths)
}

/// See [`version_args`]. `--only` states on the command line what git's
/// pathspec handling would otherwise merely imply: commit these paths and
/// nothing else, leaving the rest of the index alone.
pub(crate) fn commit_args(message: &str, paths: &[String]) -> Vec<OsString> {
    invocation(&["commit", "--only", "--message", message, "--"], paths)
}

/// Parse `git status --porcelain=v1 -z` output.
///
/// Each record is `XY<space><path>`, NUL-terminated. A rename or copy record is
/// followed by a **second** NUL-terminated field holding the original path; it
/// has to be consumed, or every subsequent record is read as a path and the
/// whole parse desynchronises.
fn parse_porcelain(bytes: &[u8]) -> Vec<Classified> {
    let mut fields = bytes.split(|byte| *byte == 0).filter(|f| !f.is_empty());
    let mut entries = Vec::new();
    while let Some(record) = fields.next() {
        // "XY p" is the shortest possible record: two codes, a space, a name.
        let Some((codes, path)) = record.split_at_checked(3) else {
            continue;
        };
        let (x, y) = (codes[0], codes[1]);
        if [x, y].iter().any(|code| *code == b'R' || *code == b'C') {
            let _ = fields.next();
        }
        entries.push((text(path), state_of(x, y)));
    }
    entries
}

/// The two porcelain status codes as a [`FileState`].
///
/// Everything that is neither untracked nor ignored is `Modified` — including
/// index-only changes such as `M ` and `A `. That is deliberate: an import's
/// safety net is `git revert`, and a target carrying *any* uncommitted content
/// is one whose pre-import state was never captured.
fn state_of(x: u8, y: u8) -> FileState {
    match (x, y) {
        (b'?', b'?') => FileState::Untracked,
        (b'!', b'!') => FileState::Ignored,
        _ => FileState::Modified,
    }
}

/// The state of one requested path, given everything `status` reported and
/// everything `ls-files` knows.
///
/// The directory arm is not defensive coding: `--ignored=matching` reports an
/// ignored file inside an ignored directory as the *directory*, so a request
/// for `imports/statement.csv` in a repository ignoring `imports/` comes back
/// as `imports/` and can only be matched on prefix.
fn classify(path: &str, reported: &[Classified], tracked: &[String]) -> FileState {
    let exact = reported
        .iter()
        .find(|(name, _)| name == path)
        .map(|(_, state)| *state);
    let under_directory = || {
        reported
            .iter()
            .find(|(name, _)| name.ends_with('/') && path.starts_with(name.as_str()))
            .map(|(_, state)| *state)
    };
    exact.or_else(under_directory).unwrap_or_else(|| {
        if tracked.iter().any(|name| name == path) {
            FileState::Clean
        } else {
            FileState::Untracked
        }
    })
}

/// Split NUL-delimited output into strings, dropping the trailing empty field.
fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(text)
        .collect()
}

/// Bytes as a displayable string. Lossy: git emits raw path bytes under `-z`,
/// and a path that is not UTF-8 must still be reportable.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The directory to start a repository search from: `path` itself when it is a
/// directory, otherwise its parent, since an import target frequently does not
/// exist yet.
fn search_dir(path: &Path) -> Option<PathBuf> {
    let candidate = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    // `Path::parent` of a bare relative name is the empty path, which is not a
    // directory any process can be spawned in.
    let candidate = if candidate.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        candidate
    };
    candidate.is_dir().then_some(candidate)
}

/// `path` with every symlink resolved, so it is comparable with the toplevel
/// git reports.
///
/// Required, not cosmetic. On macOS `/tmp` is a symlink to `/private/tmp`, so
/// `git rev-parse --show-toplevel` inside a temp directory answers
/// `/private/tmp/…` while the caller is holding `/tmp/…`; a naive
/// `strip_prefix` between the two finds no common prefix at all, and every
/// target would be reported as outside its own repository.
///
/// A path that does not exist yet cannot be canonicalized, so its parent is
/// resolved and the file name re-appended.
fn physical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => std::fs::canonicalize(parent).map(|dir| dir.join(name)),
            _ => Err(std::io::ErrorKind::NotFound.into()),
        }
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
    })
}

/// A relative path as the forward-slashed string the wire uses.
fn slashed(relative: &Path) -> String {
    let rendered = relative.to_string_lossy().into_owned();
    if cfg!(windows) {
        rendered.replace(std::path::MAIN_SEPARATOR, "/")
    } else {
        rendered
    }
}

/// The most we will say about a path we could not place inside the repository:
/// its final component, and only when it has one.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "path".to_string())
}

/// First occurrence wins, order preserved. Linear scans, over a handful of
/// import targets.
fn dedup_preserving_order(paths: Vec<String>) -> Vec<String> {
    paths.into_iter().fold(Vec::new(), |mut kept, path| {
        if !kept.contains(&path) {
            kept.push(path);
        }
        kept
    })
}

/// Drop the [`FileState`]s from a classified list.
fn names(entries: Vec<Classified>) -> Vec<String> {
    entries.into_iter().map(|(name, _)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The porcelain parser is the one piece of this module with a wire format
    /// to get wrong, and the rename form is the trap: its second NUL field
    /// would otherwise be read as the next record's status codes.
    #[test]
    fn porcelain_parses_states_paths_and_renames() {
        let raw =
            b" M mod.txt\0?? new.csv\0!! secret.csv\0R  after.txt\0before.txt\0A  staged.txt\0";
        assert_eq!(
            parse_porcelain(raw),
            vec![
                ("mod.txt".to_string(), FileState::Modified),
                ("new.csv".to_string(), FileState::Untracked),
                ("secret.csv".to_string(), FileState::Ignored),
                ("after.txt".to_string(), FileState::Modified),
                // Proof the rename's original path was consumed rather than
                // parsed: `staged.txt` is still read as its own record.
                ("staged.txt".to_string(), FileState::Modified),
            ]
        );
    }

    /// `-z` output is unquoted, so a space or a quote in a filename is just
    /// bytes — the reason we do not use the line-based form.
    #[test]
    fn porcelain_keeps_awkward_filenames_verbatim() {
        let raw = b"?? has space.csv\0 M dir/say \"hi\".journal\0?? -f\0";
        assert_eq!(
            parse_porcelain(raw),
            vec![
                ("has space.csv".to_string(), FileState::Untracked),
                ("dir/say \"hi\".journal".to_string(), FileState::Modified),
                ("-f".to_string(), FileState::Untracked),
            ]
        );
    }

    /// Index-only changes block. A journal that is staged but not committed has
    /// no committed pre-import state to revert to, which is the whole point.
    #[test]
    fn staged_only_changes_count_as_modified() {
        for (x, y) in [(b'M', b' '), (b'A', b' '), (b' ', b'M'), (b'M', b'M')] {
            assert_eq!(state_of(x, y), FileState::Modified);
        }
        assert_eq!(state_of(b'?', b'?'), FileState::Untracked);
        assert_eq!(state_of(b'!', b'!'), FileState::Ignored);
    }

    /// An ignored directory shadows the file inside it — the single most
    /// surprising thing about `--ignored=matching`, and the reason `classify`
    /// has a prefix arm at all.
    #[test]
    fn a_path_under_an_ignored_directory_is_ignored() {
        let reported = vec![("imports/".to_string(), FileState::Ignored)];
        assert_eq!(
            classify("imports/statement.csv", &reported, &[]),
            FileState::Ignored
        );
        // A sibling that merely shares a textual prefix must not be swept up.
        assert_eq!(
            classify("imports-notes.txt", &reported, &[]),
            FileState::Untracked
        );
    }

    /// The distinction `git status` cannot make on its own: silence means
    /// "clean" for a tracked path and "unknown" for anything else.
    #[test]
    fn silence_means_clean_only_for_tracked_paths() {
        let tracked = vec!["main.journal".to_string()];
        assert_eq!(classify("main.journal", &[], &tracked), FileState::Clean);
        assert_eq!(classify("new.csv", &[], &tracked), FileState::Untracked);
    }

    /// Exactness beats prefix: a directory entry must not override a report
    /// about the file itself.
    #[test]
    fn an_exact_report_wins_over_a_directory_prefix() {
        let reported = vec![
            ("imports/".to_string(), FileState::Ignored),
            ("imports/kept.csv".to_string(), FileState::Modified),
        ];
        assert_eq!(
            classify("imports/kept.csv", &reported, &[]),
            FileState::Modified
        );
    }

    /// Argument construction: global flags first, pathspecs last, `--` between.
    #[test]
    fn invocations_put_pathspecs_after_the_double_dash() {
        let args = invocation(&["add", "--"], &["-f".to_string(), "a b.csv".to_string()]);
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec![
                "--no-pager",
                "--literal-pathspecs",
                "add",
                "--",
                "-f",
                "a b.csv"
            ]
        );
    }

    /// **No invocation may ever sweep the repository.** This lints the real
    /// builders — the very vectors [`Repo::status`] and [`Repo::commit`] hand to
    /// [`Command`] — so an edit that reaches for `git add -A`, `git add .` or
    /// `commit -a` fails here rather than in somebody's repository.
    #[test]
    fn no_invocation_can_sweep_the_whole_repository() {
        let targets = vec!["one.csv".to_string()];
        let pathspec_bearing = [
            status_args(&targets),
            ls_files_args(&targets),
            add_args(&targets),
            commit_args("ledgeline: import", &targets),
        ];

        for args in &pathspec_bearing {
            let rendered: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            for flag in ["-A", "--all", "-a", "-u", "--update", "."] {
                assert!(
                    !rendered.iter().any(|arg| arg == flag),
                    "{rendered:?} contains the sweep flag {flag}"
                );
            }
            // Every pathspec list is terminated, and the paths come last, so a
            // file named `-f` can never be read as an option.
            let terminator = rendered
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or_else(|| panic!("{rendered:?} has no `--`"));
            assert_eq!(
                &rendered[terminator + 1..],
                targets.as_slice(),
                "everything after `--` must be exactly the requested paths"
            );
        }

        // Global flags lead every invocation, including the two that take no
        // pathspecs at all.
        for args in pathspec_bearing
            .iter()
            .chain([version_args(), toplevel_args()].iter())
        {
            let rendered: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            assert_eq!(&rendered[..2], &["--no-pager", "--literal-pathspecs"]);
        }
    }

    /// `commit` is the dangerous one, so its shape is pinned exactly.
    #[test]
    fn the_commit_invocation_is_only_ever_these_arguments() {
        let rendered: Vec<String> = commit_args(
            "ledgeline: import 3 transactions",
            &["statement.csv".to_string(), "main.journal".to_string()],
        )
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
        assert_eq!(
            rendered,
            vec![
                "--no-pager",
                "--literal-pathspecs",
                "commit",
                "--only",
                "--message",
                "ledgeline: import 3 transactions",
                "--",
                "statement.csv",
                "main.journal",
            ]
        );
    }

    #[test]
    fn duplicate_targets_collapse_to_one() {
        assert_eq!(
            dedup_preserving_order(vec![
                "b.csv".to_string(),
                "a.journal".to_string(),
                "b.csv".to_string(),
            ]),
            vec!["b.csv".to_string(), "a.journal".to_string()]
        );
    }

    /// No error message may carry a path the caller did not already have.
    #[test]
    fn an_outside_path_is_named_by_its_file_name_only() {
        let rendered = GitError::Outside {
            name: file_label(Path::new("/Users/someone/private/secrets.journal")),
        }
        .to_string();
        assert!(rendered.contains("secrets.journal"), "{rendered}");
        assert!(!rendered.contains("/Users/someone"), "{rendered}");
    }

    #[test]
    fn error_messages_read_as_sentences() {
        assert_eq!(
            GitError::Failed {
                command: "commit".to_string(),
                code: Some(1),
                detail: "policy: no imports on friday".to_string(),
            }
            .to_string(),
            "`git commit` failed (exit 1): policy: no imports on friday"
        );
        assert_eq!(
            GitError::NothingToCommit {
                ignored: vec!["statement.csv".to_string()],
            }
            .to_string(),
            "nothing to commit: every path is gitignored (statement.csv)"
        );
        assert_eq!(
            GitError::NothingToCommit {
                ignored: Vec::new(),
            }
            .to_string(),
            "nothing to commit"
        );
    }

    /// Every budget is finite and ordered by how much work the command may
    /// legitimately do. A zero or absent timeout is the bug this guards.
    #[test]
    fn every_timeout_is_finite_and_ordered() {
        assert!(PROBE_TIMEOUT > Duration::ZERO);
        assert!(PROBE_TIMEOUT < STATUS_TIMEOUT);
        assert!(STATUS_TIMEOUT < COMMIT_TIMEOUT);
    }

    /// A destination that does not exist yet — the CSV an import is about to
    /// write — still resolves to an absolute, symlink-free path.
    #[test]
    fn a_path_that_does_not_exist_yet_still_resolves() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let target = dir.path().join("not-written-yet.csv");
        let resolved = physical(&target);
        assert!(resolved.is_absolute(), "{}", resolved.display());
        assert_eq!(resolved.file_name(), target.file_name());
        // The parent was canonicalized, which is what makes `strip_prefix`
        // against a git toplevel work on macOS.
        assert_eq!(
            resolved.parent().map(Path::to_path_buf),
            std::fs::canonicalize(dir.path()).ok()
        );
    }

    /// `GitStatus::unavailable` is the "no version control here" answer, and it
    /// must never be mistaken for a repository with nothing wrong in it.
    #[test]
    fn unavailable_is_distinguishable_from_a_clean_repo() {
        let status = GitStatus::unavailable();
        assert!(!status.available);
        assert!(status.files.is_empty() && status.dirty.is_empty());
    }
}
