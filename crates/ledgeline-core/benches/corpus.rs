//! Deterministic synthetic-journal generator for the performance corpus.
//!
//! This is NOT a bench target (`autobenches = false` in `Cargo.toml`); it is a
//! shared module `#[path]`-included by `benches/engine.rs` and by the
//! `examples/` measurement binaries.
//!
//! # Determinism guarantee
//!
//! `generate(n)` is a pure function of `n`. Two calls with the same `n` — on any
//! machine, on any day, in any build profile — produce byte-identical output:
//!
//! - the only entropy source is [`Rng`], a SplitMix64 seeded from the [`SEED`]
//!   constant, advanced in a fixed order;
//! - no clock, no environment, no filesystem, no locale, no hashing of pointers
//!   (every map here is a `BTreeMap`/`Vec`, never a `HashMap`);
//! - all money is exact `i64` minor units; nothing goes through `f64`;
//! - dates come from an integer day counter through a proleptic-Gregorian
//!   conversion, not from any calendar library.
//!
//! Change anything about a shape's output and you must give it a new
//! [`Shape::version`] or [`Shape::label`], both of which are part of the on-disk
//! filename, so stale corpora can never be silently benchmarked against fresh
//! code.
//!
//! # Shapes
//!
//! [`V1`] reproduces the journal CLEANUP.md's Phase 6 table was measured on:
//! 300 declared accounts, 5 commodities, 1,488 `P` directives, a 30-year span
//! (1996-01-01 .. 2025-12-31), ~2.2 postings per transaction, ~9% of
//! transactions carrying an `@`/`@@` cost, plus periodic (`~`) budget rules,
//! virtual and balanced-virtual postings, tags, comments and balance
//! assertions. **`generate(n)` is frozen on it** — `docs/perf-baseline.md` §2
//! records the SHA-256 of its output, and every timing in §3 is measured
//! against those exact bytes.
//!
//! [`V2`] is the large-repo fixture: 150,000 transactions over 15 years, 200
//! commodities and 75 accounts, with 25 commodities sharing each brokerage
//! account. [`V2_FX`] is the same journal with a MESH price graph rather than a
//! `$`-star, which is what takes valuation off its direct-lookup fast path.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Bump on ANY change to a shape's emitted output — it keys the on-disk cache
/// file through [`Shape::version`]. [`V1`] is FROZEN at 1: `docs/perf-baseline.md`
/// §3 is measured against its exact bytes, so anything that moves them
/// invalidates every number in that table.
pub const CORPUS_VERSION: u32 = 1;

/// The one and only entropy source. Chosen arbitrarily; never derived from the
/// clock, the environment or the requested size.
const SEED: u64 = 0x01ED_A11E_C011_5EED;

/// Last day of every span (2025-12-31). Spans are anchored at the END so a
/// shorter corpus still finishes on the date the benches use as `as_of`.
const SPAN_END: (i64, u32, u32) = (2025, 12, 31);

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

/// Everything a corpus VARIANT can vary. One `Shape` in, one byte-identical
/// journal out.
///
/// The generator used to be a wall of consts, which made it impossible to ask
/// for the large-repo fixture (10k transactions a year for 15 years, ~200
/// commodities, 75 accounts) without editing it. Every one of those consts is
/// now a field, and [`V1`] holds their old values so `generate(n)` — the whole
/// of `docs/perf-baseline.md` — is untouched.
///
/// **Adding a field is safe. Changing [`V1`]'s values is not**: it silently
/// re-shapes the corpus the recorded SHA-256s and the whole baseline table were
/// measured on. Add a new `Shape` instead, with its own `version`/`label`.
#[derive(Debug, Clone)]
pub struct Shape {
    /// Cache-file version. Frozen at 1 for [`V1`].
    pub version: u32,
    /// Filename discriminator after the version, so two shapes of the same
    /// version and size cannot collide. Empty for [`V1`].
    pub label: &'static str,
    pub txns: usize,
    /// Exact number of `account` directives emitted.
    pub accounts: usize,
    /// Number of tradeable security commodities (`$`/`EUR` are on top).
    pub symbols: usize,
    /// Span length in years, ending at [`SPAN_END`].
    pub years: i64,
    /// Months of `P` directives, from January of the span's first year.
    pub price_months: u32,
    /// How many symbols share one brokerage position account. `1` gives the v1
    /// account-per-symbol layout; a larger number is what forces many
    /// commodities into one account's `MixedAmount`.
    pub symbols_per_position: usize,
    /// Cross-rate currencies plus non-`$`-priced symbols, turning the price
    /// graph from a star into a mesh. See [`write_prices`].
    pub cross_rates: bool,
    // Account-pool sizes. Each slices the head of the matching name pool.
    pub banks: usize,
    pub brokers: usize,
    pub card_issuers: usize,
    pub loans: usize,
    pub employers: usize,
    pub receivables: usize,
    pub other_income: usize,
    pub expense_categories: usize,
}

/// The original corpus. **FROZEN** — `docs/perf-baseline.md` §2 records the
/// SHA-256 of its output at 5k, 50k and 200k.
pub const V1: Shape = Shape {
    version: CORPUS_VERSION,
    label: "",
    txns: 0,
    accounts: 300,
    symbols: 3,
    years: 30,
    price_months: 372,
    symbols_per_position: 1,
    cross_rates: false,
    banks: 8,
    brokers: 3,
    card_issuers: 5,
    loans: 4,
    employers: 4,
    receivables: 6,
    other_income: 5,
    expense_categories: 20,
};

/// The large-repo fixture: 10k transactions a year for 15 years, ~200
/// commodities and 75 accounts.
///
/// The commodity and account counts are in deliberate tension. One position
/// account per (broker × symbol) — v1's model — would mint 400 accounts on its
/// own, five times the whole budget, so 25 symbols share each account instead.
/// That is the point of the fixture: it is the only way to get a
/// `BTreeMap<Commodity, Dec>` per account that is 25 entries deep rather than 1,
/// which is what every `MixedAmount` fold, every `value_at` and every
/// `roll_up` actually walks.
pub const V2: Shape = Shape {
    version: 2,
    label: "",
    txns: 150_000,
    accounts: 75,
    symbols: 200,
    years: 15,
    price_months: 180,
    symbols_per_position: 25,
    cross_rates: false,
    banks: 2,
    brokers: 2,
    card_issuers: 2,
    loans: 1,
    employers: 2,
    receivables: 1,
    other_income: 2,
    expense_categories: 6,
};

/// [`V2`] with a MESH price graph instead of a `$`-star.
///
/// A star keeps `reports::prices::value_at` on its direct-lookup fast path and
/// `choose_base` on its "fewer than 2 candidates" short-circuit, so a
/// star-only corpus would measure neither. Three things change here, and each
/// one disarms a specific guard:
///
/// 1. **GBP and CHF are priced in EUR and GBP**, not in `$`, so reaching `$`
///    from them is a 2- and 3-hop chain through [`PriceGraph`].
/// 2. **Roughly two sevenths of the symbols are quoted in EUR or GBP**, so
///    valuing them in `$` misses `lookup_in` and builds the graph.
/// 3. **The last symbol is never priced at all.** `choose_base` stops walking
///    candidates the moment one prices everything; one unpriceable holding means
///    no candidate ever does, so every one of them is scanned.
pub const V2_FX: Shape = Shape {
    label: "fx",
    cross_rates: true,
    ..V2
};

impl Shape {
    /// This shape at a different transaction count.
    #[must_use]
    pub fn with_txns(&self, txns: usize) -> Shape {
        Shape {
            txns,
            ..self.clone()
        }
    }

    /// First day of the span: January 1st, `years` before [`SPAN_END`]'s year.
    fn span_start(&self) -> (i64, u32, u32) {
        (SPAN_END.0 - self.years + 1, 1, 1)
    }

    /// First month of the price series — January of the span's first year.
    fn price_first_month(&self) -> (i64, u32) {
        (self.span_start().0, 1)
    }
}

/// The single account whose running `$` balance is tracked exactly so periodic
/// balance assertions can be emitted (and must therefore verify).
const ASSERTED_ACCOUNT: &str = "assets:bank:chase:checking";
/// Emit a balance-assertion transaction every this many transactions.
const ASSERT_EVERY: usize = 1000;

// ---------------------------------------------------------------------------
// Deterministic PRNG
// ---------------------------------------------------------------------------

/// SplitMix64. Small, fast, and — critically — fully specified, so its output
/// does not depend on the standard library version the way `DefaultHasher` or
/// `rand`'s `ThreadRng` would.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish in `[0, n)`. The modulo bias is irrelevant for corpus shape
    /// and keeps the generator trivially reproducible.
    fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform-ish in `[lo, hi]` (inclusive).
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(hi >= lo);
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i64
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

// ---------------------------------------------------------------------------
// Proleptic-Gregorian date math (Howard Hinnant's civil_from_days /
// days_from_civil). Integer-only, so leap years and month lengths are exact and
// `hledger`'s calendar-date validation is satisfied by construction.
// ---------------------------------------------------------------------------

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = u64::from(if m > 2 { m - 3 } else { m + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + u64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

fn iso(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Last day of month `m` in year `y`, as a day number.
fn month_end(y: i64, m: u32) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) - 1
}

// ---------------------------------------------------------------------------
// Money formatting (exact integer minor units; never `f64`)
// ---------------------------------------------------------------------------

/// `1234567` -> `"1,234,567"`. Grouping matches the `commodity $1,000.00`
/// declaration so `hledger` and the engine agree on the display style.
fn group3(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// US dollars from signed cents: `-540000` -> `"$-5,400.00"`.
fn usd(cents: i64) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let whole = group3(&(abs / 100).to_string());
    let frac = abs % 100;
    if neg {
        format!("$-{whole}.{frac:02}")
    } else {
        format!("${whole}.{frac:02}")
    }
}

/// US dollars at 4 decimal places, for FX rates: `12345` -> `"$1.2345"`.
fn usd4(ten_thousandths: i64) -> String {
    let neg = ten_thousandths < 0;
    let abs = ten_thousandths.unsigned_abs();
    let whole = group3(&(abs / 10_000).to_string());
    let frac = abs % 10_000;
    let sign = if neg { "-" } else { "" };
    format!("${sign}{whole}.{frac:04}")
}

/// EUR in the comma-decimal, symbol-right style declared by
/// `commodity 1.000,00 EUR`: `123456` -> `"1.234,56 EUR"`.
fn eur(cents: i64) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let whole = (abs / 100).to_string();
    // Same grouping routine, then swap the mark.
    let whole = group3(&whole).replace(',', ".");
    let frac = abs % 100;
    let sign = if neg { "-" } else { "" };
    format!("{sign}{whole},{frac:02} EUR")
}

/// A two-decimal amount in a symbol-right commodity: `12345`, `"GBP"` ->
/// `"123.45 GBP"`. The plain style — no digit grouping, point decimal — used for
/// every cross-rate quote.
fn plain(cents: i64, code: &str) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let sign = if neg { "-" } else { "" };
    format!("{sign}{}.{:02} {code}", abs / 100, abs % 100)
}

// ---------------------------------------------------------------------------
// Name pools. Fixed, ordered, never hashed.
// ---------------------------------------------------------------------------

const BANKS: [&str; 8] = [
    "chase", "wells", "ally", "schwab", "citi", "hsbc", "usaa", "sofi",
];
const BROKERS: [&str; 3] = ["fidelity", "vanguard", "etrade"];

/// The tickers a small corpus uses, in order. Beyond these,
/// [`symbol_names`] mints codes.
const SEED_SYMBOLS: [&str; 3] = ["AAPL", "VTI", "GLD"];

/// `count` distinct commodity symbols: [`SEED_SYMBOLS`] first, then minted
/// `Q`-prefixed four-letter codes.
///
/// `Q` because none of the seed tickers starts with one, so a minted code can
/// never collide with a real name however many are asked for. Letters only —
/// hledger needs a commodity symbol containing a digit to be double-quoted, and
/// a quoted symbol would be measuring the parser's quoting path rather than the
/// engine's commodity handling.
fn symbol_names(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| match SEED_SYMBOLS.get(i) {
            Some(name) => (*name).to_string(),
            None => {
                let n = i - SEED_SYMBOLS.len();
                let letter = |place: usize| (b'A' + ((n / place) % 26) as u8) as char;
                format!("Q{}{}{}", letter(676), letter(26), letter(1))
            }
        })
        .collect()
}

/// Opening price in cents for symbol `i`, before the monthly random walk.
///
/// The first three are [`V1`]'s literals; beyond them a coprime stride spreads
/// the openings across a realistic $10–$610 band without touching the RNG (which
/// would shift every later draw and break v1's byte-identity).
fn opening_cents(i: usize) -> i64 {
    const SEED_CENTS: [i64; 3] = [2_000, 4_500, 28_000];
    match SEED_CENTS.get(i) {
        Some(cents) => *cents,
        None => 1_000 + ((i as i64 * 977) % 60_000),
    }
}
const CARD_ISSUERS: [&str; 5] = ["visa", "amex", "mastercard", "discover", "store"];
const LOANS: [&str; 4] = ["mortgage", "auto", "student", "personal"];
const EMPLOYERS: [&str; 4] = ["acme", "globex", "initech", "umbrella"];

const EXPENSE_CATEGORIES: [&str; 20] = [
    "food",
    "housing",
    "utilities",
    "transport",
    "health",
    "travel",
    "shopping",
    "entertainment",
    "education",
    "insurance",
    "taxes",
    "personal",
    "business",
    "charity",
    "fees",
    "subscriptions",
    "pets",
    "home",
    "gifts",
    "misc",
];

const LEAF_WORDS: [&str; 40] = [
    "groceries",
    "restaurants",
    "coffee",
    "rent",
    "electric",
    "water",
    "gas",
    "internet",
    "phone",
    "fuel",
    "parking",
    "transit",
    "rideshare",
    "flights",
    "lodging",
    "activities",
    "clothing",
    "shoes",
    "books",
    "music",
    "streaming",
    "hardware",
    "software",
    "supplies",
    "postage",
    "tuition",
    "courses",
    "dental",
    "vision",
    "pharmacy",
    "premiums",
    "deductible",
    "federal",
    "state",
    "local",
    "bank",
    "interest",
    "advisory",
    "donations",
    "sundry",
];

const PAYEES: [&str; 48] = [
    "Trader Joe's",
    "Costco",
    "Safeway",
    "Whole Foods",
    "City Power & Light",
    "Metro Water",
    "Northline Gas",
    "Fibernet",
    "Cell Co",
    "Shell",
    "Chevron",
    "Parkwise",
    "Transit Authority",
    "Rideshare Inc",
    "Skyward Air",
    "Harbor Inn",
    "Museum Pass",
    "Threadbare",
    "Sole Mates",
    "Paperback Row",
    "Vinyl Vault",
    "Streamly",
    "Bolt Hardware",
    "Codeworks",
    "Office Depot",
    "Post Office",
    "State University",
    "Coursebank",
    "Bright Dental",
    "Clearview Optical",
    "Corner Pharmacy",
    "Shield Insurance",
    "Treasury",
    "Franchise Board",
    "County Clerk",
    "Chase",
    "Wells Fargo",
    "Fidelity",
    "Vanguard",
    "E*Trade",
    "Wise",
    "Acme Corp",
    "Globex",
    "Initech",
    "Umbrella Co",
    "Oakview Properties",
    "Red Cross",
    "Local Shelter",
];

const NOTES: [&str; 24] = [
    "weekly run",
    "monthly bill",
    "autopay",
    "reimbursed",
    "split with roommate",
    "annual renewal",
    "one-off",
    "quarterly",
    "gift",
    "travel day",
    "work trip",
    "family visit",
    "top up",
    "adjustment",
    "correction",
    "rebate",
    "deposit",
    "withdrawal",
    "transfer",
    "purchase",
    "settlement",
    "statement",
    "fee waived",
    "late fee",
];

const TAG_KEYS: [&str; 6] = ["receipt", "trip", "project", "payee", "category", "review"];
const TAG_VALUES: [&str; 8] = [
    "yes", "no", "q1", "q2", "alpha", "beta", "personal", "business",
];

// ---------------------------------------------------------------------------
// Chart of accounts
// ---------------------------------------------------------------------------

/// One declared account: its name and its `type:` tag, if any.
struct Decl {
    name: String,
    ty: Option<&'static str>,
}

/// Every account pool the transaction generator draws from, plus the ordered
/// declaration list that becomes the journal preamble.
struct Chart {
    decls: Vec<Decl>,
    /// `$` cash accounts safe to fund/spend from.
    cash: Vec<String>,
    /// Credit-card liabilities.
    credit: Vec<String>,
    /// `(position account, symbol, that broker's cash sweep)`. One entry per
    /// tradeable pair, so several entries SHARE an account name whenever
    /// [`Shape::symbols_per_position`] is greater than one.
    broker_positions: Vec<(String, String, String)>,
    /// The EUR-denominated account.
    eur: String,
    /// Expense leaves — the accounts most postings land on.
    expense_leaves: Vec<String>,
    /// Income accounts, salaries first (`shape.employers` of them).
    income: Vec<String>,
    /// `equity:{opening,transfers,retained}`, in that order.
    equity: Vec<String>,
    /// Every commodity symbol, in declaration order.
    symbols: Vec<String>,
}

/// The two accounts [`emit_salary`] withholds to. They are not drawn from a
/// pool — the payroll transaction names them directly — so the chart has to
/// guarantee they are declared or `hledger check -s` rejects the journal.
const SALARY_TAXES: [&str; 2] = ["expenses:taxes:federal", "expenses:taxes:state"];

/// The expense leaves for a given budget: category-major round-robin, so leaves
/// spread evenly and a category never repeats a word (the `ci * 11 + j` stride
/// is coprime with [`LEAF_WORDS`]'s length).
///
/// Pure, and called twice — once to find out whether it already produces
/// [`SALARY_TAXES`] (v1 does, at 204 leaves over 20 categories) and once for
/// real. That is what lets the reservation below cost v1 nothing.
fn leaf_names(budget: usize, categories: &[&str]) -> Vec<String> {
    let mut per_category = vec![0usize; categories.len()];
    (0..budget)
        .map(|k| {
            let ci = k % categories.len();
            let j = per_category[ci];
            per_category[ci] += 1;
            let word = LEAF_WORDS[(ci * 11 + j) % LEAF_WORDS.len()];
            format!("expenses:{}:{word}", categories[ci])
        })
        .collect()
}

fn build_chart(shape: &Shape) -> Chart {
    let symbols = symbol_names(shape.symbols);
    let mut decls: Vec<Decl> = Vec::with_capacity(shape.accounts);
    // A macro rather than a closure so `decls.len()` stays readable further down
    // (a `FnMut` closure would hold the mutable borrow for the whole function).
    macro_rules! push {
        ($name:expr, $ty:expr) => {
            decls.push(Decl {
                name: $name,
                ty: $ty,
            })
        };
    }

    // --- roots (typed) ---
    push!("assets".into(), Some("A"));
    push!("liabilities".into(), Some("L"));
    push!("equity".into(), Some("E"));
    push!("income".into(), Some("R"));
    push!("expenses".into(), Some("X"));

    // --- cash (type: C) ---
    let mut cash = Vec::new();
    for bank in &BANKS[..shape.banks] {
        for kind in ["checking", "savings"] {
            let name = format!("assets:bank:{bank}:{kind}");
            push!(name.clone(), Some("C"));
            cash.push(name);
        }
    }
    for spot in ["wallet", "safe"] {
        let name = format!("assets:cash:{spot}");
        push!(name.clone(), Some("C"));
        cash.push(name);
    }
    let eur = "assets:bank:wise:eur".to_string();
    push!(eur.clone(), Some("C"));
    push!("assets:bank:wise:usd".to_string(), Some("C"));
    cash.push("assets:bank:wise:usd".to_string());

    let mut broker_cash = Vec::new();
    for broker in &BROKERS[..shape.brokers] {
        let name = format!("assets:broker:{broker}:cash");
        push!(name.clone(), Some("C"));
        broker_cash.push(name);
    }

    // --- non-cash assets ---
    // Every broker holds every SLEEVE, and a sleeve holds
    // `symbols_per_position` commodities. At one symbol per sleeve this is v1's
    // account-per-symbol layout, named after the symbol; above that the sleeve
    // gets a number, because a name cannot spell 25 tickers.
    let sleeves: Vec<&[String]> = symbols.chunks(shape.symbols_per_position).collect();
    let mut broker_positions = Vec::new();
    for (bi, broker) in BROKERS[..shape.brokers].iter().enumerate() {
        for (si, sleeve) in sleeves.iter().enumerate() {
            let name = match sleeve {
                [only] => format!("assets:broker:{broker}:{}", only.to_lowercase()),
                _ => format!("assets:broker:{broker}:sleeve{:02}", si + 1),
            };
            push!(name.clone(), Some("A"));
            for symbol in *sleeve {
                broker_positions.push((name.clone(), symbol.clone(), broker_cash[bi].clone()));
            }
        }
    }
    for n in 1..=shape.receivables {
        push!(format!("assets:receivable:client{n:02}"), Some("A"));
    }
    for thing in ["house", "car", "art"] {
        push!(format!("assets:property:{thing}"), Some("A"));
    }

    // --- liabilities ---
    let mut credit = Vec::new();
    for issuer in &CARD_ISSUERS[..shape.card_issuers] {
        let name = format!("liabilities:cc:{issuer}");
        push!(name.clone(), Some("L"));
        credit.push(name);
    }
    // Declared but never posted to, which is realistic: 248 of the 300 declared
    // accounts see traffic.
    for loan in &LOANS[..shape.loans] {
        push!(format!("liabilities:loan:{loan}"), Some("L"));
    }
    for tax in ["federal", "state"] {
        push!(format!("liabilities:tax:{tax}"), Some("L"));
    }

    // --- equity ---
    let mut equity = Vec::new();
    for kind in ["opening", "transfers", "retained"] {
        let name = format!("equity:{kind}");
        push!(name.clone(), Some("E"));
        equity.push(name);
    }

    // --- income ---
    let mut income = Vec::new();
    for employer in &EMPLOYERS[..shape.employers] {
        let name = format!("income:salary:{employer}");
        push!(name.clone(), Some("R"));
        income.push(name);
    }
    for broker in &BROKERS[..shape.brokers] {
        let name = format!("income:dividends:{broker}");
        push!(name.clone(), Some("R"));
        income.push(name);
    }
    for bank in &BANKS[..shape.banks.min(4)] {
        let name = format!("income:interest:{bank}");
        push!(name.clone(), Some("R"));
        income.push(name);
    }
    const OTHER_INCOME: [&str; 5] = ["consulting", "rental", "refunds", "gifts", "capgains"];
    for other in &OTHER_INCOME[..shape.other_income] {
        let name = format!("income:{other}");
        push!(name.clone(), Some("R"));
        income.push(name);
    }

    // --- expenses: category parents, then exactly enough leaves to land on
    //     `shape.accounts`. Computing the leaf budget keeps the total pinned
    //     even if a pool above changes size.
    let categories = &EXPENSE_CATEGORIES[..shape.expense_categories];
    for category in categories {
        push!(format!("expenses:{category}"), None);
    }
    // `saturating_sub`, not `-`: a shape whose fixed pools already exceed its
    // account budget must fail with the assert below, which names the numbers,
    // rather than by wrapping to `usize::MAX` and aborting inside
    // `Vec::with_capacity`. That wrap is exactly what asking for 75 accounts
    // used to do.
    assert!(
        decls.len() <= shape.accounts,
        "shape's fixed account pools need {} declarations but the budget is {} — \
         shrink a pool, do not raise the budget silently",
        decls.len(),
        shape.accounts
    );
    let budget = shape.accounts.saturating_sub(decls.len());

    // Reserve room for any salary-tax account the round-robin would not produce
    // on its own. v1's 204 leaves over 20 categories already contain both, so
    // nothing is reserved there and its byte stream is untouched; a small chart
    // reserves two of its leaves instead.
    let unreserved = leaf_names(budget, categories);
    let reserved: Vec<&str> = SALARY_TAXES
        .iter()
        .filter(|required| !unreserved.iter().any(|leaf| leaf == *required))
        .copied()
        .collect();
    let expense_leaves = leaf_names(budget - reserved.len(), categories);
    assert!(
        reserved
            .iter()
            .all(|required| !expense_leaves.iter().any(|leaf| leaf == required)),
        "reserving a salary-tax account must not make the round-robin produce it"
    );
    for name in &expense_leaves {
        push!(name.clone(), None);
    }
    for name in &reserved {
        push!((*name).to_string(), None);
    }

    assert_eq!(
        decls.len(),
        shape.accounts,
        "chart of accounts must declare exactly {} accounts",
        shape.accounts
    );

    Chart {
        decls,
        cash,
        credit,
        broker_positions,
        eur,
        expense_leaves,
        income,
        equity,
        symbols,
    }
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// A posting staged before it is rendered, so the generator knows every elided
/// amount's exact value (which the balance-assertion tracker depends on).
struct Posting {
    account: String,
    /// Rendered amount, or `None` to elide it (hledger infers the balance).
    amount: Option<String>,
    comment: Option<String>,
    /// Signed `$` minor units this posting contributes to `account`. Non-`$`
    /// postings contribute 0.
    usd_cents: i64,
}

impl Posting {
    fn new(account: impl Into<String>, amount: Option<String>, usd_cents: i64) -> Self {
        Self {
            account: account.into(),
            amount,
            comment: None,
            usd_cents,
        }
    }

    fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

/// Account-name column width; amounts are then right-aligned in 12. Matches the
/// hand-authored fixture's look and puts the average transaction at ~140 bytes,
/// the density CLEANUP.md's table implies (27.7 MB / 200k txns).
const ACCOUNT_WIDTH: usize = 32;
const AMOUNT_WIDTH: usize = 12;

struct Gen {
    rng: Rng,
    out: String,
    chart: Chart,
    shape: Shape,
    /// Exact running `$` balance of [`ASSERTED_ACCOUNT`], in cents.
    asserted_cents: i64,
    postings_emitted: usize,
}

impl Gen {
    fn new(shape: &Shape, expected_bytes: usize) -> Self {
        Self {
            rng: Rng::new(SEED),
            out: String::with_capacity(expected_bytes),
            chart: build_chart(shape),
            shape: shape.clone(),
            asserted_cents: 0,
            postings_emitted: 0,
        }
    }

    fn emit_txn(
        &mut self,
        day: i64,
        status: &str,
        payee: &str,
        note: &str,
        tag: Option<(&str, &str)>,
        postings: Vec<Posting>,
    ) {
        let date = iso(day);
        match tag {
            Some((k, v)) => {
                let _ = writeln!(self.out, "{date} {status}{payee} | {note}  ; {k}: {v}");
            }
            None => {
                let _ = writeln!(self.out, "{date} {status}{payee} | {note}");
            }
        }
        for posting in &postings {
            if posting.account == ASSERTED_ACCOUNT {
                self.asserted_cents += posting.usd_cents;
            }
            self.postings_emitted += 1;
            match &posting.amount {
                Some(amount) => {
                    let width = ACCOUNT_WIDTH.max(posting.account.len() + 2);
                    let _ = write!(
                        self.out,
                        "    {:<width$}{:>AMOUNT_WIDTH$}",
                        posting.account, amount
                    );
                }
                None => {
                    let _ = write!(self.out, "    {}", posting.account);
                }
            }
            match &posting.comment {
                Some(comment) => {
                    let _ = writeln!(self.out, "  ; {comment}");
                }
                None => self.out.push('\n'),
            }
        }
        self.out.push('\n');
    }

    fn status(&mut self) -> &'static str {
        // 90% cleared, 7% unmarked, 3% pending — the same spread the fixture has.
        match self.rng.below(100) {
            0..=89 => "* ",
            90..=96 => "",
            _ => "! ",
        }
    }

    fn maybe_tag(&mut self) -> Option<(&'static str, &'static str)> {
        if self.rng.below(100) < 20 {
            let key = *self.rng.pick(&TAG_KEYS);
            let value = *self.rng.pick(&TAG_VALUES);
            Some((key, value))
        } else {
            None
        }
    }
}

/// Generate the [`V1`] journal with `txns` transactions.
///
/// Deterministic: same `txns` in, byte-identical journal out — and identical to
/// what this function produced before the generator was parameterized, which is
/// what keeps `docs/perf-baseline.md` §3 meaningful. See the module docs for the
/// guarantee's basis.
#[must_use]
pub fn generate(txns: usize) -> String {
    generate_shape(&V1.with_txns(txns))
}

/// Generate the journal `shape` describes.
///
/// Deterministic: same `shape` in, byte-identical journal out.
#[must_use]
pub fn generate_shape(shape: &Shape) -> String {
    assert!(shape.txns > 1, "corpus needs at least 2 transactions");
    // ~145 bytes/txn, plus a preamble that scales with the price series (a
    // 200-commodity corpus writes ~36k `P` directives, far past v1's 30 KB).
    // Over-reserving beats reallocating a 28 MB string.
    let price_lines = (shape.price_months as usize) * (shape.symbols + 4);
    let mut jg = Gen::new(shape, shape.txns * 160 + price_lines * 40 + 64 * 1024);

    write_preamble(&mut jg);
    write_prices(&mut jg);
    write_periodic_rules(&mut jg);
    write_transactions(&mut jg, shape.txns);

    jg.out
}

fn write_preamble(jg: &mut Gen) {
    // Destructured so the account list can be read while `out` is written to.
    let Gen {
        out, chart, shape, ..
    } = jg;
    out.push_str("; Ledgeline synthetic performance corpus — GENERATED, DO NOT EDIT.\n");
    out.push_str(concat!(
        "; Produced by crates/ledgeline-core/benches/corpus.rs (deterministic, fixed seed).\n",
        "; Regenerate with: cargo run --release -p ledgeline-core --example gen_journal -- <N>\n\n"
    ));

    out.push_str("; ---------- commodity declarations ----------\n");
    out.push_str("commodity $1,000.00\n");
    out.push_str("commodity 1.000,00 EUR\n");
    if shape.cross_rates {
        for code in CROSS_CURRENCIES {
            let _ = writeln!(out, "commodity 1.00 {code}");
        }
    }
    for symbol in &chart.symbols {
        let _ = writeln!(out, "commodity 1.00 {symbol}");
    }
    out.push_str("\nD $1,000.00\n\n");

    out.push_str("; ---------- account declarations ----------\n");
    for decl in &chart.decls {
        match decl.ty {
            Some(ty) => {
                let width = 38.max(decl.name.len() + 2);
                let _ = writeln!(out, "account {:<width$}; type: {ty}", decl.name);
            }
            None => {
                let _ = writeln!(out, "account {}", decl.name);
            }
        }
    }
    out.push('\n');
}

/// The currencies that turn the price graph into a mesh, each quoted in the one
/// before it (GBP in EUR, CHF in GBP) so reaching `$` takes 2 and 3 hops.
const CROSS_CURRENCIES: [&str; 2] = ["GBP", "CHF"];

/// One `P` directive per priced commodity per month end, from January of the
/// span's first year. [`V1`] emits `4 × 372 = 1,488` of them: EUR plus its three
/// symbols.
///
/// Under [`Shape::cross_rates`] two things change. The [`CROSS_CURRENCIES`] are
/// quoted in each OTHER rather than in `$`, and a symbol whose index is
/// `0 mod 7` or `3 mod 7` is quoted in EUR or GBP rather than `$` — so valuing
/// it in `$` cannot be a direct lookup and has to walk the graph. The LAST
/// symbol is skipped entirely: an unpriceable holding is what stops
/// `choose_base` short-circuiting on the first candidate that prices everything.
fn write_prices(jg: &mut Gen) {
    let symbols = jg.chart.symbols.clone();
    let cross = jg.shape.cross_rates;
    // Under cross-rates the last symbol is deliberately left unpriced.
    let priced = symbols.len().saturating_sub(usize::from(cross));
    let lines = jg.shape.price_months as usize * (priced + 1 + if cross { 2 } else { 0 });
    let _ = writeln!(
        jg.out,
        "; ---------- market prices ({} P directives) ----------",
        group3(&lines.to_string())
    );
    // Integer random walks. EUR is quoted at 4dp, everything else at 2dp.
    let mut eur_rate: i64 = 11_500; // $1.1500
    let mut cross_rate: [i64; 2] = [8_600, 11_200]; // GBP in EUR, CHF in GBP
    let mut symbol_cents: Vec<i64> = (0..symbols.len()).map(opening_cents).collect();
    let (mut year, mut month) = jg.shape.price_first_month();
    for _ in 0..jg.shape.price_months {
        let date = iso(month_end(year, month));
        eur_rate = (eur_rate + jg.rng.range(-400, 400)).clamp(7_000, 16_000);
        let _ = writeln!(jg.out, "P {date} EUR {}", usd4(eur_rate));
        if cross {
            for (i, code) in CROSS_CURRENCIES.iter().enumerate() {
                let quoted_in = if i == 0 {
                    "EUR"
                } else {
                    CROSS_CURRENCIES[i - 1]
                };
                cross_rate[i] = (cross_rate[i] + jg.rng.range(-300, 300)).clamp(5_000, 20_000);
                let _ = writeln!(
                    jg.out,
                    "P {date} {code} {}",
                    plain(cross_rate[i], quoted_in)
                );
            }
        }
        for (i, symbol) in symbols[..priced].iter().enumerate() {
            // Drift up slightly on average — 30 years of a flat walk would make
            // every holdings bench value at roughly the cost basis.
            let step = symbol_cents[i] / 12;
            symbol_cents[i] =
                (symbol_cents[i] + jg.rng.range(-step, step + step / 8)).clamp(200, 5_000_000);
            let quote = match (cross, i % 7) {
                (true, 0) => plain(symbol_cents[i], "EUR"),
                (true, 3) => plain(symbol_cents[i], "GBP"),
                _ => usd(symbol_cents[i]),
            };
            let _ = writeln!(jg.out, "P {date} {symbol} {quote}");
        }
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    jg.out.push('\n');
}

/// Periodic (`~`) rules, so `budget_report` has goals to compute against. They
/// use unbalanced virtual postings, which is the hledger budgeting idiom and
/// needs no balancing leg.
fn write_periodic_rules(jg: &mut Gen) {
    jg.out
        .push_str("; ---------- periodic rules (budget goals) ----------\n");
    let leaves: Vec<String> = jg.chart.expense_leaves.clone();
    for (chunk, label) in [
        (0usize, "household budget"),
        (1, "lifestyle budget"),
        (2, "obligations budget"),
    ] {
        let _ = writeln!(jg.out, "~ monthly  {label}");
        // Six goals each, strided so they hit different categories.
        for k in 0..6 {
            let idx = (chunk * 6 + k * 17) % leaves.len();
            let goal = usd(jg.rng.range(2_000, 180_000) / 100 * 100);
            let account = format!("({})", leaves[idx]);
            let width = ACCOUNT_WIDTH.max(account.len() + 2);
            let _ = writeln!(jg.out, "    {account:<width$}{goal:>AMOUNT_WIDTH$}");
        }
        jg.out.push('\n');
    }
    let _ = writeln!(jg.out, "~ yearly  annual budget");
    for k in 0..3 {
        let idx = (k * 41 + 5) % leaves.len();
        let goal = usd(jg.rng.range(20_000, 900_000) / 100 * 100);
        let account = format!("({})", leaves[idx]);
        let width = ACCOUNT_WIDTH.max(account.len() + 2);
        let _ = writeln!(jg.out, "    {account:<width$}{goal:>AMOUNT_WIDTH$}");
    }
    jg.out.push('\n');
}

fn write_transactions(jg: &mut Gen, txns: usize) {
    jg.out.push_str("; ---------- transactions ----------\n");
    let span_start = jg.shape.span_start();
    let start = days_from_civil(span_start.0, span_start.1, span_start.2);
    let end = days_from_civil(SPAN_END.0, SPAN_END.1, SPAN_END.2);
    let span = end - start;

    for i in 0..txns {
        // Non-decreasing by construction, so file order == date order and the
        // balance assertions below are checked against the balance we tracked.
        let day = start + (i as i64 * span) / (txns as i64 - 1);
        if i > 0 && i % ASSERT_EVERY == 0 {
            emit_assertion(jg, day);
        } else {
            emit_one(jg, day);
        }
    }
}

/// A one-posting transaction that asserts the exact tracked balance. A `$0.00`
/// posting balances on its own, so the transaction is valid and the assertion is
/// the only thing under test.
fn emit_assertion(jg: &mut Gen, day: i64) {
    let asserted = usd(jg.asserted_cents);
    let amount = format!("$0.00 = {asserted}");
    let postings = vec![Posting::new(ASSERTED_ACCOUNT, Some(amount), 0)];
    jg.emit_txn(day, "* ", "Chase", "statement balance", None, postings);
}

fn emit_one(jg: &mut Gen, day: i64) {
    // Weights out of 1000; see the module docs for the resulting posting mix.
    match jg.rng.below(1000) {
        0..=559 => emit_simple_expense(jg, day, false),
        560..=699 => emit_simple_expense(jg, day, true),
        700..=759 => emit_salary(jg, day),
        760..=819 => emit_transfer(jg, day),
        820..=879 => emit_split_expense(jg, day),
        880..=949 => emit_stock_trade(jg, day),
        950..=969 => emit_fx(jg, day),
        970..=989 => emit_unbalanced_virtual(jg, day),
        _ => emit_balanced_virtual(jg, day),
    }
}

/// The funding leg: a cash account or a credit card, plus its signed delta.
fn funding_account(jg: &mut Gen) -> String {
    if jg.rng.below(100) < 45 {
        jg.rng.pick(&jg.chart.credit.clone()).clone()
    } else {
        jg.rng.pick(&jg.chart.cash.clone()).clone()
    }
}

fn emit_simple_expense(jg: &mut Gen, day: i64, tagged: bool) {
    let status = jg.status();
    let leaf = jg.rng.pick(&jg.chart.expense_leaves.clone()).clone();
    let funder = funding_account(jg);
    let cents = jg.rng.range(150, 45_000);
    let payee = *jg.rng.pick(&PAYEES);
    let note = *jg.rng.pick(&NOTES);
    let tag = if tagged { jg.maybe_tag() } else { None };
    let mut expense = Posting::new(leaf, Some(usd(cents)), cents);
    if tagged && jg.rng.below(100) < 40 {
        let key = *jg.rng.pick(&TAG_KEYS);
        let value = *jg.rng.pick(&TAG_VALUES);
        expense = expense.with_comment(format!("{key}: {value}"));
    }
    let postings = vec![expense, Posting::new(funder, None, -cents)];
    jg.emit_txn(day, status, payee, note, tag, postings);
}

fn emit_split_expense(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let leaves = jg.chart.expense_leaves.clone();
    let first = jg.rng.pick(&leaves).clone();
    let second = jg.rng.pick(&leaves).clone();
    let funder = funding_account(jg);
    let a = jg.rng.range(150, 25_000);
    let b = jg.rng.range(150, 25_000);
    let payee = *jg.rng.pick(&PAYEES);
    let note = *jg.rng.pick(&NOTES);
    let tag = jg.maybe_tag();
    let postings = vec![
        Posting::new(first, Some(usd(a)), a),
        Posting::new(second, Some(usd(b)), b),
        Posting::new(funder, None, -(a + b)),
    ];
    jg.emit_txn(day, status, payee, note, tag, postings);
}

fn emit_salary(jg: &mut Gen, day: i64) {
    let status = jg.status();
    // The salary accounts are the first `shape.employers` income accounts.
    let salaries = jg.chart.income[..jg.shape.employers].to_vec();
    let employer = jg.rng.pick(&salaries).clone();
    let gross = jg.rng.range(200_000, 900_000);
    let federal = gross * 21 / 100;
    let state = gross * 6 / 100;
    let net = gross - federal - state;
    let target = jg.rng.pick(&jg.chart.cash.clone()).clone();
    let payee = *jg.rng.pick(&PAYEES);
    let postings = vec![
        Posting::new(employer, Some(usd(-gross)), 0),
        Posting::new("expenses:taxes:federal", Some(usd(federal)), 0),
        Posting::new("expenses:taxes:state", Some(usd(state)), 0),
        Posting::new(target, Some(usd(net)), net),
    ];
    jg.emit_txn(
        day,
        status,
        payee,
        "payroll",
        Some(("payee", "payroll")),
        postings,
    );
}

fn emit_transfer(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let accounts = jg.chart.cash.clone();
    let from = jg.rng.pick(&accounts).clone();
    let mut to = jg.rng.pick(&accounts).clone();
    if to == from {
        to = accounts[(accounts.iter().position(|a| *a == from).unwrap_or(0) + 1) % accounts.len()]
            .clone();
    }
    let cents = jg.rng.range(5_000, 500_000);
    let payee = *jg.rng.pick(&PAYEES);
    let postings = vec![
        Posting::new(to, Some(usd(cents)), cents),
        Posting::new(from, None, -cents),
    ];
    jg.emit_txn(day, status, payee, "transfer", None, postings);
}

/// A `@`-costed lot: whole shares at a whole-cent unit price, so the implied
/// total is exact in cents and the transaction balances without rounding.
fn emit_stock_trade(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let positions = jg.chart.broker_positions.clone();
    let (account, symbol, sweep) = jg.rng.pick(&positions).clone();
    let shares = jg.rng.range(1, 120);
    let unit_cents = jg.rng.range(500, 90_000);
    let total = shares * unit_cents;
    // ~15% of trades are sells, so the average-cost pools actually churn.
    let sell = jg.rng.below(100) < 15;
    let (share_qty, cash_delta) = if sell {
        (-shares, total)
    } else {
        (shares, -total)
    };
    let amount = format!("{share_qty} {symbol} @ {}", usd(unit_cents));
    let payee = *jg.rng.pick(&PAYEES);
    let note = if sell { "sell" } else { "buy" };
    let postings = vec![
        Posting::new(account, Some(amount), 0).with_comment(format!("name: {symbol} fund")),
        Posting::new(sweep, None, cash_delta),
    ];
    jg.emit_txn(day, status, payee, note, None, postings);
}

/// An `@@` (total-cost) FX leg — the multi-commodity case.
fn emit_fx(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let eur_cents = jg.rng.range(5_000, 400_000);
    let usd_cents = eur_cents * jg.rng.range(105, 125) / 100;
    let account = jg.chart.eur.clone();
    let funder = jg.rng.pick(&jg.chart.cash.clone()).clone();
    let amount = format!("{} @@ {}", eur(eur_cents), usd(usd_cents));
    let payee = "Wise";
    let postings = vec![
        Posting::new(account, Some(amount), 0),
        Posting::new(funder, None, -usd_cents),
    ];
    jg.emit_txn(day, status, payee, "fx transfer", None, postings);
}

/// An expense plus an unbalanced virtual `(account)` posting, which is excluded
/// from the transaction balance.
fn emit_unbalanced_virtual(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let leaf = jg.rng.pick(&jg.chart.expense_leaves.clone()).clone();
    let funder = funding_account(jg);
    let cents = jg.rng.range(500, 60_000);
    let payee = *jg.rng.pick(&PAYEES);
    let note = *jg.rng.pick(&NOTES);
    let virt = format!("({})", jg.chart.equity[1]);
    let postings = vec![
        Posting::new(leaf, Some(usd(cents)), cents),
        Posting::new(funder, None, -cents),
        Posting::new(virt, Some(usd(cents)), 0),
    ];
    jg.emit_txn(day, status, payee, note, None, postings);
}

/// An expense plus a `[a]`/`[b]` balanced-virtual pair, which must balance among
/// themselves.
fn emit_balanced_virtual(jg: &mut Gen, day: i64) {
    let status = jg.status();
    let leaf = jg.rng.pick(&jg.chart.expense_leaves.clone()).clone();
    let funder = funding_account(jg);
    let cents = jg.rng.range(500, 60_000);
    let reserve = jg.rng.range(1_000, 90_000);
    let payee = *jg.rng.pick(&PAYEES);
    let note = *jg.rng.pick(&NOTES);
    let postings = vec![
        Posting::new(leaf, Some(usd(cents)), cents),
        Posting::new(funder, None, -cents),
        Posting::new("[assets:property:house]", Some(usd(reserve)), 0),
        Posting::new("[equity:retained]", Some(usd(-reserve)), 0),
    ];
    jg.emit_txn(day, status, payee, note, None, postings);
}

// ---------------------------------------------------------------------------
// On-disk cache
// ---------------------------------------------------------------------------

/// Directory the generated journals live in: `<workspace>/target/perf`.
/// Gitignored (`/target/`) — a 28 MB synthetic journal does not belong in git.
#[must_use]
pub fn corpus_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `crates/ledgeline-core`; the workspace target dir
    // is two levels up. `CARGO_TARGET_DIR` wins when set.
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir).join("perf"),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/perf")
            .components()
            .collect(),
    }
}

/// Cache path for `shape`: `synthetic-v<version><label>-<txns>.journal`.
///
/// Version AND label are both in the name, so two shapes can never share a file
/// and a stale cache can never mask a generator change.
#[must_use]
pub fn journal_path_for(shape: &Shape) -> PathBuf {
    corpus_dir().join(format!(
        "synthetic-v{}{}-{}.journal",
        shape.version, shape.label, shape.txns
    ))
}

#[must_use]
pub fn journal_path(txns: usize) -> PathBuf {
    journal_path_for(&V1.with_txns(txns))
}

/// Path to the generated journal for `shape`, writing it first if it is missing.
///
/// # Panics
/// If the corpus directory or file cannot be written.
#[must_use]
pub fn ensure_journal_for(shape: &Shape) -> PathBuf {
    let path = journal_path_for(shape);
    if !path.exists() {
        let dir = path.parent().expect("corpus path has a parent");
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|e| panic!("could not create {}: {e}", dir.display()));
        std::fs::write(&path, generate_shape(shape))
            .unwrap_or_else(|e| panic!("could not write {}: {e}", path.display()));
    }
    path
}

/// Path to the [`V1`] journal for `txns`, writing it first if it is missing.
///
/// # Panics
/// If the corpus directory or file cannot be written.
#[must_use]
pub fn ensure_journal(txns: usize) -> PathBuf {
    ensure_journal_for(&V1.with_txns(txns))
}

/// The generated journal's TEXT for `txns`, from the on-disk cache.
///
/// # Panics
/// If the corpus cannot be written or read back.
#[must_use]
pub fn load(txns: usize) -> String {
    let path = ensure_journal(txns);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// The sizes a default `cargo bench` run covers, overridable with
/// `LEDGELINE_BENCH_SIZES` (comma-separated transaction counts).
///
/// The 200k corpus is NOT in the default set: it turns a bench run from minutes
/// into the better part of an hour. Opt in with
/// `LEDGELINE_BENCH_SIZES=5000,50000,200000 cargo bench -p ledgeline-core`.
///
/// # Panics
/// If `LEDGELINE_BENCH_SIZES` is set but does not parse.
/// The shapes a `cargo bench` run covers.
///
/// Defaults to [`V1`] at [`bench_sizes`], so the run every recorded baseline was
/// measured with is what you get unless you ask for something else.
/// `LEDGELINE_BENCH_SHAPES` takes a comma-separated list of shape names —
/// `v2`, `v2fx`, or `v2@20000` to override a shape's transaction count, which is
/// how you probe a new corpus before committing to a full-size run.
///
/// # Panics
/// If `LEDGELINE_BENCH_SHAPES` names a shape that does not exist.
#[must_use]
pub fn bench_shapes() -> Vec<Shape> {
    let Ok(raw) = std::env::var("LEDGELINE_BENCH_SHAPES") else {
        return bench_sizes()
            .into_iter()
            .map(|txns| V1.with_txns(txns))
            .collect();
    };
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let (name, txns) = match name.split_once('@') {
                Some((name, txns)) => (
                    name,
                    Some(
                        txns.parse()
                            .unwrap_or_else(|e| panic!("bad transaction count in {name:?}: {e}")),
                    ),
                ),
                None => (name, None),
            };
            let shape = match name {
                "v1" => V1.with_txns(5_000),
                "v2" => V2,
                "v2fx" => V2_FX,
                other => panic!("unknown shape {other:?} — expected v1, v2 or v2fx"),
            };
            match txns {
                Some(txns) => shape.with_txns(txns),
                None => shape,
            }
        })
        .collect()
}

/// A short label for a shape, used as the criterion benchmark id suffix.
#[must_use]
pub fn shape_id(shape: &Shape) -> String {
    let size = match shape.txns {
        n if n.is_multiple_of(1000) => format!("{}k", n / 1000),
        n => n.to_string(),
    };
    match shape.version {
        1 => size,
        version => format!("v{version}{}-{size}", shape.label),
    }
}

#[must_use]
pub fn bench_sizes() -> Vec<usize> {
    match std::env::var("LEDGELINE_BENCH_SIZES") {
        Ok(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse()
                    .unwrap_or_else(|e| panic!("bad LEDGELINE_BENCH_SIZES entry {s:?}: {e}"))
            })
            .collect(),
        Err(_) => vec![5_000, 50_000],
    }
}
