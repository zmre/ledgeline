# Shared categorization fixtures (plan 43, WP-R)

Corpus for `crates/ledgeline-core/src/rules/share.rs` and the feed template in
`rules/generate.rs`, driven by `crates/ledgeline-core/tests/rules_share.rs` and
checked against real hledger by `just rules-check`. Everything is synthetic.

| File | What it proves |
| --- | --- |
| `wells-ofx.csv.rules` + `.csv` | An OFX-shaped manual file (`description %name`). Exercises each reason a block stays: `%trntype`/`%checknum`/`%fitid` (no sync column), an `if` table and a negated matcher (opaque), an anchored whole-record pattern (layout), and `if ONLINE TRANSFER` (it sets `code` before a kept block that also sets `code`, so moving it past that block would change which rule wins) |
| `firsttech-visa.csv.rules` + `.csv` | A bank-CSV shape with its own names (`posting_date`, `merchant`, a real `description` column) and US dates, so a `%posting_date` matcher stays as layout-dependent. Its `comment`-setting blocks stay too (`SetsComment`): on a synced row they would replace the `id:` tag |
| sync rules files | End with the id guard (`if %id .` / `comment id:%id`) after the include, so a comment rule added to a categories file later still can't erase a synced row's id (the `SHELL` row in `split/firsttech-visa.sync.csv` keeps its id) |
| `split/` | Goldens: each manual file after the split, its `*.categories.rules`, and the sync rules file over the borrowed column names, with a sync-shaped CSV for hledger. Regenerate with `LEDGELINE_BLESS=1 cargo test -p ledgeline-core --test rules_share` |
| `feed/` | The default feed template (no include) and a CSV it reads, including RFC 4180 quoting and a `transacted` date |
