//! The staging area: where a dropped statement lives between the upload and the
//! import (WP-11 lane E).
//!
//! `POST /api/import/stage` converts an uploaded file to one canonical CSV. That
//! CSV then has to survive several round trips to the browser — score candidate
//! rules files, dry-run, confirm, commit — and every one of those steps needs it
//! **on disk**, because `hledger` reads files and not buffers. This module owns
//! that disk area and its lifetime.
//!
//! # A `StageId` is not a path, and cannot be turned into one
//!
//! Same discipline as [`RulesPath`](ledgeline_core::rules::RulesPath), and for
//! the same reason: a handle a client supplies must never be *arithmetic* on a
//! filesystem location. So:
//!
//! * an id is 32 hex characters from the OS CSPRNG — not derived from the file
//!   name, the upload, a counter, or the clock;
//! * [`StageId::parse`] accepts that shape and nothing else, before any lookup;
//! * resolution is [`StageArea::get`], an **exact string match against a map
//!   this process built**. There is no `root.join(id)` in this module, and the
//!   directory a stage lives in is never handed out.
//!
//! A stage from a different server session is therefore unreachable twice over:
//! it is not in this area's map, and this area's root carries its own random
//! component so the two sessions do not even share a directory.
//!
//! # The directory, and why it is `0700`
//!
//! The OS temp dir is world-readable and shared with every other user on the
//! machine. A bank statement is exactly the sort of thing that must not be world
//! readable while it waits to be imported, so the session root is created with
//! mode `0700` and every stage below it inherits that. The root is removed on
//! [`Drop`], which covers both a graceful shutdown and the desktop window
//! closing; a `SIGKILL` leaves it behind, which is what the OS temp reaper is
//! for.
//!
//! # Why a stage is materialised under the *destination's* name
//!
//! `hledger import` de-duplicates using a state file called `.latest.NAME`, kept
//! **next to the data file** and keyed to its name. A dry-run performed against
//! a temp copy called `data.csv` would consult `.latest.data.csv`, which never
//! exists — so it would report every row as new, and then the real import would
//! silently drop the back-dated ones. That is the failure mode
//! `plans/11-enhanced-import.md` calls out, and reporting it truthfully *before*
//! anything is written is the whole point of the dry-run.
//!
//! So [`Stage::materialize`] copies the canonical CSV into a run directory under
//! the file name the destination will have, and copies the destination
//! directory's own `.latest.NAME` in beside it. The dry-run then sees exactly
//! the dedup state the real import will see.
//!
//! # Every copy hledger reads is aligned to a rules file's `skip`
//!
//! [`Stage::data`] is the **canonical** CSV: header on line 1, no padding. It is
//! what the preview is of and what a user saves. It is *not* what hledger is
//! ever pointed at, because a rules file's `skip` was written against the raw
//! download and our conversion stripped the preamble out from under it — see
//! [`convert::align_to_skip`](ledgeline_core::convert::align_to_skip), which
//! owns that whole argument.
//!
//! So both routes out of a stage take the `skip` of the rules file that copy
//! will be read with: [`Stage::materialize`] for an import, and
//! [`Stage::aligned`] for candidate scoring, where every candidate has a
//! different `skip` and therefore needs a different copy.

use ledgeline_core::convert::{self, SourceFormat};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// The largest upload accepted, enforced by an axum `DefaultBodyLimit` on the
/// stage route alone (never the global limit — every other route still takes
/// small JSON bodies).
///
/// Deliberately the same number as
/// [`convert::MAX_INPUT_BYTES`](ledgeline_core::convert::MAX_INPUT_BYTES), so a
/// file that gets past the HTTP layer is one the converter will also accept. A
/// year of transactions from any real bank is a few hundred kilobytes; 16 MiB is
/// two orders of magnitude of headroom and still bounds what one request can
/// make this process hold.
pub(crate) const MAX_UPLOAD_BYTES: usize = 16 * 1024 * 1024;

/// How many stages one session keeps. Oldest-first eviction.
///
/// A cap rather than an unbounded map because every stage holds a file for the
/// life of the session, and a client that drops fifty statements without
/// importing any of them should not be able to fill the temp filesystem. Real
/// use has one live stage; a user comparing two exports has two.
const MAX_LIVE_STAGES: usize = 8;

/// The canonical converted CSV inside a stage directory. Internal — it is never
/// what hledger is pointed at, and never appears on the wire.
const DATA_FILE: &str = "data.csv";

/// The name [`Stage::aligned`] gives a per-`skip` copy, completed with the skip
/// itself. A fixed prefix and a number, so nothing a client sent reaches a file
/// name here either.
const ALIGNED_PREFIX: &str = "aligned-";

/// The biggest `.latest.NAME` we will copy. The file holds one date, so anything
/// larger is not one and copying it would be pointless work on a path an
/// attacker could aim at a large file.
const MAX_LATEST_BYTES: u64 = 1024;

/// The run directory used for a dry-run or import that should see the real
/// dedup state.
pub(crate) const RUN_WITH_LATEST: &str = "with-latest";

/// The run directory used for the second, `.latest`-free dry-run whose count
/// difference is how many rows dedup would silently drop.
pub(crate) const RUN_BARE: &str = "bare";

/// An opaque handle to one staged upload.
///
/// The field is **private and there is no constructor from an arbitrary
/// string** — only [`StageId::parse`], which enforces the shape, and the
/// private mint. Holding one is not authority to read anything; it is only a key
/// that [`StageArea::get`] may or may not recognise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StageId(String);

/// The number of hex characters in an id: 128 bits of CSPRNG output.
const ID_HEX_CHARS: usize = 32;

impl StageId {
    /// A fresh id, or `None` if the OS CSPRNG refused.
    ///
    /// A failure is propagated rather than papered over with a clock- or
    /// pid-derived fallback. Unlike the `ETag` prefix in `lib.rs` — where
    /// collisions merely cost a re-fetch — this value is the only thing standing
    /// between one browser tab's staged bank statement and another's, so a
    /// guessable id is a real weakening and refusing the upload is the honest
    /// answer.
    fn mint() -> Option<Self> {
        let mut bytes = [0u8; ID_HEX_CHARS / 2];
        getrandom::fill(&mut bytes).ok()?;
        Some(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    /// The client's string as an id, or `None` if it is not the shape this
    /// module mints.
    ///
    /// Checked **before** any lookup and on shape alone, exactly like
    /// `rules_api::validate_id`: a handle that could not have come from
    /// [`mint`](Self::mint) never reaches the map, and the rejection tells the
    /// caller only what it already sent.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let well_formed = raw.len() == ID_HEX_CHARS
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        well_formed.then(|| Self(raw.to_string()))
    }

    /// The id as it goes on the wire.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// One staged upload: a private directory holding the converted CSV.
#[derive(Debug)]
pub(crate) struct Stage {
    /// This stage's own directory, under the session root. **Never handed out** —
    /// no accessor returns it, and nothing derived from it reaches a response.
    dir: PathBuf,
    /// What the upload was detected as, for the stage response.
    format: SourceFormat,
}

impl Stage {
    /// The format this upload was detected as.
    pub(crate) fn format(&self) -> SourceFormat {
        self.format
    }

    /// The **canonical** converted CSV: header on line 1, nothing prepended.
    ///
    /// This is the artifact — what the preview shows, what a commit writes to
    /// the user's destination, what `save-csv` keeps. Nothing hledger reads
    /// comes from here directly; see [`aligned`](Self::aligned) and
    /// [`materialize`](Self::materialize), which both apply the chosen rules
    /// file's `skip` alignment on the way past.
    pub(crate) fn data(&self) -> PathBuf {
        self.dir.join(DATA_FILE)
    }

    /// A copy of the CSV aligned to `skip`, for an invocation that reads the
    /// data and nothing else — candidate scoring's `hledger print`, which does
    /// not consult `.latest` and so needs no run directory.
    ///
    /// One file per distinct `skip`, kept inside the stage directory and
    /// therefore removed with it, because scoring runs each candidate's own
    /// rules file against this data and they do not agree on a `skip`. Handed
    /// back as [`data`](Self::data) itself when there is nothing to prepend, so
    /// the ordinary `skip 1` candidate costs no write at all.
    ///
    /// # Errors
    /// [`std::io::Error`] if the canonical CSV cannot be read or the aligned
    /// copy cannot be written.
    pub(crate) fn aligned(&self, skip: u32) -> std::io::Result<PathBuf> {
        if padding_lines(skip) == 0 {
            return Ok(self.data());
        }
        let path = self.dir.join(format!("{ALIGNED_PREFIX}{skip}.csv"));
        std::fs::write(&path, self.aligned_bytes(skip)?)?;
        Ok(path)
    }

    /// Place a copy of the CSV under `name` in the run directory `slot`, with
    /// the destination directory's `.latest.NAME` beside it, and return the path
    /// to that copy.
    ///
    /// `slot` is one of this module's own constants, never a client string.
    /// `name` is a bare file name — it comes from an already-validated relative
    /// `csvPath`, and is re-checked here so this function is safe on its own
    /// terms rather than on its caller's.
    ///
    /// `skip` is the `skip` of the rules file this copy will be imported with.
    /// The copy is **written, not `fs::copy`d**, precisely so the alignment
    /// cannot be forgotten by whoever adds the next caller: there is no route
    /// out of this module that hands hledger the unaligned bytes.
    ///
    /// The run directory is **recreated** each time: a stage is materialised
    /// again whenever the user changes the destination in the form, and a stale
    /// `.latest` from the previous destination would make the next dry-run
    /// report a dedup state that belongs to a different file.
    ///
    /// # Errors
    /// [`std::io::Error`] if the name is not a bare file name, or if the copy
    /// fails.
    pub(crate) fn materialize(
        &self,
        slot: &str,
        name: &str,
        latest_from: Option<&Path>,
        skip: u32,
    ) -> std::io::Result<PathBuf> {
        if !is_bare_name(name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a staged file name must be a single plain component",
            ));
        }
        let run = self.dir.join(slot);
        // `remove_dir_all` on a path that is not there is an error we want to
        // ignore; anything else surfaces from `create_private_dir` below.
        let _ = std::fs::remove_dir_all(&run);
        create_private_dir(&run)?;

        let staged = run.join(name);
        std::fs::write(&staged, self.aligned_bytes(skip)?)?;

        if let Some(dir) = latest_from
            && let Some(bytes) = read_latest(dir, name)
        {
            std::fs::write(run.join(latest_name(name)), bytes)?;
        }
        Ok(staged)
    }

    /// The canonical CSV's bytes with `skip`'s padding in front of them.
    ///
    /// Read as text rather than bytes because that is what `align_to_skip`
    /// takes, and it is sound here for a reason worth writing down: the only
    /// writer of this file is [`StageArea::put`], and the only thing it is ever
    /// given is `convert::to_csv` output, which is a `String`.
    fn aligned_bytes(&self, skip: u32) -> std::io::Result<Vec<u8>> {
        let csv = std::fs::read_to_string(self.data())?;
        Ok(convert::align_to_skip(&csv, skip).into_bytes())
    }
}

/// How many empty records `skip` calls for — the same
/// `skip.saturating_sub(1)` [`convert::align_to_skip`] applies, asked here only
/// so [`Stage::aligned`] can answer "nothing to do" without writing a file.
fn padding_lines(skip: u32) -> u32 {
    skip.saturating_sub(1)
}

/// The dedup marker `hledger import` keeps next to a data file: the newest date
/// it has already imported from a file of that name.
///
/// Returned as text so the caller can report it verbatim. `None` covers every
/// "no dedup state" case uniformly — absent, unreadable, not a regular file,
/// oversize, empty.
pub(crate) fn latest_marker(dir: &Path, name: &str) -> Option<String> {
    let bytes = read_latest(dir, name)?;
    let text = String::from_utf8(bytes).ok()?;
    let marker = text.split_whitespace().next()?;
    (!marker.is_empty()).then(|| marker.to_string())
}

/// `.latest.NAME`'s bytes, if it is a small regular file.
///
/// [`std::fs::symlink_metadata`] rather than `metadata`: a symlink named
/// `.latest.bank.csv` pointing at something large or unreadable is refused on
/// its own file type, without a second question being asked.
fn read_latest(dir: &Path, name: &str) -> Option<Vec<u8>> {
    let path = dir.join(latest_name(name));
    let meta = std::fs::symlink_metadata(&path).ok()?;
    (meta.file_type().is_file() && meta.len() <= MAX_LATEST_BYTES)
        .then(|| std::fs::read(&path).ok())
        .flatten()
}

/// The name hledger keys its import state by.
fn latest_name(name: &str) -> String {
    format!(".latest.{name}")
}

/// Is `name` a single plain path component?
fn is_bare_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(|c| c.is_ascii_control())
}

/// The per-session staging area. One per [`AppState`](crate::AppState), shared by
/// every clone of it.
#[derive(Debug, Default)]
pub(crate) struct StageArea {
    inner: Mutex<Area>,
}

/// The area's mutable half.
#[derive(Debug, Default)]
struct Area {
    /// Created lazily on the first upload, so a read-only server (and every
    /// oneshot test router) never makes a directory it will not use.
    root: Option<PathBuf>,
    /// Live stages, oldest first. A `Vec` rather than a map because it is capped
    /// at [`MAX_LIVE_STAGES`], so a linear scan is faster than hashing and the
    /// insertion order that eviction needs comes for free.
    stages: Vec<(StageId, Arc<Stage>)>,
}

impl StageArea {
    /// Stage `csv` and hand back its handle.
    ///
    /// # Errors
    /// [`std::io::Error`] if the session root, the stage directory or the CSV
    /// could not be written, or if the OS CSPRNG refused to mint an id.
    pub(crate) fn put(
        &self,
        csv: &str,
        format: SourceFormat,
    ) -> std::io::Result<(StageId, Arc<Stage>)> {
        let mut area = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let root = match &area.root {
            Some(root) => root.clone(),
            None => {
                let root = session_root()?;
                create_private_dir(&root)?;
                area.root = Some(root.clone());
                root
            }
        };

        let id = StageId::mint().ok_or_else(|| {
            std::io::Error::other("the operating system's random source is unavailable")
        })?;
        let dir = root.join(id.as_str());
        create_private_dir(&dir)?;
        std::fs::write(dir.join(DATA_FILE), csv.as_bytes())?;

        let stage = Arc::new(Stage { dir, format });
        area.stages.push((id.clone(), Arc::clone(&stage)));
        // Evict from the front, so the stage a user is actively working with —
        // always the newest — is the last one to go.
        while area.stages.len() > MAX_LIVE_STAGES {
            let (_, evicted) = area.stages.remove(0);
            let _ = std::fs::remove_dir_all(&evicted.dir);
        }
        Ok((id, stage))
    }

    /// The stage `id` names, or `None`.
    ///
    /// This is the **only** id → stage resolution, by exact equality against a
    /// map this process built. See the module docs.
    pub(crate) fn get(&self, id: &StageId) -> Option<Arc<Stage>> {
        let area = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        area.stages
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, stage)| Arc::clone(stage))
    }
}

impl Drop for StageArea {
    /// Remove the whole session root, and with it every stage.
    ///
    /// Best-effort: a failure here is not something a shutting-down process can
    /// act on, and the alternative — refusing to exit — is worse.
    fn drop(&mut self) {
        let area = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(root) = &area.root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// A fresh, unguessable session root under the OS temp directory.
///
/// The random component is what makes two servers in one process — which is
/// exactly what the integration tests build — unable to see each other's stages
/// even before the map lookup gets a chance to refuse.
fn session_root() -> std::io::Result<PathBuf> {
    let id = StageId::mint().ok_or_else(|| {
        std::io::Error::other("the operating system's random source is unavailable")
    })?;
    Ok(std::env::temp_dir().join(format!("ledgeline-stage-{}", id.as_str())))
}

/// Create a directory only this user can enter.
///
/// `create` rather than `create_dir_all`: the name carries 128 bits of CSPRNG
/// output, so it cannot already exist unless somebody is trying to make it
/// exist — and failing on that is the point.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

/// Windows has no mode bits; the per-user temp directory is the protection
/// there. Ledgeline ships for macOS and Linux, so this is a courtesy rather than
/// a supported path.
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape gate in full. Anything that is not 32 lowercase hex characters
    /// must be refused before it can be looked up — including the spellings
    /// somebody probing for a path would try.
    #[test]
    fn only_a_minted_shape_parses_as_an_id() {
        let minted = StageId::mint().expect("the CSPRNG works");
        assert_eq!(
            StageId::parse(minted.as_str()).as_ref(),
            Some(&minted),
            "a minted id must round-trip"
        );

        for raw in [
            "",
            "../../etc/passwd",
            "0123456789abcdef0123456789abcde",   // 31
            "0123456789abcdef0123456789abcdef0", // 33
            "0123456789ABCDEF0123456789abcdef",  // upper case
            "0123456789abcdef0123456789abcdeg",  // not hex
            "0123456789abcdef0123456789abcd/f",
            "0123456789abcdef0123456789abcd.f",
        ] {
            assert!(StageId::parse(raw).is_none(), "{raw:?} must not parse");
        }
    }

    /// Two ids from one process must differ — the property the whole handle
    /// rests on.
    #[test]
    fn minted_ids_do_not_repeat() {
        let ids: std::collections::BTreeSet<String> = (0..64)
            .map(|_| {
                StageId::mint()
                    .expect("the CSPRNG works")
                    .as_str()
                    .to_string()
            })
            .collect();
        assert_eq!(ids.len(), 64);
    }

    /// A staged file lands in a private directory and reads back byte for byte.
    #[test]
    fn a_stage_holds_the_csv_it_was_given() {
        let area = StageArea::default();
        let (id, stage) = area.put("a,b\n1,2\n", SourceFormat::Csv).expect("stage");
        assert_eq!(
            std::fs::read_to_string(stage.data()).expect("read back"),
            "a,b\n1,2\n"
        );
        assert_eq!(stage.format(), SourceFormat::Csv);
        assert!(area.get(&id).is_some(), "a fresh stage resolves");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(stage.data().parent().expect("parent"))
                .expect("stat")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "a stage directory must be private");
        }
    }

    /// The map is the only resolution: an id minted by a *different* area is a
    /// stranger, even though it is perfectly well-formed.
    #[test]
    fn one_areas_id_never_resolves_in_another() {
        let mine = StageArea::default();
        let theirs = StageArea::default();
        let (id, _) = theirs.put("a\n1\n", SourceFormat::Csv).expect("stage");
        assert!(mine.get(&id).is_none());
    }

    /// Materialising names the copy after the destination and brings the
    /// destination's dedup marker with it — the whole reason this indirection
    /// exists.
    #[test]
    fn materializing_carries_the_destinations_latest_marker() {
        let area = StageArea::default();
        let (_, stage) = area.put("a\n1\n", SourceFormat::Csv).expect("stage");

        let dest = std::env::temp_dir().join(format!(
            "ledgeline-stage-test-{}",
            StageId::mint().expect("csprng").as_str()
        ));
        std::fs::create_dir(&dest).expect("dest dir");
        std::fs::write(dest.join(".latest.bank.csv"), "2026-02-05\n").expect("marker");

        let staged = stage
            .materialize(RUN_WITH_LATEST, "bank.csv", Some(&dest), 1)
            .expect("materialize");
        assert_eq!(
            staged.file_name().and_then(|n| n.to_str()),
            Some("bank.csv")
        );
        assert_eq!(
            std::fs::read_to_string(staged.parent().expect("run dir").join(".latest.bank.csv"))
                .expect("marker copied"),
            "2026-02-05\n"
        );
        assert_eq!(
            latest_marker(&dest, "bank.csv").as_deref(),
            Some("2026-02-05")
        );

        // The bare slot deliberately has no marker: the count difference between
        // the two runs is how we know what dedup would drop.
        let bare = stage
            .materialize(RUN_BARE, "bank.csv", None, 1)
            .expect("bare");
        assert!(
            !bare
                .parent()
                .expect("run dir")
                .join(".latest.bank.csv")
                .exists()
        );

        std::fs::remove_dir_all(&dest).ok();
    }

    /// A name with a separator in it must never reach the filesystem, whatever
    /// the caller believed it had already checked.
    #[test]
    fn materializing_refuses_anything_but_a_bare_name() {
        let area = StageArea::default();
        let (_, stage) = area.put("a\n1\n", SourceFormat::Csv).expect("stage");
        for name in ["../escape.csv", "sub/bank.csv", "", ".", "..", "a\u{0}.csv"] {
            assert!(
                stage.materialize(RUN_WITH_LATEST, name, None, 1).is_err(),
                "{name:?} must be refused"
            );
        }
    }

    /// Everything hledger is pointed at carries the padding its rules file's
    /// `skip` needs — and the canonical CSV never does.
    ///
    /// The bug this closes is silent: a genuine `skip 3` against an unpadded
    /// converted CSV imports the file's *tail* — zero transactions on a short
    /// statement — and **exits 0**, so a copy that forgot the alignment looks
    /// exactly like a statement with less in it than the user thought.
    /// `convert::align_to_skip` owns the padding's shape; what is asserted here
    /// is that both routes out of a stage go through it.
    #[test]
    fn every_copy_hledger_reads_is_aligned_and_the_canonical_one_is_not() {
        let area = StageArea::default();
        let csv = "Date,Description,Amount\n2026-01-05,A,-1.00\n";
        let (_, stage) = area.put(csv, SourceFormat::Csv).expect("stage");

        assert_eq!(
            std::fs::read_to_string(stage.data()).expect("canonical"),
            csv,
            "the artifact the user saves keeps the header on line 1"
        );

        let staged = stage
            .materialize(RUN_WITH_LATEST, "bank.csv", None, 3)
            .expect("materialize");
        assert_eq!(
            std::fs::read_to_string(&staged).expect("materialized"),
            format!(",,\n,,\n{csv}")
        );

        let scored = stage.aligned(3).expect("aligned copy");
        assert_eq!(
            std::fs::read_to_string(&scored).expect("aligned"),
            format!(",,\n,,\n{csv}")
        );
        assert_ne!(
            scored,
            stage.data(),
            "a padded copy is not the canonical file"
        );

        // The ordinary rules file needs nothing, so it is handed the canonical
        // file itself rather than a byte-identical duplicate of it.
        assert_eq!(stage.aligned(1).expect("skip 1"), stage.data());
        assert_eq!(stage.aligned(0).expect("skip 0"), stage.data());
    }

    /// The cap bounds what one session can hold, and evicts the oldest.
    #[test]
    fn the_oldest_stage_is_evicted_past_the_cap() {
        let area = StageArea::default();
        let ids: Vec<StageId> = (0..=MAX_LIVE_STAGES)
            .map(|n| {
                area.put(&format!("a\n{n}\n"), SourceFormat::Csv)
                    .expect("stage")
                    .0
            })
            .collect();
        assert!(area.get(&ids[0]).is_none(), "the oldest must be evicted");
        assert!(
            area.get(&ids[MAX_LIVE_STAGES]).is_some(),
            "the newest survives"
        );
    }

    /// Dropping the area takes the whole session root with it.
    #[test]
    fn dropping_the_area_removes_every_stage() {
        let area = StageArea::default();
        let (_, stage) = area.put("a\n1\n", SourceFormat::Csv).expect("stage");
        let root = stage
            .data()
            .parent()
            .and_then(Path::parent)
            .expect("session root")
            .to_path_buf();
        assert!(root.exists());
        drop(area);
        assert!(!root.exists(), "the session root must not outlive the area");
    }
}
