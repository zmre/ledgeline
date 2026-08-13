# OFX / QFX fixtures

Corpus for `crates/ledgeline-core/src/convert/ofx.rs` — the hand-rolled OFX 1.x (SGML),
OFX 2.x (XML) and QFX reader.

Every file here is **synthetic**: hand-written to the shape real statements have, with
invented account numbers, so the corpus can be read, diffed and reasoned about in a public
repo. They are structurally realistic, not anonymised captures. Each one exists to pin a
specific trap that cost us a bug or a rejected crate — see `plans/11-enhanced-import.md`
§ Preprocessor decisions for where each trap came from.

The suite that drives them is `crates/ledgeline-core/tests/convert_ofx.rs`.

> There is no `just ofx-check` counterpart to `just rules-check`: OFX has no reference
> implementation we can run the way hledger validates a rules file. The equivalent honesty
> check is `bank-v1.ofx` vs `bank-v2.ofx` — two dialects of the same statement asserted to
> produce one identical `Tabular` — plus the arithmetic assertion in `balances.ofx`.

## Files

| File | What it proves |
| --- | --- |
| `bank-v1.ofx` | OFX 1.x SGML: **unclosed leaf tags**, closed aggregates, `STMTRS`, `LEDGERBAL`. Written with **CRLF** terminators, as real 1.x files are. `TRNAMT` of `2500.0` — amounts are *not* always two decimals and the text is kept verbatim. One transaction has a `CHECKNUM` and no `MEMO`, so the row shape holds when fields are absent |
| `bank-v2.ofx` | OFX 2.x XML, LF, indented, every tag closed — **the same statement**. A test asserts the two produce an *identical* `Tabular`, which is what makes "one tolerant body parser, never branch on the declared version" a fact rather than a claim |
| `creditcard.qfx` | `CCSTMTRS` under `CREDITCARDMSGSRSV1`, plus Quicken's `INTU.BID`/`INTU.USERID` in `SONRS` — the *only* thing that makes a QFX a QFX, and proof that `.` is a legal tag character. Card sign convention: purchases negative, payment positive, closing `LEDGERBAL` negative |
| `citi-creditline.ofx` | A credit card delivered as `BANKMSGSRSV1`/`STMTRS` with `ACCTTYPE=CREDITLINE`. **Statement type is never routed on message set** — Citi ships this shape and a `CCSTMTRS`-only reader silently returns nothing |
| `investment.ofx` | `INVSTMTRS` with `BUYSTOCK`/`INCOME`/`INVPOSLIST`. Must be **refused by name** (`ConvertError::InvestmentStatement`), never partially mis-parsed into transactions with no amounts |
| `tz-dates.ofx` | `DTPOSTED` in all four widths (8/10/12/14 digits), with fractional seconds, a **fractional zone offset** (`[+5.5:IST]`), an unsigned `[0:GMT]`, and local midnight under a zone whose *name* is wrong for the season (`[-4:EDT]` in January). Every row must keep the **FI-local calendar day**; converting to UTC moves the midnight row to the 4th |
| `entities.ofx` | The entity matrix, and the reason `ofx-rs` was disqualified. `AT &amp;amp; T` → `AT & T` with **its spaces intact** (never `ATT`); `caf&#233;`/`caf&#xE9;` → `café`; a bank's double-escaped `&amp;amp;quot;` decoded **once**, to the literal text `&quot;`; and raw unescaped `&` in `P&G` and `A&B;` passing straight through instead of erroring. `&nbsp;` is undeclared in OFX and stays literal |
| `hybrid-xml-header.ofx` | An **OFX 2.x XML header wrapping an SGML unclosed-tag body** — a combination that is legal nowhere and shipped by real banks. Proves the header only ever chooses the decoder |
| `balances.ofx` | `BALLIST` carrying an opening balance beside the closing `LEDGERBAL`, so `opening + Σ(amounts) == closing` can actually be checked. The list also holds a `PERCENT` entry (an interest rate) that must **not** be mistaken for a balance. The mismatch case is not a file: the test edits the closing amount in memory, because a fixture that is wrong on disk invites someone to "fix" it |

## What is not here

**A file with a `LEDGERBAL` and no opening balance is the common case, and every other
fixture is one.** A single closing balance cannot be verified — there is nothing to add it
to — so it is recorded in `StatementMeta` (and pre-fills the balance-assertion field in the
UI) without any arithmetic claim being made. Only `balances.ofx` has both ends.

**Malformed input has no fixture.** Truncated files, stray close tags, an empty `<MEMO>`
that would otherwise swallow its sibling `<TRNAMT>`, nesting past the depth cap and
arbitrary bytes are all exercised from byte literals in the test file and from a proptest,
because their point is that they are *not* valid OFX and committing them as `.ofx` invites
exactly the well-meaning repair that would delete the test.

## Adding a fixture

State in one line which trap it pins, add it to the table above, and assert it in
`convert_ofx.rs`. A fixture with no assertion naming it is not a fixture.

Two house rules, both learned the hard way:

- **Synthetic unless the table says otherwise.** No real account numbers, even partially —
  `ACCTID` is masked to its last four characters before it leaves `ledgeline-core`, and a
  fixture is not the place to test that. The two `real-*` files below are the one exception,
  and § *The real statements* sets out what had to happen before they could be committed.
- **A fixture that is wrong on purpose must say so in this table**, or the next reader
  corrects it and the test that depended on the wrongness quietly stops proving anything.


## The real statements

`real-creditline-v102.ofx` and `real-creditline-v102.qfx` are **one genuine credit-line
statement as an institution actually delivered it**, in both dialects. They are here because
two of their properties are things nobody writing a fixture from the spec would have thought
to include:

| What it pins | Why a synthetic fixture would have missed it |
| --- | --- |
| A **credit card** shipped as `BANKMSGSRSV1/STMTRS` with `ACCTTYPE=CREDITLINE` | The spec says credit cards are `CCSTMTRS`. Routing on the message set is the obvious implementation and it finds nothing here. |
| A `NAME` of exactly **31 characters** | One under OFX's 32-char truncation limit — the single width at which an off-by-one in the value scanner is visible. |
| The same statement in **two dialects, byte-for-byte different, semantically identical** | `the_two_dialects_of_one_statement_agree_on_every_transaction` is the strongest check on the scanner: the files close their tags differently and must still agree row for row. |

### What was changed, and what was not

Every payee was replaced with `MERCHANT NN …` **padded to the original string's exact
length**, because the length is the property under test. Amounts were scaled, the institution
name and `FID`/`INTU.BID` replaced, the `BANKID` swapped for one that also fails its ABA
checksum, and the server timestamp rounded off a real moment in someone's day.

Untouched: the unclosed-leaf-tag SGML, the 14-digit `DTPOSTED` form, `LEDGERBAL`'s position
after `BANKTRANLIST`, `ACCTTYPE=CREDITLINE`, the presence of `INTU.BID` in the QFX and its
absence from the OFX, the transaction count, and every sign.

### The guard

`no_real_fixture_still_carries_an_institution_name` in `convert_ofx.rs` fails the build if a
bank name appears in either file. It exists because the realistic failure is not this scrub —
it is someone refreshing these fixtures from a fresh download six months from now and
forgetting. It has already caught one false positive: the first scrub used `PURCHASE` as
filler text, which embeds `CHASE`.

**Every assertion on these files is structural.** Nothing depends on a payee, an amount or an
account number, so they can be re-scrubbed at any time without invalidating a test — which is
the only reason committing a real statement is defensible at all.
