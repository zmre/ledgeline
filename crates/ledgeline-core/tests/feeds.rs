//! Bank feeds (plan 43, WP-A) over the committed, hand-written fixtures in
//! `fixtures/feeds/`.
//!
//! The modules' own `#[cfg(test)]` blocks cover each rule in isolation. This
//! suite covers what only whole responses and whole files can show: v1 and v2
//! reading the same data the same way, a full fetch → normalize → CSV →
//! dupes run against a journal, and every `ledgeline.toml` writer leaving the
//! rest of a hand-edited file alone. Nothing here touches the network.

mod common;

use ledgeline_core::feeds::config::{
    self, AccountLink, ConfigError, ConnectionLink, ManualAccount,
};
use ledgeline_core::feeds::dupes::{self, Existing};
use ledgeline_core::feeds::normalize::{
    FeedColumns, NormalizeOptions, Normalized, account_short, epoch_to_date, fetched_ids,
    normalize, to_csv,
};
use ledgeline_core::feeds::simplefin::{
    AccessUrl, ParseError, decode_setup_token, decode_setup_token_with, parse_accounts,
};
use ledgeline_core::feeds::{FeedAccount, FeedBatch, FeedCurrency, ProblemKind};
use ledgeline_core::parse_journal;
use ledgeline_core::reimport::{self, ID_TAG};
use proptest::prelude::*;
use std::path::PathBuf;

/// California in November.
const PACIFIC: i32 = -8 * 3_600;

fn feeds_dir() -> PathBuf {
    common::fixtures_dir().join("feeds")
}

fn read(relative: &str) -> String {
    let path = feeds_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn batch(name: &str) -> FeedBatch {
    parse_accounts(read(&format!("simplefin/{name}")).as_bytes())
        .unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

fn account<'a>(batch: &'a FeedBatch, id: &str) -> &'a FeedAccount {
    batch
        .accounts
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("{id} is in the batch"))
}

fn normalized(account: &FeedAccount, connection: &str, start: &str) -> Normalized {
    let short = account_short(connection, &account.id);
    let columns = FeedColumns::default();
    normalize(
        account,
        &NormalizeOptions {
            start_date: start,
            account_short: &short,
            columns: &columns,
            utc_offset_seconds: PACIFIC,
        },
    )
}

// ---------------------------------------------------------------------------
// simplefin: parsing
// ---------------------------------------------------------------------------

#[test]
fn demo_v2_reads_every_field() {
    let demo = batch("demo-v2.json");
    assert!(demo.problems.is_empty(), "{:?}", demo.problems);
    assert!(demo.notices.is_empty());
    assert_eq!(demo.connections.len(), 1);
    assert_eq!(demo.connections[0].id, "CON-demo-1");
    assert_eq!(demo.connections[0].name, "SimpleFIN Demo Bank");
    assert_eq!(
        demo.accounts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ACT-demo-checking",
            "ACT-demo-savings",
            "ACT-demo-brokerage"
        ]
    );

    let checking = account(&demo, "ACT-demo-checking");
    assert_eq!(checking.name, "SimpleFIN Checking");
    assert_eq!(checking.conn_id.as_deref(), Some("CON-demo-1"));
    assert_eq!(checking.org_name.as_deref(), Some("SimpleFIN Demo Bank"));
    assert_eq!(
        checking.org_url.as_deref(),
        Some("https://demo.example-bank.test/")
    );
    assert_eq!(checking.currency, FeedCurrency::Code("USD".into()));
    assert_eq!(checking.balance, "26473.36");
    assert_eq!(checking.available_balance.as_deref(), Some("26473.36"));
    assert_eq!(checking.balance_date, Some(1_794_268_800));
    assert_eq!(checking.transactions.len(), 4);

    let coffee = &checking.transactions[0];
    assert_eq!(coffee.id, "TRN-1001");
    assert_eq!(coffee.posted, 1_793_664_000);
    assert_eq!(coffee.amount, "-4.50");
    assert_eq!(coffee.payee.as_deref(), Some("Blue Bottle Coffee"));
    assert_eq!(coffee.memo, None, "an empty memo is no memo");
    assert_eq!(coffee.transacted_at, Some(1_793_642_400));
    assert!(!coffee.pending);
    assert_eq!(checking.transactions[3].payee, None);

    assert!(account(&demo, "ACT-demo-savings").transactions.is_empty());
    assert_eq!(
        account(&demo, "ACT-demo-brokerage")
            .available_balance
            .as_deref(),
        Some("0.00")
    );
}

#[test]
fn v1_and_v2_read_the_same_data_the_same_way() {
    let v1 = batch("demo-v1.json");
    let v2 = batch("demo-v2.json");
    assert!(v1.problems.is_empty(), "{:?}", v1.problems);
    assert!(v1.connections.is_empty(), "v1 has no connection objects");
    assert!(v1.accounts.iter().all(|a| a.conn_id.is_none()));
    // Apart from the connection link, every account is identical — including
    // the savings account, whose v1 `org` has only a domain.
    let unlinked: Vec<FeedAccount> = v2
        .accounts
        .iter()
        .cloned()
        .map(|a| FeedAccount { conn_id: None, ..a })
        .collect();
    assert_eq!(v1.accounts, unlinked);
}

#[test]
fn errlist_becomes_problems_and_x_api_message_notices() {
    let errors = batch("errors-v2.json");
    let summary: Vec<(ProblemKind, &str, Option<&str>, Option<&str>)> = errors
        .problems
        .iter()
        .map(|p| {
            (
                p.kind,
                p.code.as_str(),
                p.conn_id.as_deref(),
                p.account_id.as_deref(),
            )
        })
        .collect();
    // `errors` repeats the con.auth text for old clients; with `errlist`
    // present it is ignored, so nothing is reported twice.
    assert_eq!(
        summary,
        vec![
            (ProblemKind::ConAuth, "con.auth", Some("CON-chase"), None),
            (
                ProblemKind::ActFailed,
                "act.failed",
                Some("CON-chase"),
                Some("ACT-chase-card")
            ),
            (ProblemKind::GenApi, "gen.api", None, None),
            (ProblemKind::Quota, "gen.warn", None, None),
        ]
    );
    assert_eq!(
        errors.problems[0].message,
        "Chase needs you to sign in again."
    );
    assert_eq!(
        errors.notices,
        vec![
            "SimpleFIN Bridge maintenance Nov 20, 02:00-04:00 UTC.".to_string(),
            "[31mSpoofed colour".to_string(),
        ]
    );
    assert_eq!(
        errors.accounts.len(),
        1,
        "the unknown top-level field is ignored"
    );
    assert_eq!(
        errors.accounts[0].org_url.as_deref(),
        Some("https://www.chase.com/")
    );
}

#[test]
fn v1_error_strings_are_classified_by_their_text() {
    let body = br#"{
        "errors": [
            "Connection to First Tech may need attention",
            "You are nearing your daily request quota",
            "Something happened",
            ""
        ],
        "accounts": []
    }"#;
    let kinds: Vec<ProblemKind> = parse_accounts(body)
        .expect("parses")
        .problems
        .iter()
        .map(|p| p.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ProblemKind::NeedsAttention,
            ProblemKind::Quota,
            ProblemKind::Gen
        ]
    );
}

#[test]
fn a_custom_currency_is_a_url() {
    let miles = batch("custom-currency.json");
    assert_eq!(
        miles.accounts[0].currency,
        FeedCurrency::Url(
            "https://beta-bridge.simplefin.org/simplefin/currency/example-air-miles".into()
        )
    );
    assert_eq!(miles.accounts[0].balance, "52000");
}

#[test]
fn odd_amounts_are_kept_when_plain_and_reported_when_not() {
    let odd = batch("odd-amounts.json");
    assert_eq!(odd.accounts.len(), 1, "the account with no id is dropped");
    let checking = &odd.accounts[0];
    assert_eq!(checking.balance, "", "a balance with a comma isn't trusted");
    assert_eq!(checking.available_balance.as_deref(), Some("12.5"));
    assert_eq!(
        checking
            .transactions
            .iter()
            .map(|t| (t.id.as_str(), t.amount.as_str()))
            .collect::<Vec<_>>(),
        vec![("O-1", "5"), ("O-2", "-0.1"), ("O-6", "-3.25")]
    );
    // balance, O-3, O-4, O-5, the id-less row, the non-object row, and the
    // id-less account.
    assert_eq!(odd.problems.len(), 7, "{:#?}", odd.problems);
    assert!(odd.problems.iter().all(|p| p.kind == ProblemKind::Other));
    assert!(odd.problems.iter().all(|p| p.code.is_empty()));
    assert!(
        odd.problems
            .iter()
            .any(|p| p.message.contains("O-3") && p.message.contains("1,000.00"))
    );
    assert_eq!(
        odd.problems
            .iter()
            .filter(|p| p.account_id.as_deref() == Some("ACT-odd"))
            .count(),
        6
    );
    assert_eq!(normalized(checking, "CON-odd", "").rows.len(), 3);
}

#[test]
fn a_body_that_is_not_an_object_is_an_error() {
    assert_eq!(parse_accounts(b"null"), Err(ParseError::NotAnObject));
    assert!(matches!(
        parse_accounts(b"<html>403 Forbidden</html>"),
        Err(ParseError::NotJson { .. })
    ));
    // An error-only response with no `accounts` key is fine.
    let only_errors =
        parse_accounts(br#"{"errlist":[{"code":"gen.auth","msg":"no"}]}"#).expect("parses");
    assert!(only_errors.accounts.is_empty());
    assert_eq!(only_errors.problems[0].kind, ProblemKind::GenAuth);
}

// ---------------------------------------------------------------------------
// simplefin: tokens and access URLs
// ---------------------------------------------------------------------------

#[test]
fn the_demo_token_decodes() {
    // base64("https://beta-bridge.simplefin.org/simplefin/claim/DEMO").
    let token = "aHR0cHM6Ly9iZXRhLWJyaWRnZS5zaW1wbGVmaW4ub3JnL3NpbXBsZWZpbi9jbGFpbS9ERU1P";
    let decoded = decode_setup_token(token).expect("the demo token is valid");
    assert_eq!(
        decoded.claim_url,
        "https://beta-bridge.simplefin.org/simplefin/claim/DEMO"
    );
    assert_eq!(decoded.host, "beta-bridge.simplefin.org");
    assert!(decode_setup_token_with(token, true).is_ok());
}

#[test]
fn the_demo_access_url_never_prints_its_password() {
    let access = AccessUrl::parse(
        "https://demo:hunter2-demo-password@beta-bridge.simplefin.org/simplefin",
        false,
    )
    .expect("valid");
    assert_eq!(access.base, "https://beta-bridge.simplefin.org/simplefin");
    let shown = format!("{access} {access:?}");
    assert!(!shown.contains("hunter2-demo-password"), "{shown}");
    assert!(access.to_secret_string().contains("hunter2-demo-password"));
}

// ---------------------------------------------------------------------------
// normalize
// ---------------------------------------------------------------------------

#[test]
fn the_demo_checking_csv_matches_the_golden_file() {
    let demo = batch("demo-v2.json");
    let out = normalized(
        account(&demo, "ACT-demo-checking"),
        "CON-demo-1",
        "2026-11-01",
    );
    assert_eq!(
        account_short("CON-demo-1", "ACT-demo-checking"),
        "878479726e"
    );
    assert_eq!(
        (
            out.dropped_pending,
            out.dropped_before_start,
            out.dropped_zero
        ),
        (0, 0, 0)
    );
    let csv = to_csv(&out.rows, &FeedColumns::default());
    assert_eq!(csv, read("simplefin/demo-v2.checking.csv"));

    // A real CSV reader gets every field back, quotes and all.
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let records: Vec<csv::StringRecord> = reader
        .records()
        .collect::<Result<_, _>>()
        .expect("the CSV reads back");
    assert_eq!(records.len(), 4);
    assert_eq!(&records[3][2], "RENT, NOVEMBER \"UNIT 4\"");
}

#[test]
fn v1_and_v2_give_the_same_csv() {
    let csv_of = |name: &str| {
        let fed = batch(name);
        // The prefix comes from the link, not the response, so both use it.
        let out = normalized(account(&fed, "ACT-demo-checking"), "CON-demo-1", "");
        to_csv(&out.rows, &FeedColumns::default())
    };
    assert_eq!(csv_of("demo-v1.json"), csv_of("demo-v2.json"));
}

#[test]
fn the_start_date_drops_earlier_rows() {
    let demo = batch("demo-v2.json");
    let out = normalized(
        account(&demo, "ACT-demo-checking"),
        "CON-demo-1",
        "2026-11-06",
    );
    assert_eq!(out.dropped_before_start, 2);
    assert_eq!(out.rows.len(), 2);
    assert!(out.rows.iter().all(|r| r.date.as_str() >= "2026-11-06"));
}

#[test]
fn pending_rows_never_reach_the_csv() {
    let pending = batch("pending.json");
    let card = &pending.accounts[0];
    let out = normalized(card, "CON-p", "");
    assert_eq!(out.dropped_pending, 3);
    assert_eq!(out.dropped_zero, 1);
    let short = account_short("CON-p", "ACT-p");
    assert_eq!(
        out.rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        vec![format!("{short}-P-4")]
    );
    assert_eq!(out.rows[0].transacted, "", "same day, so no secondary date");
    // …while every id, pending ones included, counts as fetched.
    assert_eq!(fetched_ids(card, &short).len(), 5);
}

#[test]
fn borrowed_column_names_rename_only_the_header() {
    let demo = batch("demo-v2.json");
    let out = normalized(account(&demo, "ACT-demo-checking"), "CON-demo-1", "");
    let columns = FeedColumns {
        payee: "name".into(),
        description: "sfdescription".into(),
        ..FeedColumns::default()
    };
    let csv = to_csv(&out.rows, &columns);
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some("date,amount,name,sfdescription,memo,id,transacted,mcc,mcc_name,extra")
    );
    let golden = read("simplefin/demo-v2.checking.csv");
    assert!(golden.lines().skip(1).eq(lines));
}

// ---------------------------------------------------------------------------
// dupes, end to end
// ---------------------------------------------------------------------------

const CHECKING: &str = "assets:bank:demo:checking";

fn synced_split(journal_text: &str) -> dupes::Split {
    let journal = parse_journal(journal_text, "synced.journal").expect("journal parses");
    let demo = batch("demo-v2.json");
    let checking = account(&demo, "ACT-demo-checking");
    let short = account_short("CON-demo-1", &checking.id);
    let out = normalized(checking, "CON-demo-1", "2026-11-01");
    dupes::split(
        out.rows,
        &journal,
        CHECKING,
        &dupes::known_ids(&journal),
        &fetched_ids(checking, &short),
    )
}

#[test]
fn a_fetch_against_a_partly_synced_journal_splits_three_ways() {
    let split = synced_split(&read("simplefin/synced.journal"));
    assert_eq!(split.already, vec!["878479726e-TRN-1001"]);
    assert_eq!(
        split
            .fresh
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        // Payroll has no lookalike; the rent's lookalike shares no payee word.
        vec!["878479726e-TRN-1002", "878479726e-TRN-1004"]
    );
    assert_eq!(split.held.len(), 1);
    assert_eq!(split.held[0].row.id, "878479726e-TRN-1003");
    assert_eq!(
        split.held[0].existing,
        Existing {
            index: 3,
            date: "2026-11-06".into(),
            description: "SAFEWAY STORE 1234".into(),
            amount: "$-82.17".into(),
        }
    );
}

/// "It's a duplicate" adds the feed id as a second `id:` tag. Afterwards the
/// id index finds the transaction by either id, and the next fetch reads the
/// row as already there.
#[test]
fn its_a_duplicate_resolves_through_a_second_id_tag() {
    let marked = read("simplefin/synced.journal").replace(
        "id:FIT-20261106-77",
        "id:FIT-20261106-77, id:878479726e-TRN-1003",
    );
    let journal = parse_journal(&marked, "synced.journal").expect("parses");
    let index = reimport::build_index(&journal, ID_TAG);
    for id in ["FIT-20261106-77", "878479726e-TRN-1003"] {
        let (txn, carriers) = index.get(id).unwrap_or_else(|| panic!("{id} indexed"));
        assert_eq!(txn.description, "SAFEWAY STORE 1234");
        assert_eq!(carriers, 1);
    }
    let split = synced_split(&marked);
    assert_eq!(
        split.already,
        vec!["878479726e-TRN-1001", "878479726e-TRN-1003"]
    );
    assert!(split.held.is_empty());
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

const BRIDGE: &str = "4f6c1a2e-0000-4000-8000-000000000001";

fn first_tech() -> AccountLink {
    AccountLink {
        connection: BRIDGE.into(),
        remote_id: "ACT-12ab".into(),
        label: "First Tech · Checking ••1234".into(),
        journal_account: "assets:bank:firsttech:checking".into(),
        rules: "imports/rules/firsttech-checking.sync.csv.rules".into(),
        target: "2026.journal".into(),
        start_date: "2026-11-01".into(),
        balance_sign: 1,
        enabled: true,
    }
}

fn ally() -> AccountLink {
    AccountLink {
        remote_id: "ACT-ally-sav".into(),
        label: "Ally · Savings".into(),
        journal_account: "assets:bank:ally:savings".into(),
        rules: "imports/rules/ally-savings.sync.csv.rules".into(),
        start_date: "2026-10-15".into(),
        balance_sign: -1,
        enabled: false,
        ..first_tech()
    }
}

/// The parts of `with-ui.toml` no sync writer may change by a byte.
const UNTOUCHABLE: [&str; 3] = [
    "# Per-ledger settings for a synthetic ledger. Hand-written: every comment,\n# blank line and key order here must survive a sync write.\n\n[ui]\n# Local overrides\ncurrencyDisplay = \"symbol\"  # trailing comment\nautoSync = true\n",
    "[other]\n# A table this build doesn't know.\nx = 1\n",
    "# Linked on Nov 1\n",
];

fn assert_untouched(after: &str) {
    for part in UNTOUCHABLE {
        assert!(after.contains(part), "lost {part:?} in:\n{after}");
    }
    assert!(after.starts_with(UNTOUCHABLE[0]), "[ui] moved:\n{after}");
}

#[test]
fn the_sample_file_parses() {
    let (sync, warnings) = config::parse(&read("config/with-ui.toml"));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        sync.connections,
        vec![ConnectionLink {
            id: BRIDGE.into(),
            name: "SimpleFIN Bridge".into(),
            kind: "simplefin".into(),
        }]
    );
    assert_eq!(sync.accounts, vec![first_tech()]);
    assert!(sync.manual.is_empty());
    assert_eq!(sync.accounts[0].key(), "2562b58c16");
    assert_eq!(sync.account("2562b58c16"), Some(&first_tech()));
    assert!(sync.connection(BRIDGE).is_some());
}

#[test]
fn updating_an_account_keeps_its_place_and_comments() {
    let before = read("config/with-ui.toml");
    let changed = AccountLink {
        label: "First Tech · Everyday".into(),
        target: "2027.journal".into(),
        ..first_tech()
    };
    let after = config::upsert_account(&before, &changed).expect("writes");
    assert_untouched(&after);
    assert!(
        after.contains("label = \"First Tech · Everyday\"   # keep me\n"),
        "{after}"
    );
    assert_eq!(after.matches("[[sync.account]]").count(), 1);
    assert_eq!(config::parse(&after).0.accounts, vec![changed]);
    // Writing the same thing again changes nothing.
    assert_eq!(
        config::upsert_account(&after, &config::parse(&after).0.accounts[0]).expect("writes"),
        after
    );
    // The unchanged link writes back byte for byte.
    assert_eq!(
        config::upsert_account(&before, &first_tech()).expect("writes"),
        before
    );
}

#[test]
fn a_new_account_lands_beside_the_others() {
    let before = read("config/with-ui.toml");
    let after = config::upsert_account(&before, &ally()).expect("writes");
    assert_untouched(&after);
    let new_at = after.find("ACT-ally-sav").expect("written");
    assert!(after.find("ACT-12ab").expect("kept") < new_at);
    assert!(new_at < after.find("[other]").expect("kept"), "{after}");
    assert!(
        after.contains("startDate = 2026-10-15\n"),
        "a TOML date: {after}"
    );
    assert_eq!(config::parse(&after).0.accounts, vec![first_tech(), ally()]);
}

#[test]
fn removing_an_account_leaves_everything_else() {
    let before = read("config/with-ui.toml");
    let with_ally = config::upsert_account(&before, &ally()).expect("writes");
    let removed = config::remove_account(&with_ally, BRIDGE, "ACT-ally-sav").expect("writes");
    assert_eq!(removed, before, "add then remove is the identity");

    // The removed table's own leading comment goes with it; nothing else does.
    let none_left = config::remove_account(&before, BRIDGE, "ACT-12ab").expect("writes");
    assert!(none_left.starts_with(UNTOUCHABLE[0]), "{none_left}");
    assert!(none_left.contains(UNTOUCHABLE[1]), "{none_left}");
    assert!(!none_left.contains(UNTOUCHABLE[2]), "{none_left}");
    assert!(config::parse(&none_left).0.accounts.is_empty());
    assert_eq!(config::parse(&none_left).0.connections.len(), 1);

    // Removing what isn't there is a no-op, byte for byte.
    assert_eq!(
        config::remove_account(&before, BRIDGE, "nope").expect("writes"),
        before
    );
    assert_eq!(
        config::remove_account("", BRIDGE, "nope").expect("writes"),
        ""
    );
}

#[test]
fn removing_a_connection_keeps_its_accounts() {
    let before = read("config/with-ui.toml");
    let after = config::remove_connection(&before, BRIDGE).expect("writes");
    assert_untouched(&after);
    let (sync, warnings) = config::parse(&after);
    assert!(warnings.is_empty());
    assert!(sync.connections.is_empty());
    assert_eq!(sync.accounts, vec![first_tech()]);
}

#[test]
fn updating_a_connection_keeps_its_comment() {
    let before = read("config/with-ui.toml");
    let renamed = ConnectionLink {
        id: BRIDGE.into(),
        name: "My Bridge".into(),
        kind: "simplefin".into(),
    };
    let after = config::upsert_connection(&before, &renamed).expect("writes");
    assert_untouched(&after);
    assert!(
        after.contains("name = \"My Bridge\"  # shown in the UI\n"),
        "{after}"
    );
    assert!(after.contains("# The Bridge\n"));
    assert_eq!(config::parse(&after).0.connections, vec![renamed]);
}

#[test]
fn manual_accounts_round_trip() {
    let before = read("config/with-ui.toml");
    let vanguard = ManualAccount {
        account: "assets:invest:vanguard".into(),
        download_url: Some("https://investor.vanguard.com/".into()),
        nudge: true,
    };
    let added = config::upsert_manual(&before, &vanguard).expect("writes");
    assert_untouched(&added);
    assert_eq!(config::parse(&added).0.manual, vec![vanguard.clone()]);

    let no_url = ManualAccount {
        download_url: None,
        nudge: false,
        ..vanguard
    };
    let updated = config::upsert_manual(&added, &no_url).expect("writes");
    assert!(!updated.contains("downloadUrl"));
    assert_eq!(config::parse(&updated).0.manual, vec![no_url]);
}

#[test]
fn writing_into_an_empty_file_creates_only_sync_tables() {
    let connection = ConnectionLink {
        id: BRIDGE.into(),
        name: "SimpleFIN Bridge".into(),
        kind: "simplefin".into(),
    };
    let text = config::upsert_connection("", &connection).expect("writes");
    let text = config::upsert_account(&text, &first_tech()).expect("writes");
    assert!(!text.contains("[sync]\n"), "no bare [sync] header: {text}");
    assert!(!text.contains("[ui]"));
    let (sync, warnings) = config::parse(&text);
    assert!(warnings.is_empty());
    assert_eq!(sync.connections, vec![connection]);
    assert_eq!(sync.accounts, vec![first_tech()]);
}

#[test]
fn a_ui_only_file_gains_sync_after_ui() {
    let before = "[ui]\ntheme = \"dusk\" # mine\n";
    let after = config::upsert_account(before, &first_tech()).expect("writes");
    assert!(after.starts_with(before), "{after}");
    assert_eq!(
        config::remove_account(&after, BRIDGE, "ACT-12ab").expect("writes"),
        before
    );
}

#[test]
fn malformed_entries_are_skipped_with_a_warning_each() {
    let (sync, warnings) = config::parse(&read("config/malformed.toml"));
    assert_eq!(
        sync.connections,
        vec![ConnectionLink {
            id: "good-1".into(),
            name: "Good".into(),
            kind: "simplefin".into(),
        }]
    );
    assert_eq!(
        sync.accounts,
        vec![AccountLink {
            connection: "good-1".into(),
            remote_id: "A-1".into(),
            label: "A-1".into(),
            journal_account: "assets:a".into(),
            rules: "a.csv.rules".into(),
            target: "a.journal".into(),
            start_date: "2026-01-31".into(),
            balance_sign: 1,
            enabled: true,
        }]
    );
    assert_eq!(
        sync.manual,
        vec![
            ManualAccount {
                account: "assets:invest:vanguard".into(),
                download_url: None,
                nudge: true,
            },
            ManualAccount {
                account: "assets:invest:fidelity".into(),
                download_url: Some("https://www.fidelity.com/".into()),
                nudge: false,
            },
        ]
    );
    // 3 connections, 5 accounts, 1 manual URL.
    assert_eq!(warnings.len(), 9, "{warnings:#?}");
    assert!(warnings.iter().all(|w| w.starts_with("ledgeline.toml")));
    assert!(warnings.iter().any(|w| w.contains("repeats")));
    assert!(warnings.iter().any(|w| w.contains("startDate")));
    assert!(warnings.iter().any(|w| w.contains("downloadUrl")));
}

#[test]
fn a_broken_file_reads_as_no_links_and_refuses_writes() {
    let (sync, warnings) = config::parse("[sync\nbroken");
    assert_eq!(sync, config::SyncConfig::default());
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        config::upsert_account("[sync\nbroken", &first_tech()),
        Err(ConfigError::InvalidToml(_))
    ));
    let (_, warnings) = config::parse("sync = \"nope\"\n");
    assert_eq!(warnings.len(), 1);
    let (_, warnings) = config::parse("[sync]\naccount = 3\n");
    assert_eq!(warnings.len(), 1);
}

#[test]
fn inline_arrays_are_read_too() {
    let text = format!("sync.connection = [{{ id = \"{BRIDGE}\", name = \"Inline\" }}]\n");
    let (sync, warnings) = config::parse(&text);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sync.connections[0].name, "Inline");
}

// ---------------------------------------------------------------------------
// Properties: hostile input never panics
// ---------------------------------------------------------------------------

/// JSON-shaped bodies, so the property reaches the field readers rather than
/// stopping at "not JSON" every time.
fn json_ish() -> impl Strategy<Value = String> {
    (
        "[a-zA-Z0-9 ,\\-]{0,6}",
        prop_oneof![
            Just("\"-1.00\"".to_string()),
            "\"[-0-9.,e ]{0,8}\"",
            "-?(0|[1-9][0-9]{0,19})(\\.[0-9]{1,4})?",
            Just("null".to_string()),
            Just("[]".to_string()),
            Just("{}".to_string()),
        ],
        prop_oneof![
            "-?(0|[1-9][0-9]{0,19})",
            Just("1e300".to_string()),
            Just("\"x\"".to_string()),
            Just("null".to_string()),
        ],
        any::<bool>(),
    )
        .prop_map(|(id, amount, posted, pending)| {
            format!(
                r#"{{"errlist":[{{"code":"{id}","msg":"{id}"}}],"x-api-message":"{id}","connections":[{{"conn_id":"{id}"}}],"accounts":[{{"id":"{id}","conn_id":"{id}","balance":{amount},"balance-date":{posted},"org":{{"domain":"{id}"}},"transactions":[{{"id":"{id}","amount":{amount},"posted":{posted},"transacted_at":{posted},"pending":{pending}}}]}}]}}"#
            )
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn decode_setup_token_never_panics(input in any::<String>()) {
        let _ = decode_setup_token(&input);
        let _ = decode_setup_token_with(&input, true);
    }

    #[test]
    fn decoding_any_encoded_text_never_panics(text in any::<String>()) {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::URL_SAFE.encode(&text);
        let _ = decode_setup_token_with(&encoded, true);
    }

    #[test]
    fn access_url_parse_never_panics(input in any::<String>()) {
        let _ = AccessUrl::parse(&input, false);
        if let Ok(access) = AccessUrl::parse(&input, true) {
            prop_assert_eq!(AccessUrl::parse(&access.to_secret_string(), true), Ok(access));
        }
    }

    #[test]
    fn parse_accounts_never_panics_on_bytes(body in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = parse_accounts(&body);
    }

    #[test]
    fn parse_accounts_never_panics_on_json(body in json_ish()) {
        let batch = parse_accounts(body.as_bytes());
        prop_assert!(batch.is_ok(), "{}", body);
        for account in batch.unwrap_or_default().accounts {
            for txn in &account.transactions {
                prop_assert!(ledgeline_core::feeds::is_decimal_text(&txn.amount));
            }
            let _ = normalized(&account, "c", "2000-01-01");
        }
    }

    #[test]
    fn config_parse_never_panics(text in any::<String>()) {
        let _ = config::parse(&text);
    }

    #[test]
    fn epoch_to_date_never_panics(epoch in any::<i64>(), offset in any::<i32>()) {
        let _ = epoch_to_date(epoch, offset);
    }

    /// Whatever a bank puts in a field, a real CSV reader gets it back.
    #[test]
    fn csv_fields_round_trip(fields in proptest::collection::vec(any::<String>(), 10)) {
        let row = ledgeline_core::feeds::normalize::NormRow {
            date: fields[0].clone(),
            amount: fields[1].clone(),
            payee: fields[2].clone(),
            description: fields[3].clone(),
            memo: fields[4].clone(),
            id: fields[5].clone(),
            transacted: fields[6].clone(),
            mcc: fields[7].clone(),
            mcc_name: fields[8].clone(),
            extra: fields[9].clone(),
        };
        let text = to_csv(std::slice::from_ref(&row), &FeedColumns::default());
        let mut reader = csv::ReaderBuilder::new()
            .trim(csv::Trim::None)
            .from_reader(text.as_bytes());
        let records: Vec<csv::StringRecord> =
            reader.records().collect::<Result<_, _>>().map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(records.len(), 1);
        let back: Vec<&str> = records[0].iter().collect();
        prop_assert_eq!(back, fields.iter().map(String::as_str).collect::<Vec<_>>());
    }

    /// Every id a fetch produces is a single, comma-free hledger tag value.
    #[test]
    fn prefixed_ids_are_tag_safe(id in any::<String>()) {
        let prefixed = ledgeline_core::feeds::normalize::prefixed_id("abcdef0123", &id);
        prop_assert!(prefixed.starts_with("abcdef0123-"));
        prop_assert!(!prefixed.contains(','));
        prop_assert!(!prefixed.contains(char::is_whitespace) || !prefixed.contains(['\n', '\r', '\t']));
        prop_assert_eq!(prefixed.trim(), prefixed.as_str());
    }
}

#[test]
fn the_merchant_code_and_extra_reach_the_row() {
    let demo = batch("demo-v2.json");
    let safeway = account(&demo, "ACT-demo-checking")
        .transactions
        .iter()
        .find(|txn| txn.id == "TRN-1003")
        .expect("Safeway is in the demo");
    assert_eq!(safeway.mcc.as_deref(), Some("5411"));
    assert_eq!(
        safeway.extra.as_deref(),
        Some(r#"{"card":"Visa","category":"Groceries"}"#)
    );
    let payroll = account(&demo, "ACT-demo-checking")
        .transactions
        .iter()
        .find(|txn| txn.id == "TRN-1002")
        .expect("payroll is in the demo");
    assert_eq!(
        (payroll.mcc.as_deref(), payroll.extra.as_deref()),
        (None, None)
    );
}

#[test]
fn a_numeric_or_odd_merchant_code_is_read_or_left_out() {
    let body =
        br#"{"accounts":[{"id":"A","name":"A","currency":"USD","balance":"0","transactions":[
        {"id":"1","posted":1,"amount":"-1","description":"x","mcc":742},
        {"id":"2","posted":1,"amount":"-1","description":"x","mcc":"not a code"},
        {"id":"3","posted":1,"amount":"-1","description":"x","extra":{}}]}]}"#;
    let parsed = ledgeline_core::feeds::simplefin::parse_accounts(body).expect("parses");
    let txns = &parsed.accounts[0].transactions;
    assert_eq!(txns[0].mcc.as_deref(), Some("0742"));
    assert_eq!(txns[1].mcc, None);
    assert_eq!(txns[2].extra, None, "an empty extra is no extra");
}

#[test]
fn the_batch_lists_field_names_without_values() {
    let demo = batch("demo-v2.json");
    for name in [
        "account.balance",
        "account.extra.account-open-date",
        "transaction.mcc",
        "transaction.extra.card",
        "transaction.extra.category",
    ] {
        assert!(
            demo.field_names.contains(name),
            "{name}: {:?}",
            demo.field_names
        );
    }
    assert!(
        !demo
            .field_names
            .iter()
            .any(|name| name.contains("Groceries"))
    );
}
