//! Time tracking in the Invoicer (plan 46 Phase 2), over a private copy of
//! `invoicer/fixtures/time` (clock: 2026-11-26): the tracker, Entries and
//! their locks, Accounts, summaries, billing time as a draft (rounding,
//! items, the timesheet, tags first and rolled back), releasing it, and
//! contractor time as an accounts-payable bill.

use invoicer_core::session::Session;
use invoicer_core::time::bill::{self, BillRequest, GroupChoice, Grouping, Release};
use invoicer_core::time::entries::{self, BulkOp, EntryEdit, EntryQuery, NewEntry};
use invoicer_core::time::pay::{self, PayRequest};
use invoicer_core::time::report::{self, Group, SummaryQuery};
use invoicer_core::time::track::{self, SlotKind};
use invoicer_core::time::{Locks, TimeBooks, TimePrefs, accounts, parse_moment};
use ledgeline_core::time::TagRole;
use ledgeline_core::time::query::Filter;
use ledgeline_core::time::timeclock::Moment;
use ledgeline_core::vault::{LocalVault, Vault, VaultPath};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const TODAY: &str = "2026-11-26";

struct Books {
    _dir: tempfile::TempDir,
    root: PathBuf,
    vault: Arc<dyn Vault>,
    main: VaultPath,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

fn books() -> Books {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("books");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/time"),
        &root,
    );
    let (vault, main) = LocalVault::for_journal(&root.join("books.journal")).expect("vault");
    Books {
        _dir: dir,
        root,
        vault: Arc::new(vault),
        main,
    }
}

impl Books {
    fn time(&self) -> TimeBooks {
        TimeBooks::open(Arc::clone(&self.vault), self.main.clone()).expect("time")
    }

    fn session(&self) -> Session {
        Session::open(Arc::clone(&self.vault), self.main.clone()).expect("session")
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).expect("reads")
    }
}

fn pat() -> TimePrefs {
    TimePrefs {
        person: Some("pat".to_string()),
        ..TimePrefs::default()
    }
}

fn at(text: &str) -> Moment {
    parse_moment(text).expect("moment")
}

#[test]
fn slots_pin_fill_from_recents_and_show_today() {
    let books = books();
    let time = books.time();
    let book = time.require().expect("book");
    let prefs = TimePrefs {
        pinned: vec!["clients:globex:audit".into(), String::new()],
        other: Some("clients:initech:discovery".into()),
        ..pat()
    };
    let state = track::state(&time, Some(&book), &prefs, &at("2026-11-26T12:00:00"), None);
    assert!(state.configured);
    assert_eq!(state.slots.len(), 10);
    let slot = |key: u8| state.slots.iter().find(|s| s.key == key).expect("slot");
    assert_eq!(slot(1).kind, SlotKind::Pinned);
    assert_eq!(slot(1).account.as_deref(), Some("clients:globex:audit"));
    assert_eq!(slot(1).label.as_deref(), Some("Globex · audit"));
    // An empty pin is filled from recents: today's latest line first.
    assert_eq!(slot(2).kind, SlotKind::Recent);
    assert_eq!(slot(2).account.as_deref(), Some("internal:admin"));
    assert_eq!(slot(3).account.as_deref(), Some("clients:acme:advisory"));
    assert_eq!(slot(3).today.text, "1h");
    assert_eq!(
        slot(3).last_note.as_deref(),
        Some("call with Fred about the K-1")
    );
    assert_eq!(slot(0).kind, SlotKind::Other);
    assert_eq!(
        slot(0).account.as_deref(),
        Some("clients:initech:discovery")
    );
    // Archived accounts never fill a slot; pinned ones aren't repeated.
    assert!(
        state
            .slots
            .iter()
            .all(|s| s.account.as_deref() != Some("internal:old"))
    );
    assert_eq!(
        state
            .slots
            .iter()
            .filter(|s| s.account.as_deref() == Some("clients:globex:audit"))
            .count(),
        1
    );
    assert_eq!(state.today_total.text, "1h 15m");
    assert_eq!(state.week.len(), 7);
    assert_eq!(state.week[0].date, "2026-11-23");
    assert!(state.running.is_none());
    assert_eq!(
        state.last.as_ref().map(|l| l.account.as_str()),
        Some("internal:admin")
    );
    let hinted = track::state(
        &time,
        Some(&book),
        &prefs,
        &at("2026-11-26T12:00:00"),
        Some("clients:acme:advisory"),
    );
    assert_eq!(hinted.last.map(|l| l.slot), Some(Some(3)));
}

#[test]
fn toggling_switches_in_one_save_and_adjusts_the_start() {
    let books = books();
    let time = books.time();
    let person = Some("pat");
    let started = track::toggle(
        &time,
        None,
        person,
        "clients:acme:tax-return",
        &at("2026-11-26T14:05:12"),
    )
    .expect("start");
    assert_eq!(
        started.done.started.as_deref(),
        Some("clients:acme:tax-return")
    );
    let file = "time/pat/2026-11.timedot";
    assert!(
        books
            .read(file)
            .contains("; ● running clients:acme:tax-return  since 14:05:12"),
        "{}",
        books.read(file)
    );
    // `+5` on the running slot moves its start back five minutes.
    let moved = track::adjust(
        &time,
        None,
        person,
        "clients:acme:tax-return",
        5,
        &at("2026-11-26T14:10:00"),
    )
    .expect("adjust");
    assert_eq!(moved.done.since.as_deref(), Some("2026-11-26T14:00:12"));
    // Switching stops (banking rounded minutes) and starts, in one save.
    let switched = track::toggle(
        &time,
        None,
        person,
        "internal:admin",
        &at("2026-11-26T14:30:40"),
    )
    .expect("switch");
    let stopped = switched.done.stopped.expect("stopped");
    assert_eq!(stopped.days, vec![("2026-11-26".to_string(), 30)]);
    assert_eq!(switched.done.started.as_deref(), Some("internal:admin"));
    assert_eq!(switched.saved.len(), 1);
    let text = books.read(file);
    assert!(
        text.contains("clients:acme:tax-return          30m"),
        "{text}"
    );
    assert!(
        text.contains("running internal:admin  since 14:30:40"),
        "{text}"
    );
    // The state shows it running, with the guard quiet.
    let book = time.require().expect("book");
    let state = track::state(&time, Some(&book), &pat(), &at("2026-11-26T15:00:40"), None);
    let running = state.running.expect("running");
    assert_eq!(running.account, "internal:admin");
    assert_eq!(running.elapsed.text, "30m");
    assert!(state.guard.is_none());
    // Five hours later the long-running guard asks.
    let later = track::state(&time, Some(&book), &pat(), &at("2026-11-26T19:31:00"), None);
    assert!(later.guard.is_some_and(|g| !g.previous_day));
    let tomorrow = track::state(&time, Some(&book), &pat(), &at("2026-11-27T08:00:00"), None);
    assert!(tomorrow.guard.is_some_and(|g| g.previous_day));
    // Toggling the running slot stops it; a press under 30 s banks nothing.
    let short = track::toggle(
        &time,
        None,
        person,
        "internal:admin",
        &at("2026-11-26T14:31:00"),
    )
    .expect("stop");
    assert_eq!(short.done.stopped.expect("stopped").days, Vec::new());
    assert!(!books.read(file).contains("running"));
    // `−` with nothing running takes from today's line, never below zero.
    let less = track::adjust(
        &time,
        None,
        person,
        "internal:admin",
        -60,
        &at("2026-11-26T16:00:00"),
    )
    .expect("less");
    assert_eq!(less.done.applied, Some(-15));
    // Stop with nothing running is a conflict.
    let none = track::stop(&time, None, person, &at("2026-11-26T16:00:00"), false)
        .expect_err("nothing runs");
    assert_eq!(none.code(), "conflict");
}

#[test]
fn snap_discard_and_notes() {
    let books = books();
    let time = books.time();
    let person = Some("pat");
    track::start(
        &time,
        None,
        person,
        "clients:acme:advisory",
        &at("2026-11-26T10:20:00"),
    )
    .expect("start");
    // A timedot file records no end times: snapping needs one.
    let unknown = track::snap(&time, None, person, None).expect_err("no end known");
    assert_eq!(unknown.code(), "conflict");
    let snapped = track::snap(&time, None, person, Some(&at("2026-11-26T10:00:00"))).expect("snap");
    assert_eq!(snapped.done.since.as_deref(), Some("2026-11-26T10:00:00"));
    let refused = track::note(
        &time,
        None,
        person,
        "clients:acme:advisory",
        TODAY,
        "re: K-1",
    )
    .expect_err("a tag");
    assert_eq!(refused.code(), "note-tag");
    assert!(refused.to_string().contains("re:"), "{refused}");
    // A slot with nothing banked today gets a 0m line to hold its note.
    track::note(
        &time,
        None,
        person,
        "clients:acme:travel",
        TODAY,
        "drive back",
    )
    .expect("note");
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("clients:acme:travel               0m  ; drive back"),
        "{}",
        books.read("time/pat/2026-11.timedot")
    );
    let discarded =
        track::stop(&time, None, person, &at("2026-11-26T11:00:00"), true).expect("discard");
    assert!(discarded.done.stopped.expect("stopped").discarded);
    let text = books.read("time/pat/2026-11.timedot");
    assert!(!text.contains("running"), "{text}");
    assert!(
        text.contains("clients:acme:advisory             1h"),
        "{text}"
    );
}

#[test]
fn notes_on_existing_lines_and_new_month_files_are_included() {
    let books = books();
    let time = books.time();
    track::note(
        &time,
        None,
        Some("pat"),
        "clients:acme:advisory",
        TODAY,
        "call about the K-1",
    )
    .expect("note");
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("clients:acme:advisory             1h  ; call about the K-1")
    );
    // A person whose folder doesn't exist yet: the glob covers the new file.
    let done = track::start(
        &time,
        None,
        Some("kim"),
        "internal:admin",
        &at("2026-12-01T09:00:00"),
    )
    .expect("start");
    assert!(done.included.is_empty(), "{:?}", done.included);
    let book = time.require().expect("book");
    assert!(
        book.running
            .iter()
            .any(|r| r.person.as_deref() == Some("kim"))
    );
}

/// Fact 10: the first timeclock file Ledgeline writes brings the roll-up
/// its include and the `commodity h` that `hledger -s` needs.
#[test]
fn a_first_timeclock_file_declares_commodity_h() {
    let books = books();
    let rollup = books.read("time.journal").replace(
        &format!("{}\n", ledgeline_core::time::book::TIMECLOCK_COMMODITY),
        "",
    );
    std::fs::write(books.root.join("time.journal"), &rollup).expect("write");
    assert!(!rollup.contains("commodity h"));
    let settings = books.read("ledgeline.toml");
    std::fs::write(
        books.root.join("ledgeline.toml"),
        format!("{settings}\n[time]\ntrack-file = \"clock/{{person}}.timeclock\"\nrecord = \"timeclock\"\n"),
    )
    .expect("settings");
    let time = books.time();
    let done = track::start(
        &time,
        None,
        Some("pat"),
        "internal:admin",
        &at("2026-11-26T09:00:00"),
    )
    .expect("start");
    assert_eq!(
        done.included,
        vec![
            "include clock/*.timeclock".to_string(),
            ledgeline_core::time::book::TIMECLOCK_COMMODITY.to_string()
        ]
    );
    let rollup = books.read("time.journal");
    assert!(
        rollup.contains(&format!(
            "{}\n{}\n",
            ledgeline_core::time::book::TIMEDOT_COMMODITY,
            ledgeline_core::time::book::TIMECLOCK_COMMODITY
        )),
        "{rollup}"
    );
    // The next file needs neither.
    let done = track::start(
        &time,
        None,
        Some("kim"),
        "internal:admin",
        &at("2026-11-26T09:00:00"),
    )
    .expect("start");
    assert!(done.included.is_empty(), "{:?}", done.included);
}

#[test]
fn entries_filter_lock_and_edit() {
    let books = books();
    let time = books.time();
    let session = books.session();
    let locks = Locks::of(session.index(), session.booked_numbers());
    let book = time.require().expect("book");
    let acme = entries::list(
        &book,
        &EntryQuery {
            customer: Some("Acme Corp".into()),
            billed: Some("invoiced".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    assert_eq!(acme.entries.len(), 2);
    assert_eq!(acme.total.text, "5h");
    assert!(
        acme.entries.iter().all(|e| e.locked.is_some()),
        "101 is sent"
    );
    let id = acme.entries[0].id.clone();
    let refused = entries::update(
        &time,
        None,
        &locks,
        &EntryEdit {
            id: id.clone(),
            duration: Some("4h".into()),
            ..EntryEdit::default()
        },
    )
    .expect_err("locked");
    assert_eq!(refused.code(), "locked");
    assert_eq!(
        entries::delete(&time, None, &locks, &id)
            .expect_err("locked")
            .code(),
        "locked"
    );
    // Unbilled November, pat's, in the period.
    let november = entries::list(
        &book,
        &EntryQuery {
            from: Some("2026-11-01".into()),
            to: Some("2026-11-30".into()),
            person: Some("pat".into()),
            billable: Some(true),
            billed: Some("unbilled".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    assert_eq!(november.entries.len(), 6, "{:#?}", november.entries);
    // Edit: a new duration and account, in place; a note with a tag refused.
    let line = november
        .entries
        .iter()
        .find(|e| e.note == "gathered documents")
        .expect("line")
        .id
        .clone();
    entries::update(
        &time,
        None,
        &locks,
        &EntryEdit {
            id: line.clone(),
            duration: Some("1h45".into()),
            ..EntryEdit::default()
        },
    )
    .expect("edit");
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("clients:acme:tax-return         105m  ; gathered documents")
    );
    let tag = entries::update(
        &time,
        None,
        &locks,
        &EntryEdit {
            id: line.clone(),
            note: Some("todo: scan".into()),
            ..EntryEdit::default()
        },
    )
    .expect_err("a tag");
    assert_eq!(tag.code(), "note-tag");
    // Moving the line to another day.
    entries::update(
        &time,
        None,
        &locks,
        &EntryEdit {
            id: line,
            date: Some("2026-11-04".into()),
            ..EntryEdit::default()
        },
    )
    .expect("move");
    let text = books.read("time/pat/2026-11.timedot");
    assert!(text.contains("2026-11-04 Wed  ; person: pat\n    clients:acme:tax-return         105m  ; gathered documents"), "{text}");
    // Add.
    let added = entries::add(
        &time,
        None,
        Some("pat"),
        &NewEntry {
            date: "2026-11-27".into(),
            account: "internal:admin".into(),
            duration: "0:20".into(),
            note: "expenses".into(),
            person: None,
        },
    )
    .expect("add");
    assert_eq!(added.done.as_str(), "time/pat/2026-11.timedot");
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("internal:admin                   20m  ; expenses")
    );
    // Bulk: nobill, then person, then move; a timeclock session edits by time.
    let book = time.require().expect("book");
    let sam = entries::list(
        &book,
        &EntryQuery {
            person: Some("sam".into()),
            account: Some("clients:globex".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    let ids: Vec<String> = sam.entries.iter().map(|e| e.id.clone()).collect();
    let done = entries::bulk(
        &time,
        None,
        &locks,
        &ids,
        &BulkOp::Nobill {
            reason: "trial".into(),
        },
    )
    .expect("nobill");
    assert_eq!(done.done, 2);
    assert!(
        books
            .read("time/sam/2026-11.timedot")
            .contains("fieldwork, nobill: trial")
    );
    entries::bulk(&time, None, &locks, &ids, &BulkOp::Billable).expect("billable");
    assert!(!books.read("time/sam/2026-11.timedot").contains("nobill"));
    let clock = "time/imported/2026-09.timeclock@s0".to_string();
    entries::update(
        &time,
        None,
        &locks,
        &EntryEdit {
            id: clock.clone(),
            end: Some("2026-09-29T15:00:00".into()),
            ..EntryEdit::default()
        },
    )
    .expect("retime");
    assert!(
        books
            .read("time/imported/2026-09.timeclock")
            .contains("o 2026-09-29 15:00:00")
    );
    entries::delete(&time, None, &locks, &clock).expect("delete session");
    assert!(
        !books
            .read("time/imported/2026-09.timeclock")
            .contains("i 2026")
    );
}

#[test]
fn accounts_resolve_tags_with_their_source_and_rename() {
    let books = books();
    let time = books.time();
    let session = books.session();
    let book = time.require().expect("book");
    let view = accounts::view(&time, &book, session.index(), "2026-11");
    let row = |name: &str| {
        view.accounts
            .iter()
            .find(|r| r.account == name)
            .expect(name)
    };
    let tax = row("clients:acme:tax-return");
    assert_eq!(
        tax.customer
            .as_ref()
            .map(|c| (c.value.as_str(), c.inherited)),
        Some(("Acme Corp", true))
    );
    assert_eq!(
        tax.item.as_ref().map(|c| (c.value.as_str(), c.inherited)),
        Some(("TAX", false))
    );
    assert_eq!(
        tax.billable.as_ref().map(|c| c.from.as_str()),
        Some("clients")
    );
    assert!(tax.customer_found);
    assert_eq!(tax.month.text, "3h 40m");
    assert!(
        row("clients:initech").customer_found,
        "ledger_customer links it"
    );
    assert_eq!(view.undeclared, vec!["internal:training".to_string()]);
    // Declare it; link a customer; archive; rename with a preview first.
    accounts::declare(&time, None, "internal:training", &[]).expect("declare");
    accounts::set_tag(
        &time,
        None,
        "clients:orphan",
        TagRole::Customer,
        Some("Globex"),
    )
    .expect("link");
    let text = books.read("time-accounts.journal");
    assert!(text.contains("account internal:training"), "{text}");
    assert!(
        text.contains("account clients:orphan                            ; customer: Globex"),
        "{text}"
    );
    let preview = accounts::rename(
        &time,
        None,
        "clients:acme:advisory",
        "clients:acme:advice",
        false,
    )
    .expect("preview");
    assert!(!preview.done.applied);
    assert!(preview.saved.is_empty());
    assert!(preview.done.lines.len() >= 4, "{:#?}", preview.done.lines);
    let applied = accounts::rename(
        &time,
        None,
        "clients:acme:advisory",
        "clients:acme:advice",
        true,
    )
    .expect("apply");
    assert!(applied.done.applied);
    assert!(!books.read("time/pat/2026-11.timedot").contains("advisory"));
    assert!(
        books
            .read("time-accounts.journal")
            .contains("account clients:acme:advice")
    );
}

#[test]
fn summaries_group_cut_rank_and_series() {
    let books = books();
    let book = books.time().require().expect("book");
    let by_customer = report::summary(
        &book,
        &SummaryQuery {
            group: vec![Group::Customer],
            ..SummaryQuery::default()
        },
    );
    assert_eq!(by_customer.total.text, "33h 24m");
    let acme = by_customer
        .rows
        .iter()
        .find(|r| r.key == ["Acme Corp"])
        .expect("acme");
    assert_eq!(acme.quantity.text, "14h 04m");
    let top = report::summary(
        &book,
        &SummaryQuery {
            filter: Filter {
                period: entries::period(Some("2026-11-01"), Some("2026-11-30")),
                ..Filter::default()
            },
            group: vec![Group::Account, Group::Person],
            depth: Some(2),
            top: Some(2),
            series: report::interval("week"),
        },
    );
    assert_eq!(top.rows.len(), 2);
    assert_eq!(top.rows[0].key, ["clients:globex", "sam"]);
    assert!(top.other.is_some());
    assert_eq!(top.buckets.first().map(String::as_str), Some("2026-W45"));
    assert_eq!(
        top.rows[0].series.as_ref().map(Vec::len),
        Some(top.buckets.len())
    );
}

#[test]
fn summaries_group_by_note_and_by_many_parts() {
    let books = books();
    let book = books.time().require().expect("book");
    let november = || Filter {
        period: entries::period(Some("2026-11-01"), Some("2026-11-30")),
        ..Filter::default()
    };
    let notes = report::summary(
        &book,
        &SummaryQuery {
            filter: november(),
            group: vec![Group::Note],
            ..SummaryQuery::default()
        },
    );
    let email = notes
        .rows
        .iter()
        .find(|r| r.key == ["email"])
        .expect("email");
    assert_eq!(email.entries, 2);
    assert_eq!(email.quantity.seconds, 240);
    // Tags are not the note: "fieldwork" twice, never "bill: …".
    assert!(notes.rows.iter().all(|r| !r.key[0].contains(':')));
    assert!(notes.rows.iter().any(|r| r.key == [""]));
    assert_eq!(Group::parse("note"), Some(Group::Note));
    // The dashboard's fact table: one call, every part a function of the
    // entry, adding up to the same total.
    let facts = report::summary(
        &book,
        &SummaryQuery {
            filter: november(),
            group: vec![
                Group::Account,
                Group::Customer,
                Group::Person,
                Group::Billable,
                Group::Billed,
                Group::Period(report::interval("day").expect("day")),
            ],
            ..SummaryQuery::default()
        },
    );
    assert!(facts.rows.iter().all(|r| r.key.len() == 6));
    let seconds: i64 = facts.rows.iter().map(|r| r.quantity.seconds).sum();
    assert_eq!(seconds, facts.total.seconds);
    assert_eq!(facts.total, notes.total);
}

#[test]
fn unbilled_suggests_periods_and_finds_orphans() {
    let books = books();
    let book = books.time().require().expect("book");
    let session = books.session();
    let locks = Locks::of(session.index(), session.booked_numbers());
    let view = report::unbilled(&book, session.index(), &locks.known, TODAY);
    let acme = view
        .customers
        .iter()
        .find(|c| c.customer == "Acme Corp")
        .expect("acme");
    assert_eq!(acme.cadence.as_deref(), Some("monthly"));
    assert_eq!(acme.periods[0].from, "2026-10-01");
    assert_eq!(acme.periods[0].to, "2026-10-31");
    assert_eq!(acme.periods[0].quantity.text, "50m");
    let initech = view
        .customers
        .iter()
        .find(|c| c.customer == "Initech Inc")
        .expect("initech");
    assert_eq!(initech.title.as_deref(), Some("Initech"));
    assert_eq!(view.needs_customer[0].account, "clients:orphan");
    assert!(view.orphans.is_empty());
    let unpaid = report::unpaid(&book, session.index(), TODAY, false);
    let people: Vec<&str> = unpaid.iter().map(|p| p.person.as_str()).collect();
    assert_eq!(people, ["riley", "sam"], "pat's own time is no payable");
}

/// Plan 46 fact 4: a line billed on an indented `; invoice:` line under it
/// is billed — locked, not unbilled, never the line new minutes go onto.
#[test]
fn a_line_billed_on_a_continuation_line_is_billed_everywhere() {
    let books = books();
    let time = books.time();
    let session = books.session();
    let locks = Locks::of(session.index(), session.booked_numbers());
    let acme_entries = |book: &ledgeline_core::time::model::TimeBook| {
        report::unbilled(book, session.index(), &locks.known, TODAY)
            .customers
            .iter()
            .find(|c| c.customer == "Acme Corp")
            .map(|c| c.entries)
            .expect("acme")
    };
    let before = acme_entries(&time.require().expect("book"));
    let file = "time/pat/2026-11.timedot";
    let billed_line = "    clients:acme:advisory             1h  ; call with Fred about the K-1\n";
    let text = books.read(file);
    assert!(text.contains(billed_line));
    std::fs::write(
        books.root.join(file),
        text.replace(billed_line, &format!("{billed_line}    ; invoice: 101\n")),
    )
    .expect("write");
    let book = time.require().expect("book");
    assert_eq!(acme_entries(&book), before - 1, "no longer unbilled");
    let listed = entries::list(
        &book,
        &EntryQuery {
            from: Some(TODAY.into()),
            to: Some(TODAY.into()),
            person: Some("pat".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    let advisory = listed
        .entries
        .iter()
        .find(|e| e.account == "clients:acme:advisory")
        .expect("advisory");
    assert!(advisory.locked.is_some(), "101 is sent: {advisory:?}");
    assert_eq!(advisory.note, "call with Fred about the K-1");
    // Fifteen more minutes: a new line after the billed one's block.
    track::adjust(
        &time,
        None,
        Some("pat"),
        "clients:acme:advisory",
        15,
        &at("2026-11-26T15:00:00"),
    )
    .expect("adjust");
    let text = books.read(file);
    assert!(
        text.contains(&format!(
            "{billed_line}    ; invoice: 101\n    internal:admin                   15m\n    clients:acme:advisory            15m\n"
        )),
        "{text}"
    );
}

fn acme_november() -> BillRequest {
    BillRequest {
        customer: "Acme Corp".into(),
        from: Some("2026-11-01".into()),
        to: Some("2026-11-30".into()),
        group: Grouping::AccountPerson,
        ..BillRequest::default()
    }
}

#[test]
fn bill_preview_rounds_rows_suggests_items_and_builds_the_timesheet() {
    let books = books();
    let time = books.time();
    let book = time.require().expect("book");
    let session = books.session();
    let preview = bill::preview(
        &time,
        &book,
        session.index(),
        session.vault(),
        &acme_november(),
        TODAY,
    )
    .expect("preview");
    assert!(preview.ready, "{:?}", preview.needs);
    let group = |key: &str| preview.groups.iter().find(|g| g.key == key).expect(key);
    // Two 2-minute emails and an hour: 64 minutes, up to 66 by the row.
    let pat = group("clients:acme:advisory | pat");
    assert_eq!(pat.actual.text, "1h 04m");
    assert_eq!(pat.rounded.text, "1h 06m");
    assert_eq!(pat.hours, "1.10");
    assert_eq!(
        pat.item.as_ref().map(|i| (i.code.as_str(), i.source)),
        Some(("ARCH", "account"))
    );
    assert_eq!(pat.amount.as_deref(), Some("$275.00"));
    assert_eq!(pat.description, "Advisory (pat), Nov 2026");
    // Sam's time on Acme bills as JR (`time_items`).
    let sam = group("clients:acme:advisory | sam");
    assert_eq!(
        sam.item.as_ref().map(|i| (i.code.as_str(), i.source)),
        Some(("JR", "time-items"))
    );
    assert_eq!(sam.rate.as_deref(), Some("$150"));
    assert_eq!(preview.amount.as_deref(), Some("$1,390.00"));
    assert_eq!(preview.actual.text, "7h 14m");
    // Travel isn't billable, so it isn't on the bill.
    assert!(
        preview
            .groups
            .iter()
            .all(|g| g.account != "clients:acme:travel")
    );
    let sheet = preview.timesheet.as_deref().expect("daily timesheet");
    assert!(
        sheet.contains("| 2026-11-02 | pat | gathered documents; email | 1.53 |"),
        "{sheet}"
    );
    // A rate override and an exclusion.
    let request = BillRequest {
        groups: vec![
            GroupChoice {
                key: "clients:acme:advisory | sam".into(),
                rate: Some("$100".into()),
                ..GroupChoice::default()
            },
            GroupChoice {
                key: "clients:acme:tax-return | pat".into(),
                exclude: true,
                ..GroupChoice::default()
            },
        ],
        ..acme_november()
    };
    let changed = bill::preview(
        &time,
        &book,
        session.index(),
        session.vault(),
        &request,
        TODAY,
    )
    .expect("preview");
    assert_eq!(changed.amount.as_deref(), Some("$525.00"));
    // A customer with no item anywhere needs one.
    let initech = bill::preview(
        &time,
        &book,
        session.index(),
        session.vault(),
        &BillRequest {
            customer: "Initech".into(),
            ..BillRequest::default()
        },
        TODAY,
    )
    .expect("preview");
    assert!(!initech.ready);
    assert!(
        initech.needs[0].contains("an item or a rate"),
        "{:?}",
        initech.needs
    );
}

#[test]
fn billing_tags_first_and_release_takes_them_off() {
    let books = books();
    let time = books.time();
    let book = time.require().expect("book");
    let mut session = books.session();
    let booked: Vec<String> = session.booked_numbers().to_vec();
    let booked: Vec<&str> = booked.iter().map(String::as_str).collect();
    let request = BillRequest {
        groups: vec![GroupChoice {
            key: "clients:acme:tax-return | pat".into(),
            item: Some("TAX".into()),
            remember: true,
            ..GroupChoice::default()
        }],
        ..acme_november()
    };
    let (index, vault) = session.parts_mut();
    let created =
        bill::create(&time, &book, index, vault, &booked, &request, TODAY).expect("create");
    assert_eq!(created.number, "102");
    assert_eq!(created.tagged.len(), 6);
    let draft = books.read(created.path.as_str());
    assert!(draft.contains("status: draft"), "{draft}");
    assert!(
        draft.contains("| JR   | Advisory (sam), Nov 2026"),
        "{draft}"
    );
    assert!(draft.contains("## Timesheet"), "{draft}");
    assert!(
        books
            .read("time/sam/2026-11.timedot")
            .contains("second opinion on the K-1, invoice: 102")
    );
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("reviewed schedules C and E, invoice: 102")
    );
    // It no longer shows as unbilled; a draft's lines aren't locked.
    let book = time.require().expect("book");
    let locks = Locks::of(session.index(), session.booked_numbers());
    let tagged = entries::list(
        &book,
        &EntryQuery {
            invoice: Some("102".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    assert_eq!(tagged.entries.len(), 6);
    assert!(tagged.entries.iter().all(|e| e.locked.is_none()));
    // Released (a deleted draft): every tag comes off.
    let released = bill::release(&time, None, "102", Release::Release).expect("release");
    assert_eq!(released.done, 6);
    assert!(
        !books
            .read("time/pat/2026-11.timedot")
            .contains("invoice: 102")
    );
    // A void's nobill.
    let (index, vault) = session.parts_mut();
    bill::create(
        &time,
        &time.require().expect("book"),
        index,
        vault,
        &booked,
        &BillRequest {
            number: Some("103".into()),
            ..acme_november()
        },
        TODAY,
    )
    .expect("again");
    bill::release(&time, None, "103", Release::Nobill).expect("nobill");
    assert!(
        books
            .read("time/pat/2026-11.timedot")
            .contains("reviewed schedules C and E, nobill: invoice 103 void")
    );
}

#[test]
fn a_draft_that_cant_be_written_rolls_its_tags_back() {
    let books = books();
    let time = books.time();
    let book = time.require().expect("book");
    let mut session = books.session();
    // Something already holds the draft's file name.
    std::fs::write(
        books
            .root
            .join("invoices/2026-11-26 Acme Corp Invoice 102.md"),
        "taken\n",
    )
    .expect("write");
    let (index, vault) = session.parts_mut();
    let error = bill::create(
        &time,
        &book,
        index,
        vault,
        &["101"],
        &acme_november(),
        TODAY,
    )
    .expect_err("the name is taken");
    assert_eq!(error.code(), "exists", "{error}");
    assert!(
        !books
            .read("time/pat/2026-11.timedot")
            .contains("invoice: 102")
    );
    assert!(
        !books
            .read("time/sam/2026-11.timedot")
            .contains("invoice: 102")
    );
}

#[test]
fn paying_a_contractor_books_a_payable_and_tags_the_time() {
    let books = books();
    let time = books.time();
    let book = time.require().expect("book");
    let mut session = books.session();
    let request = |person: &str| PayRequest {
        person: person.into(),
        from: None,
        to: None,
        entries: None,
        lines: Default::default(),
        note: None,
        create_note: None,
        rate: None,
        expense_account: None,
        payable_account: None,
        terms: None,
        save_to_note: true,
        date: None,
        reference: None,
        preview: false,
    };
    let preview = pay::preview(
        &book,
        session.index(),
        session.journal(),
        &request("sam"),
        TODAY,
    )
    .expect("preview");
    assert!(preview.ready, "{:?}", preview.needs);
    assert_eq!(
        preview.payee.as_deref(),
        Some("Lee Consulting"),
        "pay_to's vendor"
    );
    assert_eq!(preview.reference, "sam timesheet 2026-11");
    assert_eq!(preview.amount.as_deref(), Some("$977.50"));
    assert_eq!(preview.due.as_deref(), Some("2026-12-11"));
    assert_eq!(
        preview.payable_account.as_deref(),
        Some("liabilities:accounts payable")
    );
    let riley = pay::preview(
        &book,
        session.index(),
        session.journal(),
        &request("riley"),
        TODAY,
    )
    .expect("preview");
    let needed: Vec<&str> = riley.needs.iter().map(|n| n.field).collect();
    assert_eq!(needed, ["rate", "expense_account"]);
    let invoice_books = session.invoice_books();
    let (index, vault) = session.parts_mut();
    let booked = pay::create(
        &time,
        &book,
        index,
        vault,
        &invoice_books,
        &request("sam"),
        TODAY,
    )
    .expect("book");
    assert_eq!(booked.tagged.len(), 4);
    let journal = books.read("books.journal");
    assert!(journal.contains("2026-11-26 Lee Consulting | sam timesheet 2026-11  ; reference: sam timesheet 2026-11, due: 2026-12-11"), "{journal}");
    assert!(
        journal.contains("liabilities:accounts payable  $-977.50  ; vendor: Lee Consulting"),
        "{journal}"
    );
    assert!(
        books
            .read("time/sam/2026-11.timedot")
            .contains("bill: sam timesheet 2026-11")
    );
    // Booked: the time is locked, and a second bill for it is refused.
    let book = time.require().expect("book");
    let again = pay::preview(
        &book,
        session.index(),
        session.journal(),
        &request("sam"),
        TODAY,
    )
    .expect("preview");
    assert!(!again.ready);
    let locks = Locks::of(session.index(), session.booked_numbers());
    let sam = entries::list(
        &book,
        &EntryQuery {
            person: Some("sam".into()),
            ..EntryQuery::default()
        },
        &locks,
    );
    assert!(sam.entries.iter().all(|e| e.locked.is_some()));
    // Riley: the answers are saved to the note when asked.
    let invoice_books = session.invoice_books();
    let (index, vault) = session.parts_mut();
    let riley = pay::create(
        &time,
        &book,
        index,
        vault,
        &invoice_books,
        &PayRequest {
            rate: Some("$60".into()),
            expense_account: Some("expenses:contractors".into()),
            ..request("riley")
        },
        TODAY,
    )
    .expect("riley");
    assert_eq!(riley.saved_to_note, ["pay_rate", "expense_account"]);
    assert!(
        books
            .read("people/Riley Chen.md")
            .contains("pay_rate: $60\n")
    );
}
