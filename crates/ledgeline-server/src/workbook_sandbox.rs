//! Workbook parsing in a resource-limited child process.
//!
//! # Why a process
//!
//! `calamine` sizes allocations from numbers a workbook *declares* — a sheet's
//! bounding box, a shared-string count, an xls `DIMENSIONS` record, a compound
//! file's FAT length, an ods `number-columns-repeated` — and none of them is
//! bounded by the upload cap, which counts *compressed* bytes. An xlsx holding
//! only `A1` and `XFD1048576` asks for ~550 GB. When an allocation like that
//! fails the process **aborts**: `CatchPanicLayer` never sees it, and the GUI
//! window and every other request go with it.
//!
//! `ledgeline-core`'s spreadsheet lane caps what it can see coming (xlsx/xlsb
//! sheets are read sparse and bounded — see `convert::spreadsheet`), but it
//! cannot reach inside `calamine`'s own `open`. So every xls/xlsx/xlsm/xlsb/ods
//! parse this binary does runs here instead: the server re-executes its own
//! binary with [`WORKER_ARG`], writes the upload to the child's stdin, and reads
//! the parsed result back as JSON from its stdout. Whatever the file does to the
//! child — exhaust its memory budget, spin, panic, abort — the server sees only
//! a failed parse and maps it to the same "could not be read" refusal a corrupt
//! file gets.
//!
//! # The limits, and why one of them is an allocator
//!
//! - **Memory: [`WorkerAllocator`]**, a counting wrapper around the system
//!   allocator that refuses any allocation taking the child past
//!   [`WORKER_MEMORY_BYTES`]. It is the primary guard because `setrlimit` is not
//!   one on macOS: measured on Darwin 27, `RLIMIT_AS` and `RLIMIT_DATA` both fail
//!   with `EINVAL` for any value below roughly 1 TiB (the process's existing
//!   virtual reservations already exceed it), so a 2 GiB limit cannot even be
//!   set there. The wrapper counts nothing until the worker switches it on, so
//!   the server and GUI pay one relaxed atomic load per allocation.
//! - **`RLIMIT_AS`** at `WORKER_ADDRESS_SPACE_BYTES`, Linux only, as a backstop
//!   for anything that maps memory without going through the Rust allocator.
//! - **`RLIMIT_CPU`** at [`WORKER_CPU_SECONDS`], and **`RLIMIT_CORE`** at zero: a
//!   worker that aborts is holding someone's bank statement, and that must not
//!   land on disk as a core file. Both of these *are* honoured on macOS
//!   (measured: a spinning child under a one-second `RLIMIT_CPU` dies of
//!   `SIGXCPU` at one second).
//! - **Wall clock: [`WORKER_WALL_CLOCK`]**, enforced by the parent, which kills
//!   the child. This is the guarantee; `RLIMIT_CPU` only stops a spinning child
//!   sooner and does nothing for a blocked one.
//! - **Output: [`WORKER_MAX_OUTPUT_BYTES`]**, enforced by the parent, which stops
//!   reading past it.
//!
//! The child is started with an empty environment, so the process token
//! (`LEDGELINE_TOKEN`) never reaches it, and with stderr discarded, so an abort
//! message does not land in the server's log as if the server had said it.
//!
//! # Which executable
//!
//! [`std::env::current_exe`] by default — in a macOS `.app` that is the bundle's
//! own binary, and [`run_worker_if_requested`] is the first thing `main` does,
//! so the child exits before any window, menu or event loop exists. A test
//! harness is a different binary that knows nothing of [`WORKER_ARG`], so the
//! in-process integration suites point [`set_worker_executable`] at the real
//! `ledgeline` binary cargo built for them.

use ledgeline_core::convert::{self, ConvertError, MAX_INPUT_BYTES, SourceFormat, Tabular};
use ledgeline_core::qb_journal::{self, QbJournal, QbJournalError};
use serde::de::DeserializeOwned;
use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

/// The hidden first argument that turns this binary into a workbook worker.
///
/// Checked by hand before `clap` ever runs, so it appears in no `--help` and no
/// usage line. Not a security boundary: anyone who can run the binary can run
/// the parser, which is all this does.
const WORKER_ARG: &str = "--ledgeline-internal-workbook-worker";

/// The most heap the worker may hold at once.
///
/// The largest sheet `convert::spreadsheet` accepts is ten million grid cells
/// (`MAX_SHEET_GRID`) at 32 bytes each, held about twice on its way to a table
/// and then rendered to strings and to JSON — around 1.5 GB at the very worst.
/// A real statement needs a few megabytes.
const WORKER_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Linux's `RLIMIT_AS` for the worker: [`WORKER_MEMORY_BYTES`] plus room for the
/// binary's own mappings, stacks and allocator slack, so the allocator budget is
/// always the limit actually hit and this is only ever a backstop.
#[cfg(target_os = "linux")]
const WORKER_ADDRESS_SPACE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// `RLIMIT_CPU` for the worker. A 16 MiB workbook parses in a few seconds.
const WORKER_CPU_SECONDS: u64 = 30;

/// How long the parent waits for the worker before killing it.
const WORKER_WALL_CLOCK: Duration = Duration::from_secs(60);

/// The most JSON the parent reads back from the worker.
///
/// A 100,000-row statement at twenty columns is around 20 MB of JSON.
const WORKER_MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

/// How often the parent checks whether the worker has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The detail on a [`ConvertError::Malformed`] when the worker did not produce
/// an answer. Fixed text, like every other spreadsheet refusal: what killed the
/// worker is not something to quote to a dialog.
const WORKER_REFUSED: &str =
    "the workbook could not be read within the memory and time a statement is allowed";

/// The detail when the worker could not be started at all — not the file's
/// fault, and worth telling apart from the refusal above when it happens.
const WORKER_UNAVAILABLE: &str = "the workbook reader could not be started";

// ---------------------------------------------------------------------------
// The three entry points the import paths call
// ---------------------------------------------------------------------------

/// [`convert::convert`], with every spreadsheet format parsed in the worker.
///
/// Delimited and OFX input never reaches `calamine` and is converted in this
/// process exactly as before. So are an empty or oversize workbook, which
/// `convert` refuses before parsing anything.
pub(crate) fn convert(format: SourceFormat, bytes: &[u8]) -> Result<Tabular, ConvertError> {
    if !format.is_spreadsheet() || bytes.is_empty() || bytes.len() > MAX_INPUT_BYTES {
        return convert::convert(format, bytes);
    }
    // The worker answers with the whole `Result`; named, because inference
    // would otherwise decode a bare `Tabular` and call every answer garbled.
    sandboxed::<Result<Tabular, ConvertError>>(Job::Convert(format), bytes).unwrap_or_else(
        |failure| {
            Err(ConvertError::Malformed {
                format,
                detail: failure.detail().to_string(),
            })
        },
    )
}

/// [`qb_journal::detect`], parsed in the worker whenever `calamine` could open
/// the bytes at all.
///
/// Every upload is asked this, CSVs included, so a spawn per CSV is avoided by
/// [`might_be_workbook`]. A worker that fails answers "not a QuickBooks
/// export", which sends the file on to [`convert`] — and a workbook that killed
/// the worker here will be refused there.
pub(crate) fn qb_detect(bytes: &[u8]) -> bool {
    if !might_be_workbook(bytes) {
        return qb_journal::detect(bytes);
    }
    sandboxed(Job::QbDetect, bytes).unwrap_or(false)
}

/// [`qb_journal::parse`], in the worker.
pub(crate) fn qb_parse(bytes: &[u8]) -> Result<QbJournal, QbJournalError> {
    sandboxed(Job::QbParse, bytes).unwrap_or(Err(QbJournalError::Unreadable))
}

/// Whether any of `calamine`'s readers could open `bytes`.
///
/// xls is a compound file, and `calamine` checks its eight-byte signature at
/// offset zero before reading anything else. xlsx and ods are zips, and a zip is
/// found from its end-of-central-directory record, whose signature must appear
/// somewhere in the bytes — which is why this does not just look for `PK` at the
/// front: a zip with junk prepended still opens. A false positive costs one
/// spawn; a false negative would parse in-process, so this errs wide.
fn might_be_workbook(bytes: &[u8]) -> bool {
    const COMPOUND_FILE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
    const ZIP_END_OF_CENTRAL_DIRECTORY: &[u8] = b"PK\x05\x06";
    bytes.starts_with(COMPOUND_FILE)
        || bytes
            .windows(ZIP_END_OF_CENTRAL_DIRECTORY.len())
            .any(|window| window == ZIP_END_OF_CENTRAL_DIRECTORY)
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// What the worker is asked to do, carried as its second argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Convert(SourceFormat),
    QbDetect,
    QbParse,
}

impl Job {
    fn arg(self) -> String {
        match self {
            Self::Convert(format) => format!("convert:{format}"),
            Self::QbDetect => "qb-detect".to_string(),
            Self::QbParse => "qb-parse".to_string(),
        }
    }

    fn from_arg(arg: &str) -> Option<Self> {
        match arg {
            "qb-detect" => Some(Self::QbDetect),
            "qb-parse" => Some(Self::QbParse),
            other => {
                let name = other.strip_prefix("convert:")?;
                SourceFormat::ALL
                    .into_iter()
                    .find(|format| format.as_str() == name)
                    .map(Self::Convert)
            }
        }
    }

    /// Run the job in this process, rendered as the JSON the parent expects.
    fn run(self, bytes: &[u8]) -> serde_json::Result<Vec<u8>> {
        match self {
            Self::Convert(format) => serde_json::to_vec(&convert::convert(format, bytes)),
            Self::QbDetect => serde_json::to_vec(&qb_journal::detect(bytes)),
            Self::QbParse => serde_json::to_vec(&qb_journal::parse(bytes)),
        }
    }
}

// ---------------------------------------------------------------------------
// The parent side
// ---------------------------------------------------------------------------

/// Why the worker produced no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerFailure {
    /// The executable could not be found or started.
    Spawn,
    /// It exited unsuccessfully: over its memory budget, over its CPU limit, a
    /// panic, or any other signal.
    Crashed,
    /// It ran past [`WORKER_WALL_CLOCK`] and was killed.
    TimedOut,
    /// It wrote more than [`WORKER_MAX_OUTPUT_BYTES`].
    OutputTooLarge,
    /// It exited cleanly with output that does not decode as the answer.
    Garbled,
}

impl WorkerFailure {
    fn detail(self) -> &'static str {
        match self {
            Self::Spawn => WORKER_UNAVAILABLE,
            Self::Crashed | Self::TimedOut | Self::OutputTooLarge | Self::Garbled => WORKER_REFUSED,
        }
    }
}

/// The parent's two limits, separate from the constants so the tests can run a
/// stand-in worker against small ones.
#[derive(Debug, Clone, Copy)]
struct Leash {
    wall_clock: Duration,
    max_output: usize,
}

const LEASH: Leash = Leash {
    wall_clock: WORKER_WALL_CLOCK,
    max_output: WORKER_MAX_OUTPUT_BYTES,
};

static WORKER_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

/// Run workbook workers from `path` instead of [`std::env::current_exe`].
///
/// For test harnesses that drive the import routes in-process: the harness is
/// not a `ledgeline` binary, so they point this at the one cargo built
/// (`env!("CARGO_BIN_EXE_ledgeline")`). First call wins; later calls are
/// ignored, so every test in a suite can make it without coordinating.
pub fn set_worker_executable(path: impl Into<PathBuf>) {
    let _ = WORKER_EXECUTABLE.set(path.into());
}

fn worker_executable() -> Option<PathBuf> {
    WORKER_EXECUTABLE
        .get()
        .cloned()
        .or_else(|| std::env::current_exe().ok())
}

/// Run `job` over `bytes` in a worker and decode its answer.
fn sandboxed<T: DeserializeOwned>(job: Job, bytes: &[u8]) -> Result<T, WorkerFailure> {
    let executable = worker_executable().ok_or(WorkerFailure::Spawn)?;
    let output = run_worker(&executable, job, bytes, LEASH)?;
    serde_json::from_slice(&output).map_err(|_| WorkerFailure::Garbled)
}

/// Start `executable` as a worker for `job`, feed it `bytes`, and collect its
/// stdout within `leash`.
///
/// stdin is written and stdout read on scoped threads so neither pipe can fill
/// and stall the other, while this thread watches the clock. Killing the child
/// closes both pipes, which is what lets those threads finish on every path.
fn run_worker(
    executable: &Path,
    job: Job,
    bytes: &[u8],
    leash: Leash,
) -> Result<Vec<u8>, WorkerFailure> {
    let mut child = Command::new(executable)
        .arg(WORKER_ARG)
        .arg(job.arg())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| WorkerFailure::Spawn)?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        reap(&mut child);
        return Err(WorkerFailure::Spawn);
    };

    let deadline = Instant::now() + leash.wall_clock;
    let (status, output) = std::thread::scope(|scope| {
        // A worker that dies early makes this write fail with a broken pipe,
        // which is fine: its exit status is what reports the failure. (Rust
        // ignores SIGPIPE, so the write fails rather than killing us.)
        scope.spawn(move || {
            let _ = stdin.write_all(bytes);
        });
        let reader = scope.spawn(move || {
            let mut output = Vec::new();
            // One byte past the limit is how "too large" is told apart from
            // "exactly the limit". Dropping the pipe here, rather than reading
            // on, is what makes an over-long writer fail instead of block.
            stdout
                .take(leash.max_output as u64 + 1)
                .read_to_end(&mut output)
                .map(|_| output)
        });
        let status = wait_until(&mut child, deadline);
        (status, reader.join())
    });

    // Too much output is checked before the exit status because it is usually
    // the *cause* of that status: the pipe closed under the worker's next write.
    match (status, output) {
        (None, _) => Err(WorkerFailure::TimedOut),
        (Some(_), Ok(Ok(output))) if output.len() > leash.max_output => {
            Err(WorkerFailure::OutputTooLarge)
        }
        (Some(status), Ok(Ok(output))) if status.success() => Ok(output),
        (Some(_), _) => Err(WorkerFailure::Crashed),
    }
}

/// The child's exit status, or `None` once `deadline` passes — by which time it
/// has been killed and reaped.
fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) | Err(_) => {
                reap(child);
                return None;
            }
        }
    }
}

/// Kill and reap a child. Both can only fail if it has already exited, which
/// is the state this is trying to reach.
fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

// ---------------------------------------------------------------------------
// The child side
// ---------------------------------------------------------------------------

/// If this process was started as a workbook worker, be one and return the exit
/// code; otherwise `None`, having touched nothing.
///
/// `main` calls this before parsing its arguments or creating any window.
#[must_use]
pub fn run_worker_if_requested() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(WORKER_ARG)) {
        return None;
    }
    let job = args
        .next()
        .and_then(|arg| arg.to_str().and_then(Job::from_arg));
    Some(job.map_or(ExitCode::FAILURE, work))
}

/// The worker's whole life: limit itself, read the upload, answer, exit.
fn work(job: Job) -> ExitCode {
    limit_self();
    let mut bytes = Vec::new();
    // One byte past the cap so the parse functions see an oversize input and
    // refuse it by name, rather than a silently truncated one.
    if std::io::stdin()
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    let Ok(output) = job.run(&bytes) else {
        return ExitCode::FAILURE;
    };
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(&output).and_then(|()| stdout.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Switch on the memory budget and apply the rlimits. Each rlimit is best
/// effort: the budget and the parent's clock are the guarantees, and an rlimit
/// the platform refuses (see the module docs on macOS) changes neither.
fn limit_self() {
    ENFORCING.store(true, Ordering::SeqCst);
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};
        let cap = |limit: u64| Rlimit {
            current: Some(limit),
            maximum: Some(limit),
        };
        let _ = setrlimit(Resource::Core, cap(0));
        let _ = setrlimit(Resource::Cpu, cap(WORKER_CPU_SECONDS));
        #[cfg(target_os = "linux")]
        let _ = setrlimit(Resource::As, cap(WORKER_ADDRESS_SPACE_BYTES));
    }
}

// ---------------------------------------------------------------------------
// The memory budget
// ---------------------------------------------------------------------------

/// Whether [`WorkerAllocator`] is counting. Only a worker ever sets it.
static ENFORCING: AtomicBool = AtomicBool::new(false);

/// Bytes allocated since counting began, net of frees. Signed because memory
/// allocated *before* counting began can be freed after, which drives it below
/// zero — loosening the budget by at most the few bytes `main` allocated before
/// [`limit_self`].
static IN_USE: AtomicIsize = AtomicIsize::new(0);

/// The system allocator, refusing any allocation that would take a worker past
/// [`WORKER_MEMORY_BYTES`].
///
/// A refusal is a null return, which Rust turns into an abort — so a workbook
/// that tries for 550 GB ends the worker within its first oversized request,
/// and the parent reports it like any other crash. Outside a worker this counts
/// nothing and forwards every call unchanged.
///
/// Installed with `#[global_allocator]` in `main.rs`; a library cannot install
/// one on its embedder's behalf, and the integration suites that link this
/// library never run a worker in their own process.
pub struct WorkerAllocator;

impl WorkerAllocator {
    /// Charge `size` bytes against the budget, or refuse.
    fn admit(size: usize) -> bool {
        if !ENFORCING.load(Ordering::Relaxed) {
            return true;
        }
        // Refused up front so the addition below cannot overflow.
        let Ok(size) = isize::try_from(size) else {
            return false;
        };
        let budget = WORKER_MEMORY_BYTES as isize;
        if size > budget {
            return false;
        }
        let before = IN_USE.fetch_add(size, Ordering::Relaxed);
        if before + size > budget {
            IN_USE.fetch_sub(size, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn release(size: usize) {
        if ENFORCING.load(Ordering::Relaxed) {
            // `size` was admitted, or allocated before counting began, so it is
            // a real allocation's size and fits in `isize`.
            IN_USE.fetch_sub(size as isize, Ordering::Relaxed);
        }
    }
}

// SAFETY: every method forwards to `System` with the caller's own arguments,
// unchanged, so `System`'s guarantees are this allocator's. The only addition is
// that an allocation may be refused with a null return, which `GlobalAlloc`
// permits.
unsafe impl GlobalAlloc for WorkerAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !Self::admit(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: forwarded unchanged; see the impl's comment.
        let block = unsafe { System.alloc(layout) };
        if block.is_null() {
            Self::release(layout.size());
        }
        block
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !Self::admit(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: forwarded unchanged; see the impl's comment.
        let block = unsafe { System.alloc_zeroed(layout) };
        if block.is_null() {
            Self::release(layout.size());
        }
        block
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged; see the impl's comment.
        unsafe { System.dealloc(block, layout) };
        Self::release(layout.size());
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old_size = layout.size();
        let growth = new_size.saturating_sub(old_size);
        if !Self::admit(growth) {
            return std::ptr::null_mut();
        }
        // SAFETY: forwarded unchanged; see the impl's comment.
        let moved = unsafe { System.realloc(block, layout, new_size) };
        if moved.is_null() {
            Self::release(growth);
        } else {
            Self::release(old_size.saturating_sub(new_size));
        }
        moved
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in worker: a shell script that ignores its arguments. Bodies use
    /// shell BUILTINS only — the worker runs with an empty environment, so with
    /// no `PATH` a Linux `sh` (the nix sandbox has no `/usr/bin` to fall back
    /// on) cannot find `sleep` or `yes`, and exits 127 instead.
    fn script(dir: &tempfile::TempDir, body: &str) -> PathBuf {
        let path = dir.path().join("worker.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the script writes");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("the script is executable");
        path
    }

    const SHORT: Leash = Leash {
        wall_clock: Duration::from_millis(500),
        max_output: 1024,
    };

    #[test]
    fn jobs_round_trip_through_their_argument() {
        let jobs = SourceFormat::ALL
            .into_iter()
            .map(Job::Convert)
            .chain([Job::QbDetect, Job::QbParse]);
        for job in jobs {
            assert_eq!(Job::from_arg(&job.arg()), Some(job));
        }
        assert_eq!(Job::from_arg("convert:docx"), None);
        assert_eq!(Job::from_arg("rm -rf"), None);
    }

    #[test]
    fn a_worker_that_hangs_is_killed_at_the_deadline() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let worker = script(&dir, "while :; do :; done");
        let started = Instant::now();
        assert_eq!(
            run_worker(&worker, Job::QbDetect, b"bytes", SHORT),
            Err(WorkerFailure::TimedOut)
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_worker_that_will_not_stop_talking_is_cut_off() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let worker = script(&dir, "while :; do echo y; done");
        assert_eq!(
            run_worker(&worker, Job::QbDetect, b"bytes", SHORT),
            Err(WorkerFailure::OutputTooLarge)
        );
    }

    #[test]
    fn a_worker_that_fails_is_a_crash_and_its_output_is_ignored() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let worker = script(&dir, "echo true; exit 3");
        assert_eq!(
            run_worker(&worker, Job::QbDetect, b"bytes", SHORT),
            Err(WorkerFailure::Crashed)
        );
    }

    #[test]
    fn a_worker_that_never_reads_its_input_does_not_wedge_the_parent() {
        // More than a pipe buffer of input, and a worker that answers without
        // reading any of it: the write fails, the answer still counts.
        let dir = tempfile::tempdir().expect("a temp dir");
        let worker = script(&dir, "echo true");
        let input = vec![b'x'; 4 * 1024 * 1024];
        assert_eq!(
            run_worker(&worker, Job::QbDetect, &input, SHORT),
            Ok(b"true\n".to_vec())
        );
    }

    #[test]
    fn a_missing_worker_is_a_spawn_failure_with_its_own_message() {
        let missing = Path::new("/nonexistent/ledgeline-worker");
        let failure = run_worker(missing, Job::QbDetect, b"bytes", SHORT).unwrap_err();
        assert_eq!(failure, WorkerFailure::Spawn);
        assert_ne!(failure.detail(), WorkerFailure::Crashed.detail());
    }

    #[test]
    fn only_bytes_a_reader_could_open_are_sent_to_a_worker() {
        assert!(!might_be_workbook(b"Date,Amount\n2026-01-05,-54.20\n"));
        assert!(might_be_workbook(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1rest"));
        // A zip with something prepended still opens, so the end record is
        // what counts, not the leading `PK\x03\x04`.
        assert!(might_be_workbook(b"junk before the archive PK\x05\x06tail"));
    }
}
