//! The projection run's real side, cached per snapshot (plan 29).
//!
//! Every edit in the Projections what-if table re-runs the projection, and
//! everything the run reads from the REAL journal is the same from one edit to
//! the next. Core computes it as plain values ([`JournalFacts`] per journal,
//! [`RealSide`] per [`RealKey`]); this holds them.
//!
//! # Keyed by the snapshot, so invalidation is structural
//!
//! The cache is a field of [`crate::Snapshot`]. An edit, a file-watcher
//! reload, a `refresh()` or File → Open publishes a NEW snapshot with an empty
//! cache, and the old one is freed with the last request still holding the
//! old snapshot. Within a snapshot the journal cannot change, so the only key
//! is the [`RealKey`]: the cutoff (where `today` and the latest real
//! transaction enter), the interval, the lookback and the valuation commodity.
//! A new day inside the same period keeps the key; one in the next period
//! does not.
//!
//! # Bounded
//!
//! At most [`SIDES`] real sides per snapshot, least recently used evicted.
//! One is the steady state (edits do not move the key); the rest cover
//! flipping the interval, the lookback or `valueIn` and back.
//!
//! # Concurrent
//!
//! Handlers run on `compute` threads at once. The facts are a `OnceLock`
//! (built once, racers share it). The sides are a short-held `Mutex` over a
//! tiny vector: look up, release, build outside the lock on a miss, re-check
//! and insert — the trade `ViewCache::diagnostics` makes. Two simultaneous
//! misses on one key both build and the first insert wins; both values are
//! identical, so either is correct. Errors (a decimal overflow) are never
//! cached.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use ledgeline_core::Journal;
use ledgeline_core::projections::{JournalFacts, RealKey, RealSide};
use ledgeline_core::reports::ReportError;

/// How many real sides one snapshot keeps.
pub(crate) const SIDES: usize = 4;

/// One snapshot's projection real sides.
#[derive(Default)]
pub(crate) struct ProjectionCache {
    facts: OnceLock<Arc<JournalFacts>>,
    /// Most recently used last.
    sides: Mutex<Vec<Arc<RealSide>>>,
}

impl ProjectionCache {
    /// The journal's facts, built on first use. `journal` must be the
    /// snapshot's own.
    ///
    /// # Errors
    /// [`ReportError`] on decimal overflow (not cached: the next call retries).
    pub(crate) fn facts(&self, journal: &Journal) -> Result<Arc<JournalFacts>, ReportError> {
        if let Some(facts) = self.facts.get() {
            return Ok(Arc::clone(facts));
        }
        let built = Arc::new(JournalFacts::new(journal)?);
        // A racer may have set it meanwhile; theirs is identical.
        Ok(Arc::clone(self.facts.get_or_init(|| built)))
    }

    /// The real side for `key`, built on a miss.
    ///
    /// # Errors
    /// [`ReportError`] on decimal overflow or bucket math (not cached).
    pub(crate) fn side(
        &self,
        journal: &Journal,
        facts: &JournalFacts,
        key: &RealKey,
    ) -> Result<Arc<RealSide>, ReportError> {
        if let Some(hit) = self.touch(key) {
            return Ok(hit);
        }
        let built = Arc::new(RealSide::build(journal, facts, key)?);
        let mut sides = self.lock();
        if let Some(index) = sides.iter().position(|side| side.key() == key) {
            // Built twice by two racing misses: keep the one already shared.
            let side = sides.remove(index);
            sides.push(Arc::clone(&side));
            return Ok(side);
        }
        if sides.len() == SIDES {
            sides.remove(0);
        }
        sides.push(Arc::clone(&built));
        Ok(built)
    }

    /// A hit, moved to most recently used.
    fn touch(&self, key: &RealKey) -> Option<Arc<RealSide>> {
        let mut sides = self.lock();
        let index = sides.iter().position(|side| side.key() == key)?;
        let side = sides.remove(index);
        sides.push(Arc::clone(&side));
        Some(side)
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<RealSide>>> {
        // A panic while holding this lock cannot leave a half-built entry:
        // every mutation is one `remove` or `push` of a finished value.
        self.sides.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::model::Commodity;
    use ledgeline_core::parse_journal;
    use ledgeline_core::reports::Interval;

    const JOURNAL: &str = "\
account assets:checking  ; type: C
account equity:opening   ; type: E

2026-01-01 opening
    assets:checking   $1,000.00
    equity:opening

2026-09-30 last
    assets:checking   $1.00
    equity:opening
";

    fn journal() -> Journal {
        parse_journal(JOURNAL, "cache.journal").expect("parses")
    }

    fn side(
        cache: &ProjectionCache,
        journal: &Journal,
        today: &str,
        interval: Interval,
        lookback: usize,
        value_in: Option<&str>,
    ) -> Arc<RealSide> {
        let facts = cache.facts(journal).expect("facts");
        let key = facts
            .key(
                today,
                interval,
                lookback,
                value_in.map(|symbol| Commodity(symbol.to_string())),
            )
            .expect("key");
        cache.side(journal, &facts, &key).expect("side")
    }

    #[test]
    fn the_facts_are_built_once() {
        let cache = ProjectionCache::default();
        let journal = journal();
        let first = cache.facts(&journal).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.facts(&journal).unwrap()));
    }

    #[test]
    fn a_repeated_run_hits() {
        let cache = ProjectionCache::default();
        let journal = journal();
        let first = side(&cache, &journal, "2026-10-20", Interval::Monthly, 2, None);
        let again = side(&cache, &journal, "2026-10-20", Interval::Monthly, 2, None);
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[test]
    fn a_new_day_in_the_same_period_hits_and_the_next_period_misses() {
        let cache = ProjectionCache::default();
        let journal = journal();
        let september = side(&cache, &journal, "2026-09-15", Interval::Monthly, 2, None);
        let later = side(&cache, &journal, "2026-09-29", Interval::Monthly, 2, None);
        assert!(Arc::ptr_eq(&september, &later), "same cutoff, same side");
        assert_eq!(september.key().cutoff, "2026-09-01");
        let october = side(&cache, &journal, "2026-10-01", Interval::Monthly, 2, None);
        assert!(!Arc::ptr_eq(&september, &october), "the period rolled over");
        assert_eq!(october.key().cutoff, "2026-10-01");
        // The journal ends 2026-09-30, so October stays the first incomplete
        // period however far into November today is: same cutoff, same side.
        let november = side(&cache, &journal, "2026-11-05", Interval::Monthly, 2, None);
        assert!(Arc::ptr_eq(&october, &november));
    }

    #[test]
    fn interval_lookback_and_value_in_each_miss() {
        let cache = ProjectionCache::default();
        let journal = journal();
        let base = side(&cache, &journal, "2026-10-20", Interval::Monthly, 2, None);
        for other in [
            side(&cache, &journal, "2026-10-20", Interval::Quarterly, 2, None),
            side(&cache, &journal, "2026-10-20", Interval::Monthly, 3, None),
            side(
                &cache,
                &journal,
                "2026-10-20",
                Interval::Monthly,
                2,
                Some("$"),
            ),
        ] {
            assert!(!Arc::ptr_eq(&base, &other));
            assert_ne!(base.key(), other.key());
        }
    }

    #[test]
    fn the_least_recently_used_side_is_evicted() {
        let cache = ProjectionCache::default();
        let journal = journal();
        let keep = side(&cache, &journal, "2026-10-20", Interval::Monthly, 0, None);
        let drop = side(&cache, &journal, "2026-10-20", Interval::Monthly, 1, None);
        for lookback in 2..=SIDES {
            // Touch `keep` so `drop` is the oldest when the cache fills.
            side(&cache, &journal, "2026-10-20", Interval::Monthly, 0, None);
            side(
                &cache,
                &journal,
                "2026-10-20",
                Interval::Monthly,
                lookback,
                None,
            );
        }
        assert_eq!(cache.lock().len(), SIDES);
        let kept = side(&cache, &journal, "2026-10-20", Interval::Monthly, 0, None);
        assert!(Arc::ptr_eq(&keep, &kept));
        let rebuilt = side(&cache, &journal, "2026-10-20", Interval::Monthly, 1, None);
        assert!(!Arc::ptr_eq(&drop, &rebuilt));
    }
}
