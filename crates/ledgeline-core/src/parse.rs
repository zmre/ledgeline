//! The hledger journal parser.
//!
//! Parses the journal format the fixtures and real journals exercise:
//! `account`/`commodity`/`decimal-mark`/`P`/`include` directives, comment lines
//! and `comment`/`end comment` blocks, and transactions with statuses, codes,
//! descriptions, comments/tags, multi-space account/amount separation, costs
//! (`@`/`@@`), balance assertions (`=`), and single-posting amount inference.
//!
//! Periodic (`~`) rule blocks are parsed into [`model::PeriodicTransaction`]s
//! and stored on `Journal.periodic_transactions` (they feed the budget report),
//! but are still kept out of `Journal.transactions` — hledger's `/transactions`
//! (`jtxns`) likewise excludes periodic and auto-generated postings, so wire
//! parity is preserved. Auto-posting (`=`) and `comment` blocks remain skipped.
//!
//! `include` deliberately diverges from hledger in one respect: an included path
//! must resolve *inside the main journal file's own directory*. hledger happily
//! follows `include /etc/passwd`, `include ../../../etc/hosts` and symlinks out
//! of the tree, which makes a hostile journal a local file-read oracle (the
//! offending line is quoted back in the parse error, and anything that parses is
//! absorbed into the journal and then served over HTTP) and points the
//! live-reload watcher at arbitrary directories. Cycles and nesting depth are
//! bounded too — see [`admit_include`].
//!
//! Every file is read through a [`Vault`] ([`parse_journal_in`]): an `include`
//! is placed lexically inside the vault, its symlinks resolved by
//! [`Vault::canonicalize`], and read with [`Vault::read_pinned`] — so a caller
//! gets back the exact bytes parsed and their [`Revision`]. The text-only entry
//! points ([`parse_journal`], [`parse_journal_with_overrides`]) build the vault
//! from the journal's own directory; nothing here consults the process's
//! working directory.
//!
//! `Y` sets the default year for yearless dates; every transaction/`P` date is
//! normalized to ISO `YYYY-MM-DD` (accepting `-`/`/`/`.` separators) and
//! validated against the calendar, leap years included. Directives that would
//! silently change results if ignored (`apply account`, and anything else
//! unrecognized) are still rejected with a clear error rather than misparsed.
//!
//! # `alias` is read but NOT applied
//!
//! `alias`/`end aliases` are parsed into [`model::AliasDirective`]s on
//! `Journal.aliases`, in file order, with their scope resolved — and that is
//! all. Ledgeline does not rewrite account names when it reads a journal.
//!
//! This is a narrowing of the rule above, made deliberately and for two reasons.
//! The first is that until now an `alias` line failed the WHOLE journal, so a
//! user who has one could not open their books here at all — and the import
//! pipeline, which is the thing that needs these directives, cannot import into
//! a journal that will not parse. The second is that applying the `/REGEX/` form
//! means reproducing hledger's regex dialect (Haskell `regex-tdfa`, POSIX ERE,
//! case-insensitive, `\1` backreferences in the replacement) over every account
//! name in someone's books. Rust's `regex` crate is a different dialect with
//! different replacement syntax and no backtracking, so a near-miss
//! reimplementation would be exactly the silent wrong answer this parser refuses
//! elsewhere. Declining to apply is visible — the account tree shows the names
//! as written — where a subtly different regex engine would not be.
//!
//! See [`crate::aliases`] for what is done with them instead.
//!
//! # What is an error, and what is a diagnostic
//!
//! A [`ParseError`] means the journal cannot be READ. It aborts the parse, so it
//! is reserved for input with no sensible interpretation — a date that does not
//! exist, an amount that is not a number, an `include` cycle.
//!
//! An unbalanced transaction is NOT one of those. It is a diagnostic: see
//! [`check_transaction_balances`], and [`crate::assertions`] for the same
//! decision about a failed balance assertion. The journal always opens, and both
//! surface through the wire's `diagnostics` array
//! ([`crate::wire::journal_to_diagnostics`]).
//!
//! One hledger behaviour is reproduced despite being a trap: a `comment` block
//! with no `end comment` silently swallows the rest of the file. Erroring would
//! make a journal hledger loads refuse to open — see the parity test in
//! `tests/parse_fixes.rs`.

use crate::decimal::{Dec, DecError, MAX_PARSE_PLACES};
use crate::model::{
    AccountDeclaration, AccountName, AliasDirective, Amount, AmountStyle, Anchor, AutoAmount,
    AutoPosting, AutoPostingRule, BalanceAssertion, Commodity, CommoditySide, Cost, CostKind,
    DigitGroups, Journal, PeriodExpr, PeriodKind, PeriodSpec, PeriodicTransaction, Posting,
    PostingType, PriceDirective, SourcePos, Status, Tindex, Transaction,
};
use crate::vault::{LocalVault, MemVault, Revision, Vault, VaultError, VaultPath};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

/// Errors produced while parsing a journal.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ParseError {
    /// A decimal literal failed to parse or overflowed.
    #[error("decimal error: {0}")]
    Decimal(#[from] DecError),
    /// An amount token could not be split into commodity + quantity.
    #[error("malformed amount: '{0}'")]
    MalformedAmount(String),
    /// A directive line was structurally invalid.
    #[error("malformed directive: '{0}'")]
    MalformedDirective(String),
    /// A date could not be parsed/normalized (bad components, or yearless with
    /// no `Y` default-year directive in effect).
    #[error("malformed date: {0}")]
    MalformedDate(String),
    /// A directive keyword we do not (yet) support was encountered.
    #[error("unsupported directive: '{0}'")]
    UnsupportedDirective(String),
    /// An `include`d file could not be read.
    #[error("include error: {0}")]
    Include(String),
    /// More than one posting in a transaction omitted its amount.
    #[error("transaction on line {0} has more than one posting with no amount")]
    MultipleElidedPostings(u32),
    /// A stray, non-transaction indented line appeared at the top level.
    #[error("unexpected indented line (expected a transaction, directive, or blank line)")]
    UnexpectedIndent(u32),
    /// A located wrapper: an underlying error at a specific file + line, with
    /// the line's text, so diagnostics point at the exact source — crucially,
    /// naming which `include`d file the problem is in.
    #[error("{source_name}:{line}: {message}\n    {line} | {line_text}")]
    Located {
        source_name: String,
        line: u32,
        line_text: String,
        message: String,
    },
}

/// Hard cap on `include` nesting depth. hledger 1.52 imposes no depth limit at
/// all, but unbounded recursion here is a stack overflow (`SIGABRT`, not a
/// catchable panic), so a cap is required. 20 is far beyond any real journal
/// layout and, because the error aborts the whole parse immediately, it also
/// bounds a "billion laughs" include bomb to ~20 file reads.
const MAX_INCLUDE_DEPTH: usize = 20;

/// Hard cap on the total number of `include`d files in one parse. Depth alone
/// still permits a wide fan-out bomb (a binary tree of depth 19 is ~500k
/// parses); this bounds the total work regardless of shape. Generous relative
/// to real journals, which split into tens of files, not thousands.
const MAX_INCLUDE_FILES: usize = 1000;

/// Hard cap on the number of digit-group sizes kept in an [`AmountStyle`].
///
/// **Why a cap at all.** A `commodity` directive's style is cloned into every
/// amount of that commodity (see [`parse_amount`]), so the group vector is
/// retained once per *amount* while the directive that declared it is charged
/// once against the file size. `commodity $1,1,…,1.00` with 20k separators
/// across 2k postings turned a 127 KB journal into 80 MB of retained group
/// entries, and the ratio grows with both factors without bound.
///
/// **Why 39 loses nothing.** Group sizes describe the INTEGER digits, and every
/// quantity the engine can hold is an `i128` mantissa — 39 decimal digits at the
/// very widest, since `i128::MAX` is `1.7 × 10^38`. A renderer walks the sizes
/// right-to-left consuming at least one digit each (`groupDigits` in
/// `web/src/lib/domain/money.ts` stops outright on a non-positive size), so it
/// runs out of digits before it can reach a 40th entry. Real groupings are one
/// or two entries: `[3]` for thousands, `[3,2]` for the Indian lakh.
const MAX_DIGIT_GROUPS: usize = 39;

/// Canonical display style per commodity, built from `commodity` directives (or
/// first occurrence).
type Styles = HashMap<Commodity, AmountStyle>;

/// The context needed to parse an amount token: the known commodity styles, the
/// journal-wide default decimal mark (from a `decimal-mark` directive), and the
/// default commodity + style (from a `D` directive) applied to bare numbers.
/// `Copy` so it threads cheaply through the parse helpers.
#[derive(Clone, Copy)]
struct AmountCtx<'a> {
    styles: &'a Styles,
    default_mark: Option<char>,
    default_commodity: Option<&'a (Commodity, AmountStyle)>,
    /// Styles declared OUTSIDE this text — a projection scenario reads its
    /// main journal's `commodity` directives through this — consulted only
    /// after the text's own declarations and only when it sets no
    /// `decimal-mark` of its own. See [`parse_journal_with_styles`].
    inherited: &'a Styles,
}

/// Mutable accumulators shared across the top-level file and any `include`d
/// files, so transaction indices and declarations continue seamlessly.
struct Ctx<'v> {
    /// Where every `include` is read from, and confined to.
    vault: &'v dyn Vault,
    /// In-memory replacements for files, by vault path: the editor's
    /// reparse-to-validate input. A file present here is never read.
    overrides: Option<&'v HashMap<VaultPath, String>>,
    /// The main file's vault path and the key it is recorded under. The key is
    /// [`Vault::key`] for every caller but the text-only wrappers, which keep
    /// the key they always recorded (see [`resolve_source_file`]).
    main: VaultPath,
    main_key: PathBuf,
    /// Every file read from the vault during this parse, in first-read order,
    /// with exactly the bytes parsed and their [`Revision`].
    sources: Vec<ParsedSource>,
    styles: Styles,
    /// See [`AmountCtx::inherited`].
    inherited: Styles,
    default_decimal_mark: Option<char>,
    default_commodity: Option<(Commodity, AmountStyle)>,
    default_year: Option<i32>,
    commodity_styles: Vec<(Commodity, AmountStyle)>,
    commodity_tags: Vec<(Commodity, Vec<(String, String)>)>,
    accounts: Vec<AccountDeclaration>,
    aliases: Vec<AliasDirective>,
    /// Indices into [`Self::aliases`] of the aliases in force **right here**.
    ///
    /// hledger's aliases are positional and file-scoped, so this is a stack
    /// discipline rather than a set: an `include` inherits the current scope
    /// (aliases flow inward), and the scope is restored when that file returns
    /// (they never flow back out). `end aliases` clears it. Verified against
    /// hledger 1.52 in all three directions.
    alias_scope: Vec<usize>,
    prices: Vec<PriceDirective>,
    transactions: Vec<Transaction>,
    periodic_transactions: Vec<PeriodicTransaction>,
    auto_rules: Vec<AutoPostingRule>,
    /// Every source file read so far (main + `include`s), in first-read order and
    /// deduplicated, as resolved absolute paths. Feeds [`Journal::source_files`].
    source_files: Vec<PathBuf>,
    /// The directory every `include` must resolve inside: the main journal
    /// file's own directory. See [`admit_include`].
    include_root: VaultPath,
    /// The canonical paths of the files on the current `include` stack — the
    /// ancestors of the file being parsed, NOT including that file itself. A
    /// stack (rather than a global visited set) so a diamond include resolves
    /// like hledger's: including the same file from two different branches is
    /// legal and parses it twice.
    include_stack: Vec<VaultPath>,
    /// How many `include`s have been admitted so far, capped by
    /// [`MAX_INCLUDE_FILES`].
    includes_admitted: usize,
    /// The MAIN file's leading comment, if it opens with one. Set once, by the
    /// first [`parse_source`] call; `include`d files never touch it. Feeds
    /// [`Journal::leading_comment`] — see [`leading_comment`].
    leading_comment: Option<String>,
    tindex: u32,
}

impl<'v> Ctx<'v> {
    fn new(
        vault: &'v dyn Vault,
        main: &VaultPath,
        main_key: PathBuf,
        overrides: Option<&'v HashMap<VaultPath, String>>,
    ) -> Self {
        Ctx {
            vault,
            overrides,
            include_root: main.parent().unwrap_or_default(),
            main: main.clone(),
            main_key,
            sources: Vec::new(),
            styles: HashMap::new(),
            inherited: HashMap::new(),
            default_decimal_mark: None,
            default_commodity: None,
            default_year: None,
            commodity_styles: Vec::new(),
            commodity_tags: Vec::new(),
            accounts: Vec::new(),
            aliases: Vec::new(),
            alias_scope: Vec::new(),
            prices: Vec::new(),
            transactions: Vec::new(),
            periodic_transactions: Vec::new(),
            auto_rules: Vec::new(),
            source_files: Vec::new(),
            include_stack: Vec::new(),
            includes_admitted: 0,
            leading_comment: None,
            tindex: 0,
        }
    }

    fn into_journal(self, source_name: &str) -> Journal {
        Journal {
            source_name: source_name.to_string(),
            source_files: self.source_files,
            transactions: self.transactions,
            periodic_transactions: self.periodic_transactions,
            auto_rules: self.auto_rules,
            accounts: self.accounts,
            aliases: self.aliases,
            commodity_styles: self.commodity_styles,
            commodity_tags: self.commodity_tags,
            prices: self.prices,
            default_commodity: self.default_commodity.map(|(commodity, _style)| commodity),
            leading_comment: self.leading_comment,
        }
    }
}

/// The absolute key a journal file is addressed by when it is named by a path
/// rather than a vault: canonicalized when it exists on disk, else the path as
/// given (so an in-memory source with a placeholder name still resolves to a
/// stable key). The text-only entry points ([`parse_journal`],
/// [`parse_journal_with_overrides`]) record the main file under this key, which
/// for any journal that exists is exactly the [`Vault::key`] a [`LocalVault`]
/// would give it.
pub(crate) fn resolve_source_file(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// One source file a vault parse read, as it was read.
#[derive(Debug, Clone)]
pub struct ParsedSource {
    /// Where it is in the vault.
    pub path: VaultPath,
    /// The exact text parsed, and the revision of those bytes. `None` when the
    /// text did not come from storage — it was an override, or the caller's.
    pub read: Option<(String, Revision)>,
}

/// A journal parsed through a vault ([`parse_journal_in`]), with every source
/// file it read.
///
/// The sources are what lets a caller that goes on to *write* those files (the
/// editor) pin the write to exactly the bytes that were parsed, instead of
/// reading every file a second time and hoping it had not changed in between.
#[derive(Debug, Clone)]
pub struct ParsedJournal {
    pub journal: Journal,
    /// Main file first, then each `include`d file in first-read order; a file
    /// included twice (a diamond) appears once, as first read.
    pub sources: Vec<ParsedSource>,
}

/// Parse the journal whose main file is `main` in `vault`, reading every
/// `include` through the vault and confining it to `main`'s directory.
///
/// A file whose vault path is a key in `overrides` is parsed from that text and
/// never read; every other file is read with [`Vault::read_pinned`], and its
/// text and [`Revision`] come back in [`ParsedJournal::sources`].
///
/// Source files are recorded under [`Vault::key`]; the journal's `source_name`
/// is the main file's key.
///
/// # Errors
/// [`ParseError::Include`] when the main file cannot be read (including
/// [`crate::vault::VaultError::NotDownloaded`]), and every error
/// [`parse_journal`] reports.
pub fn parse_journal_in(
    vault: &dyn Vault,
    main: &VaultPath,
    overrides: &HashMap<VaultPath, String>,
) -> Result<ParsedJournal, ParseError> {
    let main_key = vault.key(main);
    let source_name = main_key.to_string_lossy().into_owned();
    parse_in(vault, main, main_key, &source_name, overrides)
}

/// [`parse_journal_in`] with the main file's recorded key and `source_name`
/// chosen by the caller — the editor keeps whatever spelling the journal was
/// opened with, exactly as [`parse_journal`] does.
pub(crate) fn parse_in(
    vault: &dyn Vault,
    main: &VaultPath,
    main_key: PathBuf,
    source_name: &str,
    overrides: &HashMap<VaultPath, String>,
) -> Result<ParsedJournal, ParseError> {
    let (text, revision) = match overrides.get(main) {
        Some(text) => (std::borrow::Cow::Borrowed(text.as_str()), None),
        None => {
            let (text, revision) = read_source(vault, main)
                .map_err(|e| ParseError::Include(format!("could not read {source_name}: {e}")))?;
            (std::borrow::Cow::Owned(text), Some(revision))
        }
    };
    let mut ctx = Ctx::new(vault, main, main_key, Some(overrides));
    parse_source(&text, source_name, main, &mut ctx)?;
    let main_read = match (text, revision) {
        (std::borrow::Cow::Owned(text), Some(revision)) => Some((text, revision)),
        _ => None,
    };
    let sources = std::iter::once(ParsedSource {
        path: main.clone(),
        read: main_read,
    })
    .chain(std::mem::take(&mut ctx.sources))
    .collect();
    Ok(ParsedJournal {
        journal: ctx.into_journal(source_name),
        sources,
    })
}

/// Read one source file as UTF-8 with its revision.
///
/// Non-UTF-8 content is an error, never a lossy decode — the editor reads
/// through here, and a lossy decode would later write mojibake back over the
/// original. A bare "stream did not contain valid UTF-8" reads as an
/// unexplained failure to open the user's books (a `£` or a `ñ` in a Latin-1
/// payee is enough), so the message says what to do about it.
fn read_source(vault: &dyn Vault, path: &VaultPath) -> Result<(String, Revision), String> {
    let (bytes, revision) = vault
        .read_pinned(path, None)
        .map_err(|error| read_failure(&error))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        "the file is not valid UTF-8. Ledgeline reads and writes UTF-8 journals only; convert \
         it first (e.g. `iconv -f latin1 -t utf-8`) so no character is silently rewritten"
            .to_string()
    })?;
    Ok((text, revision))
}

/// Why a source could not be read, without the vault error's own path prefix
/// (every caller names the file itself).
fn read_failure(error: &VaultError) -> String {
    match error {
        VaultError::NotFound { .. } => "No such file or directory".to_string(),
        VaultError::NotDownloaded { .. } => {
            "the file is not downloaded to this device yet".to_string()
        }
        VaultError::Io { source, .. } => source.to_string(),
        other => std::io::Error::from(other.kind()).to_string(),
    }
}

/// The vault a text-only entry point resolves `include`s through, and the main
/// file's place in it.
///
/// A journal that is named by an absolute path (or one that exists) is a
/// [`LocalVault`] rooted at its real directory — the same vault the desktop
/// opens it with. A name with no real directory (an in-memory placeholder such
/// as `"test.journal"`) gets an EMPTY vault: its `include`s resolve nowhere,
/// rather than against whatever the process's working directory happens to be.
pub(crate) fn vault_for_source_name(source_name: &str) -> (Box<dyn Vault>, VaultPath) {
    let path = Path::new(source_name);
    if let Ok((vault, main)) = LocalVault::for_journal(path)
        && vault.root().is_absolute()
    {
        return (Box::new(vault), main);
    }
    let main = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|name| VaultPath::root().child(name).ok())
        .unwrap_or_else(|| VaultPath::new("journal").unwrap_or_default());
    (Box::new(MemVault::with_root("")), main)
}

/// Parse `text` (the contents of the journal at `source_name`) into a balanced
/// [`Journal`]. `include`d files are resolved relative to `source_name`'s real
/// directory and confined to it; a `source_name` with no real directory
/// includes nothing (see [`vault_for_source_name`]).
pub fn parse_journal(text: &str, source_name: &str) -> Result<Journal, ParseError> {
    let (vault, main) = vault_for_source_name(source_name);
    let mut ctx = Ctx::new(&*vault, &main, resolve_source_file(source_name), None);
    parse_source(text, source_name, &main, &mut ctx)?;
    Ok(ctx.into_journal(source_name))
}

/// [`parse_journal`], with `inherited` commodity styles in force as though a
/// file that included this one had declared them first.
///
/// What a projection scenario needs from its main journal: hledger reads
/// `$11,344` as 11.344 in a file with no `commodity $1,000.00` of its own
/// (a lone comma followed by three digits is a decimal mark to it), and a
/// sibling `-f main.journal` does NOT lend its directives. Read as the user
/// means it — the way `include main.journal` would — a scenario's amounts
/// follow the main journal's declared styles. The scenario's own `commodity`
/// and `decimal-mark` directives still win. Verified against hledger 1.52,
/// 2026-10-28.
///
/// # Errors
/// As [`parse_journal`].
pub fn parse_journal_with_styles(
    text: &str,
    source_name: &str,
    inherited: &[(Commodity, AmountStyle)],
) -> Result<Journal, ParseError> {
    let (vault, main) = vault_for_source_name(source_name);
    let mut ctx = Ctx::new(&*vault, &main, resolve_source_file(source_name), None);
    ctx.inherited = inherited.iter().cloned().collect();
    parse_source(text, source_name, &main, &mut ctx)?;
    Ok(ctx.into_journal(source_name))
}

/// Parse the journal rooted at `main_source_name`, resolving `include`s from an
/// in-memory `overrides` map before falling back to disk.
///
/// Any file whose resolved absolute path (see [`resolve_source_file`]) is a key
/// in `overrides` is read from that map; every other file (the main file
/// included) is read from disk exactly as [`parse_journal`] would. The path
/// form of [`parse_journal_in`], kept for callers that hold paths.
pub fn parse_journal_with_overrides(
    main_source_name: &str,
    overrides: &HashMap<PathBuf, String>,
) -> Result<Journal, ParseError> {
    let (vault, main) = vault_for_source_name(main_source_name);
    let main_key = resolve_source_file(main_source_name);
    let by_vault_path: HashMap<VaultPath, String> = overrides
        .iter()
        .filter_map(|(key, text)| {
            let resolved = resolve_source_file(key);
            let path = if resolved == main_key {
                Some(main.clone())
            } else {
                crate::vault::place_native(&*vault, &resolved)
            };
            path.map(|path| (path, text.clone()))
        })
        .collect();
    parse_in(&*vault, &main, main_key, main_source_name, &by_vault_path)
        .map(|parsed| parsed.journal)
}

/// Parse one journal source (the top file or an included one, at `source` in
/// the vault) into `ctx`.
fn parse_source(
    text: &str,
    source_name: &str,
    source: &VaultPath,
    ctx: &mut Ctx,
) -> Result<(), ParseError> {
    let source_file = if *source == ctx.main {
        ctx.main_key.clone()
    } else {
        ctx.vault.key(source)
    };
    // Are we the MAIN file? Every parse starts with an empty `source_files`, and
    // the push below is the first thing that ever writes to it, so "nothing
    // recorded yet" is exactly "this is the top-level file". Read BEFORE the push.
    let is_main_file = ctx.source_files.is_empty();
    // Record this file (main or `include`d) so the whole dependency set is known
    // for live-reload watching, even for directive-only includes. Deduplicated to
    // stay stable if the same file is included more than once.
    if !ctx.source_files.contains(&source_file) {
        ctx.source_files.push(source_file.clone());
    }
    // PARSE-9: a UTF-8 BOM is not `char::is_whitespace`, so `trim_start` left it
    // attached to the first token and the first-char dispatch below fell through
    // to `other =>` — failing the WHOLE file with
    // `unsupported directive: '\u{feff}2024-01-01'`. hledger strips it per source
    // file, and Windows/Excel-exported journals routinely carry one. Stripped
    // here (not in `parse_journal`) so an `include`d file is covered too.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // Retain the main file's opening comment for `crate::title`. AFTER the BOM
    // strip above, or a Windows-exported journal's title would silently begin
    // with U+FEFF. `include`d files deliberately do not contribute: the title
    // names the journal the user opened, and an included file's header describes
    // that file alone.
    if is_main_file {
        ctx.leading_comment = leading_comment(text);
    }
    let lines: Vec<&str> = text.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let line_no = to_u32(i + 1);
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let Some(first) = trimmed.chars().next() else {
            i += 1;
            continue;
        };
        if matches!(first, ';' | '#' | '*') {
            // A comment line (`;`/`#`/`*`); hledger allows these to be indented,
            // so this check must precede the stray-indent guard below.
            i += 1;
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // A non-comment indented line only ever appears inside a transaction
            // or block directive (both consumed wholesale); reaching here means a
            // stray posting or an unsupported indented subdirective.
            return Err(locate(
                source_name,
                line_no,
                line,
                ParseError::UnexpectedIndent(line_no),
            ));
        }
        if first.is_ascii_digit() {
            ctx.tindex += 1;
            let amt = AmountCtx {
                styles: &ctx.styles,
                default_mark: ctx.default_decimal_mark,
                default_commodity: ctx.default_commodity.as_ref(),
                inherited: &ctx.inherited,
            };
            let (txn, next) = parse_transaction(
                &lines,
                i,
                ctx.tindex,
                amt,
                source_name,
                &source_file,
                ctx.default_year,
            )?;
            ctx.transactions.push(txn);
            i = next;
            continue;
        }

        // `=QUERY` with no space is an auto-posting rule too: hledger reads
        // `=revenue:other` exactly as `= revenue:other` (verified, 1.52), so
        // it must not fall through to "unsupported directive" and refuse the
        // whole file.
        let keyword = if trimmed.starts_with('=') {
            "="
        } else {
            trimmed.split_whitespace().next().unwrap_or("")
        };
        match keyword {
            "account" => {
                let decl = parse_account_directive(trimmed, line_no, &source_file)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.accounts.push(decl);
            }
            "commodity" => {
                let (commodity, style, tags) = parse_commodity_directive(trimmed)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                // A symbol-only `commodity $` declares no style (its amounts
                // style themselves); a full spec establishes the canonical style.
                if let Some(style) = style {
                    ctx.styles.insert(commodity.clone(), style.clone());
                    ctx.commodity_styles.push((commodity.clone(), style));
                }
                if !tags.is_empty() {
                    ctx.commodity_tags.push((commodity, tags));
                }
            }
            "decimal-mark" => {
                ctx.default_decimal_mark = Some(
                    parse_decimal_mark_directive(trimmed)
                        .map_err(|e| locate(source_name, line_no, line, e))?,
                );
            }
            "P" => {
                let amt = AmountCtx {
                    styles: &ctx.styles,
                    default_mark: ctx.default_decimal_mark,
                    default_commodity: ctx.default_commodity.as_ref(),
                    inherited: &ctx.inherited,
                };
                let price = parse_price_directive(trimmed, amt, ctx.default_year, &source_file)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.prices.push(price);
            }
            // `D AMOUNT` declares the default commodity + its style; bare-number
            // amounts adopt both. It also establishes that commodity's style,
            // like a `commodity` directive.
            "D" => {
                let (commodity, style) = parse_default_commodity_directive(trimmed)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.styles.insert(commodity.clone(), style.clone());
                ctx.commodity_styles
                    .push((commodity.clone(), style.clone()));
                ctx.default_commodity = Some((commodity, style));
            }
            "include" => {
                let as_written =
                    include_target(trimmed).map_err(|e| locate(source_name, line_no, line, e))?;
                // Confinement, cycle and budget checks all run BEFORE the file is
                // read, so a rejected include never has its contents echoed back
                // through a parse error (SEC-6's read oracle).
                let path = admit_include(&as_written, source, ctx)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                let key = ctx.vault.key(&path);
                let (included, revision) = match ctx.overrides.and_then(|map| map.get(&path)) {
                    Some(text) => (std::borrow::Cow::Borrowed(text.as_str()), None),
                    None => {
                        let (text, revision) = read_source(ctx.vault, &path).map_err(|e| {
                            locate(
                                source_name,
                                line_no,
                                line,
                                ParseError::Include(format!("{}: {e}", key.display())),
                            )
                        })?;
                        (std::borrow::Cow::Owned(text), Some(revision))
                    }
                };
                // The slot is taken now, so sources stay in first-read order;
                // the text moves into it once the nested parse is done with it.
                // A file read twice (a diamond) keeps its first read.
                let slot = (!ctx.sources.iter().any(|source| source.path == path)).then(|| {
                    ctx.sources.push(ParsedSource {
                        path: path.clone(),
                        read: None,
                    });
                    ctx.sources.len() - 1
                });
                // Push the INCLUDING file so the nested parse sees its own full
                // ancestor chain; pop unconditionally to keep the stack accurate
                // if the caller ever recovers from the error.
                ctx.include_stack.push(source.clone());
                // Aliases flow INTO an include and never back out (verified
                // against hledger 1.52), so the nested parse inherits this
                // scope and the parent's is restored whatever it did with it —
                // including an `end aliases`, which kills the parent's aliases
                // only within the child.
                let outer_scope = ctx.alias_scope.clone();
                let nested = parse_source(&included, &key.to_string_lossy(), &path, ctx);
                ctx.alias_scope = outer_scope;
                ctx.include_stack.pop();
                if let (Some(slot), std::borrow::Cow::Owned(text), Some(revision)) =
                    (slot, included, revision)
                {
                    ctx.sources[slot].read = Some((text, revision));
                }
                nested?;
            }
            "alias" => {
                let alias = parse_alias_directive(trimmed, line_no, &source_file)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.alias_scope.push(ctx.aliases.len());
                ctx.aliases.push(alias);
            }
            // `end aliases` — and nothing else. `end apply account` and any
            // other `end` form stays an unsupported directive, so a construct
            // whose effect we do not model still fails loudly.
            "end" => {
                end_aliases_keyword(trimmed).map_err(|e| locate(source_name, line_no, line, e))?;
                close_alias_scope(ctx, &source_file);
            }
            // Declarations with no effect on transaction parsing.
            "payee" | "tag" => {}
            // A periodic (`~`) rule: parse it into `periodic_transactions` (for
            // the budget report) but keep it out of `transactions`/`jtxns`.
            "~" => {
                let amt = AmountCtx {
                    styles: &ctx.styles,
                    default_mark: ctx.default_decimal_mark,
                    default_commodity: ctx.default_commodity.as_ref(),
                    inherited: &ctx.inherited,
                };
                let (periodic, next) = parse_periodic_transaction(
                    &lines,
                    i,
                    amt,
                    source_name,
                    &source_file,
                    ctx.default_year,
                )?;
                ctx.periodic_transactions.push(periodic);
                i = next;
                continue;
            }
            // An `= QUERY` auto-posting rule: parsed into `auto_rules`, never
            // into `transactions` (hledger-web's `/transactions` omits them
            // too). Where a rule applies is the caller's business — see
            // `AutoPostingRule`.
            "=" => {
                let amt = AmountCtx {
                    styles: &ctx.styles,
                    default_mark: ctx.default_decimal_mark,
                    default_commodity: ctx.default_commodity.as_ref(),
                    inherited: &ctx.inherited,
                };
                let (rule, next) = parse_auto_rule(&lines, i, amt, source_name, &source_file)?;
                ctx.auto_rules.push(rule);
                i = next;
                continue;
            }
            "comment" => {
                i = skip_comment_block(&lines, i);
                continue;
            }
            k if is_year_directive(k) => {
                let year = parse_year_directive(trimmed)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.default_year = Some(year);
            }
            other => {
                return Err(locate(
                    source_name,
                    line_no,
                    line,
                    ParseError::UnsupportedDirective(other.to_string()),
                ));
            }
        }
        i += 1;
    }
    Ok(())
}

/// A journal source's LEADING COMMENT: the text of its first non-empty line
/// when that line is a comment, with the marker and the whitespace around it
/// stripped. `None` when the file is empty or opens with anything else.
///
/// hledger's line-comment markers are `;`, `#` and `*`, and a comment line may
/// be indented — the same three the parse loop dispatches on. The whole leading
/// RUN of them is stripped, so the common `;;;;` banner and the org-mode `****`
/// heading reduce to their text instead of keeping a stray marker.
///
/// The parse loop otherwise discards comments outright. This one is kept because
/// it is where a journal names itself; [`crate::title`] turns it into the title
/// the UI shows, and owns every judgement about whether it reads as one. Nothing
/// is rejected here — this reports what the file says, not what it means.
fn leading_comment(text: &str) -> Option<String> {
    let first = text.lines().find(|line| !line.trim().is_empty())?;
    let trimmed = first.trim_start();
    if !trimmed.starts_with([';', '#', '*']) {
        return None;
    }
    Some(
        trimmed
            .trim_start_matches([';', '#', '*'])
            .trim()
            .to_string(),
    )
}

/// Convert a `usize` line/column index to `u32`, saturating (line counts here
/// never approach `u32::MAX`).
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Attach source location (file, line, and the line's text) to an error, unless
/// it already carries one — so the innermost `include` location wins.
fn locate(source_name: &str, line: u32, line_text: &str, err: ParseError) -> ParseError {
    if matches!(err, ParseError::Located { .. }) {
        return err;
    }
    ParseError::Located {
        source_name: source_name.to_string(),
        line,
        line_text: line_text.to_string(),
        message: err.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Directives
// ---------------------------------------------------------------------------

/// Split an `account` directive's body into the account NAME and any trailing
/// comment, by hledger's rule rather than at the first `;`.
///
/// Verified against the hledger 1.52 binary, not read off the manual:
///
/// ```text
/// account two:space  ; type: A    -> name "two:space",         tag APPLIED
/// account one:space ; type: A     -> name "one:space ; type: A", tag IGNORED
/// account a:b is here  ; type: A  -> name "a:b is here",       tag APPLIED
/// account tab:name<TAB>; type: A  -> name "tab:name ; type: A", tag IGNORED
/// account two:words  junk         -> parse ERROR
/// ```
///
/// An account name may contain SINGLE spaces, so nothing but a run of two or
/// more whitespace characters can end one — the same rule that separates a
/// posting's account from its amount, and the reason a lone tab is part of the
/// name (normalized to a space) while a space-then-tab is a separator.
///
/// Splitting at the first `;`, as [`split_comment`] does for the directives
/// whose value cannot contain spaces, is wrong here and wrong INVISIBLY: it
/// turns `account a:b ; type: A` into a declaration of the account literally
/// named `a:b ; type: A`, which matches no posting and carries no type. The
/// journal parses, the report is simply missing an account, and nothing says so.
///
/// Returns `None` when the text after the separator is neither empty nor a
/// comment — hledger rejects that outright ("expecting ';', end of input, or
/// newline") and so do we.
///
/// The third element of the tuple is the byte offset, relative to `body`,
/// where the separator run begins (or `body.len()` when there is none) —
/// `pub(crate)` so [`crate::accounts`] can locate exactly where a
/// declaration's comment starts without re-deriving this whitespace-run
/// grammar a second time, which is the kind of silent divergence the doc
/// comment above already records one instance of.
pub(crate) fn split_account_name(body: &str) -> Option<(String, Option<&str>, usize)> {
    let mut run_start: Option<usize> = None;
    let mut separator: Option<usize> = None;
    for (at, ch) in body.char_indices() {
        if ch == ' ' || ch == '\t' {
            match run_start {
                // The second consecutive whitespace ends the name, which stops
                // at the FIRST character of the run.
                Some(start) => {
                    separator = Some(start);
                    break;
                }
                None => run_start = Some(at),
            }
        } else {
            run_start = None;
        }
    }

    let tail = separator.unwrap_or(body.len());
    let (name_part, rest) = match separator {
        Some(at) => (&body[..at], body[at..].trim_start()),
        None => (body, ""),
    };
    // A lone tab inside the name reads as a space, matching how hledger prints
    // the account back.
    let name = name_part.replace('\t', " ").trim().to_string();
    if name.is_empty() {
        return None;
    }
    match rest.strip_prefix(';') {
        Some(comment) => Some((name, Some(comment), tail)),
        None if rest.is_empty() => Some((name, None, tail)),
        None => None,
    }
}

fn parse_account_directive(
    line: &str,
    line_no: u32,
    source_file: &Path,
) -> Result<AccountDeclaration, ParseError> {
    let after = line
        .strip_prefix("account")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (name, comment, _tail) = split_account_name(after)
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (comment_text, tags) = build_comment(comment);
    Ok(AccountDeclaration {
        name: AccountName(name),
        tags,
        comment: comment_text,
        // `account` directives are always top-level, so the keyword sits at
        // column 1 (hledger reports the same).
        position: SourcePos {
            line: line_no,
            column: 1,
        },
        source_file: source_file.to_path_buf(),
    })
}

/// Parse an `alias OLD = NEW` or `alias /REGEX/ = REPLACEMENT` directive.
///
/// Both forms are hledger's, and the split rules below are the binary's, checked
/// against hledger 1.52 rather than read off the manual:
///
/// - **The plain form splits at the FIRST `=`.** `alias a = b = c` maps `a` to
///   the account literally named `b = c`.
/// - **The regex form does NOT.** `alias /a=b/ = c` is a regex containing an
///   equals sign; the closing delimiter is the first unescaped `/`, and only
///   after it does the `=` separate the replacement. `alias /a\/b/ = c` really
///   does match the account `a/b`, so `\/` is an escape, not a terminator.
/// - **Both sides are whitespace-trimmed, and NEITHER is comment-stripped.**
///   `alias a = b ; note` declares the account `b ; note`.
/// - **A line with no `=` is a hard error in hledger**, so it is one here.
///   Accepting it would let a journal hledger refuses open here and report
///   numbers hledger never would.
///
/// An empty pattern or replacement is *not* rejected: hledger takes them, and
/// `parse` refusing a file hledger reads is the failure this parser cares most
/// about. Such an alias is simply never forwarded — see
/// [`crate::aliases::forward`].
fn parse_alias_directive(
    line: &str,
    line_no: u32,
    source_file: &Path,
) -> Result<AliasDirective, ParseError> {
    let malformed = || ParseError::MalformedDirective(line.to_string());
    let after = line
        .strip_prefix("alias")
        .ok_or_else(malformed)?
        .trim_start();
    let (pattern, replacement, regex) = match after.strip_prefix('/') {
        Some(rest) => {
            let close = unescaped_slash(rest).ok_or_else(malformed)?;
            let value = rest[close + 1..]
                .trim_start()
                .strip_prefix('=')
                .ok_or_else(malformed)?;
            (&rest[..close], value, true)
        }
        None => {
            let (name, value) = after.split_once('=').ok_or_else(malformed)?;
            (name, value, false)
        }
    };
    Ok(AliasDirective {
        pattern: pattern.trim().to_string(),
        replacement: replacement.trim().to_string(),
        regex,
        source_file: source_file.to_path_buf(),
        position: SourcePos {
            line: line_no,
            column: 1,
        },
        ended: false,
    })
}

/// The byte offset of the first `/` in `text` that is not preceded by a
/// backslash — a regex alias's closing delimiter.
pub(crate) fn unescaped_slash(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Skip the escaped character whatever it is, so `\\/` closes and
            // `\/` does not.
            b'\\' => i += 2,
            b'/' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Accept exactly `end aliases`, with an optional trailing comment.
///
/// hledger 1.52 rejects the singular `end alias` and accepts `end aliases ; x`;
/// both were checked against the binary.
fn end_aliases_keyword(line: &str) -> Result<(), ParseError> {
    let (main, _comment) = split_comment(line);
    let mut tokens = main.split_whitespace();
    match (tokens.next(), tokens.next(), tokens.next()) {
        (Some("end"), Some("aliases"), None) => Ok(()),
        // Anything else beginning `end` is a directive we do not model, and it
        // is reported as one rather than silently ignored.
        _ => Err(ParseError::UnsupportedDirective("end".to_string())),
    }
}

/// Apply an `end aliases`: every alias in force is out of scope from here.
///
/// Only the ones declared in **this** file are marked
/// [`ended`](AliasDirective::ended). An inherited alias from an including file
/// is merely out of scope for the rest of this file — it resumes in its own file
/// after the `include` returns, which the caller restores.
fn close_alias_scope(ctx: &mut Ctx, source_file: &Path) {
    for index in ctx.alias_scope.drain(..) {
        if let Some(alias) = ctx.aliases.get_mut(index)
            && alias.source_file == source_file
        {
            alias.ended = true;
        }
    }
}

/// Parse a `commodity` directive into its commodity, optional style (absent for
/// a symbol-only `commodity $`), and comment tags.
#[allow(clippy::type_complexity)]
fn parse_commodity_directive(
    line: &str,
) -> Result<(Commodity, Option<AmountStyle>, Vec<(String, String)>), ParseError> {
    let after = line
        .strip_prefix("commodity")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (spec_part, comment) = split_comment(after);
    let spec = spec_part.trim();
    let (_comment_text, tags) = build_comment(comment);
    if spec.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    // A symbol-only spec (no number, e.g. `commodity $`) declares the commodity
    // without a style; a full spec (`commodity $1,000.00`) yields both.
    match parse_commodity_style_spec(spec) {
        Ok((commodity, style)) => Ok((commodity, Some(style), tags)),
        Err(_) => Ok((Commodity(spec.to_string()), None, tags)),
    }
}

/// Parse a `D AMOUNT` default-commodity directive into its commodity + style.
fn parse_default_commodity_directive(line: &str) -> Result<(Commodity, AmountStyle), ParseError> {
    let after = line
        .strip_prefix('D')
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (spec_part, _comment) = split_comment(after);
    parse_commodity_style_spec(spec_part.trim())
}

/// Parse a commodity + amount-style specimen (e.g. `$1,000.00`, `1.000,00 EUR`)
/// into the commodity symbol and its canonical display style.
fn parse_commodity_style_spec(spec: &str) -> Result<(Commodity, AmountStyle), ParseError> {
    let (commodity, number, side, spaced) = split_commodity_spec(spec)?;
    let (decimal_mark, digit_groups, precision) = analyze_number(&number, None);
    let style = AmountStyle {
        side,
        spaced,
        decimal_mark,
        digit_groups,
        // The same cap an amount's precision gets, which is its parsed
        // `places`. A directive is nothing BUT a style, so this is the one
        // precision that never went through `Dec::parse`: one long run of
        // zeros here set the width every amount in the commodity laid out to.
        precision: precision.min(MAX_PARSE_PLACES),
    };
    Ok((Commodity(commodity), style))
}

/// Parse a `decimal-mark .` / `decimal-mark ,` directive into its mark char.
fn parse_decimal_mark_directive(line: &str) -> Result<char, ParseError> {
    let after = line
        .strip_prefix("decimal-mark")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (spec, _comment) = split_comment(after);
    match spec.trim() {
        "." => Ok('.'),
        "," => Ok(','),
        _ => Err(ParseError::MalformedDirective(line.to_string())),
    }
}

/// The target of an `include PATH` directive, exactly as written — which is
/// what diagnostics quote, so a rejected include never discloses a path the
/// journal did not already name (e.g. a symlink's target).
fn include_target(line: &str) -> Result<String, ParseError> {
    let after = line
        .strip_prefix("include")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (path_part, _comment) = split_comment(after);
    let path_str = path_part.trim();
    if path_str.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    Ok(path_str.to_string())
}

/// Place a path a file *wrote* (an `include` target, a rules file's `source`)
/// in the vault — **lexically**, relative to `base`, the writing file's own
/// directory — or `None` when it lands outside the vault.
///
/// An absolute value, or one that climbs out with `..`, cannot be a
/// [`VaultPath`] as written. When the vault has a real directory behind it, such
/// a spelling is resolved the way the desktop always resolved it (the deepest
/// existing ancestor canonicalized) and kept if it lands back inside —
/// `include /books/2026.journal` from `/books/main.journal` is an ordinary
/// journal. A vault with no real directory has no other spelling to try.
/// Nothing here consults the process's working directory.
pub(crate) fn place_written(
    vault: &dyn Vault,
    base: &VaultPath,
    written: &str,
) -> Option<VaultPath> {
    if !Path::new(written).is_absolute()
        && let Ok(path) = base.join(written)
    {
        return Some(path);
    }
    let native = if Path::new(written).is_absolute() {
        PathBuf::from(written)
    } else {
        vault.local_path(base)?.join(written)
    };
    crate::vault::place_native(vault, &native)
}

/// `path` with every symlink resolved, when that lands inside `root`; `None`
/// otherwise. A path whose tail does not exist yet has its deepest existing
/// ancestor resolved (see [`crate::vault::canonicalize_existing_prefix`]), so
/// a not-yet-created file under a directory link out of the tree is still
/// outside.
///
/// The containment test, named once: the `include` guard (SEC-6), the
/// import-rules discovery guard and the create-path guard ([`resolve_new_in`])
/// all ask it, so they cannot drift apart. Callers own the error message,
/// because what may be disclosed differs by caller.
pub fn confine(vault: &dyn Vault, root: &VaultPath, path: &VaultPath) -> Option<VaultPath> {
    crate::vault::canonicalize_existing_prefix(vault, path)
        .ok()
        .filter(|resolved| resolved.starts_with(root))
}

/// Why [`resolve_new_in`] refused, one variant per guard it applies.
///
/// Callers map this onto their own refusal type, because what may be *told* to
/// a client differs by feature and the split matters: [`OutsideRoot`] and
/// [`DirectoryMissing`] are the two that could answer a question about the
/// filesystem, so a caller must collapse them into the same sentence every
/// other resolution failure returns. [`Exists`] is safe to report as itself —
/// it is only reachable for a confined, non-hidden, listable name below the
/// root, which is the exact set the feature's own index already publishes.
///
/// [`OutsideRoot`]: NewPathRefusal::OutsideRoot
/// [`DirectoryMissing`]: NewPathRefusal::DirectoryMissing
/// [`Exists`]: NewPathRefusal::Exists
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewPathRefusal {
    /// Not a shape a scan could ever have produced, or one it would skip: a
    /// traversal, a hidden component, a skipped directory, a name the feature
    /// does not list.
    Malformed,
    /// Resolves outside `root`.
    OutsideRoot,
    /// The directory it would go in is not there, is not a directory, or is
    /// reached through a symlink. **No directory is ever created.**
    DirectoryMissing,
    /// Something is already at that name, of any file type, symlinks included.
    Exists,
}

/// Where a **new** file called `id` would go below `root` in `vault` — the one
/// place in this crate a caller's string is joined onto a root.
///
/// # Why this exists at all, given a discovery scan
///
/// A scan can only ever return a file it already found, which is the whole of
/// its security value and is exactly why it cannot serve a *create*: the file
/// is not there yet, so there is nothing to match against. `root.join(id)` is
/// unavoidable, so it happens here, once, beside [`confine`] — for the same
/// reason `confine` itself was extracted. Two features need this chain, and a
/// second hand-rolled copy of it is exactly the near-duplicate that gets one of
/// its copies fixed and not the other.
///
/// # The guards, in order, and what each is for
///
/// 1. **Shape** ([`NewPathRefusal::Malformed`]) — before any storage call, and
///    deliberately a *second* copy of the question the HTTP layer's own
///    `validate_id` asks. This function does not get to assume its caller
///    checked: a `..` reaching a `join` is the whole traversal bug class.
/// 2. **Discoverability** ([`NewPathRefusal::Malformed`]) — no component may be
///    hidden or in `skip_dirs`, and the final name must satisfy `name_ok`,
///    because a scan skips exactly those. Creating a file the scan will never
///    list would write something the user cannot then open.
/// 3. **Confinement** ([`NewPathRefusal::OutsideRoot`]) — [`confine`], the same
///    function `include` is held to, which resolves the deepest existing
///    ancestor's links and re-appends the rest.
/// 4. **A real parent directory, reached without a symlink**
///    ([`NewPathRefusal::DirectoryMissing`]) — two tests, and it takes both.
///    Comparing `confine`'s output against the path as joined refuses a link
///    anywhere in the id (a canonical path has no links left in it to see),
///    exactly as a scan refuses one, and catches a case-folding filesystem
///    rewriting a component; [`Vault::stat`], which never follows a link, then
///    requires the parent to be a real directory. No directory is ever created.
/// 5. **Nothing there already** ([`NewPathRefusal::Exists`]).
///
/// Guard 5 is **not** what makes the create safe, and must not be relied on as
/// if it were: it expires the moment it returns. The write itself has to be
/// exclusive ([`Vault::create_new`]), and every caller performs it that way —
/// this is the courtesy that produces a good error message, and the create is
/// what produces the guarantee.
///
/// The [`VaultPath`] that comes back is deliberately naked. Each caller wraps it
/// in its own unforgeable newtype (`RulesPath`, `ProjectionPath`), which is what
/// carries "you may only write to a path that was resolved" into the write
/// path; this function cannot mint one and should not be able to.
///
/// # Errors
/// [`NewPathRefusal`], one variant per guard above.
pub fn resolve_new_in(
    vault: &dyn Vault,
    root: &VaultPath,
    id: &str,
    max_depth: usize,
    skip_dirs: &[&str],
    name_ok: fn(&str) -> bool,
) -> Result<VaultPath, NewPathRefusal> {
    let parts: Vec<&str> = id.split('/').collect();
    let well_formed = !id.is_empty()
        && parts.len() <= max_depth + 1
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && !part.starts_with('.')
                && !part.contains('\\')
                && !part.contains(':')
                && !part.chars().any(|c| c.is_ascii_control())
        })
        // Every directory component must be one the scan would descend into,
        // and the file name one it would list. See guard 2.
        && parts[..parts.len() - 1]
            .iter()
            .all(|part| !skip_dirs.contains(part))
        && parts.last().is_some_and(|name| name_ok(name));
    if !well_formed {
        return Err(NewPathRefusal::Malformed);
    }

    // THE join. Everything above is what earns it.
    let candidate = root.join(id).map_err(|_| NewPathRefusal::Malformed)?;
    let Some(resolved) = confine(vault, root, &candidate) else {
        return Err(NewPathRefusal::OutsideRoot);
    };
    // **A symlink anywhere in the id is refused**, and this is the test that
    // does it: `confine` resolves links, so a `linked/x` whose `linked` points
    // elsewhere in the root comes back as the target — somewhere the id does
    // not name, and therefore somewhere the scan lists under a DIFFERENT id,
    // so the user could not open what they had just created.
    if resolved != candidate {
        return Err(NewPathRefusal::DirectoryMissing);
    }
    let Some(parent) = resolved.parent() else {
        return Err(NewPathRefusal::DirectoryMissing);
    };
    // Belt and braces after the comparison above, with a `stat` that never
    // follows a link.
    if !vault.is_dir(&parent) {
        return Err(NewPathRefusal::DirectoryMissing);
    }
    match vault.stat(&resolved) {
        Ok(None) => Ok(resolved),
        // Present, or unreadable in a way that is not "absent". Either way this
        // is not a name a create may have.
        _ => Err(NewPathRefusal::Exists),
    }
}

/// The first HIDDEN component of `path` below `root` — the one thing an
/// `include` may not reach into, on top of confinement.
///
/// # Why only hidden entries
/// `rules::discovery` skips two classes when it scans this same directory:
/// hidden entries, and a `SKIP_DIRS` list of routinely-enormous tool
/// directories. Only the first belongs here, and the difference is what
/// each class holds rather than where it sits. A dot-entry in a journal
/// directory is `.git/config`, `.env`, `.netrc`, `.ssh/` — credentials, and the
/// whole reason this guard exists. `build/`, `dist/`, `target/` and `vendor/`
/// hold none of that; for a *scan* they are a cost worth avoiding, but an
/// `include` is a path the user typed deliberately, so refusing
/// `include build/2026.journal` would be a regression that buys no security.
///
/// # Measured below the root, never on it
/// A journal that lives in `~/.finance` or `~/.config/ledgeline` is an ordinary
/// choice, and the user naming their own journal directory is not the case this
/// guards — an `include` reaching sideways into a hidden sibling of their
/// journals is. Components are read from the RESOLVED path, so a symlink
/// pointing into `.git` is caught after resolution rather than by its spelling.
fn hidden_component<'a>(path: &'a VaultPath, root: &VaultPath) -> Option<&'a str> {
    path.segments()
        .skip(root.segments().count())
        .find(|name| name.starts_with('.'))
}

/// Decide whether an `include` target may be parsed, returning its resolved
/// vault path. Rejects, in order:
///
/// 1. **Escapes the journal directory** (SEC-6). An `include` naming an absolute
///    path, traversing with `..`, or crossing a symlink out of the tree turns a
///    hostile journal into a local file-read oracle: whatever the included file
///    fails to parse as is quoted back in the error, which reaches stderr and the
///    GUI's error dialog — and anything that DOES parse is absorbed into the
///    journal and served over HTTP. hledger permits all three; we deliberately
///    do not (see the module docs).
/// 2. **Hidden entries.** Confinement bounds the *tree* an include may read, but
///    journals routinely live inside a git repo, so that tree contains
///    `.git/config`, `.env` and `.netrc`. `include .git/config` was admitted,
///    read, and — because [`ParseError::Located`] quotes the first line that
///    failed to parse — echoed back to stderr and the GUI error dialog, which is
///    the same file-read oracle SEC-6 closed for paths outside the tree,
///    reopened for paths inside it. Refusing BEFORE the read is what makes the
///    refusal disclose nothing.
/// 3. **Cycles** (SEC-4) — the target is the including file itself or one of its
///    ancestors. Unbounded recursion here overflows the stack, which aborts the
///    process with `SIGABRT` and cannot be caught.
/// 4. **Depth / total-file budget** — bounds an acyclic include bomb, which
///    cycle detection alone does not.
fn admit_include(
    as_written: &str,
    includer: &VaultPath,
    ctx: &mut Ctx,
) -> Result<VaultPath, ParseError> {
    let escapes = || {
        ParseError::Include(format!(
            "'{as_written}' resolves outside the journal directory {}; \
             includes may not escape the main journal's directory",
            ctx.vault.key(&ctx.include_root).display()
        ))
    };
    let lexical = place_written(
        ctx.vault,
        &includer.parent().unwrap_or_default(),
        as_written,
    )
    .filter(|path| path.starts_with(&ctx.include_root))
    .ok_or_else(escapes)?;
    let path = match crate::vault::canonicalize_existing_prefix(ctx.vault, &lexical) {
        Ok(resolved) if resolved.starts_with(&ctx.include_root) => resolved,
        Ok(_) | Err(VaultError::OutsideRoot { .. }) => return Err(escapes()),
        // The vault could not say (a permission error part-way down, say): the
        // name as written is already confined, and the read reports the rest.
        Err(_) => lexical,
    };
    if let Some(hidden) = hidden_component(&path, &ctx.include_root) {
        return Err(ParseError::Include(format!(
            "'{as_written}' resolves into the hidden entry '{hidden}', which \
             includes may not read; a hidden entry is not a journal file"
        )));
    }
    if path == *includer || ctx.include_stack.contains(&path) {
        return Err(ParseError::Include(format!(
            "this included file forms a cycle: {}",
            ctx.vault.key(&path).display()
        )));
    }
    if ctx.include_stack.len() + 1 > MAX_INCLUDE_DEPTH {
        return Err(ParseError::Include(format!(
            "include nesting deeper than {MAX_INCLUDE_DEPTH} levels at '{as_written}'"
        )));
    }
    ctx.includes_admitted += 1;
    if ctx.includes_admitted > MAX_INCLUDE_FILES {
        return Err(ParseError::Include(format!(
            "more than {MAX_INCLUDE_FILES} included files in one journal"
        )));
    }
    Ok(path)
}

fn parse_price_directive(
    line: &str,
    amt: AmountCtx,
    default_year: Option<i32>,
    source_file: &Path,
) -> Result<PriceDirective, ParseError> {
    let malformed = || ParseError::MalformedDirective(line.to_string());
    let rest = line.trim_start().strip_prefix('P').ok_or_else(malformed)?;
    let (date, rest) = next_token(rest).ok_or_else(malformed)?;
    // hledger allows an optional clock time after the date
    // (`P DATE [HH:MM[:SS]] COMMODITY PRICE`); only the day is retained, matching
    // hledger's date-only market prices.
    let rest = match next_token(rest) {
        Some((token, after_time)) if is_time_token(token) => after_time,
        _ => rest,
    };
    // PARSE-9: the commodity used to be taken as a whitespace-delimited token,
    // which split a quoted symbol (`"green apples"` became `"green`) and left
    // the rest of the name in front of the price.
    let (commodity, rest) = split_price_commodity(rest).ok_or_else(malformed)?;
    let price_str = rest.trim();
    if price_str.is_empty() {
        return Err(malformed());
    }
    let price = parse_amount(price_str, amt)?;
    Ok(PriceDirective {
        date: normalize_date(date, default_year)?,
        commodity: Commodity(commodity),
        price,
        source_file: source_file.to_path_buf(),
    })
}

/// The next whitespace-delimited token in `text`, plus the remainder after it.
fn next_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    Some((&text[..end], &text[end..]))
}

/// The commodity symbol at the start of a `P` directive's remainder: either a
/// double-quoted name (which may contain spaces) or a plain token.
fn split_price_commodity(text: &str) -> Option<(String, &str)> {
    let text = text.trim_start();
    if text.starts_with('"') {
        return split_quoted_commodity(text);
    }
    next_token(text).map(|(token, rest)| (token.to_string(), rest))
}

/// Whether a token is a clock time (`HH:MM` / `HH:MM:SS`) rather than a
/// commodity symbol — used to skip the optional time in a `P` directive.
/// (Unquoted commodity symbols never contain `:`.)
fn is_time_token(token: &str) -> bool {
    token.contains(':') && token.chars().all(|c| c.is_ascii_digit() || c == ':')
}

/// Whether a keyword is a `Y` default-year directive (`Y 2026` or `Y2026`).
fn is_year_directive(keyword: &str) -> bool {
    keyword == "Y"
        || (keyword.len() > 1
            && keyword.starts_with('Y')
            && keyword[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// Parse a `Y YEAR` / `YYEAR` default-year directive into its year.
fn parse_year_directive(line: &str) -> Result<i32, ParseError> {
    let after = line
        .strip_prefix('Y')
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (spec, _comment) = split_comment(after);
    spec.trim()
        .parse::<i32>()
        .map_err(|_| ParseError::MalformedDirective(line.to_string()))
}

/// Normalize a journal date to ISO `YYYY-MM-DD`: accept `-`/`/`/`.` separators,
/// zero-pad the components, and expand a yearless `MM-DD` date using
/// `default_year` (from a `Y` directive). hledger emits all dates in this form.
fn normalize_date(token: &str, default_year: Option<i32>) -> Result<String, ParseError> {
    let comps: Vec<&str> = token.split(['-', '/', '.']).collect();
    let (year, month, day) = match comps.len() {
        3 => (
            parse_date_part(comps[0], token)?,
            parse_date_part(comps[1], token)?,
            parse_date_part(comps[2], token)?,
        ),
        2 => {
            let year = default_year.ok_or_else(|| {
                ParseError::MalformedDate(format!(
                    "'{token}' has no year and no `Y` default-year directive is in effect"
                ))
            })?;
            (
                year,
                parse_date_part(comps[0], token)?,
                parse_date_part(comps[1], token)?,
            )
        }
        _ => return Err(ParseError::MalformedDate(format!("'{token}'"))),
    };
    // PARSE-6: the day must be valid FOR ITS MONTH. A blanket `1..=31` accepted
    // `2024-02-30`, `2023-02-29` and `2024-04-31` — all three of which hledger
    // rejects ("This is not a valid date, please fix it.") — and the bogus ISO
    // string then flowed into period bucketing, sorting and every date-filtered
    // report, where `2026-02-31` silently rolls forward to March 3.
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return Err(ParseError::MalformedDate(format!("'{token}'")));
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// The number of days in `month` (1-12) of `year`. `0` for an out-of-range
/// month, so a containment test against it is always false.
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The proleptic Gregorian leap-year rule hledger's `Data.Time` calendar uses.
fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Parse a single numeric date component (year/month/day).
fn parse_date_part(part: &str, token: &str) -> Result<i32, ParseError> {
    part.trim()
        .parse::<i32>()
        .map_err(|_| ParseError::MalformedDate(format!("'{token}'")))
}

/// Advance past a `comment` ... `end comment` block, returning the index after
/// the terminating line (or end of input).
///
/// PARSE-9: an UNTERMINATED block therefore swallows the rest of the file, and
/// every transaction after it vanishes from every report. That is deliberate
/// parity — hledger 1.52 does exactly the same and exits 0 (verified) — because
/// erroring instead would make a journal hledger loads refuse to open here. It
/// cannot be a diagnostic either: the wire contract has exactly two rules,
/// `unbalanced` and `assertion`. Pinned by a test in `tests/parse_fixes.rs` so
/// the behaviour is at least deliberate rather than accidental.
fn skip_comment_block(lines: &[&str], start: usize) -> usize {
    let mut j = start + 1;
    while j < lines.len() {
        if lines[j].trim() == "end comment" {
            return j + 1;
        }
        j += 1;
    }
    j
}

/// Parse a `~ PERIODEXPR  [DESCRIPTION]` periodic rule and its indented
/// postings, returning the rule and the index of the first following line.
///
/// The postings are parsed and balanced exactly like a normal transaction's
/// (reusing [`parse_posting`]/[`balance_postings`]), so an elided balancing
/// posting is inferred and unbalanced-virtual `(account)` postings are excluded
/// from balancing. The period expression and description must be separated by
/// two-or-more spaces (matching hledger, which errors with "a double space is
/// required between period expression and description/comment"); the expression
/// itself is read by the infallible [`parse_period_spec`], so a rule can no
/// longer refuse the whole journal.
fn parse_periodic_transaction(
    lines: &[&str],
    start: usize,
    amt: AmountCtx,
    source_name: &str,
    source_file: &Path,
    default_year: Option<i32>,
) -> Result<(PeriodicTransaction, usize), ParseError> {
    let header_no = to_u32(start + 1);
    let header_line = lines[start];
    let after_tilde = header_line
        .trim_start()
        .strip_prefix('~')
        .unwrap_or("")
        .trim_start();
    let (main, header_comment) = split_comment(after_tilde);
    // hledger requires a two-space gap between the period expression and the
    // description; `split_account_amount` splits on exactly that.
    let (period_part, desc_part) = split_account_amount(main.trim());
    let period = parse_period_spec(period_part.trim());
    let description = desc_part.trim().to_string();
    // The rule's OWN comment, built exactly as `parse_header` builds a
    // transaction's, so `; growth: 3%/yr` tags the same way in both places.
    let (mut comment, mut tags) = build_comment(header_comment);

    let mut raw_postings: Vec<RawPosting> = Vec::new();
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        // PARSE-7: as in a real transaction, a comment-only line belongs to the
        // preceding posting, or to the rule itself while no posting has been
        // seen. Verified against hledger 1.52: `print --forecast` emits a
        // pre-posting comment line above the forecast transaction's first
        // posting and a post-posting one below it, i.e. exactly this split.
        if let Some(content) = comment_line_content(line) {
            match raw_postings.last_mut() {
                Some(posting) => {
                    append_comment_line(&mut posting.comment, &mut posting.tags, content);
                }
                None => append_comment_line(&mut comment, &mut tags, content),
            }
            j += 1;
            continue;
        }
        let posting_no = to_u32(j + 1);
        let posting = parse_posting(line, posting_no, amt)
            .map_err(|e| locate(source_name, posting_no, line, e))?;
        raw_postings.push(posting);
        j += 1;
    }
    // A periodic rule's postings carry no transaction date, so a yearless
    // `date:` posting tag inherits only the `Y` default year.
    resolve_posting_dates(&mut raw_postings, default_year, lines, source_name)?;

    // Recorded BEFORE balancing fills it in (see `PeriodicTransaction::elided`).
    let elided = raw_postings
        .iter()
        .position(|posting| posting.ptype == PostingType::Regular && posting.amount.is_none());
    let postings = balance_postings(raw_postings, header_no)
        .map_err(|e| locate(source_name, header_no, header_line, e))?;
    // `j` is the index of the first line the rule did NOT consume, so its
    // 1-based number is already "the line after the last one" — the same
    // end-exclusive convention `parse_transaction` builds by hand.
    let source_span = (
        SourcePos {
            line: header_no,
            column: 1,
        },
        SourcePos {
            line: to_u32(j + 1),
            column: 1,
        },
    );
    Ok((
        PeriodicTransaction {
            period,
            description,
            comment,
            tags,
            postings,
            elided,
            source_span,
            source_file: source_file.to_path_buf(),
        },
        j,
    ))
}

/// Parse an `= QUERY` auto-posting rule and its indented postings, returning
/// the rule and the index of the first following line.
///
/// Unlike a `~` rule, the postings are NOT balanced here: an amount may be a
/// multiplier of a posting that does not exist yet, so balancing is
/// [`crate::auto`]'s job, per matched posting. A comment-only line belongs to
/// the preceding posting, or to the rule while no posting has been seen, as in
/// a transaction.
fn parse_auto_rule(
    lines: &[&str],
    start: usize,
    amt: AmountCtx,
    source_name: &str,
    source_file: &Path,
) -> Result<(AutoPostingRule, usize), ParseError> {
    let header_no = to_u32(start + 1);
    let header = lines[start].trim_start();
    // `=QUERY` and `= QUERY` alike (hledger reads both).
    let (query_text, header_comment) = split_comment(header.strip_prefix('=').unwrap_or(""));
    let (mut comment, mut tags) = build_comment(header_comment);
    let mut postings: Vec<AutoPosting> = Vec::new();
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        if let Some(content) = comment_line_content(line) {
            match postings.last_mut() {
                Some(posting) => {
                    append_comment_line(&mut posting.comment, &mut posting.tags, content)
                }
                None => append_comment_line(&mut comment, &mut tags, content),
            }
            j += 1;
            continue;
        }
        let line_no = to_u32(j + 1);
        postings.push(
            parse_auto_posting(line, line_no, amt)
                .map_err(|e| locate(source_name, line_no, line, e))?,
        );
        j += 1;
    }
    let raw: String = lines[start..j]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    Ok((
        AutoPostingRule {
            query: crate::query::Query::parse(query_text),
            postings,
            comment,
            tags,
            raw,
            source_span: (
                SourcePos {
                    line: header_no,
                    column: 1,
                },
                SourcePos {
                    line: to_u32(j + 1),
                    column: 1,
                },
            ),
            source_file: source_file.to_path_buf(),
        },
        j,
    ))
}

/// One posting line of an `=` rule: the posting grammar of
/// [`parse_posting`], with the amount read as an [`AutoAmount`].
fn parse_auto_posting(line: &str, line_no: u32, amt: AmountCtx) -> Result<AutoPosting, ParseError> {
    let (main, comment) = split_comment(line);
    let (comment, tags) = build_comment(comment);
    let trimmed = main.trim_start();
    let (status, after_status) = if let Some(r) = trimmed.strip_prefix('*') {
        (Status::Cleared, r.trim_start())
    } else if let Some(r) = trimmed.strip_prefix('!') {
        (Status::Pending, r.trim_start())
    } else {
        (Status::Unmarked, trimmed)
    };
    let (account_part, amount_part) = split_account_amount(after_status);
    let (ptype, account) = posting_type_and_account(account_part.trim());
    let expr = amount_part.trim();
    // Whether a multiplier names a commodity to re-denominate in: anything
    // beyond a signed number.
    let bare = |text: &str| {
        text.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | ' '))
    };
    let read = |text: &str| -> Result<Amount, ParseError> {
        parse_amount_and_assertion(text, main, line_no, amt)?
            .0
            .ok_or_else(|| ParseError::MalformedAmount(text.to_string()))
    };
    let amount = if expr.is_empty() {
        AutoAmount::Inferred
    } else if let Some(factor) = expr.strip_prefix('*') {
        let factor = factor.trim();
        let parsed = read(factor)?;
        AutoAmount::Scale {
            factor: parsed.quantity,
            commodity: (!bare(factor)).then_some(parsed),
        }
    } else {
        AutoAmount::Fixed(read(expr)?)
    };
    Ok(AutoPosting {
        status,
        ptype,
        account,
        amount,
        comment,
        tags,
    })
}

// ---------------------------------------------------------------------------
// Period expressions
// ---------------------------------------------------------------------------

/// The largest `every N …` multiplier this parser will accept.
///
/// The cap exists for one reason only: `0` must never reach the occurrence walk,
/// because a zero step is an infinite loop. A ceiling is thrown in beside the
/// floor because a five-digit multiplier is not a recurrence anyone wrote on
/// purpose, and an expression we refuse merely locks its rule rather than
/// refusing the journal.
const MAX_PERIOD_MULTIPLIER: u32 = 1000;

/// Read a periodic rule's period expression. **Infallible.**
///
/// The grammar is hledger 1.52's, verified by driving `print --forecast` and
/// `bal -M --budget` over scratch journals (the table in
/// `plans/21-periodic-rule-parser.md`). Anything it does not recognise —
/// including the forms hledger itself rejects — becomes
/// [`PeriodKind::Unsupported`] with [`PeriodSpec::raw`] intact. That is the whole
/// point: a period expression is one line of one rule, and a journal that
/// hledger opens must open here too.
///
/// The shape is hledger's `[INTERVAL] [in] [PERIOD]` — see [`read_period`] for
/// how the two halves are told apart and [`parse_bounds`] for every PERIOD
/// form. With no interval words the expression is a single occurrence. The
/// forms added after plan 21 (`every 1st apr`, `every weekday`,
/// `monthly in 2027`, `2027q2`, …) are tabled in
/// `plans/30-auto-postings-and-period-grammar.md`.
///
/// Shared, rather than copied, for [`crate::periodic`], whose scan must reach the
/// *same* verdict about a header as this parse does — a block's ordinal is joined
/// against `Journal::periodic_transactions` by position, so two readings that
/// disagreed about whether a line is a rule would land an edit on the wrong one.
/// It used to be a hand-kept copy of the five-word match, with a doc comment
/// asking the next maintainer to keep them in step. Sharing the function is the
/// version of that request which cannot be forgotten.
///
/// It is `pub` for the same reason one step further out: the projections run
/// endpoint takes a scenario in a request BODY, where a period arrives as the
/// words a user typed rather than as bytes already read from a file. Parsing
/// those words here is what guarantees a projected line fires on the days the
/// same line would fire on once it is saved to disk and read back — the
/// alternative is a second grammar at the HTTP boundary, which is the one place
/// a divergence would be invisible to `tests/budget_golden.rs`.
#[must_use]
pub fn parse_period_spec(raw: &str) -> PeriodSpec {
    let raw = raw.trim();
    // Case is not significant to hledger, but `raw` keeps the original bytes.
    let lower = raw.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let (kind, bounds) = read_period(&tokens).map_or(
        (PeriodKind::Unsupported, Bounds::default()),
        |(_, kind, bounds)| (kind, bounds),
    );
    PeriodSpec {
        raw: raw.to_string(),
        kind,
        start: bounds.start,
        end: bounds.end,
    }
}

/// How many leading whitespace-separated words of `raw` are its INTERVAL
/// words — everything before the bounds clause — or `None` when the
/// expression is not one [`parse_period_spec`] understands.
///
/// The same cut [`parse_period_spec`] makes, exposed so
/// [`PeriodSpec::interval_words`] can slice the original text at exactly the
/// place the parser did. A bounds clause no longer always starts with
/// `from`/`to` (`monthly in 2027`, `monthly 2027-01..2027-04`), so a scan for
/// those two words would cut in the wrong place.
#[must_use]
pub(crate) fn period_head_len(raw: &str) -> Option<usize> {
    let lower = raw.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    read_period(&tokens).map(|(head, ..)| head)
}

/// The `[start, end)` a `date:` query names (`date:2026`, `date:2026-01-02`,
/// `date:2026-01..2026-04`, `date:from 2026-03`) — the bounds grammar of a
/// period expression with no interval words, where a single date covers its
/// whole span. `None` when it is not one this engine reads.
pub(crate) fn date_query_bounds(text: &str) -> Option<(Option<String>, Option<String>)> {
    let lower = text.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let bounds = parse_bounds(&tokens, true)?;
    (bounds.start.is_some() || bounds.end.is_some()).then_some((bounds.start, bounds.end))
}

/// A period expression's bounds, ISO `YYYY-MM-DD`. `end` is EXCLUSIVE.
#[derive(Debug, Default)]
struct Bounds {
    start: Option<String>,
    end: Option<String>,
}

/// Split `tokens` into interval words and a bounds clause, and read both.
///
/// hledger's grammar is `[INTERVAL] [in] [PERIOD]`, but nothing marks where
/// the interval ends: `monthly 2027` is "monthly, during 2027", while
/// `every 1000 days` is all interval. So every cut is tried, LONGEST interval
/// first, and the first one under which both halves read is the answer. The
/// two grammars barely overlap — a bound's date always has a four-digit
/// year, an interval's numbers are multipliers and ordinals — so in practice
/// exactly one cut reads.
///
/// With no interval words at all the expression is a single occurrence
/// ([`PeriodKind::Once`]): `~ 2027-03-01`, `~ from D to D2`, `~ 2027q2`.
fn read_period(tokens: &[&str]) -> Option<(usize, PeriodKind, Bounds)> {
    (0..=tokens.len()).rev().find_map(|cut| {
        let (head, tail) = tokens.split_at(cut);
        let bounds = parse_bounds(tail, !head.is_empty())?;
        let kind = if head.is_empty() {
            PeriodKind::Once
        } else {
            parse_interval_words(head)?
        };
        Some((cut, kind, bounds))
    })
}

/// The bounds clause, in the forms hledger 1.52 accepts (each verified with
/// `print --forecast` on 2026-10-27):
///
/// - a single `D`, meaning the whole span that date names: `2027` is the
///   year, `2027-03` the month, `2027q2` the quarter. `in D` is the same;
/// - `D1..D2` or `D1-D2` as one token, or `D1 to D2`, `D1 .. D2`, `D1 - D2`;
/// - `from D` / `since D`, where `D` may itself be a joined range, or
///   `from D1 to D2` (or `..`, `-` for `to`);
/// - `to D`, `until D`, `.. D` or `- D`: an end with no start.
///
/// `until` is ONLY the last form: hledger rejects `from D until D2` and
/// `D until D2`. And `in` takes only a single date — `in D1..D2` is an error
/// there too.
///
/// Every `to` is EXCLUSIVE and uses its date's START (`to 2027q3` ends on
/// 2027-07-01).
///
/// `bounded_span` says whether a single `D` sets an end too. Under an
/// interval it must — `monthly 2027-03` fires once, in March — but a bare
/// `~ 2027-03` is a one-off whose date is its own start, and has always
/// been read with no end.
///
/// `None` — the whole expression is unsupported — for anything else,
/// including a bound with an unreadable date. Refusing is the safe reading:
/// a bound we misread would silently move every occurrence.
fn parse_bounds(tokens: &[&str], bounded_span: bool) -> Option<Bounds> {
    // `to` between two dates; `until` is not one of them.
    let is_to = |token: &str| matches!(token, "to" | ".." | "-");
    let bounds = |start: Option<&SmartDate>, end: Option<&SmartDate>| Bounds {
        start: start.map(|date| date.start.clone()),
        end: end.map(|date| date.start.clone()),
    };
    let span = |date: SmartDate| Bounds {
        start: Some(date.start),
        end: bounded_span.then_some(date.end),
    };
    match tokens {
        [] => Some(Bounds::default()),
        ["in", only] => smart_date(only).map(span),
        [only] => match period_range(only)? {
            (date, Some(to)) => Some(bounds(Some(&date), Some(&to))),
            (date, None) => Some(span(date)),
        },
        ["from" | "since", from] => {
            let (date, to) = period_range(from)?;
            Some(bounds(Some(&date), to.as_ref()))
        }
        ["from" | "since", from, to_word, to] if is_to(to_word) => {
            Some(bounds(Some(&smart_date(from)?), Some(&smart_date(to)?)))
        }
        ["to" | "until" | ".." | "-", to] => Some(bounds(None, Some(&smart_date(to)?))),
        [from, to_word, to] if is_to(to_word) => {
            Some(bounds(Some(&smart_date(from)?), Some(&smart_date(to)?)))
        }
        _ => None,
    }
}

/// One date as a period bound writes it: where it starts, and where the span
/// it names ends (exclusive).
#[derive(Debug)]
struct SmartDate {
    start: String,
    end: String,
}

/// A token holding one date, or two joined by `..` or `-` (`2027-01..2027-04`,
/// `2027-01-2027-04` — hledger reads both, verified).
///
/// A whole-token date is tried first, so `2027-01-15` is a day and never
/// `2027-01` to `15`; a dash split also needs a full four-digit year on its
/// right, which is what keeps the two readings apart.
fn period_range(token: &str) -> Option<(SmartDate, Option<SmartDate>)> {
    if let Some(date) = smart_date(token) {
        return Some((date, None));
    }
    if let Some((from, to)) = token.split_once("..") {
        return Some((smart_date(from)?, Some(smart_date(to)?)));
    }
    token.match_indices('-').find_map(|(at, _)| {
        Some((
            smart_date(&token[..at])?,
            Some(smart_date(&token[at + 1..])?),
        ))
    })
}

/// A date as a period bound writes it, with the span it names:
///
/// - `YYYY` — the year;
/// - `YYYY-MM`, or six digits `YYYYMM` — the month;
/// - `YYYY-MM-DD`, or eight digits `YYYYMMDD` — the day;
/// - `YYYYqN` — the quarter.
///
/// `-`, `/` and `.` all separate. hledger expands `from 2027` to `2027-01-01`
/// and `to 2027-06` to `2027-06-01` (both verified), i.e. a bound is its
/// span's first day.
///
/// Deliberately **not** [`normalize_date`]. That function reads a
/// two-component token as a yearless `MM-DD` against the `Y` default year —
/// the right reading for a transaction date and the wrong one here, because
/// hledger reads `from 2027-01` as January 2027, not as the 1st of some other
/// year. The year must therefore be written in full, which is also what
/// disambiguates the two. Relative dates (`jan`, `this year`, `q4`) are
/// refused: a projection must not change meaning overnight.
fn smart_date(token: &str) -> Option<SmartDate> {
    fn number(text: &str) -> Option<i32> {
        text.parse::<i32>().ok()
    }
    let all_digits = token.bytes().all(|byte| byte.is_ascii_digit());
    let (year, month, day, months) = if let Some((year, quarter)) = token.split_once('q') {
        let quarter = number(quarter).filter(|q| (1..=4).contains(q))?;
        (four_digit_year(year)?, quarter * 3 - 2, None, 3)
    } else if all_digits && token.len() == 6 {
        (four_digit_year(&token[..4])?, number(&token[4..])?, None, 1)
    } else if all_digits && token.len() == 8 {
        let day = number(&token[6..])?;
        (
            four_digit_year(&token[..4])?,
            number(&token[4..6])?,
            Some(day),
            0,
        )
    } else {
        let components: Vec<&str> = token.split(['-', '/', '.']).collect();
        match components.as_slice() {
            [year] => (four_digit_year(year)?, 1, None, 12),
            [year, month] => (four_digit_year(year)?, number(month)?, None, 1),
            [year, month, day] => (
                four_digit_year(year)?,
                number(month)?,
                Some(number(day)?),
                0,
            ),
            _ => return None,
        }
    };
    if !(1..=12).contains(&month) {
        return None;
    }
    if let Some(day) = day
        && !(1..=days_in_month(year, month)).contains(&day)
    {
        return None;
    }
    let iso = |(year, month, day): (i32, i32, i32)| format!("{year:04}-{month:02}-{day:02}");
    let start = (year, month, day.unwrap_or(1));
    let end = match day {
        // A day's span ends on the next day.
        Some(day) if day < days_in_month(year, month) => (year, month, day + 1),
        Some(_) => add_months(year, month, 1),
        None => add_months(year, month, months),
    };
    Some(SmartDate {
        start: iso(start),
        end: iso(end),
    })
}

/// Four ASCII digits, as a year in `1..=9999`. Four, always: it is what tells
/// `2027-01` (a month) apart from a yearless `MM-DD`, and it is what hledger's
/// own period grammar requires.
fn four_digit_year(text: &str) -> Option<i32> {
    (text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<i32>().ok())
        .flatten()
        .filter(|year| (1..=9999).contains(year))
}

/// The first day of the month `months` after `year`-`month`.
fn add_months(year: i32, month: i32, months: i32) -> (i32, i32, i32) {
    let index = year * 12 + (month - 1) + months;
    (index.div_euclid(12), index.rem_euclid(12) + 1, 1)
}

/// The interval half of a period expression — everything before the bounds.
fn parse_interval_words(tokens: &[&str]) -> Option<PeriodKind> {
    let every = |unit, multiplier| Some(PeriodKind::Every { unit, multiplier });
    match tokens {
        ["daily"] => every(PeriodExpr::Daily, 1),
        ["weekly"] => every(PeriodExpr::Weekly, 1),
        ["monthly"] => every(PeriodExpr::Monthly, 1),
        ["quarterly"] => every(PeriodExpr::Quarterly, 1),
        ["yearly"] => every(PeriodExpr::Yearly, 1),
        // hledger's own synonyms, verified to step by two units each.
        ["biweekly"] | ["fortnightly"] => every(PeriodExpr::Weekly, 2),
        ["bimonthly"] => every(PeriodExpr::Monthly, 2),
        ["every", rest @ ..] => parse_every(rest),
        _ => None,
    }
}

/// The words after `every`: the forms of hledger's "More complex report
/// intervals" and "Multiple weekday intervals", each verified against 1.52.
///
/// `of month` / `of year` are optional where hledger makes them optional
/// (`every 15th day`, `every 3rd mon`, `every 1st apr`), and required nowhere.
fn parse_every(tokens: &[&str]) -> Option<PeriodKind> {
    match tokens {
        [word] => every_word(word),
        [first, second] => every_pair(first, second),
        [nth, "day", "of", "week"] => ordinal(nth, 7).map(|weekday| PeriodKind::Anchored {
            unit: PeriodExpr::Weekly,
            anchor: Anchor::Weekday(weekday),
        }),
        [nth, what, "of", "month"] => monthly_anchor(nth, what),
        [first, second, "of", "year"] => annual(first, second),
        [month_and_day, "of", "year"] => {
            month_day(month_and_day).map(|(month, day)| PeriodKind::Annual { month, day })
        }
        _ => None,
    }
}

/// `every UNIT`, `every WEEKDAY`, `every weekday` / `every weekendday`,
/// `every mon,wed,fri`, `every MM/DD`.
fn every_word(word: &str) -> Option<PeriodKind> {
    if let Some(unit) = period_unit(word) {
        return Some(PeriodKind::Every {
            unit,
            multiplier: 1,
        });
    }
    let weekly = |anchor| {
        Some(PeriodKind::Anchored {
            unit: PeriodExpr::Weekly,
            anchor,
        })
    };
    if let Some(weekday) = weekday_number(word) {
        return weekly(Anchor::Weekday(weekday));
    }
    match word {
        // hledger's documented shorthands for `mon,tue,wed,thu,fri` and
        // `sat,sun`.
        "weekday" => return weekly(Anchor::Weekdays(0b001_1111)),
        "weekendday" => return weekly(Anchor::Weekdays(0b110_0000)),
        _ => {}
    }
    if word.contains(',') {
        let mask = word
            .split(',')
            .map(|day| weekday_number(day).map(|weekday| 1u8 << (weekday - 1)))
            .try_fold(0u8, |mask, bit| bit.map(|bit| mask | bit))?;
        return weekly(Anchor::Weekdays(mask));
    }
    // `every 12/25`: one occurrence a year on a fixed month/day.
    month_day(word).map(|(month, day)| PeriodKind::Annual { month, day })
}

/// Two words after `every`: a multiplied unit (`every 2 weeks`), a monthly
/// anchor without its optional `of month` (`every 15th day`, `every 3rd mon`),
/// or a yearly one without `of year` (`every 1st apr`, `every apr 1st`).
fn every_pair(first: &str, second: &str) -> Option<PeriodKind> {
    if let Some(multiplier) = period_multiplier(first) {
        // `every 2 weeks`, never `every 2 weekly`: hledger rejects the adverb
        // after a count (verified).
        return period_unit(second)
            .filter(|_| !second.ends_with("ly"))
            .map(|unit| PeriodKind::Every { unit, multiplier });
    }
    monthly_anchor(first, second).or_else(|| annual(first, second))
}

/// `Nth day` or `Nth WEEKDAY`, of each month.
fn monthly_anchor(nth: &str, what: &str) -> Option<PeriodKind> {
    let monthly = |anchor| {
        Some(PeriodKind::Anchored {
            unit: PeriodExpr::Monthly,
            anchor,
        })
    };
    if what == "day" {
        return monthly(Anchor::DayOfMonth(ordinal(nth, 31)?));
    }
    let weekday = weekday_number(what)?;
    // Five is the most any weekday can occur in a month.
    monthly(Anchor::NthWeekday {
        nth: ordinal(nth, 5)?,
        weekday,
    })
}

/// `Nth MONTHNAME` or `MONTHNAME Nth`: once a year.
///
/// The ordinal suffix is REQUIRED here — hledger rejects `every 1 apr` and
/// `every apr 1` (verified) — and a day past the month's end is accepted, as
/// hledger accepts it: `every 30th feb` fires on the date February 1st plus
/// 29 days lands on. [`crate::reports::budget`] reproduces that.
fn annual(first: &str, second: &str) -> Option<PeriodKind> {
    let (nth, month) = match month_number(second) {
        Some(month) => (first, month),
        None => (second, month_number(first)?),
    };
    let day = ordinal_with_suffix(nth, 31)?;
    Some(PeriodKind::Annual { month, day })
}

/// One of the five units, singular or plural (`week`, `weeks`, `weekly`).
fn period_unit(word: &str) -> Option<PeriodExpr> {
    Some(match word {
        "day" | "days" | "daily" => PeriodExpr::Daily,
        "week" | "weeks" | "weekly" => PeriodExpr::Weekly,
        "month" | "months" | "monthly" => PeriodExpr::Monthly,
        "quarter" | "quarters" | "quarterly" => PeriodExpr::Quarterly,
        "year" | "years" | "yearly" => PeriodExpr::Yearly,
        _ => return None,
    })
}

/// The `N` of `every N units`. Zero is refused — see [`MAX_PERIOD_MULTIPLIER`].
fn period_multiplier(word: &str) -> Option<u32> {
    word.parse::<u32>()
        .ok()
        .filter(|n| (1..=MAX_PERIOD_MULTIPLIER).contains(n))
}

/// An ordinal (`15th`, `3rd`, `1st`, or a bare integer), bounded by `max`.
fn ordinal(word: &str, max: u32) -> Option<u32> {
    let digits = strip_ordinal_suffix(word).unwrap_or(word);
    digits.parse::<u32>().ok().filter(|n| (1..=max).contains(n))
}

/// An ordinal that MUST carry its `st`/`nd`/`rd`/`th` suffix.
fn ordinal_with_suffix(word: &str, max: u32) -> Option<u32> {
    strip_ordinal_suffix(word).and_then(|digits| ordinal(digits, max))
}

fn strip_ordinal_suffix(word: &str) -> Option<&str> {
    ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|suffix| word.strip_suffix(*suffix))
}

/// A weekday name or its three-letter abbreviation, as an ISO number
/// (1 = Monday), matching `reports::periods`' convention.
fn weekday_number(word: &str) -> Option<u32> {
    Some(match word {
        "mon" | "monday" => 1,
        "tue" | "tuesday" => 2,
        "wed" | "wednesday" => 3,
        "thu" | "thursday" => 4,
        "fri" | "friday" => 5,
        "sat" | "saturday" => 6,
        "sun" | "sunday" => 7,
        _ => return None,
    })
}

/// A month name or its three-letter abbreviation, as 1-12.
fn month_number(word: &str) -> Option<u32> {
    Some(match word {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    })
}

/// The `12/25` of `every 12/25` — a month and a day, with no year.
///
/// The day must exist in that month in SOME year: hledger rejects
/// `every 2/30` and `every 4/31` but takes `every 2/29` (all verified), which
/// it fires on March 1st in a non-leap year.
fn month_day(token: &str) -> Option<(u32, u32)> {
    let (month, day) = token.split_once(['/', '-', '.'])?;
    let month = month.parse::<u32>().ok().filter(|m| (1..=12).contains(m))?;
    let day = day.parse::<u32>().ok()?;
    // 2000 is a leap year, so February admits its 29th.
    let longest = u32::try_from(days_in_month(2000, i32::try_from(month).ok()?)).ok()?;
    (1..=longest).contains(&day).then_some((month, day))
}
// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Parsed transaction header fields.
struct Header {
    date: String,
    date2: Option<String>,
    status: Status,
    code: String,
    description: String,
    comment: String,
    tags: Vec<(String, String)>,
}

/// A posting before amount inference (its amount may be `None` if elided).
struct RawPosting {
    status: Status,
    ptype: PostingType,
    account: String,
    amount: Option<Amount>,
    balance_assertion: Option<BalanceAssertion>,
    date: Option<String>,
    date2: Option<String>,
    comment: String,
    tags: Vec<(String, String)>,
    /// 1-based line the posting itself was written on. Kept because its dates
    /// can only be resolved once every following continuation comment line has
    /// been merged (PARSE-7), by which point the line is no longer in hand.
    line: u32,
}

fn parse_transaction(
    lines: &[&str],
    start: usize,
    tindex: u32,
    amt: AmountCtx,
    source_name: &str,
    source_file: &Path,
    default_year: Option<i32>,
) -> Result<(Transaction, usize), ParseError> {
    let header_no = to_u32(start + 1);
    let header =
        parse_header(lines[start]).map_err(|e| locate(source_name, header_no, lines[start], e))?;
    let date = normalize_date(&header.date, default_year)
        .map_err(|e| locate(source_name, header_no, lines[start], e))?;
    let date2 = header
        .date2
        .as_deref()
        .map(|d| normalize_date(d, default_year))
        .transpose()
        .map_err(|e| locate(source_name, header_no, lines[start], e))?;
    // A posting's `date:`/`date2:` tag may be yearless (`3/4`); hledger infers
    // the year from the transaction's primary date.
    let txn_year = iso_year(&date);

    let mut header = header;
    let mut raw_postings: Vec<RawPosting> = Vec::new();
    let mut last_body_line = header_no;
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        last_body_line = to_u32(j + 1);
        // PARSE-7: an indented comment-only line inside the body used to be
        // dropped outright, losing its text AND its tags. hledger attaches it to
        // the preceding posting, or to the transaction itself while no posting
        // has been seen — which is what makes a `subscription: false` override
        // written on its own line work.
        if let Some(content) = comment_line_content(line) {
            match raw_postings.last_mut() {
                Some(posting) => {
                    append_comment_line(&mut posting.comment, &mut posting.tags, content);
                }
                None => append_comment_line(&mut header.comment, &mut header.tags, content),
            }
            j += 1;
            continue;
        }
        let posting_no = last_body_line;
        let posting = parse_posting(line, posting_no, amt)
            .map_err(|e| locate(source_name, posting_no, line, e))?;
        raw_postings.push(posting);
        j += 1;
    }
    // Only now is each posting's comment complete, so only now can its `date:`/
    // `date2:` tags and `[DATE=DATE2]` brackets be read (either may be written
    // on a continuation line).
    resolve_posting_dates(&mut raw_postings, txn_year, lines, source_name)?;

    let postings = balance_postings(raw_postings, header_no)
        .map_err(|e| locate(source_name, header_no, lines[start], e))?;
    let source_span = (
        SourcePos {
            line: header_no,
            column: 1,
        },
        // hledger's end position is the line after the last line the transaction
        // consumed — a trailing comment line included.
        SourcePos {
            line: last_body_line.saturating_add(1),
            column: 1,
        },
    );

    let transaction = Transaction {
        index: Tindex(tindex),
        date,
        date2,
        status: header.status,
        code: header.code,
        description: header.description,
        comment: header.comment,
        preceding_comment: String::new(),
        tags: header.tags,
        postings,
        source_span,
        source_file: source_file.to_path_buf(),
    };
    Ok((transaction, j))
}

/// The text after `;` on an indented comment-ONLY line, or `None` when the line
/// is not one. Comment text is stored trimmed, as [`build_comment`] stores an
/// inline comment.
fn comment_line_content(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix(';').map(str::trim)
}

/// Append a continuation comment line's `content` to an accumulating
/// comment/tag pair (PARSE-7).
///
/// hledger models a comment as its lines joined by `\n` with a trailing `\n`,
/// where the FIRST line is the same-line comment — empty when there was none.
/// So a lone continuation line on a posting with no inline comment yields a
/// LEADING newline (`"\nsubscription: false\n"`, exactly what hledger emits),
/// while one following an inline comment simply extends it
/// (`"own: x\nmore: y\n"`).
fn append_comment_line(comment: &mut String, tags: &mut Vec<(String, String)>, content: &str) {
    if comment.is_empty() {
        comment.push('\n');
    }
    comment.push_str(content);
    comment.push('\n');
    tags.extend(parse_tags(content));
}

/// Resolve every posting's `date`/`date2` from its now-complete comment.
///
/// Errors are located at the posting's own line: a bad date may have arrived on
/// a continuation line, but the posting is the unambiguous anchor for it.
fn resolve_posting_dates(
    postings: &mut [RawPosting],
    txn_year: Option<i32>,
    lines: &[&str],
    source_name: &str,
) -> Result<(), ParseError> {
    for posting in postings.iter_mut() {
        let line_text = lines
            .get(posting.line.saturating_sub(1) as usize)
            .copied()
            .unwrap_or_default();
        let (date, date2) = posting_dates(&posting.comment, &posting.tags, txn_year)
            .map_err(|e| locate(source_name, posting.line, line_text, e))?;
        posting.date = date;
        posting.date2 = date2;
    }
    Ok(())
}

fn parse_header(line: &str) -> Result<Header, ParseError> {
    let (main, comment) = split_comment(line);
    let (comment_text, tags) = build_comment(comment);

    let rest = main.trim();
    let (date_token, after_date) = match rest.find(char::is_whitespace) {
        Some(pos) => (&rest[..pos], rest[pos..].trim_start()),
        None => (rest, ""),
    };
    let (date, date2) = split_date(date_token);

    let (status, after_status) = if let Some(r) = after_date.strip_prefix('*') {
        (Status::Cleared, r.trim_start())
    } else if let Some(r) = after_date.strip_prefix('!') {
        (Status::Pending, r.trim_start())
    } else {
        (Status::Unmarked, after_date)
    };

    let (code, after_code) = if let Some(r) = after_status.strip_prefix('(') {
        match r.find(')') {
            Some(close) => (r[..close].to_string(), r[close + 1..].trim_start()),
            None => (String::new(), after_status),
        }
    } else {
        (String::new(), after_status)
    };

    Ok(Header {
        date,
        date2,
        status,
        code,
        description: after_code.trim().to_string(),
        comment: comment_text,
        tags,
    })
}

fn split_date(token: &str) -> (String, Option<String>) {
    match token.split_once('=') {
        Some((primary, secondary)) => (primary.to_string(), Some(secondary.to_string())),
        None => (token.to_string(), None),
    }
}

/// Parse one posting line. Its `date`/`date2` are left unresolved — see
/// [`resolve_posting_dates`], which fills them in once any continuation comment
/// lines have been merged.
fn parse_posting(line: &str, line_no: u32, amt: AmountCtx) -> Result<RawPosting, ParseError> {
    let (main, comment) = split_comment(line);
    let (comment_text, tags) = build_comment(comment);

    let trimmed = main.trim_start();
    let (status, after_status) = if let Some(r) = trimmed.strip_prefix('*') {
        (Status::Cleared, r.trim_start())
    } else if let Some(r) = trimmed.strip_prefix('!') {
        (Status::Pending, r.trim_start())
    } else {
        (Status::Unmarked, trimmed)
    };

    let (account_part, amount_part) = split_account_amount(after_status);
    let (ptype, account) = posting_type_and_account(account_part.trim());
    let amount_expr = amount_part.trim();

    let (amount, balance_assertion) = if amount_expr.is_empty() {
        (None, None)
    } else {
        parse_amount_and_assertion(amount_expr, main, line_no, amt)?
    };

    Ok(RawPosting {
        status,
        ptype,
        account,
        amount,
        balance_assertion,
        date: None,
        date2: None,
        comment: comment_text,
        tags,
        line: line_no,
    })
}

/// Whether a posting source line **elides its amount**, leaving the value for
/// the balancing pass to infer.
///
/// # Why the write path asks
/// An elided amount is not cosmetic. Once the inferred value is written out
/// explicitly, that leg can no longer disagree with the others, so hledger's own
/// imbalance detection is permanently disabled for the transaction — the very
/// check that catches a mistyped amount.
///
/// # What counts as elided
/// A bare account, and also a posting whose only content after the account is a
/// balance assertion (`assets:cash    = $99.00`, the reconcile-to-statement
/// idiom): [`parse_amount_and_assertion`] records `amount: None` for that too.
///
/// Shares [`split_comment`] and [`split_account_amount`] with [`parse_posting`],
/// so the answer is by construction the one the parser reached. A second,
/// independent notion of "has an amount" would be exactly the drift this exists
/// to prevent.
pub(crate) fn posting_line_elides_amount(line: &str) -> bool {
    let (main, _) = split_comment(line);
    let trimmed = main.trim_start();
    let after_status = trimmed
        .strip_prefix(['*', '!'])
        .map_or(trimmed, str::trim_start);
    let amount_expr = split_account_amount(after_status).1.trim();
    // Mirrors `parse_amount_and_assertion`: the written amount is whatever
    // precedes the first `=`.
    amount_expr
        .find('=')
        .map_or(amount_expr, |eq| &amount_expr[..eq])
        .trim()
        .is_empty()
}

/// Classify a posting's account field by its wrapping brackets and return the
/// bare account name: `(a)` -> unbalanced virtual, `[a]` -> balanced virtual,
/// anything else (including parens/brackets in the middle) -> regular.
fn posting_type_and_account(account: &str) -> (PostingType, String) {
    if let Some(inner) = account.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        (PostingType::Virtual, inner.trim().to_string())
    } else if let Some(inner) = account.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        (PostingType::BalancedVirtual, inner.trim().to_string())
    } else {
        (PostingType::Regular, account.to_string())
    }
}

/// A posting's `(date, date2)`, read from its complete comment: the `date:` /
/// `date2:` tags first, then hledger's bracket shorthand as a fallback.
fn posting_dates(
    comment: &str,
    tags: &[(String, String)],
    txn_year: Option<i32>,
) -> Result<(Option<String>, Option<String>), ParseError> {
    let (bracket_date, bracket_date2) = bracket_posting_dates(comment, txn_year)?;
    Ok((
        posting_date_tag(tags, "date", txn_year)?.or(bracket_date),
        posting_date_tag(tags, "date2", txn_year)?.or(bracket_date2),
    ))
}

/// The ISO-normalized value of the first posting comment tag named `key`
/// (`date`/`date2`), or `None` when absent. Yearless values take `txn_year`.
fn posting_date_tag(
    tags: &[(String, String)],
    key: &str,
    txn_year: Option<i32>,
) -> Result<Option<String>, ParseError> {
    match tags.iter().find(|(k, _)| k == key) {
        Some((_, value)) => Ok(Some(normalize_date(value.trim(), txn_year)?)),
        None => Ok(None),
    }
}

/// hledger's bracket posting-date shorthand, written anywhere in a posting
/// comment: `[DATE]`, `[=DATE2]` or `[DATE=DATE2]` (PARSE-9).
///
/// Only the `date:`/`date2:` tag spellings were recognised before, so a posting
/// written the bracket way kept the transaction's date and was bucketed in the
/// wrong period by every periodic report.
///
/// A bracketed group containing anything other than date characters is prose
/// (`; see [note]`), not a date, and is ignored exactly as hledger ignores it —
/// but one that IS all date characters must parse, so `[2024-02-30]` is the same
/// hard error hledger raises. The brackets stay in the stored comment text, and
/// produce no tag, matching hledger.
fn bracket_posting_dates(
    comment: &str,
    txn_year: Option<i32>,
) -> Result<(Option<String>, Option<String>), ParseError> {
    let date_chars = |group: &str| {
        group
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '/' | '.' | '='))
    };
    for group in bracketed_groups(comment).filter(|g| !g.is_empty() && date_chars(g)) {
        let (primary, secondary) = match group.split_once('=') {
            Some((primary, secondary)) => (primary, secondary),
            None => (group, ""),
        };
        let date = optional_date(primary, txn_year)?;
        let date2 = optional_date(secondary, txn_year)?;
        if date.is_some() || date2.is_some() {
            return Ok((date, date2));
        }
    }
    Ok((None, None))
}

/// Normalize `token` unless it is empty (the `[=DATE2]` / `[DATE=]` halves).
fn optional_date(token: &str, txn_year: Option<i32>) -> Result<Option<String>, ParseError> {
    if token.is_empty() {
        return Ok(None);
    }
    normalize_date(token, txn_year).map(Some)
}

/// Each `[...]`-bracketed group in `text`, in order, without its brackets.
fn bracketed_groups(text: &str) -> impl Iterator<Item = &str> {
    text.split('[')
        .skip(1)
        .filter_map(|rest| rest.split_once(']').map(|(inner, _)| inner))
}

/// The year component of an ISO `YYYY-MM-DD` date string.
fn iso_year(date: &str) -> Option<i32> {
    date.split('-').next().and_then(|year| year.parse().ok())
}

fn parse_amount_and_assertion(
    expr: &str,
    main: &str,
    line_no: u32,
    amt: AmountCtx,
) -> Result<(Option<Amount>, Option<BalanceAssertion>), ParseError> {
    if let Some(eq) = expr.find('=') {
        let amount_str = expr[..eq].trim();
        let after = &expr[eq..]; // starts with '='
        let total = after.starts_with("==");
        let after = after.trim_start_matches('=');
        let inclusive = after.starts_with('*');
        let assertion_str = after.trim_start_matches('*').trim();

        // PARSE-9: an asserted amount may carry a cost (`= 10 AAA @ $5.00`),
        // which hledger accepts and records on `baamount.acost`. Routing the
        // assertion text through `parse_amount` instead rejected the whole
        // journal with `malformed amount: '10 AAA @ $5.00'`. (The cost plays no
        // part in evaluating the assertion — see `crate::assertions`.)
        let assertion_amount = parse_primary_and_cost(assertion_str, amt)?;
        let column = main
            .chars()
            .position(|c| c == '=')
            .map_or(1, |p| to_u32(p + 1));
        // PARSE-9: `    a       = $150.00` — the reconcile-to-statement idiom —
        // is a posting with NO amount, only an assertion. It used to fail the
        // whole journal with `malformed amount: ''`; it is an elided posting,
        // and the balancing pass infers its amount like any other.
        let amount = if amount_str.is_empty() {
            None
        } else {
            Some(parse_primary_and_cost(amount_str, amt)?)
        };
        let assertion = BalanceAssertion {
            amount: assertion_amount,
            inclusive,
            total,
            position: SourcePos {
                line: line_no,
                column,
            },
        };
        Ok((amount, Some(assertion)))
    } else {
        Ok((Some(parse_primary_and_cost(expr, amt)?), None))
    }
}

/// A lot cost annotation: `{UNITPRICE}` or `{{TOTALPRICE}}`.
struct LotCost {
    price: String,
    total: bool,
}

/// Split a trailing lot annotation off an amount expression (PARSE-5).
///
/// Recognises `{UNITPRICE}` / `{{TOTALPRICE}}` written after the quantity and
/// before any `@`/`@@` cost, plus an optional following lot date `[DATE]`
/// (which hledger accepts and ignores for valuation, as we do). Returns the
/// expression with the annotation removed, plus the lot price if one was given.
///
/// Without this the whole annotation fell into the commodity name, so
/// `10 AAPL {$5.00}` became 10 of the commodity `AAPL {$5.00}` — a second,
/// bogus position alongside any plain `AAPL` holding.
fn split_lot_notation(expr: &str) -> Result<(String, Option<LotCost>), ParseError> {
    let malformed = || ParseError::MalformedAmount(expr.trim().to_string());
    let Some(open) = expr.find('{') else {
        // A closing brace with nothing to close is malformed, not a commodity.
        if expr.contains('}') {
            return Err(malformed());
        }
        return Ok((expr.to_string(), None));
    };
    let total = expr[open + 1..].starts_with('{');
    let (inner_start, closer) = if total {
        (open + 2, "}}")
    } else {
        (open + 1, "}")
    };
    let close = expr
        .get(inner_start..)
        .and_then(|rest| rest.find(closer))
        .ok_or_else(malformed)?
        + inner_start;
    let price = expr[inner_start..close].trim().to_string();

    let mut rest = expr[close + closer.len()..].trim_start();
    if let Some(after_open) = rest.strip_prefix('[') {
        let end = after_open.find(']').ok_or_else(malformed)?;
        rest = after_open[end + 1..].trim_start();
    }
    // Only one annotation is allowed, and a lot label (`"lot1"`) is a parse
    // error in hledger 1.52 — anything else left over is malformed.
    if rest.contains(['{', '}', '[', ']', '"']) {
        return Err(malformed());
    }

    // `{}` is an empty lot: hledger accepts it and records no cost.
    let lot = (!price.is_empty()).then_some(LotCost { price, total });
    Ok((format!("{} {rest}", &expr[..open]).trim().to_string(), lot))
}

/// The cost hledger derives from a lot annotation: always a **total** cost, and
/// deliberately *not* normalized (`10 AAPL {$5.00}` yields `$50.00` at scale 2,
/// where `@@ $50.00` yields `$50` at scale 0).
fn cost_from_lot(quantity: Dec, lot: &LotCost, amt: AmountCtx) -> Result<Cost, ParseError> {
    let mut price = parse_amount(lot.price.trim(), amt)?;
    if !lot.total {
        // Scale the unit lot price by the quantity, keeping both scales (so the
        // written precision survives) rather than using `Dec::mul`, which
        // normalizes trailing zeros away.
        let mantissa = quantity
            .mantissa
            .checked_mul(price.quantity.mantissa)
            .ok_or(DecError::Overflow)?;
        let places = quantity
            .places
            .checked_add(price.quantity.places)
            .ok_or(DecError::Overflow)?;
        price.quantity = Dec::new(mantissa, places);
    }
    Ok(Cost {
        kind: CostKind::Total,
        amount: price,
    })
}

/// Parse `AMOUNT [{LOTPRICE}] [@ PRICE | @@ PRICE]` into an amount with an
/// optional cost. An explicit `@`/`@@` cost overrides the lot price, matching
/// hledger.
fn parse_primary_and_cost(expr: &str, amt: AmountCtx) -> Result<Amount, ParseError> {
    let (without_lot, lot) = split_lot_notation(expr)?;
    let expr = without_lot.as_str();
    if let Some((primary, price)) = expr.split_once("@@") {
        let mut amount = parse_amount(primary.trim(), amt)?;
        let mut cost_amount = parse_amount(price.trim(), amt)?;
        // hledger stores a total-cost (@@) amount at its natural scale (trailing
        // zeros stripped), while keeping the as-written display precision.
        cost_amount.quantity = cost_amount.quantity.normalized();
        amount.cost = Some(Box::new(Cost {
            kind: CostKind::Total,
            amount: cost_amount,
        }));
        Ok(amount)
    } else if let Some((primary, price)) = expr.split_once('@') {
        let mut amount = parse_amount(primary.trim(), amt)?;
        let cost_amount = parse_amount(price.trim(), amt)?;
        amount.cost = Some(Box::new(Cost {
            kind: CostKind::Unit,
            amount: cost_amount,
        }));
        Ok(amount)
    } else {
        let mut amount = parse_amount(expr.trim(), amt)?;
        if let Some(lot) = &lot {
            amount.cost = Some(Box::new(cost_from_lot(amount.quantity, lot, amt)?));
        }
        Ok(amount)
    }
}

/// Parse a single commodity+quantity token, applying the commodity's canonical
/// style (with as-written precision). Undeclared commodities honor a journal
/// `decimal-mark` default before falling back to literal inference.
fn parse_amount(token: &str, amt: AmountCtx) -> Result<Amount, ParseError> {
    let (symbol, number, side, spaced) = split_commodity_number(token)?;

    // A bare number adopts the default commodity and its full style (including
    // its precision) from a `D` directive, if one is in effect.
    if symbol.is_empty()
        && let Some((commodity, style)) = amt.default_commodity
    {
        let quantity = Dec::parse(&number, style.decimal_mark.unwrap_or('.'))?;
        return Ok(Amount {
            commodity: commodity.clone(),
            quantity,
            style: style.clone(),
            cost: None,
        });
    }

    let commodity = Commodity(symbol);
    // The text's own declaration first; then one inherited from outside it,
    // unless the text set its own `decimal-mark` (which then outranks a style
    // it never saw).
    let canonical = amt.styles.get(&commodity).or_else(|| {
        amt.default_mark
            .is_none()
            .then(|| amt.inherited.get(&commodity))
            .flatten()
    });
    // Infer the literal's own mark/groups first: it is both the last resort for
    // the decimal mark and (for an undeclared commodity) the display style.
    // PARSE-3 — this inference used to be computed only for the style and then
    // thrown away for parsing, so `1,50 CHF` read as 150 instead of 1.50.
    let (inferred_mark, inferred_groups, _) = analyze_number(&number, amt.default_mark);
    let decimal_mark = canonical
        .and_then(|style| style.decimal_mark)
        .or(amt.default_mark)
        .or(inferred_mark);
    let quantity = Dec::parse_with_mark(&number, decimal_mark)?;
    let precision = quantity.places;

    let style = match canonical {
        Some(style) => AmountStyle {
            side: style.side,
            spaced: style.spaced,
            decimal_mark: style.decimal_mark,
            digit_groups: style.digit_groups.clone(),
            precision,
        },
        None => {
            // Undeclared commodity: infer the format from the literal, honoring
            // a declared `decimal-mark` default when present.
            AmountStyle {
                side,
                spaced,
                // hledger emits "." even for a no-decimal amount of a commodity
                // that is never written with one (e.g. `$1`), so default to
                // Some here. The true per-commodity canonical null (a commodity
                // seen ONLY without a decimal, e.g. `-1712 D`) needs a post-parse
                // style pass — that's the still-open `precision` corpus gap.
                decimal_mark: Some(
                    inferred_mark
                        .unwrap_or_else(|| default_display_mark(inferred_groups.as_deref())),
                ),
                digit_groups: inferred_groups,
                precision,
            }
        }
    };

    Ok(Amount {
        commodity,
        quantity,
        style,
        cost: None,
    })
}

// ---------------------------------------------------------------------------
// Amount inference (balancing)
// ---------------------------------------------------------------------------

/// One commodity's running balance while inferring an elided posting: the
/// signed total, the maximum contributing precision, and the display style of
/// the first contributing amount. Any inferred leg adopts that style (its
/// side/spacing/mark/groups), so a right-side or grouped commodity infers
/// correctly instead of defaulting to a bare left-side style.
struct CommoditySum {
    commodity: Commodity,
    total: Dec,
    precision: u32,
    style: AmountStyle,
}

/// Fill in the single elided posting (if any) so the transaction balances per
/// commodity, then finalize all postings.
///
/// Real and balanced-virtual (`[a]`) postings balance within their own separate
/// groups; an unbalanced virtual (`(a)`) posting is excluded from balancing (and
/// only ever keeps an explicit amount here).
fn balance_postings(raw: Vec<RawPosting>, line_no: u32) -> Result<Vec<Posting>, ParseError> {
    let regular_sums = group_sums(&raw, PostingType::Regular, line_no)?;
    let balanced_virtual_sums = group_sums(&raw, PostingType::BalancedVirtual, line_no)?;

    raw.into_iter()
        .map(|posting| {
            let sums = match posting.ptype {
                PostingType::Regular => regular_sums.as_slice(),
                PostingType::BalancedVirtual => balanced_virtual_sums.as_slice(),
                PostingType::Virtual => &[],
            };
            finalize_posting(posting, sums)
        })
        .collect()
}

/// Accumulate the per-commodity balance of the `ptype` postings, preserving
/// first-seen order, the maximum contributing precision, and the first-seen
/// style. Errors if more than one posting in the group is elided (only one can
/// be inferred).
///
/// # Why the index map
/// The lookup used to be `sums.iter_mut().find(...)`, a linear scan of the
/// running list per posting — quadratic in the number of distinct commodities in
/// one transaction. That is reachable from a journal (a single transaction with
/// 8k commodities took 16x the time of one with 2k, on a file that grew 4x), and
/// a journal is attacker-influenced input. [`group_residual`] already solves the
/// same shape with a `BTreeMap`, so this uses one too.
///
/// The map holds an INDEX rather than the sum itself, because first-seen order
/// is observable: [`finalize_posting`] emits the inferred leg's amounts in
/// `sums` order, and `C10` sorts before `C2`. Keeping the `Vec` keeps the order;
/// the map only replaces the scan.
fn group_sums(
    raw: &[RawPosting],
    ptype: PostingType,
    line_no: u32,
) -> Result<Vec<CommoditySum>, ParseError> {
    let group = || raw.iter().filter(|posting| posting.ptype == ptype);
    if group().filter(|posting| posting.amount.is_none()).count() > 1 {
        return Err(ParseError::MultipleElidedPostings(line_no));
    }

    let mut sums: Vec<CommoditySum> = Vec::new();
    let mut index: BTreeMap<Commodity, usize> = BTreeMap::new();
    for posting in group() {
        if let Some(amount) = &posting.amount {
            let (commodity, quantity, precision, style) = cost_contribution(amount)?;
            match index.get(&commodity).copied() {
                Some(at) => {
                    let entry = &mut sums[at];
                    entry.total = entry.total.add(quantity)?;
                    entry.precision = entry.precision.max(precision);
                }
                None => {
                    // Cloned only on first sight, so this is one extra clone per
                    // distinct commodity, not per posting.
                    index.insert(commodity.clone(), sums.len());
                    sums.push(CommoditySum {
                        commodity,
                        total: quantity,
                        precision,
                        style,
                    });
                }
            }
        }
    }
    Ok(sums)
}

/// A posting's contribution to the transaction balance: its cost value (and the
/// cost commodity's style) if priced, otherwise the amount itself (and its own
/// style).
fn cost_contribution(amount: &Amount) -> Result<(Commodity, Dec, u32, AmountStyle), ParseError> {
    let (commodity, quantity, style) = cost_value(amount)?;
    Ok((commodity.clone(), quantity, style.precision, style.clone()))
}

/// The borrowed form of [`cost_contribution`]: the `(commodity, quantity,
/// style)` an amount contributes to its transaction's balance. Shared with
/// [`check_transaction_balances`], so the verification and the inference value
/// a priced amount identically.
///
/// The commodity and quantity come from [`Amount::at_cost`] — the engine's one
/// definition of "at cost" — so the balance sheet's at-cost totals and this
/// balancing pass can never disagree about what a priced posting is worth. Only
/// the display STYLE is chosen here, since valuation has no use for it.
fn cost_value(amount: &Amount) -> Result<(&Commodity, Dec, &AmountStyle), ParseError> {
    let (commodity, quantity) = amount.at_cost()?;
    let style = match &amount.cost {
        None => &amount.style,
        Some(cost) => &cost.amount.style,
    };
    Ok((commodity, quantity, style))
}

fn finalize_posting(raw: RawPosting, sums: &[CommoditySum]) -> Result<Posting, ParseError> {
    let amounts = match raw.amount {
        Some(amount) => vec![amount],
        // PARSE-9: a commodity that nets to exactly zero is still part of the
        // inferred amount. hledger emits `[$0.00, -3 AAPL]` where filtering the
        // zero away emitted only `[-3 AAPL]` — and `[$0.00]` where it emitted
        // nothing at all.
        None => sums
            .iter()
            .map(|entry| {
                Ok(Amount {
                    commodity: entry.commodity.clone(),
                    quantity: entry.total.neg()?,
                    // The commodity's own style bits, with the precision carried
                    // through from the contributing amounts.
                    style: AmountStyle {
                        precision: entry.precision,
                        ..entry.style.clone()
                    },
                    cost: None,
                })
            })
            .collect::<Result<Vec<_>, ParseError>>()?,
    };

    Ok(Posting {
        status: raw.status,
        ptype: raw.ptype,
        account: AccountName(raw.account),
        amounts,
        balance_assertion: raw.balance_assertion,
        date: raw.date,
        date2: raw.date2,
        comment: raw.comment,
        tags: raw.tags,
    })
}

// ---------------------------------------------------------------------------
// Balance verification (PARSE-1)
// ---------------------------------------------------------------------------

/// A transaction whose postings do not sum to zero.
///
/// Carries structured facts rather than a pre-rendered string, like
/// [`crate::assertions::AssertionFailure`], so a caller can surface it as a
/// `Problem`-shaped diagnostic or as the hledger-style text of
/// [`Display`](std::fmt::Display).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnbalancedTransaction {
    /// 0-based index of the transaction in [`Journal::transactions`].
    pub transaction_index: usize,
    /// The transaction's primary date.
    pub transaction_date: String,
    /// The file the transaction was parsed from (the `include`d file, for one
    /// that came from an include) — matching which path hledger names.
    pub source_file: PathBuf,
    /// The transaction's first line, relative to [`Self::source_file`].
    pub position: SourcePos,
    /// Which balancing group failed: [`PostingType::Regular`] (hledger's "real
    /// postings") or [`PostingType::BalancedVirtual`].
    pub group: PostingType,
    /// The residual, i.e. every commodity whose sum is NOT zero, in lexical
    /// commodity order and styled for display. Never empty.
    pub residual: Vec<Amount>,
    /// Whether the group's postings span more than one commodity — which is
    /// only ever the wording of the message, never the verdict.
    pub multi_commodity: bool,
}

impl UnbalancedTransaction {
    /// The diagnostic body, worded as hledger 1.52 words it.
    #[must_use]
    pub fn message(&self) -> String {
        let group = match self.group {
            PostingType::BalancedVirtual => "balanced virtual",
            // A `(virtual)` posting is never balanced, so never reported here.
            PostingType::Regular | PostingType::Virtual => "real",
        };
        let residual: Vec<String> = self
            .residual
            .iter()
            .map(|amount| {
                crate::assertions::render_amount(amount.quantity, &amount.commodity, &amount.style)
            })
            .collect();
        let sum = format!(
            "The {group} postings' sum should be 0 but is: {}",
            residual.join(", ")
        );
        if self.multi_commodity {
            format!(
                "This multi-commodity transaction is unbalanced.\n{sum}\n\
                 Consider adjusting this entry's amounts, adding missing postings,\n\
                 or recording conversion price(s) with @, @@ or equity postings."
            )
        } else {
            format!("This transaction is unbalanced.\n{sum}")
        }
    }
}

impl std::fmt::Display for UnbalancedTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.source_file.display(),
            self.position.line,
            self.position.column,
            self.message()
        )
    }
}

/// The two groups that must each balance on their own. An unbalanced virtual
/// (`(a)`) posting is excluded from balancing entirely.
const BALANCED_GROUPS: [PostingType; 2] = [PostingType::Regular, PostingType::BalancedVirtual];

/// Verify that every transaction balances, returning the ones that do not
/// (PARSE-1).
///
/// # A post-parse pass, and not an error
///
/// Nothing verified this before: [`group_sums`] computed the per-commodity
/// totals only to infer an elided posting, so a transaction with every amount
/// written out was accepted however far from zero it summed — and the phantom
/// value entered the balance sheet, net worth and every period report. The
/// editor's write path was strict about journals it *wrote* while the reader
/// producing every number was not.
///
/// This runs over the already-balanced [`Journal`] rather than inside
/// [`balance_postings`] for two reasons: the diagnostic needs each
/// transaction's index in `Journal::transactions`, and re-summing the FINAL
/// postings is exactly equivalent — an inferred posting is defined as the
/// negated residual, so a group that had an elided posting sums to zero by
/// construction either way.
///
/// The result is a list, not an `Err`. See
/// [`crate::assertions::check_balance_assertions`] for the reasoning, which
/// applies here identically: an unbalanced transaction is a diagnostic, the
/// journal stays readable, and promoting it to a [`ParseError`] would make a
/// previously-openable journal refuse to open and reject every unrelated edit
/// through the editor's reparse guard.
///
/// # hledger's rule, reproduced exactly
///
/// A group balances when the number of commodities with a NON-ZERO residual is
/// 0 or **2**. Two is not a typo: hledger treats a two-commodity residual as an
/// implicit conversion and infers the cost that balances it, so
/// `a 10 AAA` / `b $-50.00` loads cleanly (verified against
/// `hledger -f … check autobalanced`). One, or three or more, is unbalanced.
///
/// "Non-zero" is [`looks_zero`], hledger's own test — see there for why an
/// exact-zero test is wrong, and for the guarantee that the tolerance can never
/// hide a discrepancy at a precision the journal actually wrote.
///
/// # Errors
/// Returns [`ParseError::Decimal`] only if summing a group overflows `i128`.
pub fn check_transaction_balances(
    journal: &Journal,
) -> Result<Vec<UnbalancedTransaction>, ParseError> {
    let styles = crate::assertions::display_styles(journal);
    let mut failures = Vec::new();
    for (transaction_index, transaction) in journal.transactions.iter().enumerate() {
        for group in BALANCED_GROUPS {
            let sums = group_residual(transaction, group)?;
            let residual: Vec<Amount> = sums
                .iter()
                .filter(|(_, entry)| !looks_zero(entry.total, entry.tolerance_precision()))
                .map(|(commodity, entry)| Amount {
                    commodity: (*commodity).clone(),
                    quantity: entry.total,
                    // The journal-wide display style, as hledger renders its
                    // own message; the first contributing amount's style covers
                    // a commodity that only ever appears as a cost.
                    style: styles
                        .get(commodity)
                        .copied()
                        .unwrap_or(entry.style)
                        .clone(),
                    cost: None,
                })
                .collect();
            // 0 balances; 2 is hledger's inferred conversion; 1 or 3+ is not.
            if residual.is_empty() || residual.len() == 2 {
                continue;
            }
            failures.push(UnbalancedTransaction {
                transaction_index,
                transaction_date: transaction.date.clone(),
                source_file: transaction.source_file.clone(),
                position: transaction.source_span.0,
                group,
                residual,
                multi_commodity: sums.len() > 1,
            });
        }
    }
    Ok(failures)
}

/// One commodity's residual within one balancing group.
struct Residual<'a> {
    /// The exact signed sum.
    total: Dec,
    /// Style of the first contributing amount, the fallback for rendering a
    /// commodity that has no journal-wide style of its own.
    style: &'a AmountStyle,
    /// The widest precision the journal actually WROTE for this commodity in
    /// this group — cost amounts excluded, because their places are derived,
    /// not typed. `None` when every contribution came through a cost.
    written_precision: Option<u32>,
    /// The widest precision among the contributing cost amounts, the fallback
    /// when [`Self::written_precision`] is `None`.
    cost_precision: u32,
}

impl Residual<'_> {
    /// The precision [`looks_zero`] measures against.
    fn tolerance_precision(&self) -> u32 {
        self.written_precision.unwrap_or(self.cost_precision)
    }
}

/// One balancing group's per-commodity residual, keyed lexically so both the
/// residual list and hledger's message order fall out of the iteration.
///
/// Commodities that net to zero are kept: they are what tells an ordinary
/// single-commodity transaction ("This transaction is unbalanced") apart from a
/// multi-commodity one, which hledger words differently.
fn group_residual<'a>(
    transaction: &'a Transaction,
    group: PostingType,
) -> Result<BTreeMap<&'a Commodity, Residual<'a>>, ParseError> {
    transaction
        .postings
        .iter()
        .filter(|posting| posting.ptype == group)
        .flat_map(|posting| &posting.amounts)
        .try_fold(BTreeMap::new(), |mut sums, amount| {
            // The value a priced amount contributes is its COST, exactly as the
            // inference path values it.
            let (commodity, quantity, style) = cost_value(amount)?;
            let entry = match sums.entry(commodity) {
                // Seeded with the FIRST contribution rather than with a zero at
                // scale 0. `Dec::add` rescales BOTH operands to the wider scale,
                // and `10^255` overflows `i128` however small the mantissa it
                // multiplies — so `Dec::zero().add(5e-255)` failed, and the whole
                // balance check returned `Overflow` for a journal that parses
                // cleanly and balances exactly. Seeding is arithmetically
                // identical (`0 + q == q`) and cannot overflow.
                Entry::Vacant(slot) => slot.insert(Residual {
                    total: quantity,
                    style,
                    written_precision: None,
                    cost_precision: 0,
                }),
                Entry::Occupied(slot) => {
                    let entry = slot.into_mut();
                    entry.total = entry.total.add(quantity)?;
                    entry
                }
            };
            if amount.cost.is_none() {
                let written = entry.written_precision.unwrap_or(0).max(style.precision);
                entry.written_precision = Some(written);
            } else {
                entry.cost_precision = entry.cost_precision.max(style.precision);
            }
            Ok(sums)
        })
}

/// hledger's `amountLooksZero`, reproduced exactly: a residual counts as zero
/// when it rounds away at `precision` decimal places (`|mantissa| <= 5·10^(e-d-1)`
/// once `e > d`), and must be exactly zero otherwise.
///
/// # Why not a plain exact-zero test
///
/// Because hledger is not exact here, and a stricter check produces FALSE
/// diagnostics on journals hledger loads cleanly.
/// `fixtures/corpus/precision.journal` is the proof:
///
/// ```text
/// 2010-01-01 x
///     A  55.3653 C @ 30.92189512 D
///     A  -1712 D
/// ```
///
/// `55.3653 × 30.92189512 = 1712.000000112664`, so the residual is
/// `0.000000112664 D` — yet `hledger -f … print` and
/// `hledger -f … check autobalanced` both accept it, because a unit price is a
/// rounded quotient and the extra places are an artefact of multiplying it out.
/// An exact test flags every real journal that records a price this way.
///
/// # The tolerance cannot hide a real discrepancy
///
/// `precision` is the widest precision the journal WROTE for that commodity in
/// that group — never a cost's derived precision, never a `commodity`
/// directive's declared one (verified: declaring `commodity 1.00000 D` does not
/// change hledger's verdict). Summing amounts written at `d` places yields a
/// residual at exactly `d` places ([`Dec::add`] takes the max), so `e > d` is
/// reachable ONLY through a cost multiplication. Every discrepancy at a
/// precision a human typed still falls through to the exact test: `$0.001`
/// three times over against `$0.00` is reported, as hledger reports it.
fn looks_zero(residual: Dec, precision: u32) -> bool {
    let Some(extra) = residual.places.checked_sub(precision + 1) else {
        return residual.mantissa == 0;
    };
    // Past ~38 extra places the threshold exceeds every representable i128
    // mantissa, so nothing can be over it.
    i128::checked_pow(10, extra)
        .and_then(|scale| scale.checked_mul(5))
        .is_none_or(|threshold| residual.mantissa.unsigned_abs() <= threshold.unsigned_abs())
}

// ---------------------------------------------------------------------------
// Lexical helpers
// ---------------------------------------------------------------------------

/// Split a line at its first `;`, returning `(before, Some(after))` or
/// `(line, None)`.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    match line.find(';') {
        Some(pos) => (&line[..pos], Some(&line[pos + 1..])),
        None => (line, None),
    }
}

/// Build a stored comment string (trailing newline, or empty) plus its parsed
/// tags from the raw text following a `;`.
fn build_comment(raw: Option<&str>) -> (String, Vec<(String, String)>) {
    match raw {
        None => (String::new(), Vec::new()),
        Some(text) => {
            let content = text.trim();
            if content.is_empty() {
                (String::new(), Vec::new())
            } else {
                (format!("{content}\n"), parse_tags(content))
            }
        }
    }
}

/// Whether one comma-separated `segment` of a comment is a `name:value` tag,
/// and if so, what it names.
///
/// `pub(crate)` rather than private, and factored out of [`parse_tags`] rather
/// than duplicated: [`crate::accounts`] must classify a candidate note
/// fragment by this *exact* rule before writing it — a second, approximate
/// copy of "what counts as a tag" is exactly the kind of silent divergence
/// `parse_account_directive`'s doc comment already records one instance of.
pub(crate) fn tag_in_segment(segment: &str) -> Option<(String, String)> {
    let colon = segment.find(':')?;
    let name = segment[..colon].split_whitespace().next_back()?;
    if name.is_empty() {
        return None;
    }
    let value = segment[colon + 1..].trim().to_string();
    Some((name.to_string(), value))
}

/// Extract `name:value` tags from a comment body. The tag name is the last
/// whitespace-delimited token before a `:`; its value runs to the next comma.
pub(crate) fn parse_tags(comment: &str) -> Vec<(String, String)> {
    comment.split(',').filter_map(tag_in_segment).collect()
}

/// Split a posting's `after-status` remainder into `(account, amount)` at the
/// first run of two-or-more spaces (or a tab). A single space is part of the
/// account name.
pub(crate) fn split_account_amount(text: &str) -> (&str, &str) {
    let mut prev_space: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        if ch == '\t' {
            return (&text[..idx], &text[idx..]);
        }
        if ch == ' ' {
            if let Some(start) = prev_space {
                return (&text[..start], &text[start..]);
            }
            prev_space = Some(idx);
        } else {
            prev_space = None;
        }
    }
    (text, "")
}

/// Whether a character can begin/continue a commodity symbol (excludes digits,
/// signs, separators, whitespace, and amount operators).
///
/// `{`/`}` are excluded because they open hledger's lot-cost notation
/// (`10 AAPL {$5.00}`); without that, the whole annotation used to be absorbed
/// into the commodity name (PARSE-5). `[`/`]` are deliberately **not** excluded
/// — hledger accepts `10 AA[PL` as the commodity `AA[PL`, and only treats
/// `[...]` as a lot date directly after a `{...}` price.
fn is_commodity_char(c: char) -> bool {
    !c.is_ascii_digit()
        && !c.is_whitespace()
        && !matches!(
            c,
            '-' | '+' | '.' | ',' | '@' | '=' | ';' | '(' | ')' | '{' | '}'
        )
}

/// The decimal mark hledger displays for a literal that has none of its own
/// (`1.234.567`, where every separator is a digit group).
///
/// It is `.` by default, but a literal whose group separator already IS `.`
/// displays with `,` — one character cannot serve as both.
fn default_display_mark(groups: Option<&DigitGroups>) -> char {
    if groups.map(|group| group.mark) == Some('.') {
        ','
    } else {
        '.'
    }
}

/// The byte length of a digit-group separator at the start of `rest`, if any.
///
/// hledger accepts a plain space and U+00A0 NO-BREAK SPACE between digit
/// groups. It rejects `_` (`1_000.00` is a parse error), so that is not
/// included.
fn digit_group_space_len(rest: &str) -> Option<usize> {
    rest.chars()
        .next()
        .filter(|c| matches!(c, ' ' | '\u{a0}'))
        .map(char::len_utf8)
}

/// The byte length of the leading numeric literal in `token`: an optional sign,
/// digits and decimal/group marks, and a scientific exponent (`e`/`E`, optional
/// sign, one or more digits). A trailing `e`/`E` with no exponent digits is left
/// for the commodity (hledger parses `1.00005e` as `1.00005` in commodity `e`).
fn numeric_prefix_len(token: &str) -> usize {
    let bytes = token.as_bytes();
    let mut i = 0;
    if i < bytes.len() && matches!(bytes[i], b'-' | b'+') {
        i += 1;
    }
    loop {
        while i < bytes.len() && (bytes[i].is_ascii_digit() || matches!(bytes[i], b'.' | b',')) {
            i += 1;
        }
        // PARSE-4: a space or NBSP *between two digits* is a digit-group
        // separator (`1 000.00 EUR`), not the end of the number. Stopping here
        // used to hand `000.00 EUR` to the commodity and leave the quantity at
        // 1. Anything else (including `_`, which hledger rejects outright) ends
        // the numeric prefix.
        let Some(separator) = digit_group_space_len(&token[i..]) else {
            break;
        };
        if !token[i + separator..].starts_with(|c: char| c.is_ascii_digit()) {
            break;
        }
        i += separator;
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < bytes.len() && matches!(bytes[j], b'-' | b'+') {
            j += 1;
        }
        let exponent_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > exponent_start {
            i = j;
        }
    }
    i
}

/// Split a `commodity`/`D` directive specimen into `(commodity, number, side,
/// spaced)`. Unlike [`split_commodity_number`], the number may contain a space
/// digit-group separator (`1 000.00 EUR`), so the commodity is taken as the
/// maximal run of commodity characters at whichever end it occupies.
fn split_commodity_spec(spec: &str) -> Result<(String, String, CommoditySide, bool), ParseError> {
    let spec = spec.trim();
    let first = spec
        .chars()
        .next()
        .ok_or_else(|| ParseError::MalformedAmount(spec.to_string()))?;
    if is_commodity_char(first) {
        let end = spec
            .char_indices()
            .find(|(_, c)| !is_commodity_char(*c))
            .map_or(spec.len(), |(idx, _)| idx);
        let commodity = spec[..end].to_string();
        let rest = &spec[end..];
        let spaced = rest.starts_with(char::is_whitespace);
        let number = rest.trim().to_string();
        if number.is_empty() {
            return Err(ParseError::MalformedAmount(spec.to_string()));
        }
        Ok((commodity, number, CommoditySide::Left, spaced))
    } else {
        let start = spec
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_commodity_char(*c))
            .map(|(idx, _)| idx)
            .last()
            .ok_or_else(|| ParseError::MalformedAmount(spec.to_string()))?;
        let commodity = spec[start..].to_string();
        let prefix = &spec[..start];
        let spaced = prefix.ends_with(char::is_whitespace);
        let number = prefix.trim().to_string();
        if number.is_empty() {
            return Err(ParseError::MalformedAmount(spec.to_string()));
        }
        Ok((commodity, number, CommoditySide::Right, spaced))
    }
}

/// Split an amount token into `(commodity, number, side, spaced)`.
fn split_commodity_number(
    token: &str,
) -> Result<(String, String, CommoditySide, bool), ParseError> {
    let token = token.trim();
    let first = token
        .chars()
        .next()
        .ok_or_else(|| ParseError::MalformedAmount(token.to_string()))?;

    // A double-quoted commodity on the left (`"green apples" 3`). Quoting is how
    // hledger writes a symbol containing spaces or digits, and the quotes are
    // delimiters — they are not part of the name (PARSE-9).
    if first == '"' {
        let (commodity, rest) = split_quoted_commodity(token)
            .ok_or_else(|| ParseError::MalformedAmount(token.to_string()))?;
        let spaced = rest.starts_with(char::is_whitespace);
        let number = rest.trim().to_string();
        if number.is_empty() {
            return Err(ParseError::MalformedAmount(token.to_string()));
        }
        return Ok((commodity, number, CommoditySide::Left, spaced));
    }

    if is_commodity_char(first) {
        let end = token
            .char_indices()
            .find(|(_, c)| !is_commodity_char(*c))
            .map_or(token.len(), |(idx, _)| idx);
        let commodity = token[..end].to_string();
        let rest = &token[end..];
        let spaced = rest.starts_with(char::is_whitespace);
        let number = rest.trim().to_string();
        if number.is_empty() {
            return Err(ParseError::MalformedAmount(token.to_string()));
        }
        Ok((commodity, number, CommoditySide::Left, spaced))
    } else {
        // A leading sign may precede a left-side commodity (`-$1,658.91`, where
        // the sign belongs to the number) or a plain number (`-12 NVDA`).
        let sign_len = if matches!(first, '-' | '+') {
            first.len_utf8()
        } else {
            0
        };
        let after_sign = &token[sign_len..];
        if after_sign.chars().next().is_some_and(is_commodity_char) {
            // Sign, then commodity on the left: reattach the sign to the number.
            let end = after_sign
                .char_indices()
                .find(|(_, c)| !is_commodity_char(*c))
                .map_or(after_sign.len(), |(idx, _)| idx);
            let commodity = after_sign[..end].to_string();
            let rest = &after_sign[end..];
            let spaced = rest.starts_with(char::is_whitespace);
            let number = format!("{}{}", &token[..sign_len], rest.trim());
            if rest.trim().is_empty() {
                return Err(ParseError::MalformedAmount(token.to_string()));
            }
            return Ok((commodity, number, CommoditySide::Left, spaced));
        }

        let end = numeric_prefix_len(token);
        let number = token[..end].to_string();
        let rest = &token[end..];
        let trailing = rest.trim();
        if number.is_empty() {
            return Err(ParseError::MalformedAmount(token.to_string()));
        }
        if trailing.is_empty() {
            // A bare number with no commodity symbol: hledger models this as the
            // empty commodity, displayed left-side with no spacing.
            return Ok((String::new(), number, CommoditySide::Left, false));
        }
        let spaced = rest.starts_with(char::is_whitespace);
        let commodity = if trailing.starts_with('"') {
            // A quoted right-side commodity (`3 "green apples"`) may contain
            // spaces, but nothing may follow the closing quote.
            match split_quoted_commodity(trailing) {
                Some((commodity, after)) if after.trim().is_empty() => commodity,
                _ => return Err(ParseError::MalformedAmount(token.to_string())),
            }
        } else {
            // PARSE-4: everything left over used to become the commodity name,
            // so `0x10 XX` silently parsed as 0 of commodity `x10 XX` and
            // `100.0O USD` as 100.0 of `O USD`. hledger rejects both, and
            // requires quoting for any symbol containing a space.
            if trailing.chars().any(char::is_whitespace) {
                return Err(ParseError::MalformedAmount(token.to_string()));
            }
            trailing.to_string()
        };
        Ok((commodity, number, CommoditySide::Right, spaced))
    }
}

/// Split a leading double-quoted commodity symbol off `text`, returning the
/// unquoted name and the remainder. `None` when `text` does not start with a
/// quote or the quote is never closed.
fn split_quoted_commodity(text: &str) -> Option<(String, &str)> {
    let quoted = text.strip_prefix('"')?;
    let end = quoted.find('"')?;
    Some((quoted[..end].to_string(), &quoted[end + 1..]))
}

/// Analyze a bare numeric literal, returning `(decimal_mark, digit_groups,
/// precision)`. Used for `commodity` directives and undeclared-commodity
/// fallbacks.
///
/// When `forced_mark` is `Some`, that character is the decimal mark (from a
/// `decimal-mark` directive) and any other of `.`/`,` is the group separator.
/// Otherwise the mark is inferred: when both `.` and `,` appear, the rightmost
/// is the decimal mark.
fn analyze_number(
    literal: &str,
    forced_mark: Option<char>,
) -> (Option<char>, Option<Arc<DigitGroups>>, u32) {
    let body = literal.trim().trim_start_matches(['-', '+']);
    let last_dot = body.rfind('.');
    let last_comma = body.rfind(',');
    let dot_count = body.matches('.').count();
    let comma_count = body.matches(',').count();

    let (decimal_mark, group_mark): (Option<char>, Option<char>) = match forced_mark {
        Some(mark) => {
            let other = if mark == '.' { ',' } else { '.' };
            let group = if body.contains(other) {
                Some(other)
            } else {
                None
            };
            (Some(mark), group)
        }
        None => match (last_dot, last_comma) {
            (Some(d), Some(c)) => {
                if d > c {
                    (Some('.'), Some(','))
                } else {
                    (Some(','), Some('.'))
                }
            }
            (Some(_), None) if dot_count == 1 => (Some('.'), None),
            (Some(_), None) => (None, Some('.')),
            (None, Some(_)) if comma_count == 1 => (Some(','), None),
            (None, Some(_)) => (None, Some(',')),
            (None, None) => (None, None),
        },
    };

    // A whitespace character (e.g. the space in `1 000.00`) is always a
    // digit-group separator, never a decimal mark; adopt it when no `.`/`,`
    // group was found.
    let group_mark = group_mark.or_else(|| body.chars().find(|c| c.is_whitespace()));

    let precision = match decimal_mark {
        Some(mark) => {
            let pos = body.rfind(mark).map_or(body.len(), |p| p + mark.len_utf8());
            to_u32(body[pos..].chars().filter(char::is_ascii_digit).count())
        }
        None => 0,
    };

    let digit_groups = group_mark.map(|mark| {
        let integer_part = match decimal_mark {
            Some(dm) => &body[..body.rfind(dm).unwrap_or(body.len())],
            None => body,
        };
        let mut sizes: Vec<u8> = integer_part
            .split(mark)
            .map(|segment| u8::try_from(segment.chars().count()).unwrap_or(u8::MAX))
            .collect();
        sizes.reverse();
        // Sizes run right-to-left. The leftmost group is a partial group and is
        // dropped unless it is at least as wide as the group beside it, matching
        // hledger: `1,000.00` -> [3] and `1.234.567` -> [3,3], but
        // `123,456.00` -> [3,3] and `1.2.3` -> [1,1,1].
        let leading_is_full = match sizes.as_slice() {
            [.., inner, leading] => leading >= inner,
            _ => false,
        };
        if !leading_is_full {
            sizes.pop();
        }
        // Unreachable sizes are dropped HERE rather than earlier, so the rule
        // above still sees the whole list and no real literal changes shape.
        // `sizes` runs right-to-left, so what survives is exactly what a
        // renderer reaches first. `shrink_to_fit` so the style that gets cloned
        // into every amount does not carry the original capacity with it.
        sizes.truncate(MAX_DIGIT_GROUPS);
        sizes.shrink_to_fit();
        Arc::new(DigitGroups { mark, sizes })
    });

    (decimal_mark, digit_groups, precision)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `account` directive's name/comment boundary, pinned against the
    /// hledger 1.52 binary rather than the manual. Every expectation below was
    /// read off `hledger -f … accounts --declared` and a `bal type:A` query.
    ///
    /// This diverged silently once already: our parser split at the first `;`,
    /// so `account a:b ; type: A` declared the account `a:b` with a type while
    /// hledger declared one literally named `a:b ; type: A` with none. The
    /// journal parsed on both sides and the reports disagreed.
    mod account_directive_names {
        use super::*;

        fn parsed(line: &str) -> (String, Vec<(String, String)>) {
            let decl =
                parse_account_directive(line, 1, Path::new("t.journal")).expect("directive parses");
            (decl.name.0, decl.tags)
        }

        #[test]
        fn two_spaces_separate_the_name_from_its_comment() {
            let (name, tags) = parsed("account two:space  ; type: A");
            assert_eq!(name, "two:space");
            assert_eq!(tags, vec![("type".to_string(), "A".to_string())]);
        }

        /// ONE space does not. hledger takes the whole rest of the line as the
        /// account name, tag and all.
        #[test]
        fn one_space_leaves_the_comment_inside_the_name() {
            let (name, tags) = parsed("account one:space ; type: A");
            assert_eq!(name, "one:space ; type: A");
            assert!(tags.is_empty(), "no comment was parsed, so no tags");
        }

        #[test]
        fn no_space_at_all_leaves_the_comment_inside_the_name() {
            let (name, tags) = parsed("account no:space; type: A");
            assert_eq!(name, "no:space; type: A");
            assert!(tags.is_empty());
        }

        /// Single spaces are legal INSIDE an account name, which is the whole
        /// reason a single space cannot end one.
        #[test]
        fn a_name_may_contain_single_spaces() {
            let (name, tags) = parsed("account trailing:words is here  ; type: A");
            assert_eq!(name, "trailing:words is here");
            assert_eq!(tags, vec![("type".to_string(), "A".to_string())]);
        }

        /// A lone tab reads as a single space — part of the name, and printed
        /// back as a space — while a space-then-tab is two whitespace and does
        /// separate.
        #[test]
        fn a_lone_tab_is_part_of_the_name_but_a_pair_is_a_separator() {
            let (name, tags) = parsed("account tab:name\t; type: A");
            assert_eq!(name, "tab:name ; type: A");
            assert!(tags.is_empty());

            let (name, tags) = parsed("account tabsp:name \t; type: A");
            assert_eq!(name, "tabsp:name");
            assert_eq!(tags, vec![("type".to_string(), "A".to_string())]);
        }

        #[test]
        fn a_bare_name_needs_no_comment() {
            let (name, tags) = parsed("account plain:name");
            assert_eq!(name, "plain:name");
            assert!(tags.is_empty());
        }

        /// hledger: "expecting ';', end of input, or newline". Anything else
        /// after the separator is a hard error, not silently-kept text.
        #[test]
        fn text_after_the_separator_that_is_not_a_comment_is_refused() {
            assert!(
                parse_account_directive(
                    "account two:words  trailing junk",
                    1,
                    Path::new("t.journal")
                )
                .is_err()
            );
        }

        /// A name-less directive whose body is only a comment is NOT an error:
        /// with no two-whitespace run there is nothing to separate, so hledger
        /// declares an account literally named `; type: A`. Checked against the
        /// binary, which prints exactly that and exits 0 — following it here
        /// costs nothing and diverging would be one more silent disagreement.
        #[test]
        fn a_body_that_is_only_a_comment_becomes_the_name() {
            let (name, tags) = parsed("account   ; type: A");
            assert_eq!(name, "; type: A");
            assert!(tags.is_empty());
        }

        /// A bare `account` with no body IS an error, as it is in hledger.
        #[test]
        fn a_bare_directive_is_refused() {
            assert!(parse_account_directive("account", 1, Path::new("t.journal")).is_err());
            assert!(parse_account_directive("account   ", 1, Path::new("t.journal")).is_err());
        }
    }

    fn eur_styles() -> Styles {
        let (commodity, style, _tags) =
            parse_commodity_directive("commodity 1.000,00 EUR").unwrap();
        let mut styles = HashMap::new();
        styles.insert(commodity, style.unwrap());
        styles
    }

    /// Build an [`AmountCtx`] over `styles` with no `decimal-mark` default.
    fn ctx(styles: &Styles) -> AmountCtx<'_> {
        AmountCtx {
            styles,
            default_mark: None,
            default_commodity: None,
            inherited: Box::leak(Box::default()),
        }
    }

    #[test]
    fn commodity_directive_dollar_style() {
        let (commodity, style, _tags) = parse_commodity_directive("commodity $1,000.00").unwrap();
        let style = style.unwrap();
        assert_eq!(commodity, Commodity("$".to_string()));
        assert_eq!(style.side, CommoditySide::Left);
        assert!(!style.spaced);
        assert_eq!(style.decimal_mark, Some('.'));
        assert_eq!(
            style.digit_groups,
            Some(Arc::new(DigitGroups {
                mark: ',',
                sizes: vec![3]
            }))
        );
        assert_eq!(style.precision, 2);
    }

    #[test]
    fn commodity_directive_eur_comma_style() {
        let (commodity, style, _tags) =
            parse_commodity_directive("commodity 1.000,00 EUR").unwrap();
        let style = style.unwrap();
        assert_eq!(commodity, Commodity("EUR".to_string()));
        assert_eq!(style.side, CommoditySide::Right);
        assert!(style.spaced);
        assert_eq!(style.decimal_mark, Some(','));
        assert_eq!(
            style.digit_groups,
            Some(Arc::new(DigitGroups {
                mark: '.',
                sizes: vec![3]
            }))
        );
        assert_eq!(style.precision, 2);
    }

    #[test]
    fn eur_amount_uses_declared_decimal_mark() {
        let styles = eur_styles();
        let amount = parse_amount("645,00 EUR", ctx(&styles)).unwrap();
        assert_eq!(amount.quantity, Dec::new(64500, 2));
        assert_eq!(amount.style.decimal_mark, Some(','));
        assert_eq!(amount.style.precision, 2);
    }

    /// Pins the `Arc<DigitGroups>` fix at `model.rs`: every amount of a
    /// commodity with a declared style shares ONE allocation rather than each
    /// cloning its own `Vec`. Value equality alone (`assert_eq!`) would not
    /// catch a regression back to a per-amount clone — this checks identity.
    #[test]
    fn amounts_of_a_declared_commodity_share_the_digit_groups_allocation() {
        let styles = eur_styles();
        let first = parse_amount("645,00 EUR", ctx(&styles)).unwrap();
        let second = parse_amount("1.234,50 EUR", ctx(&styles)).unwrap();
        let (Some(a), Some(b)) = (&first.style.digit_groups, &second.style.digit_groups) else {
            panic!("EUR is declared with digit groups");
        };
        assert!(Arc::ptr_eq(a, b));
    }

    #[test]
    fn tags_take_last_token_before_colon() {
        assert_eq!(
            parse_tags("WP-08 problem record: uncategorized"),
            vec![("record".to_string(), "uncategorized".to_string())]
        );
        assert_eq!(
            parse_tags("name: Apple Inc."),
            vec![("name".to_string(), "Apple Inc.".to_string())]
        );
    }

    #[test]
    fn account_and_amount_split_on_two_spaces() {
        let (account, amount) = split_account_amount("expenses:housing:rent      $1,800.00");
        assert_eq!(account, "expenses:housing:rent");
        assert_eq!(amount.trim(), "$1,800.00");

        let (account, amount) = split_account_amount("assets:bank:checking");
        assert_eq!(account, "assets:bank:checking");
        assert_eq!(amount, "");
    }

    #[test]
    fn header_parses_code_and_status() {
        let header =
            parse_header("2025-11-01 * (2101) Oakview Properties | rent (paid by check)").unwrap();
        assert_eq!(header.status, Status::Cleared);
        assert_eq!(header.code, "2101");
        assert_eq!(
            header.description,
            "Oakview Properties | rent (paid by check)"
        );
    }

    #[test]
    fn header_empty_description() {
        let header = parse_header("2026-06-28").unwrap();
        assert_eq!(header.status, Status::Unmarked);
        assert_eq!(header.code, "");
        assert_eq!(header.description, "");
    }

    #[test]
    fn header_pending_with_tag_comment() {
        let header =
            parse_header("2026-07-02 ! Delta Airlines | flight to Denver  ; trip: denver").unwrap();
        assert_eq!(header.status, Status::Pending);
        assert_eq!(header.description, "Delta Airlines | flight to Denver");
        assert_eq!(header.comment, "trip: denver\n");
        assert_eq!(
            header.tags,
            vec![("trip".to_string(), "denver".to_string())]
        );
    }

    #[test]
    fn decimal_mark_directive_sets_default_for_undeclared_commodity() {
        // With `decimal-mark ,`, an undeclared commodity's `1.234,50` parses as
        // 1234.50 (dot = group, comma = decimal), and the elided leg balances.
        let text = concat!(
            "decimal-mark ,\n",
            "\n",
            "2024-01-01 test\n",
            "    expenses:foo   1.234,50 CHF\n",
            "    assets:bank\n",
        );
        let journal = parse_journal(text, "test.journal").unwrap();
        assert_eq!(journal.transactions.len(), 1);
        let postings = &journal.transactions[0].postings;
        let amount = &postings[0].amounts[0];
        assert_eq!(amount.commodity, Commodity("CHF".to_string()));
        assert_eq!(amount.quantity, Dec::new(123450, 2));
        assert_eq!(amount.style.decimal_mark, Some(','));
        assert_eq!(
            amount.style.digit_groups,
            Some(Arc::new(DigitGroups {
                mark: '.',
                sizes: vec![3]
            }))
        );
        let counter = &postings[1].amounts[0];
        assert_eq!(counter.quantity, Dec::new(-123450, 2));
    }

    #[test]
    fn an_auto_rule_with_no_space_after_the_equals_sign_does_not_refuse_the_journal() {
        // hledger 1.52 reads `=revenue:other` exactly as `= revenue:other`.
        // Before 2026-10-28 it fell through to "unsupported directive" and
        // refused the whole file.
        let text = concat!(
            "=revenue:other\n",
            "    (liabilities:tax)  *0.05\n",
            "\n",
            "2026-01-01 real\n",
            "    revenue:other  $-100\n",
            "    assets:checking\n",
        );
        let journal = parse_journal(text, "t.journal").expect("the journal opens");
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].postings.len(), 2);
    }

    #[test]
    fn periodic_rule_parsed_but_excluded_from_transactions() {
        // A `~` rule is parsed into `periodic_transactions` (balanced like a real
        // transaction, so the elided `assets:bank` leg is inferred), yet it is
        // kept out of `transactions`. An `=` auto-posting block stays skipped.
        let text = concat!(
            "~ monthly  household budget\n",
            "    expenses:rent    $1000\n",
            "    assets:bank\n",
            "\n",
            "= expenses:food\n",
            "    (budget:food)  *0.1\n",
            "\n",
            "2024-01-01 real\n",
            "    expenses:x   $2.00\n",
            "    assets:bank\n",
        );
        let journal = parse_journal(text, "t.journal").unwrap();

        // The regular transaction, and only it, lands in `transactions`.
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].description, "real");
        assert_eq!(journal.transactions[0].index, Tindex(1));

        // The periodic rule is captured separately, with its balancing leg.
        assert_eq!(journal.periodic_transactions.len(), 1);
        let periodic = &journal.periodic_transactions[0];
        assert_eq!(periodic.period.interval(), Some(PeriodExpr::Monthly));
        assert_eq!(periodic.period.raw, "monthly");
        assert_eq!(periodic.description, "household budget");
        assert_eq!(periodic.postings.len(), 2);
        assert_eq!(
            periodic.postings[0].account,
            AccountName("expenses:rent".into())
        );
        assert_eq!(periodic.postings[0].amounts[0].quantity, Dec::new(1000, 0));
        assert_eq!(
            periodic.postings[1].account,
            AccountName("assets:bank".into())
        );
        assert_eq!(periodic.postings[1].amounts[0].quantity, Dec::new(-1000, 0));
    }

    /// A rule records WHERE it is, on the same end-exclusive convention a
    /// transaction's span uses. Without this the budget editor can report a goal
    /// but never rewrite one, so the numbers are pinned rather than assumed.
    #[test]
    fn periodic_rule_records_its_span_and_file() {
        let text = concat!(
            "; a header comment\n",
            "\n",
            "~ monthly  household budget\n",      // line 3
            "    (expenses:rent)  $1000\n",       // line 4
            "    ; a trailing posting comment\n", // line 5, consumed by the rule
            "\n",
            "~ yearly  annual\n",          // line 7
            "    (expenses:tax)   $500\n", // line 8
        );
        let journal = parse_journal(text, "t.journal").unwrap();
        let rules = &journal.periodic_transactions;
        assert_eq!(rules.len(), 2);

        // Start is the `~` line; end is the line AFTER the last one consumed —
        // the indented comment counts as consumed, exactly as it does for a
        // transaction.
        assert_eq!(rules[0].source_span.0, SourcePos { line: 3, column: 1 });
        assert_eq!(rules[0].source_span.1, SourcePos { line: 6, column: 1 });
        assert_eq!(rules[1].source_span.0, SourcePos { line: 7, column: 1 });
        assert_eq!(rules[1].source_span.1, SourcePos { line: 9, column: 1 });

        // The file is the resolved path the parse read, the same one
        // `Transaction::source_file` and `Journal::source_files` record.
        assert_eq!(rules[0].source_file, resolve_source_file("t.journal"));
        assert_eq!(rules[1].source_file, rules[0].source_file);
        assert!(journal.source_files.contains(&rules[0].source_file));
    }

    /// The one rule of this fixture family: whatever the header says, the
    /// journal opens. Returns the single rule's spec.
    fn period_spec(expr: &str) -> PeriodSpec {
        let src = format!("~ {expr}\n    (a)  $1\n");
        let journal = parse_journal(&src, "t.journal")
            .unwrap_or_else(|e| panic!("`~ {expr}` must not fail the journal: {e}"));
        journal.periodic_transactions[0].period.clone()
    }

    /// Every accepted row of the table in `plans/21-periodic-rule-parser.md`,
    /// each verified against `hledger 1.52` before it was written down here.
    #[test]
    fn periodic_rule_accepts_every_form_hledger_accepts() {
        let every = |unit, multiplier| PeriodKind::Every { unit, multiplier };

        for (expr, kind, start, end) in [
            ("daily", every(PeriodExpr::Daily, 1), None, None),
            ("weekly", every(PeriodExpr::Weekly, 1), None, None),
            ("monthly", every(PeriodExpr::Monthly, 1), None, None),
            ("quarterly", every(PeriodExpr::Quarterly, 1), None, None),
            ("yearly", every(PeriodExpr::Yearly, 1), None, None),
            // `every UNIT` is a synonym for the bare word.
            ("every month", every(PeriodExpr::Monthly, 1), None, None),
            ("every day", every(PeriodExpr::Daily, 1), None, None),
            // Multipliers, and the two words that are one.
            ("every 2 weeks", every(PeriodExpr::Weekly, 2), None, None),
            ("biweekly", every(PeriodExpr::Weekly, 2), None, None),
            ("fortnightly", every(PeriodExpr::Weekly, 2), None, None),
            ("bimonthly", every(PeriodExpr::Monthly, 2), None, None),
            // Anchored forms.
            (
                "every 15th day of month",
                PeriodKind::Anchored {
                    unit: PeriodExpr::Monthly,
                    anchor: Anchor::DayOfMonth(15),
                },
                None,
                None,
            ),
            (
                "every 3rd tuesday of month",
                PeriodKind::Anchored {
                    unit: PeriodExpr::Monthly,
                    anchor: Anchor::NthWeekday { nth: 3, weekday: 2 },
                },
                None,
                None,
            ),
            (
                "every tuesday",
                PeriodKind::Anchored {
                    unit: PeriodExpr::Weekly,
                    anchor: Anchor::Weekday(2),
                },
                None,
                None,
            ),
            (
                "every 2nd day of week",
                PeriodKind::Anchored {
                    unit: PeriodExpr::Weekly,
                    anchor: Anchor::Weekday(2),
                },
                None,
                None,
            ),
            (
                "every 12/25",
                PeriodKind::Annual { month: 12, day: 25 },
                None,
                None,
            ),
            // Bounds. A partial date is the first of its unit, and `to` is the
            // EXCLUSIVE end hledger writes.
            (
                "quarterly from 2027",
                every(PeriodExpr::Quarterly, 1),
                Some("2027-01-01"),
                None,
            ),
            (
                "every 2 months from 2027-01",
                every(PeriodExpr::Monthly, 2),
                Some("2027-01-01"),
                None,
            ),
            (
                "monthly from 2026 to 2028",
                every(PeriodExpr::Monthly, 1),
                Some("2026-01-01"),
                Some("2028-01-01"),
            ),
            // Single occurrences.
            ("2027-03-01", PeriodKind::Once, Some("2027-03-01"), None),
            ("2027-03", PeriodKind::Once, Some("2027-03-01"), None),
            (
                "from 2027-03-01 to 2027-04-01",
                PeriodKind::Once,
                Some("2027-03-01"),
                Some("2027-04-01"),
            ),
        ] {
            let spec = period_spec(expr);
            assert_eq!(spec.kind, kind, "kind of `{expr}`");
            assert_eq!(spec.start.as_deref(), start, "start of `{expr}`");
            assert_eq!(spec.end.as_deref(), end, "end of `{expr}`");
            // `raw` is the bytes between `~` and the double space, verbatim.
            assert_eq!(spec.raw, expr, "raw of `{expr}`");
        }
    }

    /// Every form plan 30 added, each verified against `hledger 1.52 print
    /// --forecast` on 2026-10-27 before it was written down here. The first
    /// three are the lines that reported "not a recurrence this engine can
    /// enumerate".
    #[test]
    fn periodic_rule_accepts_the_forms_plan_30_added() {
        let annual = |month, day| PeriodKind::Annual { month, day };
        let monthly = |anchor| PeriodKind::Anchored {
            unit: PeriodExpr::Monthly,
            anchor,
        };
        let weekly = |anchor| PeriodKind::Anchored {
            unit: PeriodExpr::Weekly,
            anchor,
        };
        let every = |unit, multiplier| PeriodKind::Every { unit, multiplier };
        let month = every(PeriodExpr::Monthly, 1);
        let mon_wed_fri = Anchor::Weekdays(0b001_0101);

        for (expr, kind, start, end) in [
            (
                "every 1st Apr from 2027",
                annual(4, 1),
                Some("2027-01-01"),
                None,
            ),
            (
                "every 15th Nov from 2026 to 2026-12-31",
                annual(11, 15),
                Some("2026-01-01"),
                Some("2026-12-31"),
            ),
            (
                "every 1st July from 2027",
                annual(7, 1),
                Some("2027-01-01"),
                None,
            ),
            // Month name first, `of year`, full and short names, a
            // zero-padded ordinal, and a day past the month's end.
            ("every Nov 15th", annual(11, 15), None, None),
            ("every 15th Nov of year", annual(11, 15), None, None),
            ("every Nov 15th of year", annual(11, 15), None, None),
            ("every 11/15 of year", annual(11, 15), None, None),
            ("every 01st april", annual(4, 1), None, None),
            ("every 30th feb", annual(2, 30), None, None),
            ("every 2/29", annual(2, 29), None, None),
            // `of month` is optional.
            ("every 2nd day", monthly(Anchor::DayOfMonth(2)), None, None),
            (
                "every 15th day from 2027-03",
                monthly(Anchor::DayOfMonth(15)),
                Some("2027-03-01"),
                None,
            ),
            (
                "every 3rd mon",
                monthly(Anchor::NthWeekday { nth: 3, weekday: 1 }),
                None,
                None,
            ),
            (
                "every 2nd Monday",
                monthly(Anchor::NthWeekday { nth: 2, weekday: 1 }),
                None,
                None,
            ),
            // Several weekdays a week.
            (
                "every weekday",
                weekly(Anchor::Weekdays(0b001_1111)),
                None,
                None,
            ),
            (
                "every weekendday",
                weekly(Anchor::Weekdays(0b110_0000)),
                None,
                None,
            ),
            ("every mon,wed,fri", weekly(mon_wed_fri), None, None),
            ("every fri,mon,wed", weekly(mon_wed_fri), None, None),
            (
                "every mon,wed,fri to 2027-01-20",
                weekly(mon_wed_fri),
                None,
                Some("2027-01-20"),
            ),
            // `in`, `since`, `until`, `..`, `-`, and a lone date naming a span.
            (
                "monthly in 2027",
                month,
                Some("2027-01-01"),
                Some("2028-01-01"),
            ),
            (
                "monthly 2027",
                month,
                Some("2027-01-01"),
                Some("2028-01-01"),
            ),
            (
                "monthly 2027-03",
                month,
                Some("2027-03-01"),
                Some("2027-04-01"),
            ),
            (
                "monthly 20270301",
                month,
                Some("2027-03-01"),
                Some("2027-03-02"),
            ),
            ("monthly from 202703", month, Some("2027-03-01"), None),
            (
                "monthly in 2027q2",
                month,
                Some("2027-04-01"),
                Some("2027-07-01"),
            ),
            ("monthly from 2027q2", month, Some("2027-04-01"), None),
            (
                "monthly from 2027 to 2027q3",
                month,
                Some("2027-01-01"),
                Some("2027-07-01"),
            ),
            ("monthly since 2027-03", month, Some("2027-03-01"), None),
            ("monthly until 2027-04", month, None, Some("2027-04-01")),
            (
                "monthly since 2027-03 to 2027-06",
                month,
                Some("2027-03-01"),
                Some("2027-06-01"),
            ),
            (
                "monthly since 2027-03..2027-06",
                month,
                Some("2027-03-01"),
                Some("2027-06-01"),
            ),
            (
                "monthly 2027-03 to 2027-06",
                month,
                Some("2027-03-01"),
                Some("2027-06-01"),
            ),
            ("monthly .. 2027-06", month, None, Some("2027-06-01")),
            ("monthly - 2027-06", month, None, Some("2027-06-01")),
            (
                "monthly 2027-01..2027-04",
                month,
                Some("2027-01-01"),
                Some("2027-04-01"),
            ),
            (
                "monthly from 2027-01 .. 2027-04",
                month,
                Some("2027-01-01"),
                Some("2027-04-01"),
            ),
            (
                "monthly from 2027-01 - 2027-04",
                month,
                Some("2027-01-01"),
                Some("2027-04-01"),
            ),
            (
                "monthly from 2027-01-2027-04",
                month,
                Some("2027-01-01"),
                Some("2027-04-01"),
            ),
            (
                "monthly 2027-02-15..2027-05",
                month,
                Some("2027-02-15"),
                Some("2027-05-01"),
            ),
            (
                "every 2 weeks in 2027-03",
                every(PeriodExpr::Weekly, 2),
                Some("2027-03-01"),
                Some("2027-04-01"),
            ),
            (
                "every 1st apr in 2027",
                annual(4, 1),
                Some("2027-01-01"),
                Some("2028-01-01"),
            ),
            // A multiplier that looks like a year is still a multiplier.
            (
                "every 1000 days",
                every(PeriodExpr::Daily, 1000),
                None,
                None,
            ),
            // One-offs: a quarter, a range, `in`.
            ("2027q2", PeriodKind::Once, Some("2027-04-01"), None),
            ("2027Q3", PeriodKind::Once, Some("2027-07-01"), None),
            ("in 2027-03", PeriodKind::Once, Some("2027-03-01"), None),
            (
                "2027-03..2027-06",
                PeriodKind::Once,
                Some("2027-03-01"),
                Some("2027-06-01"),
            ),
            (
                "2027-03 to 2027-06",
                PeriodKind::Once,
                Some("2027-03-01"),
                Some("2027-06-01"),
            ),
        ] {
            let spec = period_spec(expr);
            assert_eq!(spec.kind, kind, "kind of `{expr}`");
            assert_eq!(spec.start.as_deref(), start, "start of `{expr}`");
            assert_eq!(spec.end.as_deref(), end, "end of `{expr}`");
            assert_eq!(spec.raw, expr, "raw of `{expr}`");
        }
    }

    /// The interval words are cut where the PARSER cut them, whatever the
    /// bounds clause looks like — which is what lets the projections table
    /// rewrite a bound without eating part of the interval.
    #[test]
    fn interval_words_stop_where_the_bounds_start() {
        for (expr, words) in [
            ("every 1st Apr from 2027", Some("every 1st Apr")),
            ("every 1st apr in 2027", Some("every 1st apr")),
            (
                "every 15th Nov of year to 2028",
                Some("every 15th Nov of year"),
            ),
            ("every 2 weeks 2027-01..2027-04", Some("every 2 weeks")),
            ("every 3rd mon since 2027-02", Some("every 3rd mon")),
            ("every mon,wed,fri until 2027-06", Some("every mon,wed,fri")),
            ("every 1000 days", Some("every 1000 days")),
            ("2027q2", None),
        ] {
            assert_eq!(
                period_spec(expr).interval_words().as_deref(),
                words,
                "words of `{expr}`"
            );
        }
    }

    /// Only a BARE fixed interval answers `interval()`. The single word it
    /// returns cannot express a multiplier, an anchor or a bound, so a caller
    /// that wrote it back would drop the rest of the rule.
    #[test]
    fn only_a_bare_fixed_interval_is_simple() {
        for expr in ["daily", "weekly", "monthly", "quarterly", "yearly"] {
            let spec = period_spec(expr);
            assert!(spec.interval().is_some(), "`{expr}` is a bare interval");
            assert!(spec.is_simple(), "`{expr}` is editable");
        }
        for expr in [
            "every 2 weeks",
            "biweekly",
            "every 15th day of month",
            "every tuesday",
            "every 12/25",
            "monthly from 2027",
            "monthly to 2027",
            "2027-03-01",
        ] {
            let spec = period_spec(expr);
            assert_eq!(spec.interval(), None, "`{expr}` is not a bare interval");
            assert!(!spec.is_simple(), "`{expr}` is not editable");
        }
    }

    /// The scenario editor's reading of a header: the interval regardless of
    /// bounds, and the words its bounds can be rewritten around. Each header is
    /// written with AND without a two-space description, because the
    /// description is cut before the period is parsed and must never leak into
    /// either answer.
    #[test]
    fn bounded_headers_keep_their_interval_and_their_words() {
        /// `(header, bounded_interval, interval_words, start, end)`.
        type Case<'a> = (
            &'a str,
            Option<PeriodExpr>,
            Option<&'a str>,
            Option<&'a str>,
            Option<&'a str>,
        );
        let cases: [Case; 10] = [
            (
                "monthly",
                Some(PeriodExpr::Monthly),
                Some("monthly"),
                None,
                None,
            ),
            (
                "monthly from 2026-11-01",
                Some(PeriodExpr::Monthly),
                Some("monthly"),
                Some("2026-11-01"),
                None,
            ),
            (
                "monthly to 2027-06-01",
                Some(PeriodExpr::Monthly),
                Some("monthly"),
                None,
                Some("2027-06-01"),
            ),
            (
                "yearly from 2026-11-01 to 2030-01-01",
                Some(PeriodExpr::Yearly),
                Some("yearly"),
                Some("2026-11-01"),
                Some("2030-01-01"),
            ),
            (
                "every month from 2026-11",
                Some(PeriodExpr::Monthly),
                Some("every month"),
                Some("2026-11-01"),
                None,
            ),
            // A multiplier is not one of the five words, but its bounds are
            // still the trailing clause.
            (
                "every 3 months from 2026-11-01",
                None,
                Some("every 3 months"),
                Some("2026-11-01"),
                None,
            ),
            (
                "every 15th day of month to 2027-01-01",
                None,
                Some("every 15th day of month"),
                None,
                Some("2027-01-01"),
            ),
            // Case is kept as written.
            ("Every 2 Weeks", None, Some("Every 2 Weeks"), None, None),
            // A one-off's bounds ARE its occurrence; nothing to rewrite around.
            (
                "from 2027-01-01 to 2027-02-01",
                None,
                None,
                Some("2027-01-01"),
                Some("2027-02-01"),
            ),
            // Bounds beside a weekday set are rewritten around its words.
            (
                "every weekday from 2026-11-01",
                None,
                Some("every weekday"),
                Some("2026-11-01"),
                None,
            ),
        ];
        for (expr, interval, words, start, end) in cases {
            for header in [
                expr.to_string(),
                format!("{expr}  unbudgeted — average over 2025-11-01 to 2026-10-22"),
            ] {
                let spec = period_spec(&header);
                assert_eq!(spec.raw, expr, "raw of `{header}`");
                assert_eq!(spec.bounded_interval(), interval, "interval of `{header}`");
                assert_eq!(
                    spec.interval_words().as_deref(),
                    words,
                    "words of `{header}`"
                );
                assert_eq!(spec.start.as_deref(), start, "start of `{header}`");
                assert_eq!(spec.end.as_deref(), end, "end of `{header}`");
            }
        }
    }

    /// Forms hledger ITSELF rejects, a single-space description (which hledger
    /// reads as part of the period expression, and errors on), and relative
    /// dates. All degrade to a locked rule with `raw` intact — **the journal still
    /// opens**, which is the whole point of this change.
    #[test]
    fn hledger_rejects_degrade_rather_than_failing_the_journal() {
        for expr in [
            "every last day of month",
            "every 15th,last day of month",
            "every jan",
            // hledger: "a double space is required between period expression and
            // description/comment". A single space means the rest is part of the
            // expression, which is what makes this unreadable rather than a rule
            // named "budget".
            "monthly budget",
            // Rejected by hledger 1.52 too (verified 2026-10-27): a month-name
            // day needs its ordinal suffix, a quarter has no ordinal form, an
            // interval word takes no multiplier, `MM/DD` must be a real day,
            // two bare dates need a `to` between them, `until` only opens a
            // range with no start, and `in` takes one date, not a range.
            "every nov 15",
            "every 15 nov",
            "every 1 apr",
            "every 2nd quarter",
            "every 2 weekly",
            "every 2/30",
            "every 4/31",
            "every 32nd day",
            "monthly 2027-01 2027-04",
            "monthly since 2027-03 until 2027-06",
            "monthly from 2027-01-15 until 2027-05",
            "monthly 2027-03 until 2027-06",
            "monthly in 2027-03..2027-06",
            "monthly in from 2027-03",
            // Relative dates: hledger reads them against today, and a rule
            // must not change meaning overnight.
            "monthly from jan to apr",
            "monthly in this year",
        ] {
            let spec = period_spec(expr);
            assert_eq!(
                spec.kind,
                PeriodKind::Unsupported,
                "`{expr}` is unsupported"
            );
            assert_eq!(spec.raw, expr, "`{expr}` keeps its raw text");
            assert_eq!(spec.interval(), None);
        }
    }

    /// A rule's header comment and its tags are kept, by the same code that
    /// keeps a posting's — so `growth: 3%/yr` means the same thing in both
    /// places. Before this, the header comment was read and thrown away.
    #[test]
    fn periodic_rule_keeps_its_comment_and_tags() {
        let journal = parse_journal(
            "~ monthly  rent  ; growth: 3%/yr, scenario: base\n    (a)  $1\n",
            "t.journal",
        )
        .unwrap();
        let rule = &journal.periodic_transactions[0];
        assert_eq!(rule.description, "rent");
        assert_eq!(rule.comment, "growth: 3%/yr, scenario: base\n");
        assert_eq!(
            rule.tags,
            vec![
                ("growth".to_string(), "3%/yr".to_string()),
                ("scenario".to_string(), "base".to_string()),
            ]
        );
    }

    /// A comment-only line before the first posting belongs to the RULE; one
    /// after a posting belongs to that posting. Verified against hledger 1.52:
    /// `print --forecast` emits the first above the forecast transaction's
    /// postings and the second below them.
    #[test]
    fn periodic_rule_comment_continuation_splits_at_the_first_posting() {
        let journal = parse_journal(
            "~ monthly  rent  ; own: x\n    ; more: y\n    (a)  $1\n    ; posting: z\n",
            "t.journal",
        )
        .unwrap();
        let rule = &journal.periodic_transactions[0];
        assert_eq!(rule.comment, "own: x\nmore: y\n");
        assert_eq!(
            rule.tags,
            vec![
                ("own".to_string(), "x".to_string()),
                ("more".to_string(), "y".to_string()),
            ]
        );
        assert_eq!(rule.postings[0].comment, "\nposting: z\n");
        assert_eq!(
            rule.postings[0].tags,
            vec![("posting".to_string(), "z".to_string())]
        );
    }

    #[test]
    fn comment_block_is_skipped() {
        let text = concat!(
            "comment\n",
            "this is ignored\n",
            "  2024-01-01 not a real txn\n",
            "end comment\n",
            "\n",
            "2024-01-02 real\n",
            "    expenses:x   $2.00\n",
            "    assets:bank\n",
        );
        let journal = parse_journal(text, "t.journal").unwrap();
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].description, "real");
    }

    #[test]
    fn include_directive_merges_files_and_continues_tindex() {
        let dir = std::env::temp_dir().join("ledgeline_parse_include_test");
        std::fs::create_dir_all(&dir).unwrap();
        let sub = dir.join("sub.journal");
        let main = dir.join("main.journal");
        std::fs::write(
            &sub,
            "2024-02-01 sub txn\n    expenses:foo   $5.00\n    assets:bank\n",
        )
        .unwrap();
        let main_text = "2024-01-01 main txn\n    expenses:bar   $3.00\n    assets:bank\n\ninclude sub.journal\n";
        std::fs::write(&main, main_text).unwrap();

        let text = std::fs::read_to_string(&main).unwrap();
        let journal = parse_journal(&text, &main.to_string_lossy()).unwrap();
        assert_eq!(journal.transactions.len(), 2);
        assert_eq!(journal.transactions[0].description, "main txn");
        assert_eq!(journal.transactions[1].description, "sub txn");
        assert_eq!(journal.transactions[0].index, Tindex(1));
        assert_eq!(journal.transactions[1].index, Tindex(2));

        // Each transaction records the resolved file it was parsed from: the main
        // txn from the main file, the included txn from the sub file. The sub
        // txn's span is relative to sub.journal (line 1), NOT the main file.
        assert_eq!(
            journal.transactions[0].source_file,
            resolve_source_file(&main)
        );
        assert_eq!(
            journal.transactions[1].source_file,
            resolve_source_file(&sub)
        );
        assert_ne!(
            journal.transactions[0].source_file,
            journal.transactions[1].source_file
        );
        assert_eq!(journal.transactions[1].source_span.0.line, 1);
    }

    #[test]
    fn source_files_records_main_and_directive_only_includes() {
        // `source_files` must cover EVERY file the journal reads — including an
        // `include`d file that contributes only directives (no transactions), which
        // per-transaction `source_file` tracking would miss. The live-reload
        // watcher relies on this to monitor the full dependency set.
        let dir = std::env::temp_dir().join("ledgeline_parse_source_files_test");
        std::fs::create_dir_all(&dir).unwrap();
        let accounts = dir.join("accounts.journal");
        let txns = dir.join("txns.journal");
        let main = dir.join("main.journal");
        // A directive-only include: declares an account, holds no transactions.
        std::fs::write(&accounts, "account assets:bank\n").unwrap();
        std::fs::write(
            &txns,
            "2024-02-01 sub txn\n    expenses:foo   $5.00\n    assets:bank\n",
        )
        .unwrap();
        std::fs::write(&main, "include accounts.journal\ninclude txns.journal\n").unwrap();

        let text = std::fs::read_to_string(&main).unwrap();
        let journal = parse_journal(&text, &main.to_string_lossy()).unwrap();

        // Main file first, then each include in read order, all resolved.
        assert_eq!(
            journal.source_files,
            vec![
                resolve_source_file(&main),
                resolve_source_file(&accounts),
                resolve_source_file(&txns),
            ]
        );
        // The directive-only include is present even though no transaction points
        // at it (only `txns.journal` shows up in transaction `source_file`s).
        assert!(
            journal
                .source_files
                .contains(&resolve_source_file(&accounts)),
            "directive-only include must be tracked for watching"
        );
        assert!(
            !journal
                .transactions
                .iter()
                .any(|t| t.source_file == resolve_source_file(&accounts)),
            "the directive-only include has no transactions"
        );
    }

    #[test]
    fn source_files_dedups_a_repeated_include() {
        // The same file included twice is recorded once (a stable set for watching).
        let dir = std::env::temp_dir().join("ledgeline_parse_source_files_dedup_test");
        std::fs::create_dir_all(&dir).unwrap();
        let sub = dir.join("sub.journal");
        let main = dir.join("main.journal");
        std::fs::write(&sub, "account assets:bank\n").unwrap();
        std::fs::write(&main, "include sub.journal\ninclude sub.journal\n").unwrap();

        let text = std::fs::read_to_string(&main).unwrap();
        let journal = parse_journal(&text, &main.to_string_lossy()).unwrap();
        assert_eq!(
            journal.source_files,
            vec![resolve_source_file(&main), resolve_source_file(&sub)]
        );
    }

    #[test]
    fn overrides_reparse_included_file_from_memory() {
        // `parse_journal_with_overrides` reads any file present in the map from
        // memory (here an EDITED sub file) and everything else from disk.
        let dir = std::env::temp_dir().join("ledgeline_parse_overrides_test");
        std::fs::create_dir_all(&dir).unwrap();
        let sub = dir.join("sub.journal");
        let main = dir.join("main.journal");
        std::fs::write(
            &sub,
            "2024-02-01 sub txn\n    expenses:foo   $5.00\n    assets:bank\n",
        )
        .unwrap();
        std::fs::write(&main, "include sub.journal\n").unwrap();

        // Override the sub file with an edited account name; the main file is read
        // from disk (absent from the map).
        let mut overrides = HashMap::new();
        overrides.insert(
            resolve_source_file(&sub),
            "2024-02-01 sub txn\n    expenses:renamed   $5.00\n    assets:bank\n".to_string(),
        );
        let journal = parse_journal_with_overrides(&main.to_string_lossy(), &overrides).unwrap();
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(
            journal.transactions[0].postings[0].account,
            AccountName("expenses:renamed".into())
        );
        // The reparsed txn is still attributed to the sub file.
        assert_eq!(
            journal.transactions[0].source_file,
            resolve_source_file(&sub)
        );
    }

    #[test]
    fn unsupported_directive_still_errors_with_location() {
        let err = parse_journal("apply account assets:foo\n", "t.journal").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("t.journal:1"), "{msg}");
        assert!(msg.contains("unsupported directive: 'apply'"), "{msg}");
    }

    #[test]
    fn alias_directives_parse_in_both_forms_and_keep_their_order() {
        // Every split rule here was checked against hledger 1.52, including the
        // two that look wrong: a plain alias splits at the FIRST `=`, and a
        // regex one does not split at an `=` inside its slashes.
        let text = concat!(
            "alias PW Roth IRA - 3077 = assets:morganstanley:pw-roth-ira\n",
            "alias /^CC (.+)$/ = liabilities:\\1\n",
            "alias a = b = c\n",
            "alias /a=b/ = c\n",
            "alias /a\\/b/ = c\n",
            "alias trailing = b ; not a comment\n",
            "\n",
            "2026-01-01 t\n",
            "    a   $1\n",
            "    b\n",
        );
        let journal = parse_journal(text, "t.journal").unwrap();
        let seen: Vec<(&str, &str, bool, u32)> = journal
            .aliases
            .iter()
            .map(|alias| {
                (
                    alias.pattern.as_str(),
                    alias.replacement.as_str(),
                    alias.regex,
                    alias.position.line,
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (
                    "PW Roth IRA - 3077",
                    "assets:morganstanley:pw-roth-ira",
                    false,
                    1
                ),
                ("^CC (.+)$", "liabilities:\\1", true, 2),
                ("a", "b = c", false, 3),
                ("a=b", "c", true, 4),
                ("a\\/b", "c", true, 5),
                // NOT comment-stripped: hledger declares the account literally
                // named `b ; not a comment`.
                ("trailing", "b ; not a comment", false, 6),
            ]
        );
        assert!(journal.aliases.iter().all(|alias| !alias.ended));
        // Recorded, never applied: the transaction's accounts are as written.
        assert_eq!(
            journal.transactions[0].postings[0].account,
            AccountName("a".into())
        );
    }

    #[test]
    fn end_aliases_closes_the_scope_and_a_later_alias_reopens_it() {
        let text = concat!(
            "alias one = a:one\n",
            "alias two = a:two\n",
            "end aliases ; done\n",
            "alias three = a:three\n",
        );
        let journal = parse_journal(text, "t.journal").unwrap();
        let ended: Vec<bool> = journal.aliases.iter().map(|alias| alias.ended).collect();
        assert_eq!(ended, vec![true, true, false]);
        let in_force: Vec<&str> = journal
            .aliases_in_force()
            .map(|alias| alias.pattern.as_str())
            .collect();
        assert_eq!(in_force, vec!["three"]);
    }

    #[test]
    fn malformed_alias_and_end_forms_are_refused() {
        // hledger errors on each of these, so accepting them would let a journal
        // hledger will not read open here and report numbers hledger never would.
        for (src, needle) in [
            ("alias foo\n", "malformed directive"),
            ("alias /abc = d\n", "malformed directive"),
            ("end alias\n", "unsupported directive: 'end'"),
            ("end apply account\n", "unsupported directive: 'end'"),
        ] {
            let err = parse_journal(src, "t.journal").unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains(needle), "{src:?}: {msg}");
            assert!(msg.contains("t.journal:1"), "{src:?}: {msg}");
        }
    }

    #[test]
    fn alias_scope_flows_into_an_include_and_never_back_out() {
        // All three directions verified against hledger 1.52: an alias reaches a
        // file included after it; an alias declared inside an include does not
        // escape it; and an `end aliases` inside the include kills the parent's
        // alias only for the rest of that file.
        let dir = std::env::temp_dir().join("ledgeline_parse_alias_scope_test");
        std::fs::create_dir_all(&dir).unwrap();
        let sub = dir.join("sub.journal");
        let main = dir.join("main.journal");
        std::fs::write(&sub, "alias inner = a:inner\nend aliases\n").unwrap();
        std::fs::write(
            &main,
            "alias outer = a:outer\ninclude sub.journal\nalias tail = a:tail\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&main).unwrap();
        let journal = parse_journal(&text, &main.to_string_lossy()).unwrap();
        let seen: Vec<(&str, bool)> = journal
            .aliases
            .iter()
            .map(|alias| (alias.pattern.as_str(), alias.ended))
            .collect();
        assert_eq!(
            seen,
            vec![
                // The include's `end aliases` took `outer` out of scope for the
                // rest of the INCLUDED file, and the parent resumed afterwards —
                // so `outer` is still in force where an import would append.
                ("outer", false),
                ("inner", true),
                ("tail", false),
            ]
        );
        assert_eq!(
            journal.aliases[1].source_file,
            resolve_source_file(&sub),
            "an alias records the file it was declared in"
        );
    }

    #[test]
    fn errors_report_file_line_and_content() {
        // A stray indented line (e.g. an unsupported subdirective) is reported
        // with the file, the line number, and the offending line's text — so
        // the source is unambiguous even across `include`s.
        let err =
            parse_journal("account foo\n    subdirective here\n", "acct.journal").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("acct.journal:2"), "{msg}");
        assert!(msg.contains("subdirective here"), "{msg}");
    }

    #[test]
    fn indented_comment_lines_are_skipped() {
        // hledger allows comment lines to be indented (e.g. a note trailing a
        // block of `P` price directives); they must not be treated as a stray
        // indent.
        let text = concat!(
            "P 2026-06-30 AAPL $270.25\n",
            "                    ; Prices fetched from yahoo on 2026-06-30T22:28:02-06:00\n",
            "\n",
            "2026-01-01 t\n",
            "    expenses:x   $1.00\n",
            "    assets:bank\n",
        );
        let journal = parse_journal(text, "prices.journal").unwrap();
        assert_eq!(journal.prices.len(), 1);
        assert_eq!(journal.transactions.len(), 1);
    }

    #[test]
    fn price_directive_optional_time_and_high_precision() {
        // `P DATE TIME COMMODITY PRICE`: the clock time is skipped (only the day
        // is kept), and a many-place price parses exactly into an i128 mantissa.
        let journal = parse_journal(
            "P 2026-06-30 00:00:00 AAPL $289.3599853515625\n",
            "prices.journal",
        )
        .unwrap();
        assert_eq!(journal.prices.len(), 1);
        let price = &journal.prices[0];
        assert_eq!(price.date, "2026-06-30");
        assert_eq!(price.commodity, Commodity("AAPL".to_string()));
        assert_eq!(price.price.commodity, Commodity("$".to_string()));
        // Capped to 10 places, half-to-even — matches hledger (…5625 rounds up).
        assert_eq!(price.price.quantity, Dec::new(2_893_599_853_516, 10));
    }

    #[test]
    fn default_year_and_date_normalization() {
        // `Y` sets the default year; dates normalize to ISO with `/`/`.`
        // separators and unpadded components handled (matches hledger).
        let text = concat!(
            "Y 2026\n",
            "\n",
            "01-15 yearless\n",
            "    a   $1\n",
            "    b\n",
            "\n",
            "2026/2/5 slash unpadded\n",
            "    a   $1\n",
            "    b\n",
            "\n",
            "2024.07.01 dot full\n",
            "    a   $1\n",
            "    b\n",
        );
        let journal = parse_journal(text, "j.journal").unwrap();
        let dates: Vec<&str> = journal
            .transactions
            .iter()
            .map(|t| t.date.as_str())
            .collect();
        assert_eq!(dates, vec!["2026-01-15", "2026-02-05", "2024-07-01"]);
    }

    #[test]
    fn yearless_date_without_y_errors_clearly() {
        let err = parse_journal("01-15 x\n    a   $1\n    b\n", "j.journal").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("j.journal:1"), "{msg}");
        assert!(msg.contains("no year"), "{msg}");
    }

    #[test]
    fn sign_before_left_commodity() {
        // `-$1,658.91` (sign outside a left-side commodity) parses to a negative
        // `$` amount — both this and `$-1,658.91` are valid hledger.
        let styles = HashMap::new();
        let amount = parse_amount("-$1,658.91", ctx(&styles)).unwrap();
        assert_eq!(amount.commodity, Commodity("$".to_string()));
        assert_eq!(amount.quantity, Dec::new(-165_891, 2));
        assert_eq!(amount.style.side, CommoditySide::Left);
        assert!(!amount.style.spaced);

        // The reported real-world line: a `==` assertion whose amount is written
        // sign-before-commodity now parses.
        let journal = parse_journal(
            "2026-01-01 t\n    liabilities:citi   $0  ==  -$1,658.91\n    equity:x   $0\n",
            "t.journal",
        )
        .unwrap();
        let assertion = journal.transactions[0].postings[0]
            .balance_assertion
            .as_ref()
            .unwrap();
        assert!(assertion.total); // `==`
        assert_eq!(assertion.amount.quantity, Dec::new(-165_891, 2));
    }
}
