# Re-import fixtures

The corpus for `crates/ledgeline-core/src/reimport.rs` and the id-matching half of
`crates/ledgeline-server/src/import_api.rs`: matching a **re-downloaded** statement against the
journal it was already imported into, by the row id the bank gives it.

Everything here is synthetic. No file contains a real account number, a real payee or a real
balance. The account number is a routing/account pair from the public test ranges and the amounts
are invented.

Unlike `ofx/`, these files are not read for their own sake — they are read as one **scenario**, in
order, by a test that imports the first, edits the journal, and imports the second. The point is
what the second import notices about the first one's results.

## `pending-then-cleared/`

The YTD-redownload workflow, and the bug named in `TODO.md`'s "Import improvements" section:

> a row that was **pending when imported and later settled** with a different date or amount sits
> before `.latest` and is never re-imported, so the journal silently keeps the pending version …
> the dry-run's "N rows skipped" count cannot distinguish "already imported identically" from
> "already imported differently".

| File | What it proves |
| --- | --- |
| `first.ofx` | The initial download. `FIT0001` is an authorization **hold** (`TRNTYPE HOLD`, the only pending signal OFX carries) and `FIT0002` has settled. |
| `redownload.ofx` | The same statement pulled again three days later. `FIT0001` has settled (`HOLD` → `DEBIT`) and is otherwise identical; `FIT0002` is byte-identical to before; `FIT0003` is genuinely new and dated after the first import's `.latest`. |
| `bank.csv.rules` | A rules file that names the id (`comment id:%fitid`) **and** maps `status` from `%trntype`. Both are needed: without the id nothing matches, and without a `status` assignment a status difference could not legitimately be a status-only one — see `reimport.rs`'s `maps_status`. |
| `no-id.csv.rules` | The same file with the `comment id:` line removed and nothing else changed. The opt-in control: an import through this file must be byte-for-byte the import it was before this feature existed, and must report `idMatches: null` rather than an empty object. |

### The wrong implementations it defeats

Each of the three rows exists to make a different plausible reading fail.

1. **`FIT0001` — classifying against the ordinary proposal.** It is dated `2026-01-05`, before the
   `.latest` the first import recorded (`2026-01-06`), so `hledger import --dry-run` does not
   propose it at all. An implementation that matched ids against that proposal would see nothing to
   match and report the settled hold as if it had never been re-downloaded — which is the bug
   verbatim. Only the dedup-free second run (`import_api::bare_proposal`) contains it.
2. **`FIT0002` — treating a match as authority to overwrite.** The test hand-edits its amount in the
   journal after the first import, exactly as a person correcting a figure would. It must come back
   as `conflicting`, with the diff, and the journal's own bytes must be untouched. An implementation
   that "synced" a matched row would silently discard the correction.
3. **`FIT0003` — letting an id decide what to import.** It is the only row of the three that should
   land. An implementation that imported everything the *dedup-free* proposal contained (rather than
   filtering the ordinary one) would re-import all three and duplicate two.

There is a fourth, in the pair of rules files rather than in a row: a rules file with no id must
change nothing at all. That is asserted by running both files over the same statement and comparing
the resulting journals byte for byte.

### Notes on the OFX itself

`TRNTYPE HOLD` is a real value of OFX's `TRNTYPE` enumeration and is how a statement says a charge
is authorized but not yet posted. It matters that it is `TRNTYPE` and not a column of its own:
`convert::ofx` emits exactly seven columns (`date,amount,name,memo,trntype,fitid,checknum`) and
**none of them is a status**, so `%trntype` is the only handle a rules file has on this. See
`fixtures/import/ofx/README.md` for the conversion's own corpus.

`FITID` is likewise not special-cased anywhere. It is emitted as an ordinary column and reaches the
journal only because this rules file interpolates it — which is why a `.csv` with a synthetic id
column would work identically, and why nothing in the engine knows the word "fitid".

## Adding a fixture

Same rule as the sibling corpora: say in the table above which **wrong** implementation it makes
fail, then add the assertion. A fixture whose test would still pass with the guard it covers removed
is documentation, not a test.

## Running the checks

```sh
cargo test -p ledgeline-core --lib reimport      # the classifier's own unit tests
cargo test -p ledgeline-core --test reimport     # the four-way split over parsed journals
LEDGELINE_HLEDGER_IMPORT_CHECK=1 cargo test -p ledgeline --test import_endpoints
LEDGELINE_HLEDGER_IMPORT_CHECK=1 cargo test -p ledgeline --test import_cli
```

The last two are where the scenario above actually runs, because it needs a real `hledger import` to
produce a real `.latest` — which is the thing the whole fixture is about.
