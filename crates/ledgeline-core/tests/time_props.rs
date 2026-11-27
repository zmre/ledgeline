//! Property tests for the time writer (plan 46 Phase 1):
//!
//! - parse → write → parse: every edit on a generated timedot file reads
//!   back as exactly that edit, every line it didn't name is byte-for-byte
//!   the same, and `tidy` changes no meaning and is idempotent;
//! - minutes are conserved: a stopped session's minutes per day add up to
//!   its rounded total across any number of midnights, and a run of switches
//!   through the tracker books exactly the sum of each session's rounding.

use ledgeline_core::time::TagNames;
use ledgeline_core::time::TimeQty;
use ledgeline_core::time::book::{self, TimeConfig, WeekdayStyle, Words, default_seed, setup_plan};
use ledgeline_core::time::files::{TimeFiles, Tracker};
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::time::timedot::{self, LineKind};
use ledgeline_core::time::write::{self, DayStyle};
use ledgeline_core::vault::{MemVault, VaultPath};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::sync::Arc;

const ACCOUNTS: [&str; 4] = [
    "clients:acme",
    "clients:acme:tax",
    "internal:admin",
    "a b:c",
];

fn amount() -> impl Strategy<Value = String> {
    prop_oneof![
        (1u32..300).prop_map(|m| format!("{m}m")),
        (1u32..9).prop_map(|h| format!("{h}h")),
        (1usize..9).prop_map(|n| ".".repeat(n)),
        (1u32..40).prop_map(|t| format!("{}.{}", t / 10, t % 10)),
        Just("ab".to_string()),
    ]
}

fn comment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("  ; a note".to_string()),
        Just("  ; invoice: 101".to_string()),
        Just(";bill: x, nobill:".to_string()),
    ]
}

fn line() -> impl Strategy<Value = String> {
    (
        prop_oneof![Just("    "), Just("  "), Just(""), Just("** ")],
        0..ACCOUNTS.len(),
        prop_oneof![Just("  "), Just("   "), Just("\t\t")],
        amount(),
        comment(),
    )
        .prop_map(|(indent, account, gap, amount, comment)| {
            format!("{indent}{}{gap}{amount}{comment}", ACCOUNTS[account])
        })
}

fn day() -> impl Strategy<Value = (u32, Vec<String>)> {
    (1u32..6, prop::collection::vec(line(), 0..5))
}

fn file() -> impl Strategy<Value = String> {
    prop::collection::vec(day(), 0..5).prop_map(|days| {
        let mut out = String::from("# generated\n\n");
        for (d, lines) in days {
            out.push_str(&format!("2026-11-0{d} Mon\n"));
            for line in lines {
                out.push_str(&line);
                out.push('\n');
            }
            out.push('\n');
        }
        out
    })
}

/// Total time per `(date, account)`.
fn totals(text: &str) -> BTreeMap<(String, String), TimeQty> {
    let file = timedot::parse(text);
    let mut out: BTreeMap<(String, String), TimeQty> = BTreeMap::new();
    for posting in file.postings() {
        let slot = out
            .entry((posting.day.date.clone(), posting.entry.account.clone()))
            .or_default();
        *slot = slot.plus(posting.quantity);
    }
    out
}

fn style<'a>(tags: &'a TagNames, words: &'a Words) -> DayStyle<'a> {
    DayStyle {
        tags,
        words,
        weekday: WeekdayStyle::Short,
        person: None,
    }
}

/// The lines of `before` an edit didn't touch, all still in `after`, in
/// order.
fn keeps_untouched(before: &str, after: &str, touched: usize) {
    let after_lines: Vec<&str> = after.lines().collect();
    let kept = before
        .lines()
        .filter(|line| after_lines.contains(line))
        .count();
    assert!(
        kept + touched >= before.lines().count(),
        "more than {touched} lines changed:\n{before}\n---\n{after}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn add_minutes_changes_one_total_by_what_it_applied(
        text in file(),
        day in 1u32..7,
        account in 0..ACCOUNTS.len(),
        minutes in -200i64..200,
    ) {
        let (tags, words) = (TagNames::default(), Words::default());
        let date = format!("2026-11-0{day}");
        let before = totals(&text);
        let added = write::add_minutes(&text, &date, ACCOUNTS[account], minutes, &style(&tags, &words))
            .expect("add_minutes");
        let mut want = before.clone();
        if added.applied != 0 {
            let key = (date.clone(), ACCOUNTS[account].to_string());
            let slot = want.entry(key).or_default();
            *slot = slot.plus(TimeQty::from_minutes(added.applied));
        }
        let after = totals(&added.text);
        // A line taken to zero and removed drops its (zero) key.
        want.retain(|key, qty| !qty.is_zero() || after.contains_key(key));
        prop_assert_eq!(after, want);
        prop_assert!(minutes.signum() * added.applied >= 0 && added.applied.abs() <= minutes.abs());
        keeps_untouched(&text, &added.text, 1);
    }

    #[test]
    fn tags_and_notes_round_trip_on_any_entry(text in file(), pick in 0usize..20, value in "[a-z0-9-]{1,8}") {
        let file = timedot::parse(&text);
        let entries: Vec<usize> = file.lines.iter().enumerate()
            .filter(|(_, l)| matches!(l.kind, LineKind::Entry(_)))
            .map(|(at, _)| at)
            .collect();
        prop_assume!(!entries.is_empty());
        let at = entries[pick % entries.len()];
        let tagged = write::set_entry_tag(&text, at, "invoice", Some(&value)).expect("tag");
        let reread = timedot::parse(&tagged);
        let entry = reread.entry(at).expect("still an entry");
        prop_assert!(entry.tags.contains(&("invoice".to_string(), value.clone())));
        prop_assert_eq!(totals(&tagged), totals(&text));
        keeps_untouched(&text, &tagged, 1);
        let untagged = write::set_entry_tag(&tagged, at, "invoice", None).expect("untag");
        prop_assert!(timedot::parse(&untagged).entry(at).expect("entry").tags.iter().all(|(n, _)| n != "invoice"));
        let noted = write::set_note(&text, at, "checked the figures").expect("note");
        let renoted = timedot::parse(&noted);
        prop_assert_eq!(renoted.entry(at).expect("entry").note.as_str(), "checked the figures");
        prop_assert_eq!(totals(&noted), totals(&text));
    }

    #[test]
    fn tidy_keeps_meaning_and_is_idempotent(text in file()) {
        let tidied = write::tidy(&text).expect("tidy");
        prop_assert_eq!(totals(&tidied), totals(&text));
        prop_assert_eq!(write::tidy(&tidied).expect("again"), tidied.clone());
        prop_assert!(!tidied.lines().any(|line| line.contains('\t') && timedot::parse(line).lines.iter().all(|l| matches!(l.kind, LineKind::Entry(_)))));
    }

    #[test]
    fn insert_then_reread(text in file(), day in 1u32..9, account in 0..ACCOUNTS.len(), minutes in 1i64..600) {
        let (tags, words) = (TagNames::default(), Words::default());
        let date = format!("2026-11-0{day}");
        let out = write::insert_entry(&text, &date, ACCOUNTS[account], TimeQty::from_minutes(minutes), "", &[], &style(&tags, &words))
            .expect("insert");
        let key = (date, ACCOUNTS[account].to_string());
        let before = totals(&text).get(&key).copied().unwrap_or_default();
        prop_assert_eq!(totals(&out)[&key], before.plus(TimeQty::from_minutes(minutes)));
        keeps_untouched(&text, &out, 0);
        // Days stay in date order when the file was.
        let dates: Vec<String> = timedot::parse(&out).lines.iter().filter_map(|l| match &l.kind {
            LineKind::Date(d) => Some(d.date.clone()),
            _ => None,
        }).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        let was_sorted = {
            let before: Vec<String> = timedot::parse(&text).lines.iter().filter_map(|l| match &l.kind {
                LineKind::Date(d) => Some(d.date.clone()),
                _ => None,
            }).collect();
            let mut s = before.clone();
            s.sort();
            s == before
        };
        prop_assert!(!was_sorted || dates == sorted);
    }

    #[test]
    fn a_stop_conserves_minutes_across_midnights(start in 0u32..86_400, length in 0i64..(3 * 86_400)) {
        let since = Moment::new("2026-11-29", start);
        let end_epoch = since.epoch_seconds() + length;
        let days = end_epoch.div_euclid(86_400);
        let now = Moment::new(
            ledgeline_core::reports::periods::iso_from_days(days),
            u32::try_from(end_epoch.rem_euclid(86_400)).expect("seconds"),
        );
        let plan = write::stop_plan(&since, &now);
        let total: i64 = plan.iter().map(|(_, m)| m).sum();
        prop_assert_eq!(total, TimeQty::from_seconds(length).round_minutes());
        prop_assert!(plan.iter().all(|(_, m)| *m > 0));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Switching through the tracker: each press stops the running slot and
    /// starts the next; the book ends with exactly Σ round(session) minutes.
    #[test]
    fn switching_conserves_minutes(gaps in prop::collection::vec(0i64..5_000, 1..8), slots in prop::collection::vec(0usize..3, 8)) {
        let vault = Arc::new(MemVault::with_root("/books"));
        vault.insert_file("books.journal", "");
        let main = VaultPath::new("books.journal").expect("main");
        let config = TimeConfig::default();
        let plan = setup_plan(&*vault, &main, &config, Some("pat"), "2026-11-30", &default_seed(&[], &config)).expect("plan");
        book::create(&*vault, &plan).expect("create");
        let paths = book::paths(&main, &config).expect("paths");
        let tracker = Tracker { paths: &paths, config: &config, person: Some("pat") };
        let accounts = ["clients:acme", "internal:admin", "clients:globex"];
        let moment = |epoch: i64| Moment::new(
            ledgeline_core::reports::periods::iso_from_days(epoch.div_euclid(86_400)),
            u32::try_from(epoch.rem_euclid(86_400)).expect("seconds"),
        );
        let mut clock = Moment::new("2026-11-30", 22 * 3600).epoch_seconds();
        let mut expected = 0i64;
        let mut files = TimeFiles::new(vault.clone());
        files.start(&tracker, accounts[slots[0]], &moment(clock)).expect("start");
        files.save().expect("save");
        for (n, gap) in gaps.iter().enumerate() {
            let loaded = book::load(&*vault, &main, &config, None).expect("load").expect("book");
            let running = loaded.running.first().expect("running").clone();
            clock += gap;
            expected += TimeQty::from_seconds(*gap).round_minutes();
            let mut files = TimeFiles::with_book(vault.clone(), &loaded);
            files.stop(&tracker, &running, &moment(clock)).expect("stop");
            files.start(&tracker, accounts[slots[(n + 1) % slots.len()]], &moment(clock)).expect("start");
            files.save().expect("save");
        }
        let loaded = book::load(&*vault, &main, &config, None).expect("load").expect("book");
        let booked: i64 = loaded.entries.iter().map(|e| e.quantity.round_minutes()).sum();
        prop_assert_eq!(booked, expected);
        prop_assert_eq!(loaded.running.len(), 1);
        prop_assert!(loaded.findings.iter().all(|f| f.finding.severity < ledgeline_core::time::Severity::Warning));
    }
}
