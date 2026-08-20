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

use crate::decimal::{Dec, DecError};
use crate::model::{
    AccountDeclaration, AccountName, AliasDirective, Amount, AmountStyle, BalanceAssertion,
    Commodity, CommoditySide, Cost, CostKind, DigitGroups, Journal, PeriodExpr,
    PeriodicTransaction, Posting, PostingType, PriceDirective, SourcePos, Status, Tindex,
    Transaction,
};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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
    /// A `~` periodic rule's period expression is not one of the supported fixed
    /// intervals (`daily`/`weekly`/`monthly`/`quarterly`/`yearly`). Richer period
    /// expressions are deferred; the period expr and description must be
    /// separated by two-or-more spaces.
    #[error("unsupported period expression: '{0}'")]
    UnsupportedPeriodExpr(String),
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
}

/// Mutable accumulators shared across the top-level file and any `include`d
/// files, so transaction indices and declarations continue seamlessly.
struct Ctx {
    styles: Styles,
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
    /// Every source file read so far (main + `include`s), in first-read order and
    /// deduplicated, as resolved absolute paths. Feeds [`Journal::source_files`].
    source_files: Vec<PathBuf>,
    /// The directory every `include` must resolve inside: the main journal
    /// file's own directory, canonicalized. See [`admit_include`].
    include_root: PathBuf,
    /// The canonical paths of the files on the current `include` stack — the
    /// ancestors of the file being parsed, NOT including that file itself. A
    /// stack (rather than a global visited set) so a diamond include resolves
    /// like hledger's: including the same file from two different branches is
    /// legal and parses it twice.
    include_stack: Vec<PathBuf>,
    /// How many `include`s have been admitted so far, capped by
    /// [`MAX_INCLUDE_FILES`].
    includes_admitted: usize,
    tindex: u32,
}

impl Ctx {
    fn new(main_source_name: &str) -> Self {
        Ctx {
            styles: HashMap::new(),
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
            source_files: Vec::new(),
            include_root: include_root_for(main_source_name),
            include_stack: Vec::new(),
            includes_admitted: 0,
            tindex: 0,
        }
    }

    fn into_journal(self, source_name: &str) -> Journal {
        Journal {
            source_name: source_name.to_string(),
            source_files: self.source_files,
            transactions: self.transactions,
            periodic_transactions: self.periodic_transactions,
            accounts: self.accounts,
            aliases: self.aliases,
            commodity_styles: self.commodity_styles,
            commodity_tags: self.commodity_tags,
            prices: self.prices,
            default_commodity: self.default_commodity.map(|(commodity, _style)| commodity),
        }
    }
}

/// The absolute key a journal file is addressed by: canonicalized when it exists
/// on disk, else the path as given (so an in-memory source with a placeholder
/// name still resolves to a stable key). Used to record each transaction's
/// [`Transaction::source_file`] and to key the editor's per-file ropes and the
/// in-memory override map consistently.
pub(crate) fn resolve_source_file(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Read a source file's text: from `overrides` (keyed by its resolved absolute
/// path, see [`resolve_source_file`]) when an entry is present there, else from
/// disk. This lets the editor reparse the WHOLE journal against edited but
/// not-yet-saved in-memory file contents.
fn read_source_text(
    resolved: &Path,
    overrides: Option<&HashMap<PathBuf, String>>,
) -> std::io::Result<String> {
    if let Some(map) = overrides
        && let Some(text) = map.get(&resolve_source_file(resolved))
    {
        return Ok(text.clone());
    }
    std::fs::read_to_string(resolved)
}

/// Parse `text` (the contents of the journal at `source_name`) into a balanced
/// [`Journal`]. `include`d files are resolved relative to `source_name`.
pub fn parse_journal(text: &str, source_name: &str) -> Result<Journal, ParseError> {
    let mut ctx = Ctx::new(source_name);
    parse_source(text, source_name, &mut ctx, None)?;
    Ok(ctx.into_journal(source_name))
}

/// Parse the journal rooted at `main_source_name`, resolving `include`s from an
/// in-memory `overrides` map before falling back to disk.
///
/// Any file whose resolved absolute path (see [`resolve_source_file`]) is a key
/// in `overrides` is read from that map; every other file (the main file
/// included) is read from disk exactly as [`parse_journal`] would. This is the
/// editor's reparse-to-validate entry point: it lets an edit that spans an
/// `include`d file be validated against the EDITED in-memory content of every
/// touched file, not the stale on-disk copies.
pub fn parse_journal_with_overrides(
    main_source_name: &str,
    overrides: &HashMap<PathBuf, String>,
) -> Result<Journal, ParseError> {
    let main_text = read_source_text(Path::new(main_source_name), Some(overrides))
        .map_err(|e| ParseError::Include(format!("could not read {main_source_name}: {e}")))?;
    let mut ctx = Ctx::new(main_source_name);
    parse_source(&main_text, main_source_name, &mut ctx, Some(overrides))?;
    Ok(ctx.into_journal(main_source_name))
}

/// Parse one journal source (the top file or an included one) into `ctx`. When
/// `overrides` is `Some`, `include`d files are read from it before disk.
fn parse_source(
    text: &str,
    source_name: &str,
    ctx: &mut Ctx,
    overrides: Option<&HashMap<PathBuf, String>>,
) -> Result<(), ParseError> {
    let source_file = resolve_source_file(source_name);
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

        let keyword = trimmed.split_whitespace().next().unwrap_or("");
        match keyword {
            "account" => {
                let decl = parse_account_directive(trimmed, line_no)
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
                };
                let price = parse_price_directive(trimmed, amt, ctx.default_year)
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
                let (target, as_written) = resolve_include(trimmed, source_name)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                // Confinement, cycle and budget checks all run BEFORE the file is
                // read, so a rejected include never has its contents echoed back
                // through a parse error (SEC-6's read oracle).
                let path = admit_include(&target, &as_written, &source_file, ctx)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                let included = read_source_text(&path, overrides).map_err(|e| {
                    locate(
                        source_name,
                        line_no,
                        line,
                        ParseError::Include(format!("{}: {e}", path.display())),
                    )
                })?;
                // Push the INCLUDING file so the nested parse sees its own full
                // ancestor chain; pop unconditionally to keep the stack accurate
                // if the caller ever recovers from the error.
                ctx.include_stack.push(source_file.clone());
                // Aliases flow INTO an include and never back out (verified
                // against hledger 1.52), so the nested parse inherits this
                // scope and the parent's is restored whatever it did with it —
                // including an `end aliases`, which kills the parent's aliases
                // only within the child.
                let outer_scope = ctx.alias_scope.clone();
                let nested = parse_source(&included, &path.to_string_lossy(), ctx, overrides);
                ctx.alias_scope = outer_scope;
                ctx.include_stack.pop();
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
                };
                let (periodic, next) =
                    parse_periodic_transaction(&lines, i, amt, source_name, ctx.default_year)?;
                ctx.periodic_transactions.push(periodic);
                i = next;
                continue;
            }
            // Auto-posting (`=`) rule blocks: still skipped (excluded from
            // `jtxns`, like hledger-web's `/transactions`).
            "=" => {
                i = skip_indented_block(&lines, i);
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
fn split_account_name(body: &str) -> Option<(String, Option<&str>)> {
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
        Some(comment) => Some((name, Some(comment))),
        None if rest.is_empty() => Some((name, None)),
        None => None,
    }
}

fn parse_account_directive(line: &str, line_no: u32) -> Result<AccountDeclaration, ParseError> {
    let after = line
        .strip_prefix("account")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (name, comment) = split_account_name(after)
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
        precision,
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

/// Resolve an `include PATH` target relative to the including file's directory.
/// Returns the joined path plus the target exactly as written in the directive
/// — the latter is what diagnostics quote, so a rejected include never discloses
/// a path the journal did not already name (e.g. a symlink's target).
fn resolve_include(line: &str, source_name: &str) -> Result<(PathBuf, String), ParseError> {
    let after = line
        .strip_prefix("include")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (path_part, _comment) = split_comment(after);
    let path_str = path_part.trim();
    if path_str.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    let path = Path::new(path_str);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let base = Path::new(source_name)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        base.join(path)
    };
    Ok((joined, path_str.to_string()))
}

/// The directory `include`s are confined to: the main journal file's own
/// directory, canonicalized. A main file named without a directory (a bare
/// `t.journal`, or an in-memory placeholder) roots at the current directory,
/// which is exactly where [`resolve_include`] already resolves its relative
/// includes.
pub(crate) fn include_root_for(main_source_name: &str) -> PathBuf {
    let main = canonical_include(Path::new(main_source_name));
    match main.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The absolute, symlink-free form of an `include` target. Uses
/// [`std::fs::canonicalize`] when the whole path exists — the only case that can
/// leak content — so `..` traversal and symlinks are both resolved before the
/// confinement test.
///
/// A target that is not on disk (a not-yet-saved file supplied through the
/// editor's override map, or simply a typo) cannot be canonicalized outright, so
/// its deepest EXISTING ancestor is canonicalized and the remaining components
/// re-appended. That matters for correctness as much as for security: on macOS
/// the journal directory itself routinely canonicalizes through a symlink
/// (`/tmp` -> `/private/tmp`), and a purely lexical fallback would then read as
/// "outside the journal directory" for an ordinary misspelled sibling.
pub(crate) fn canonical_include(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let normalized = lexically_normalize(&absolute);

    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = normalized.clone();
    while let (Some(name), Some(parent)) = (
        cursor.file_name().map(std::ffi::OsStr::to_os_string),
        cursor.parent().map(Path::to_path_buf),
    ) {
        trailing.push(name);
        if let Ok(canonical) = std::fs::canonicalize(&parent) {
            return trailing
                .iter()
                .rev()
                .fold(canonical, |resolved, part| resolved.join(part));
        }
        cursor = parent;
    }
    normalized
}

/// Collapse `.` and `..` components textually, without touching the filesystem.
fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    path.components()
        .fold(PathBuf::new(), |mut out, component| {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    // `pop` fails only at a root/prefix, where `..` has nowhere
                    // to go; keep it there so the result still fails any
                    // containment test rather than silently becoming the root.
                    if !out.pop() {
                        out.push(Component::ParentDir);
                    }
                }
                other => out.push(other),
            }
            out
        })
}

/// The containment test, named once: `path`'s canonical form when it lies inside
/// `root`, otherwise `None`.
///
/// Extracted from [`admit_include`] so the `include` guard (SEC-6) and the
/// import-rules discovery guard cannot drift apart — a second, hand-rolled
/// traversal check is exactly the kind of near-duplicate that gets one of its
/// copies fixed and not the other. Canonicalizing FIRST is the whole point: `..`
/// components and symlinks are both resolved before the prefix comparison, so a
/// lexical-only test cannot be walked around.
///
/// Callers own the error message, because what may be disclosed differs by
/// caller: an `include` diagnostic quotes the journal directory (the user named
/// it), while a rules-file diagnostic quotes neither path.
///
/// `pub` rather than `pub(crate)` since the enhanced-import work: writing an
/// `hledger.conf` is a new write target in `ledgeline-server`, and a second
/// hand-rolled traversal check there is exactly the near-duplicate this function
/// was extracted to prevent. It answers usefully for a path that does not exist
/// yet — [`canonical_include`] canonicalizes the deepest existing ancestor and
/// re-joins the rest — which is what a not-yet-created config file needs.
pub fn confine(path: &Path, root: &Path) -> Option<PathBuf> {
    let path = canonical_include(path);
    path.starts_with(root).then_some(path)
}

/// Decide whether an `include` target may be parsed, returning its canonical
/// path. Rejects, in order:
///
/// 1. **Escapes the journal directory** (SEC-6). An `include` naming an absolute
///    path, traversing with `..`, or crossing a symlink out of the tree turns a
///    hostile journal into a local file-read oracle: whatever the included file
///    fails to parse as is quoted back in the error, which reaches stderr and the
///    GUI's error dialog — and anything that DOES parse is absorbed into the
///    journal and served over HTTP. hledger permits all three; we deliberately
///    do not (see the module docs).
/// 2. **Cycles** (SEC-4) — the target is the including file itself or one of its
///    ancestors. Unbounded recursion here overflows the stack, which aborts the
///    process with `SIGABRT` and cannot be caught.
/// 3. **Depth / total-file budget** — bounds an acyclic include bomb, which
///    cycle detection alone does not.
fn admit_include(
    target: &Path,
    as_written: &str,
    includer: &Path,
    ctx: &mut Ctx,
) -> Result<PathBuf, ParseError> {
    let path = confine(target, &ctx.include_root).ok_or_else(|| {
        ParseError::Include(format!(
            "'{as_written}' resolves outside the journal directory {}; \
             includes may not escape the main journal's directory",
            ctx.include_root.display()
        ))
    })?;
    if path == includer || ctx.include_stack.contains(&path) {
        return Err(ParseError::Include(format!(
            "this included file forms a cycle: {}",
            path.display()
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

/// Advance past a block directive's indented body (`~`/`=` rules), returning the
/// index of the first following non-indented line.
fn skip_indented_block(lines: &[&str], start: usize) -> usize {
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        j += 1;
    }
    j
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
/// two-or-more spaces (matching hledger); only the fixed intervals are
/// supported (see [`parse_period_expr`]).
fn parse_periodic_transaction(
    lines: &[&str],
    start: usize,
    amt: AmountCtx,
    source_name: &str,
    default_year: Option<i32>,
) -> Result<(PeriodicTransaction, usize), ParseError> {
    let header_no = to_u32(start + 1);
    let header_line = lines[start];
    let after_tilde = header_line
        .trim_start()
        .strip_prefix('~')
        .unwrap_or("")
        .trim_start();
    let (main, _comment) = split_comment(after_tilde);
    // hledger requires a two-space gap between the period expression and the
    // description; `split_account_amount` splits on exactly that.
    let (period_part, desc_part) = split_account_amount(main.trim());
    let period = parse_period_expr(period_part.trim())
        .map_err(|e| locate(source_name, header_no, header_line, e))?;
    let description = desc_part.trim().to_string();

    let mut raw_postings: Vec<RawPosting> = Vec::new();
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        // PARSE-7: as in a real transaction, a comment-only line belongs to the
        // preceding posting. A rule has nowhere to keep one written before its
        // first posting (`PeriodicTransaction` has no comment field), so that
        // case is still skipped.
        if let Some(content) = comment_line_content(line) {
            if let Some(posting) = raw_postings.last_mut() {
                append_comment_line(&mut posting.comment, &mut posting.tags, content);
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

    let postings = balance_postings(raw_postings, header_no)
        .map_err(|e| locate(source_name, header_no, header_line, e))?;
    Ok((
        PeriodicTransaction {
            period,
            description,
            postings,
        },
        j,
    ))
}

/// Parse a periodic rule's period expression. Only the fixed intervals are
/// supported; anything else (multi-word/anchored/bounded expressions, or a
/// description not separated by two spaces) is deferred with a clear error.
fn parse_period_expr(expr: &str) -> Result<PeriodExpr, ParseError> {
    match expr {
        "daily" => Ok(PeriodExpr::Daily),
        "weekly" => Ok(PeriodExpr::Weekly),
        "monthly" => Ok(PeriodExpr::Monthly),
        "quarterly" => Ok(PeriodExpr::Quarterly),
        "yearly" => Ok(PeriodExpr::Yearly),
        other => Err(ParseError::UnsupportedPeriodExpr(other.to_string())),
    }
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
    let canonical = amt.styles.get(&commodity);
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
                    inferred_mark.unwrap_or_else(|| default_display_mark(inferred_groups.as_ref())),
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
    for posting in group() {
        if let Some(amount) = &posting.amount {
            let (commodity, quantity, precision, style) = cost_contribution(amount)?;
            match sums.iter_mut().find(|entry| entry.commodity == commodity) {
                Some(entry) => {
                    entry.total = entry.total.add(quantity)?;
                    entry.precision = entry.precision.max(precision);
                }
                None => sums.push(CommoditySum {
                    commodity,
                    total: quantity,
                    precision,
                    style,
                }),
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

/// Extract `name:value` tags from a comment body. The tag name is the last
/// whitespace-delimited token before a `:`; its value runs to the next comma.
fn parse_tags(comment: &str) -> Vec<(String, String)> {
    comment
        .split(',')
        .filter_map(|segment| {
            let colon = segment.find(':')?;
            let name = segment[..colon].split_whitespace().next_back()?;
            if name.is_empty() {
                return None;
            }
            let value = segment[colon + 1..].trim().to_string();
            Some((name.to_string(), value))
        })
        .collect()
}

/// Split a posting's `after-status` remainder into `(account, amount)` at the
/// first run of two-or-more spaces (or a tab). A single space is part of the
/// account name.
fn split_account_amount(text: &str) -> (&str, &str) {
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
) -> (Option<char>, Option<DigitGroups>, u32) {
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
        DigitGroups { mark, sizes }
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
            let decl = parse_account_directive(line, 1).expect("directive parses");
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
            assert!(parse_account_directive("account two:words  trailing junk", 1).is_err());
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
            assert!(parse_account_directive("account", 1).is_err());
            assert!(parse_account_directive("account   ", 1).is_err());
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
            Some(DigitGroups {
                mark: ',',
                sizes: vec![3]
            })
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
            Some(DigitGroups {
                mark: '.',
                sizes: vec![3]
            })
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
            Some(DigitGroups {
                mark: '.',
                sizes: vec![3]
            })
        );
        let counter = &postings[1].amounts[0];
        assert_eq!(counter.quantity, Dec::new(-123450, 2));
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
        assert_eq!(periodic.period, PeriodExpr::Monthly);
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

    #[test]
    fn periodic_rule_period_forms_and_deferrals() {
        // The five fixed intervals parse; a two-space gap separates an optional
        // description; a single space (or a richer period expression) is a clear
        // deferral error rather than a misparse.
        for (src, expected) in [
            ("~ daily\n    (a)  $1\n", PeriodExpr::Daily),
            ("~ weekly\n    (a)  $1\n", PeriodExpr::Weekly),
            ("~ monthly\n    (a)  $1\n", PeriodExpr::Monthly),
            ("~ quarterly\n    (a)  $1\n", PeriodExpr::Quarterly),
            ("~ yearly\n    (a)  $1\n", PeriodExpr::Yearly),
        ] {
            let journal = parse_journal(src, "t.journal").unwrap();
            assert_eq!(journal.periodic_transactions[0].period, expected);
            assert_eq!(journal.periodic_transactions[0].description, "");
        }

        // Single-space "description" is part of the period expression → deferred.
        let err = parse_journal("~ monthly budget\n    (a)  $1\n", "t.journal").unwrap_err();
        assert!(
            err.to_string().contains("unsupported period expression"),
            "{err}"
        );
        // A richer period expression is deferred too.
        let err = parse_journal("~ every 2 weeks\n    (a)  $1\n", "t.journal").unwrap_err();
        assert!(
            err.to_string().contains("unsupported period expression"),
            "{err}"
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
