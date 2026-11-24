use super::*;
use crate::settings::Settings;
use ledgeline_core::vault::MemVault;

/// A hand-written catalog: comments in the frontmatter, prose and a second
/// table around the catalog's, a hand-widened column, CRLF-free.
const HAND_WRITTEN: &str = "---
title: Catalog
type: catalog
owner: Pat   # who keeps it
---

Our rates for 2026.

| Code   | Description                   | Unit | Rate    | *Account*                                              |
|:-------|:------------------------------|:-----|--------:|:-------------------------------------------------------|
| ARCH   | Senior architect hours        | h    | $800    | revenue:consulting                                     |
| JR     | Junior engineer               | h    | $200    | revenue:consulting:delivery                            |
| SAAS-Y | Platform subscription, 1 year | ea   | $12,000 | liabilities:deferred-revenue → revenue:saas, 12 months |

## Old rates

| Year | Rate |
|------|------|
| 2025 | $700 |
";

fn path() -> VaultPath {
    VaultPath::new("invoices/catalog.md").expect("path")
}

fn open(files: &[(&str, &str)]) -> (MemVault, DocumentIndex) {
    let vault = files
        .iter()
        .fold(MemVault::new(), |v, (p, t)| v.with_file(p, t));
    let index = DocumentIndex::open(&vault, Settings::default(), Money::default());
    (vault, index)
}

fn form() -> GridForm {
    form_of(path(), HAND_WRITTEN, &Words::default(), &Money::default())
        .table
        .expect("table")
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_string()).collect()
}

#[test]
fn the_form_is_the_tables_headers_roles_and_raw_cells() {
    let form = form_of(path(), HAND_WRITTEN, &Words::default(), &Money::default());
    assert_eq!(form.title, "Catalog");
    assert_eq!(form.hash, content_hash(HAND_WRITTEN.as_bytes()));
    assert!(form.problems.is_empty(), "{:?}", form.problems);
    let table = form.table.expect("table");
    let roles: Vec<&str> = table.columns.iter().map(|c| c.role.as_str()).collect();
    assert_eq!(roles, ["item", "description", "unit", "rate", "books"]);
    assert_eq!(table.columns[4].header, "*Account*");
    assert_eq!(table.rows.len(), 3);
    assert_eq!(
        table.rows[2][4],
        "liabilities:deferred-revenue → revenue:saas, 12 months"
    );
}

#[test]
fn an_unchanged_grid_is_the_file_byte_for_byte() {
    let text = apply(HAND_WRITTEN, &form(), &Words::default(), &Money::default()).expect("applies");
    assert_eq!(text, HAND_WRITTEN);
    // Blank rows the editor added and never filled are dropped, not written.
    let mut grid = form();
    grid.rows.push(row(&["", " ", "", "", ""]));
    let text = apply(HAND_WRITTEN, &grid, &Words::default(), &Money::default()).expect("applies");
    assert_eq!(text, HAND_WRITTEN);
}

#[test]
fn an_edit_changes_only_the_tables_bytes() {
    let mut grid = form();
    // Edit, add, reorder and delete.
    grid.rows[0][3] = "$850".into();
    grid.rows.push(row(&[
        "WKSHP",
        "Discovery | workshop",
        "ea",
        "$2,000",
        "revenue:consulting",
    ]));
    grid.rows.swap(0, 1);
    grid.rows.remove(2); // SAAS-Y
    let text = apply(HAND_WRITTEN, &grid, &Words::default(), &Money::default()).expect("applies");
    let before = HAND_WRITTEN.find("| Code").expect("table");
    let after_old = HAND_WRITTEN.find("\n\n## Old rates").expect("after");
    assert_eq!(
        text[..before],
        HAND_WRITTEN[..before],
        "frontmatter and prose"
    );
    assert!(text.ends_with(&HAND_WRITTEN[after_old..]), "the rest");
    let table = &text[before..text.find("\n\n## Old rates").expect("after")];
    // Aligned, the alignments kept, the hand-widened columns kept.
    assert_eq!(
        table,
        "| Code   | Description                   | Unit | Rate    | *Account*                                              |
|:-------|:------------------------------|:-----|--------:|:-------------------------------------------------------|
| JR     | Junior engineer               | h    | $200    | revenue:consulting:delivery                            |
| ARCH   | Senior architect hours        | h    | $850    | revenue:consulting                                     |
| WKSHP  | Discovery \\| workshop         | ea   | $2,000  | revenue:consulting                                     |"
    );
    let (catalog, problems) =
        catalog::Catalog::parse(path(), &text, &Words::default(), &Money::default());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        catalog.item("wkshp").map(|i| i.description.as_str()),
        Some("Discovery \\| workshop")
    );
}

#[test]
fn validation_names_every_cell_that_doesnt_read() {
    let mut grid = form();
    grid.rows
        .push(row(&["arch", "Again", "h", "$1", "revenue:x"]));
    grid.rows.push(row(&["", "No code", "", "", ""]));
    grid.rows.push(row(&["TEN", "Rate", "h", "ten", ""]));
    grid.rows.push(row(&["A:B", "", "", "", "revenue  bad"]));
    grid.rows.push(row(&[
        "BAD DEF",
        "Deferred",
        "",
        "$1",
        "liabilities:x → , 0 months",
    ]));
    let Err(CatalogError::Invalid(problems)) =
        apply(HAND_WRITTEN, &grid, &Words::default(), &Money::default())
    else {
        panic!("refused");
    };
    let at: Vec<(usize, Option<usize>)> = problems.iter().map(|p| (p.row, p.column)).collect();
    assert_eq!(
        at,
        [
            (3, Some(0)),
            (4, Some(0)),
            (5, Some(3)),
            (6, Some(0)),
            (6, Some(1)),
            (6, Some(4)),
            (7, Some(0)),
            (7, Some(4)),
        ],
        "{problems:?}"
    );
    assert!(
        problems[0].message.contains("used twice"),
        "{}",
        problems[0].message
    );
    assert!(
        problems[3].message.contains("--from-catalog"),
        "{}",
        problems[3].message
    );
    let message = CatalogError::Invalid(problems).to_string();
    assert!(
        message.starts_with("row 4: the code arch is used twice"),
        "{message}"
    );
}

#[test]
fn a_header_the_catalog_cant_have_is_refused() {
    let mut grid = form();
    grid.columns[0].header = "Name".into();
    assert!(matches!(
        apply(HAND_WRITTEN, &grid, &Words::default(), &Money::default()),
        Err(CatalogError::Table(_))
    ));
    // A note without a catalog table can't be edited here.
    assert!(matches!(
        apply(
            "---\ntype: catalog\n---\n\nNothing yet.\n",
            &form(),
            &Words::default(),
            &Money::default()
        ),
        Err(CatalogError::Table(_))
    ));
}

#[test]
fn a_new_header_is_written_fresh() {
    let mut grid = form();
    grid.columns.remove(2);
    for r in &mut grid.rows {
        r.remove(2);
    }
    let text = apply(HAND_WRITTEN, &grid, &Words::default(), &Money::default()).expect("applies");
    assert!(
        text.contains("| Code   | Description                   | Rate    | *Account*"),
        "{text}"
    );
    assert!(
        text.contains("|:-------|:------------------------------|--------:|:----"),
        "{text}"
    );
}

#[test]
fn save_writes_over_the_revision_read_and_refuses_an_outside_edit() {
    let (vault, mut index) = open(&[("invoices/catalog.md", HAND_WRITTEN)]);
    let hash = content_hash(HAND_WRITTEN.as_bytes());
    let mut grid = form();
    grid.rows[0][3] = "$900".into();
    let saved = save(&mut index, &vault, &path(), &hash, &grid).expect("saves");
    assert!(saved.written);
    assert_eq!(saved.items, 3);
    let text = vault.read_to_string(&path()).expect("reads");
    assert_eq!(saved.hash, content_hash(text.as_bytes()));
    assert!(text.contains("| $900    |"));
    // The old hash is refused; nothing is written.
    grid.rows[0][3] = "$950".into();
    let Err(CatalogError::Changed { current, .. }) =
        save(&mut index, &vault, &path(), &hash, &grid)
    else {
        panic!("refused");
    };
    assert_eq!(current, saved.hash);
    assert_eq!(vault.read_to_string(&path()).expect("reads"), text);
    // An unchanged grid writes nothing.
    grid.rows[0][3] = "$900".into();
    let again = save(&mut index, &vault, &path(), &saved.hash, &grid).expect("saves");
    assert!(!again.written);
    assert_eq!(again.hash, saved.hash);
}

#[test]
fn save_refuses_a_note_that_isnt_a_catalog_or_isnt_there() {
    let other = "---\ntype: note\n---\n\n| Code | Description |\n|---|---|\n| A | B |\n";
    let (vault, mut index) = open(&[("notes/x.md", other)]);
    let p = VaultPath::new("notes/x.md").expect("path");
    assert!(matches!(
        save(
            &mut index,
            &vault,
            &p,
            &content_hash(other.as_bytes()),
            &form()
        ),
        Err(CatalogError::NotACatalog(_))
    ));
    assert!(matches!(
        save(&mut index, &vault, &path(), "x", &form()),
        Err(CatalogError::NotFound(_))
    ));
}

#[test]
fn create_writes_an_empty_catalog_once() {
    let (vault, mut index) = open(&[]);
    let made = create(&mut index, &vault).expect("creates");
    assert_eq!(made.path, path());
    assert_eq!(made.items, 0);
    let text = vault.read_to_string(&path()).expect("reads");
    assert_eq!(
        text,
        "---\ntitle: Catalog\ntype: catalog\n---\n\n| Code | Description | Unit | Rate | *Account* |\n|:-----|:------------|:-----|-----:|:----------|\n"
    );
    let listed = list(&index, &vault);
    assert_eq!(listed.len(), 1);
    assert!(listed[0].problems.is_empty(), "{:?}", listed[0].problems);
    let grid = listed[0].table.clone().expect("table");
    assert!(grid.rows.is_empty());
    // The first item goes into the empty table.
    let mut grid = grid;
    grid.rows.push(row(&[
        "ARCH",
        "Architect",
        "h",
        "$800",
        "revenue:consulting",
    ]));
    let saved = save(&mut index, &vault, &path(), &made.hash, &grid).expect("saves");
    assert_eq!(saved.items, 1);
    assert!(matches!(
        create(&mut index, &vault),
        Err(CatalogError::Exists(_))
    ));
}

#[test]
fn the_fixture_catalog_saves_unchanged_and_one_edit_moves_only_its_row() {
    const FIXTURE: &str = include_str!("../../../../fixtures/accrual/invoices/catalog.md");
    let words = Words::default();
    let money = Money::default();
    let form = form_of(path(), FIXTURE, &words, &money);
    assert!(form.problems.is_empty(), "{:?}", form.problems);
    let mut grid = form.table.expect("table");
    assert_eq!(
        apply(FIXTURE, &grid, &words, &money).expect("applies"),
        FIXTURE
    );
    grid.rows[1][3] = "$250".into();
    let text = apply(FIXTURE, &grid, &words, &money).expect("applies");
    let changed: Vec<(&str, &str)> = FIXTURE
        .lines()
        .zip(text.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        [(
            "| JR     | Junior engineer               | h    | $200    | revenue:consulting:delivery                            |",
            "| JR     | Junior engineer               | h    | $250    | revenue:consulting:delivery                            |"
        )]
    );
}
