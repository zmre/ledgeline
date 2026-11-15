# Shared categorization fixtures (plan 43, WP-R)

Corpus for `crates/ledgeline-core/src/rules/share.rs` and the feed template in
`rules/generate.rs`, driven by `crates/ledgeline-core/tests/rules_share.rs` and
checked against real hledger by `just rules-check`. Everything is synthetic.

| File | What it proves |
| --- | --- |
| `wells-ofx.csv.rules` + `.csv` | An OFX-shaped manual file (`description %name`). Exercises each reason a block stays: `%trntype`/`%checknum`/`%fitid` (no sync column), an `if` table and a negated matcher (opaque), an anchored whole-record pattern (layout), and `if ONLINE TRANSFER` (it sets `code` before a kept block that also sets `code`, so moving it past that block would change which rule wins) |
| `firsttech-visa.csv.rules` + `.csv` | A bank-CSV shape with its own names (`posting_date`, `merchant`, a real `description` column) and US dates, so a `%posting_date` matcher stays as layout-dependent |
| `split/` | Goldens: each manual file after the split, its `*.categories.rules`, and the sync rules file over the borrowed column names, with a sync-shaped CSV for hledger. Regenerate with `LEDGELINE_BLESS=1 cargo test -p ledgeline-core --test rules_share` |
| `feed/` | The default feed template (no include) and a CSV it reads, including RFC 4180 quoting and a `transacted` date |
