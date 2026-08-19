//! Locating, version-gating and running the `hledger` binary (WP-11).
//!
//! # This module and `git.rs` are the ONLY places `Command::new` may appear
//!
//! Ledgeline has, until now, run no subprocesses at all. WP-11 introduces
//! exactly two: `hledger` (here) and `git` (`git.rs`). Every other module that
//! needs a subprocess goes through one of them. The rule is worth stating as a
//! rule because the alternative — process spawning scattered across the import
//! pipeline — is how the four invariants below get quietly broken one call site
//! at a time. `docs/imports.md` says the same thing.
//!
//! The four invariants, enforced by [`Invocation`] and documented at its
//! definition:
//!
//! 1. **Arguments are a `Vec<OsString>`, never a shell string.** There is no
//!    `sh -c` anywhere in this codebase. Journal paths, rules paths and account
//!    names are all user data containing spaces, quotes and `$`.
//! 2. **Every call has a wall-clock timeout.** Ledgeline is a desktop GUI; a
//!    subprocess that never exits (a `git` GPG passphrase prompt, an `hledger`
//!    reading a FIFO) must not be able to hang the window forever.
//! 3. **stdout and stderr are captured SEPARATELY.** This one is load-bearing
//!    rather than tidy: `hledger import --dry-run` writes the proposed
//!    transactions to **stdout** as re-parseable journal text and its
//!    `would import N new transactions from FILE:` status line to **stderr**.
//!    The entire preview feature is that split. Merging the streams would put a
//!    human-readable status line in the middle of the journal text and force us
//!    to regex it back out.
//! 4. **The child never inherits our stdin.** It is `/dev/null` unless the
//!    caller supplies bytes, so nothing can block reading a terminal that a
//!    windowed app does not have.
//! 5. **No hledger we run reads a config file.** Every argv starts with
//!    [`NO_CONF`]. See below — this is the one invariant that defends against a
//!    file nobody in this process chose.
//!
//! # Why every invocation passes `--no-conf`
//!
//! hledger 1.40 introduced automatic config files, and the search is not opt-in:
//! it reads `hledger.conf` from the **current directory or any directory above
//! it**, then `$HOME/.hledger.conf`, then the XDG config dir. [`Invocation`]
//! deliberately sets no working directory, so without this flag we would inherit
//! whatever config happens to sit above wherever Ledgeline was launched from —
//! a file this application never chose, never read and cannot see.
//!
//! That is not untidiness. A config file can **replace the command**. Verified
//! against hledger 1.52, with a `hledger.conf` whose entire content is the bare
//! word `balance`:
//!
//! ```text
//! $ hledger import --dry-run -f rt.journal --rules ms.csv.rules ms.csv
//! hledger: Error: Unknown flag: --dry-run
//! * while parsing the following args, final command line:
//! *  balance import --dry-run -f rt.journal --rules ms.csv.rules ms.csv
//! ```
//!
//! Our command was demoted to an argument of somebody else's. hledger's own
//! manual states the rule: a first word in the config's general section that does
//! not begin with a dash is taken as the command, overriding the command line.
//! That example errors, but the shape generalises: a config could inject
//! `--auto`, `--forecast`, `-b`/`-e`, `--depth` or `--alias` into invocations
//! whose output this crate **parses** and whose results it **appends to the
//! user's journal**. Explicit flags do beat config ones (`-O json` on our command
//! line was verified to beat `-O csv` in a config), so the danger is not only
//! breakage — it is that our subprocesses stop being determined by this process.
//!
//! So the flag is added in [`Invocation::argv`], which is the single point every
//! invocation's arguments pass through on the way to `Command::args`, rather than
//! at the call sites — a call site is a thing that can be added without it.
//! `import_api`'s own tests lint the built argv of every invocation the import
//! path makes, because a behavioural test would need a hostile config file
//! planted on the machine running it.
//!
//! Reading a config file is a separate concern, deliberately kept separate:
//! [`ledgeline_core::hledger_conf`] parses one *itself* for the `--alias` options
//! it declares. We never delegate to a config by passing `--conf`.
//!
//! # Resolution order
//!
//! Fixed, and chosen so the most specific answer wins:
//!
//! 1. `prefs.hledger_path` — the user pointed at one in the settings form.
//! 2. `$LEDGELINE_HLEDGER` — a per-launch override, and what the tests drive.
//! 3. A path baked in at COMPILE time via `option_env!("LEDGELINE_HLEDGER_PATH")`,
//!    so the Nix flake can pin the exact store path it built against. Note that
//!    without a `build.rs` emitting `cargo:rerun-if-env-changed`, changing this
//!    variable does not by itself force a rebuild.
//! 4. `hledger` on `$PATH`.
//!
//! Steps 1–3 are explicit paths and are stat-checked
//! ([`prefs::is_executable_file`](crate::prefs::is_executable_file)) before we
//! try to run them; a candidate that is missing or unrunnable falls through to
//! the next. That fall-through is deliberate: a Nix garbage-collect can delete
//! the binary a preference names, and silently using a working `hledger` beats
//! refusing to import. What does NOT fall through is a binary that runs and
//! reports too old a version — see [`Hledger::resolve`].
//!
//! # Why 1.40 is the floor
//!
//! `--rules-file` was renamed to `--rules` in hledger 1.40 (the old spelling
//! survives only as a hidden alias). We emit `--rules`. Against 1.39 that is an
//! unrecognised flag and the failure surfaces mid-import as an unhelpful usage
//! dump, so the version is checked once, up front, and reported as
//! [`HledgerError::TooOld`] with a number the user can act on.

use crate::prefs::{self, Prefs};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use thiserror::Error;

/// The oldest hledger we will run: the release that renamed `--rules-file` to
/// `--rules`.
pub(crate) const MIN_HLEDGER: Version = Version {
    major: 1,
    minor: 40,
};

/// Env override naming an `hledger` binary (resolution step 2).
const HLEDGER_ENV: &str = "LEDGELINE_HLEDGER";

/// An `hledger` path baked in at build time (resolution step 3), for the Nix
/// flake to pin its own store path. `None` in an ordinary `cargo build`.
const BAKED_HLEDGER: Option<&str> = option_env!("LEDGELINE_HLEDGER_PATH");

/// Bare name looked up on `$PATH` (resolution step 4).
const PATH_LOOKUP: &str = "hledger";

/// The flag that stops hledger reading a config file nobody in this process
/// chose. Prepended to **every** argv by [`Invocation::argv`]; see the module
/// docs for what a config file can otherwise do to a command.
///
/// The long spelling rather than `-n`: when hledger echoes an argv back in a
/// usage dump, `--no-conf` says what it is, and there is no short option to
/// confuse it with.
pub(crate) const NO_CONF: &str = "--no-conf";

/// Wall-clock budget for the `--version` probe. Short: this runs during startup
/// / the capabilities request, and the answer is a single line of output.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Default wall-clock budget for an ordinary invocation. Generous enough for a
/// large `import --dry-run`, short enough that a hung child does not look like a
/// frozen application. Override per call with [`Invocation::timeout`].
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the wait loop checks whether the child has exited. The cost is one
/// wakeup per interval for the life of the call; the benefit is that a fast
/// command is not rounded up to a coarse tick.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Minimum time allowed for a drain thread to hand over output the child has
/// ALREADY finished writing. See [`collect`].
const COLLECT_GRACE: Duration = Duration::from_millis(250);

/// How long a spawn may keep waiting on a binary the kernel reports as *busy* —
/// generous, because the condition lasts microseconds and the alternative is
/// telling the user their hledger does not exist. See
/// [`spawn_retrying_while_busy`].
const BUSY_BUDGET: Duration = Duration::from_millis(250);

/// How often that wait retries. The cost of a needless attempt is one failed
/// `exec`.
const BUSY_INTERVAL: Duration = Duration::from_millis(5);

/// An hledger release, compared as NUMBERS.
///
/// Which matters more than it looks: hledger 1.9 and hledger 1.40 sort the wrong
/// way round as strings, and 1.9 is exactly the sort of ancient distro package
/// this gate exists to catch. Deriving `Ord` over `(major, minor)` in that field
/// order gives the correct comparison for free.
///
/// Patch level is deliberately not modelled. Nothing we branch on has ever been
/// introduced in a patch release, and parsing one more component is one more way
/// to fail to recognise a version we could have run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    pub(crate) major: u32,
    pub(crate) minor: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl Version {
    /// Parse the leading `hledger MAJOR.MINOR` out of `hledger --version` output.
    ///
    /// Real output is one line, shaped like `hledger 1.52, mac-aarch64`, but the
    /// tail varies by build: a patch component (`1.32.3`), a git describe suffix
    /// (`1.42.1-g8f3a2b1-20260115`), a different platform triple, or nothing at
    /// all. So this reads only what it needs — the first token must be
    /// `hledger`, and the second contributes its leading dotted-numeric run —
    /// and ignores the rest rather than trying to model it.
    ///
    /// A missing minor component reads as `.0`, so a hypothetical `hledger 2`
    /// is 2.0 rather than a parse failure. An optional leading `v` is tolerated
    /// because the cost of accepting a spelling hledger does not currently emit
    /// is nil, while rejecting a genuine binary costs the user their import.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut tokens = text
            .lines()
            .find(|line| !line.trim().is_empty())?
            .split_whitespace();
        if tokens.next()? != "hledger" {
            return None;
        }
        let mut parts = tokens
            .next()?
            .trim_start_matches(['v', 'V'])
            .split(|c: char| !c.is_ascii_digit() && c != '.')
            .next()?
            .split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Ok(0), str::parse).ok()?;
        Some(Self { major, minor })
    }
}

/// A resolved, version-checked `hledger`. Holding one is the proof that a
/// runnable binary of at least [`MIN_HLEDGER`] exists at [`path`](Self::path) —
/// which is why both fields are private and the only constructor is
/// [`resolve`](Self::resolve).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hledger {
    path: PathBuf,
    version: Version,
}

/// Why we have no usable `hledger`.
///
/// The three variants named in the WP-11 contract, plus [`TimedOut`](Self::TimedOut)
/// — a hung child is a distinct condition from a missing one and the banner has
/// to say so, where "could not run hledger" would send the user looking for a
/// binary that is right there.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum HledgerError {
    #[error("hledger was not found")]
    NotFound,
    #[error("hledger {found} is older than {min}")]
    TooOld { found: Version, min: Version },
    #[error("could not run hledger")]
    Unrunnable,
    #[error("hledger did not finish within {}s", .after.as_secs())]
    TimedOut { after: Duration },
}

/// The outcome of probing one candidate path.
enum Probe {
    /// It ran and reported this version.
    Reported(Version),
    /// Nothing runnable there — try the next candidate.
    Absent,
    /// Something ran but did not answer like hledger. Terminal: a binary named
    /// `hledger` that prints something else is a misconfiguration to report, not
    /// one to paper over by quietly using a different one.
    Unrecognised,
}

impl Hledger {
    /// Find a usable `hledger`, in the order documented on the module.
    ///
    /// # Errors
    /// [`HledgerError::NotFound`] if no candidate could be run at all;
    /// [`HledgerError::TooOld`] if the first candidate that answered reports
    /// below [`MIN_HLEDGER`]; [`HledgerError::Unrunnable`] if it answered with
    /// something that is not an hledger version banner.
    ///
    /// `TooOld` is terminal — it does NOT fall through to the next candidate.
    /// Silently ignoring the binary a user explicitly configured, because we
    /// found a newer one elsewhere, is how "I set the path and it still used the
    /// wrong one" happens. A too-old answer is a real answer, so we report it.
    pub(crate) fn resolve(prefs: &Prefs) -> Result<Self, HledgerError> {
        // Lazy: `find` stops at the first candidate that answers, so the common
        // case spawns exactly one process.
        let answered = Self::candidates(prefs)
            .map(|path| {
                let probe = probe_version(&path);
                (path, probe)
            })
            .find(|(_, probe)| !matches!(probe, Probe::Absent));

        match answered {
            None | Some((_, Probe::Absent)) => Err(HledgerError::NotFound),
            Some((_, Probe::Unrecognised)) => Err(HledgerError::Unrunnable),
            Some((path, Probe::Reported(version))) if version >= MIN_HLEDGER => {
                Ok(Self { path, version })
            }
            Some((_, Probe::Reported(found))) => Err(HledgerError::TooOld {
                found,
                min: MIN_HLEDGER,
            }),
        }
    }

    /// The candidate paths, best-first and lazily produced.
    ///
    /// The three explicit sources are stat-checked here so a stale preference
    /// costs a `stat` rather than a failed `fork`/`exec`, and so a path that
    /// names a directory or a FIFO is dropped before `Command` ever sees it. The
    /// `$PATH` fallback is a bare name by definition and cannot be checked
    /// without reimplementing the loader's search, so it is always offered and
    /// allowed to fail at spawn.
    fn candidates(prefs: &Prefs) -> impl Iterator<Item = PathBuf> {
        let explicit = [
            prefs.hledger_path.clone(),
            std::env::var_os(HLEDGER_ENV).map(PathBuf::from),
            BAKED_HLEDGER.map(PathBuf::from),
        ];
        explicit
            .into_iter()
            .flatten()
            .filter(|path| !path.as_os_str().is_empty())
            .filter(|path| prefs::is_executable_file(path))
            .chain(std::iter::once(PathBuf::from(PATH_LOOKUP)))
    }

    /// A resolved-looking `Hledger` for tests that only BUILD invocations.
    ///
    /// The private fields and the single `resolve` constructor exist so that
    /// holding one of these is proof a runnable binary was found. This bypasses
    /// that proof, which is why it is `#[cfg(test)]`: the argv lints in
    /// `import_api` assert on arguments and must not spawn a process to do it,
    /// and requiring a real hledger on the machine would make an
    /// argument-level assertion depend on the test host's installation.
    ///
    /// `allow(dead_code)` because `tests/prefs.rs` compiles this module in too
    /// (via `#[path]`) and resolves a real stub instead, so it has no use for
    /// this — and that file deliberately carries no blanket allow of its own.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn for_tests(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            version: MIN_HLEDGER,
        }
    }

    /// The version this binary reported at [`resolve`](Self::resolve) time.
    pub(crate) fn version(&self) -> Version {
        self.version
    }

    /// The resolved binary. Absolute unless it came from the `$PATH` fallback,
    /// in which case it is the bare name the OS loader will search for.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Build an invocation of this hledger. Nothing runs until
    /// [`Invocation::run`].
    ///
    /// Arguments are taken as anything `OsStr`-like and stored as `OsString`, so
    /// a journal path with a space, a quote or a `$` in it is passed through
    /// verbatim — there is no shell to quote for.
    ///
    /// ```ignore
    /// let output = hledger
    ///     .invoke(["import", "--dry-run", "--rules"])   // &str args
    ///     .arg(rules_path)                              // or an OsStr-like one
    ///     .timeout(Duration::from_secs(120))
    ///     .run()?;
    /// let proposed = output.stdout_lossy();   // journal text
    /// let status   = output.stderr_lossy();   // "would import N new transactions"
    /// ```
    pub(crate) fn invoke<I, S>(&self, args: I) -> Invocation
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Invocation {
            program: self.path.clone(),
            args: args
                .into_iter()
                .map(|arg| arg.as_ref().to_os_string())
                .collect(),
            timeout: DEFAULT_TIMEOUT,
            stdin: None,
            read_config: false,
        }
    }
}

/// One prepared subprocess call: program, arguments, timeout, optional stdin.
///
/// Immutable-by-move builder — every method consumes `self` and returns it — so
/// a half-configured invocation cannot be shared or reused by accident, and the
/// only thing that can run is a complete one.
///
/// This type is where the module's four invariants actually live:
///
/// * `args` is a `Vec<OsString>` handed straight to `Command::args`. **Never a
///   shell string, and there is no `sh -c`.**
/// * `timeout` always has a value (defaulting to [`DEFAULT_TIMEOUT`]); there is
///   no way to construct an unbounded call.
/// * `run` pipes stdout and stderr **separately** — see invariant 3 in the
///   module docs, which the whole dry-run preview depends on.
/// * stdin is `/dev/null` unless [`stdin`](Self::stdin) supplied bytes.
/// * [`argv`](Self::argv) prepends [`NO_CONF`], so no invocation this type can
///   build reads a config file. Invariant 5, and the only exception in the
///   crate is [`probe_version`]'s fallback, which is argued at its call site.
///
/// [`run`](Self::run) BLOCKS. Call it inside `tokio::task::spawn_blocking`
/// (under the existing `reports_api::compute` semaphore), never on the async
/// runtime's threads.
pub(crate) struct Invocation {
    program: PathBuf,
    /// The caller's arguments, **without** [`NO_CONF`]. See [`argv`](Self::argv).
    args: Vec<OsString>,
    timeout: Duration,
    stdin: Option<Vec<u8>>,
    /// Let hledger read a config file. `false` everywhere but the legacy
    /// version probe; there is no public setter.
    read_config: bool,
}

impl Invocation {
    /// The complete argument vector, exactly as `Command::args` will receive it.
    ///
    /// **This is the choke point for [`NO_CONF`]**, and the reason the flag is
    /// here rather than at any call site: every invocation in this crate is
    /// eventually run by [`run`](Self::run), `run` builds its `Command` from
    /// this method, and this method is the only thing that constructs the final
    /// vector. A new call site therefore inherits the flag by construction —
    /// there is nothing for it to forget.
    ///
    /// The flag goes **first**, before the subcommand, because it is a general
    /// option and a config file's own injected command word would otherwise sit
    /// ahead of it.
    ///
    /// `pub(crate)` so `import_api`'s tests can lint it. A behavioural test
    /// would have to plant a hostile `hledger.conf` above the test runner's
    /// working directory — which is the repository — so the argument-level
    /// assertion is the one that can actually run in CI.
    pub(crate) fn argv(&self) -> Vec<OsString> {
        if self.read_config {
            return self.args.clone();
        }
        std::iter::once(OsString::from(NO_CONF))
            .chain(self.args.iter().cloned())
            .collect()
    }

    /// Append one more argument.
    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Append several more arguments.
    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    /// Replace the wall-clock budget for this one call.
    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Feed `bytes` to the child on stdin and close it.
    ///
    /// Needed for the concatenation form the balance check requires: hledger's
    /// balance ASSERTIONS do not aggregate across two `-f` flags (it answers a
    /// silently wrong number, not an error), so combined verification has to be
    /// `cat A B | hledger -f- check`. This is that pipe, without a shell.
    pub(crate) fn stdin(mut self, bytes: Vec<u8>) -> Self {
        self.stdin = Some(bytes);
        self
    }

    /// Run to completion, or kill the child at the timeout.
    ///
    /// # Errors
    /// [`HledgerError::Unrunnable`] if the process could not be spawned or its
    /// pipes could not be read; [`HledgerError::TimedOut`] if it outlived the
    /// budget (it is killed, and whatever it had already written is discarded —
    /// partial journal text is worse than none).
    ///
    /// # Why the streams are drained on their own threads
    ///
    /// A pipe holds ~64 KiB. A child writing more than that to stdout blocks
    /// until someone reads it, so the obvious `wait()`-then-`read()` deadlocks
    /// on exactly the payload this module was built for — an `import --dry-run`
    /// of a few hundred transactions is comfortably past that. Both pipes are
    /// therefore drained concurrently while this thread waits for exit, and
    /// stdin is written on its own thread for the mirror-image reason (a child
    /// that does not read its input would otherwise block us mid-`write_all`).
    ///
    /// # Why those threads are DETACHED rather than joined
    ///
    /// The obvious spelling is `std::thread::scope`, which joins them for us.
    /// It hangs, and a test pins it (`a_hung_child_is_killed_at_the_timeout`).
    ///
    /// Killing a child does not kill ITS children, and a grandchild inherits the
    /// same stdout pipe. So after we time out and kill, the read end can stay
    /// open — held by a process we did not spawn and cannot see — and the join
    /// blocks for as long as the grandchild lives. Measured: a 250 ms timeout
    /// took 120 s to return. Joining the drain threads reintroduces exactly the
    /// unbounded wait the timeout exists to prevent.
    ///
    /// So the threads own their pipes outright, report through a channel, and
    /// are abandoned on the timeout path. An abandoned reader holds one pipe and
    /// one buffer and exits as soon as the last writer to that pipe does; the
    /// caller is not made to wait for it.
    pub(crate) fn run(self) -> Result<Output, HledgerError> {
        // Fixed before the spawn, so the process's own start-up cost counts
        // against the caller's budget rather than extending it.
        let deadline = Instant::now() + self.timeout;
        let mut command = Command::new(&self.program);
        command
            // Invariant 5 lands here: `argv`, never `self.args`.
            .args(self.argv())
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                // Invariant 4: never `inherit`. A windowed app has no terminal
                // for a child to block reading.
                Stdio::null()
            })
            // Separately, never merged and never inherited: invariant 3, which
            // the whole dry-run preview depends on.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Bounded by the caller's own deadline as well as by the retry budget,
        // so waiting out a busy binary can never extend a call past its timeout.
        let mut child = spawn_retrying_while_busy(&mut command, deadline)
            .map_err(|_| HledgerError::Unrunnable)?;

        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), self.stdin) {
            std::thread::spawn(move || {
                // A broken pipe here means the child exited without reading its
                // input, which is the child's business and not an error of ours;
                // the exit status and stderr describe what happened. Dropping
                // `pipe` at the end of this closure is what sends EOF.
                let _ = pipe.write_all(&bytes);
                let _ = pipe.flush();
            });
        }

        let status = wait_until(&mut child, deadline, self.timeout)?;
        Ok(Output {
            status,
            stdout: collect(&out, deadline, self.timeout)?,
            stderr: collect(&err, deadline, self.timeout)?,
        })
    }
}

/// Spawn, waiting out a kernel that says the program is **busy**.
///
/// Linux refuses to `exec` a file that any process still holds open for WRITING,
/// with `ETXTBSY` — and the process holding it need not be one that is doing any
/// writing. `fork` copies the file-descriptor table, so a write descriptor that
/// another thread had open at that instant lives on in the forked child until it
/// reaches its own `exec`, and any attempt to run that file in the window loses.
/// Writing a program and immediately running it from a multi-threaded process is
/// therefore a race, on a file that is complete, closed and executable.
///
/// Untreated it is the worst possible diagnosis: [`probe_version`] reads a spawn
/// failure as "not here, try the next candidate", so a transient `ETXTBSY` on the
/// binary the user configured resolves to `hledger was not found` — sending them
/// to look for a missing installation that is sitting right there. A package
/// manager replacing hledger during an import is the real-world trigger; this
/// crate's own tests, which write a stub and run it from a test binary running
/// tests on every core, are the frequent one and the reason this was found.
///
/// The window closes as soon as the forked child `exec`s — microseconds — so a
/// short poll covers it. macOS does not implement the behaviour at all, which is
/// why the test that pins this is Linux-only.
fn spawn_retrying_while_busy(
    command: &mut Command,
    deadline: Instant,
) -> std::io::Result<std::process::Child> {
    let give_up = deadline.min(Instant::now() + BUSY_BUDGET);
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < give_up =>
            {
                std::thread::sleep(BUSY_INTERVAL);
            }
            settled => return settled,
        }
    }
}

/// Drain one pipe to EOF on a detached thread, reporting through a channel.
///
/// Generic over the reader so `ChildStdout` and `ChildStderr` — distinct types
/// with no common trait object we can name — share one implementation.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // A send failure means the caller already gave up (it timed out and
        // dropped the receiver), which is not this thread's problem.
        let _ = sender.send(read_all(pipe));
    });
    receiver
}

/// Read everything the pipe will give us. An absent pipe reads as empty rather
/// than as an error: `Command` only leaves `None` where we did not ask for a
/// pipe, which cannot happen here, and inventing a failure for it would be noise.
fn read_all<R: Read>(pipe: Option<R>) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut buffer)?;
    }
    Ok(buffer)
}

/// Take one drained stream, still bounded by the call's deadline.
///
/// The child has already exited by the time this runs, so the pipes are closed
/// and the reader is at most microseconds from finishing — but "the child
/// exited" does not prove the pipe is closed (see [`Invocation::run`] on
/// grandchildren), so this waits with a bound rather than forever.
///
/// [`COLLECT_GRACE`] is the floor on that bound. Without it a command that
/// finished legitimately, just barely inside its budget, would report a timeout
/// for output that is already sitting in the channel.
fn collect(
    stream: &Receiver<std::io::Result<Vec<u8>>>,
    deadline: Instant,
    timeout: Duration,
) -> Result<Vec<u8>, HledgerError> {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(COLLECT_GRACE);
    match stream.recv_timeout(remaining) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(_)) => Err(HledgerError::Unrunnable),
        Err(RecvTimeoutError::Timeout) => Err(HledgerError::TimedOut { after: timeout }),
        // The reader thread panicked, which for a `read_to_end` into a `Vec`
        // means an allocation failure we cannot do anything useful about.
        Err(RecvTimeoutError::Disconnected) => Err(HledgerError::Unrunnable),
    }
}

/// Wait for the child to exit, or kill it at `deadline`.
///
/// A poll loop rather than a blocking `wait()` on a helper thread because we
/// need the `&mut Child` to kill with, and handing it to another thread to get a
/// blocking wait means we no longer have it when the timeout fires. The sleep is
/// clamped to the remaining budget, so a short timeout is honoured to within one
/// syscall rather than rounded up to a whole poll interval.
///
/// `timeout` is carried separately only to report it in the error; `deadline` is
/// what is enforced.
fn wait_until(
    child: &mut std::process::Child,
    deadline: Instant,
    timeout: Duration,
) -> Result<ExitStatus, HledgerError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Err(_) => return Err(HledgerError::Unrunnable),
            Ok(None) => {}
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            // Reap it, so a killed child does not linger as a zombie for the
            // life of a long-running desktop session.
            let _ = child.wait();
            return Err(HledgerError::TimedOut { after: timeout });
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline - now));
    }
}

/// What a finished invocation produced. `stdout` and `stderr` are separate
/// buffers and stay that way — see invariant 3 on the module.
#[derive(Debug, Clone)]
pub(crate) struct Output {
    /// The child's exit status. hledger uses a non-zero exit for a failed check
    /// or a rules error, which is normal and expected, so this is reported
    /// rather than turned into an `Err`.
    pub(crate) status: ExitStatus,
    /// For `import --dry-run`, the proposed transactions as journal text.
    pub(crate) stdout: Vec<u8>,
    /// For `import --dry-run`, the `would import N new transactions` line — and
    /// for a failure, hledger's own diagnostic, which is good enough to show the
    /// user verbatim.
    pub(crate) stderr: Vec<u8>,
}

impl Output {
    /// Did the child exit zero?
    pub(crate) fn success(&self) -> bool {
        self.status.success()
    }

    /// stdout as text, replacing any invalid UTF-8 rather than failing: this is
    /// shown to a user or re-parsed as a journal, and neither is improved by an
    /// error about byte 4,318.
    pub(crate) fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// stderr as text. See [`stdout_lossy`](Self::stdout_lossy).
    pub(crate) fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Ask one candidate binary what version it is.
///
/// A spawn failure or a timeout is [`Probe::Absent`] — "not here, try the next"
/// — because both are what a stale preference or a broken symlink look like.
/// Output that does not parse is [`Probe::Unrecognised`], which is terminal.
/// A binary that is momentarily *busy* looks like neither and is waited out
/// before it can be mistaken for a missing one: [`spawn_retrying_while_busy`].
///
/// # The one invocation that may fall back to reading a config
///
/// [`NO_CONF`] did not exist before hledger 1.40 — the same release that
/// introduced config files, and the same release [`MIN_HLEDGER`] is set to. So
/// against a 1.39 the flag is an unrecognised option: hledger prints a usage
/// dump, nothing parses as a version, and this would answer
/// [`Probe::Unrecognised`], which resolves to [`HledgerError::Unrunnable`] —
/// "could not run hledger" — for a binary sitting right there that we could have
/// named a version for.
///
/// Losing [`HledgerError::TooOld`] would be losing the one message that tells
/// that user what to do. So an unrecognised answer is retried **once** without
/// the flag, and only that retry may read a config. It is safe to make the
/// exception exactly here and nowhere else:
///
/// * it is only ever reached on a binary that has just rejected `--no-conf`,
///   i.e. one too old to read a config file at all;
/// * the sole thing done with the result is compare it against [`MIN_HLEDGER`],
///   which the retry can only make *more* conservative — a config cannot invent
///   a version banner, and anything unparseable stays `Unrecognised`;
/// * nothing it produces is parsed as a journal or written anywhere.
fn probe_version(program: &Path) -> Probe {
    let ask = |read_config: bool| {
        Invocation {
            program: program.to_path_buf(),
            args: vec![OsString::from("--version")],
            timeout: VERSION_TIMEOUT,
            stdin: None,
            read_config,
        }
        .run()
        .map_or(Probe::Absent, |output| {
            Version::parse(&output.stdout_lossy()).map_or(Probe::Unrecognised, Probe::Reported)
        })
    };
    match ask(false) {
        Probe::Unrecognised => ask(true),
        answered => answered,
    }
}
