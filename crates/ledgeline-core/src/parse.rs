//! The hledger journal parser.
//!
//! Parses the journal format the fixtures and real journals exercise:
//! `account`/`commodity`/`decimal-mark`/`P`/`include` directives, comment lines
//! and `comment`/`end comment` blocks, and transactions with statuses, codes,
//! descriptions, comments/tags, multi-space account/amount separation, costs
//! (`@`/`@@`), balance assertions (`=`), and single-posting amount inference.
//!
//! Periodic (`~`) and auto-posting (`=`) rule blocks are recognized and skipped:
//! hledger's `/transactions` (`jtxns`) likewise excludes periodic and
//! auto-generated postings, so skipping them keeps wire parity. They will be
//! captured for the budget report in a later phase.
//!
//! `Y` sets the default year for yearless dates; every transaction/`P` date is
//! normalized to ISO `YYYY-MM-DD` (accepting `-`/`/`/`.` separators). Directives
//! that would silently change results if ignored (`alias`, `apply account`, and
//! `D` default-commodity) are still rejected with a clear error rather than
//! misparsed.

use crate::decimal::{Dec, DecError};
use crate::model::{
    AccountDeclaration, AccountName, Amount, AmountStyle, BalanceAssertion, Commodity,
    CommoditySide, Cost, CostKind, DigitGroups, Journal, Posting, PriceDirective, SourcePos,
    Status, Tindex, Transaction,
};
use std::collections::HashMap;
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

/// Canonical display style per commodity, built from `commodity` directives (or
/// first occurrence).
type Styles = HashMap<Commodity, AmountStyle>;

/// The context needed to parse an amount token: the known commodity styles plus
/// the journal-wide default decimal mark (from a `decimal-mark` directive, if
/// any). `Copy` so it threads cheaply through the parse helpers.
#[derive(Clone, Copy)]
struct AmountCtx<'a> {
    styles: &'a Styles,
    default_mark: Option<char>,
}

/// Mutable accumulators shared across the top-level file and any `include`d
/// files, so transaction indices and declarations continue seamlessly.
struct Ctx {
    styles: Styles,
    default_decimal_mark: Option<char>,
    default_year: Option<i32>,
    commodity_styles: Vec<(Commodity, AmountStyle)>,
    accounts: Vec<AccountDeclaration>,
    prices: Vec<PriceDirective>,
    transactions: Vec<Transaction>,
    tindex: u32,
}

/// Parse `text` (the contents of the journal at `source_name`) into a balanced
/// [`Journal`]. `include`d files are resolved relative to `source_name`.
pub fn parse_journal(text: &str, source_name: &str) -> Result<Journal, ParseError> {
    let mut ctx = Ctx {
        styles: HashMap::new(),
        default_decimal_mark: None,
        default_year: None,
        commodity_styles: Vec::new(),
        accounts: Vec::new(),
        prices: Vec::new(),
        transactions: Vec::new(),
        tindex: 0,
    };
    parse_source(text, source_name, &mut ctx)?;

    Ok(Journal {
        source_name: source_name.to_string(),
        transactions: ctx.transactions,
        accounts: ctx.accounts,
        commodity_styles: ctx.commodity_styles,
        prices: ctx.prices,
    })
}

/// Parse one journal source (the top file or an included one) into `ctx`.
fn parse_source(text: &str, source_name: &str, ctx: &mut Ctx) -> Result<(), ParseError> {
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
            };
            let (txn, next) =
                parse_transaction(&lines, i, ctx.tindex, amt, source_name, ctx.default_year)?;
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
                let (commodity, style) = parse_commodity_directive(trimmed)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.styles.insert(commodity.clone(), style.clone());
                ctx.commodity_styles.push((commodity, style));
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
                };
                let price = parse_price_directive(trimmed, amt, ctx.default_year)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                ctx.prices.push(price);
            }
            "include" => {
                let path = resolve_include(trimmed, source_name)
                    .map_err(|e| locate(source_name, line_no, line, e))?;
                let included = std::fs::read_to_string(&path).map_err(|e| {
                    locate(
                        source_name,
                        line_no,
                        line,
                        ParseError::Include(format!("{}: {e}", path.display())),
                    )
                })?;
                parse_source(&included, &path.to_string_lossy(), ctx)?;
            }
            // Declarations with no effect on transaction parsing.
            "payee" | "tag" => {}
            // Periodic / auto-posting rule blocks: skip (excluded from `jtxns`,
            // like hledger-web's `/transactions`).
            "~" | "=" => {
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

fn parse_account_directive(line: &str, line_no: u32) -> Result<AccountDeclaration, ParseError> {
    let after = line
        .strip_prefix("account")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (name_part, comment) = split_comment(after);
    let name = name_part.trim();
    if name.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    let (comment_text, tags) = build_comment(comment);
    Ok(AccountDeclaration {
        name: AccountName(name.to_string()),
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

fn parse_commodity_directive(line: &str) -> Result<(Commodity, AmountStyle), ParseError> {
    let after = line
        .strip_prefix("commodity")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .trim_start();
    let (spec_part, _comment) = split_comment(after);
    let spec = spec_part.trim();
    let (commodity, number, side, spaced) = split_commodity_number(spec)?;
    let (decimal_mark, digit_groups, precision) = analyze_number(&number, None);
    let style = AmountStyle {
        side,
        spaced,
        decimal_mark: decimal_mark.unwrap_or('.'),
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
fn resolve_include(line: &str, source_name: &str) -> Result<PathBuf, ParseError> {
    let after = line
        .strip_prefix("include")
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?;
    let (path_part, _comment) = split_comment(after);
    let path_str = path_part.trim();
    if path_str.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    let path = Path::new(path_str);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let base = Path::new(source_name)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        Ok(base.join(path))
    }
}

fn parse_price_directive(
    line: &str,
    amt: AmountCtx,
    default_year: Option<i32>,
) -> Result<PriceDirective, ParseError> {
    let mut tokens = line.split_whitespace().peekable();
    let _p = tokens.next(); // "P"
    let date = tokens
        .next()
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .to_string();
    // hledger allows an optional clock time after the date
    // (`P DATE [HH:MM[:SS]] COMMODITY PRICE`); only the day is retained, matching
    // hledger's date-only market prices.
    if tokens.peek().is_some_and(|t| is_time_token(t)) {
        tokens.next();
    }
    let commodity = tokens
        .next()
        .ok_or_else(|| ParseError::MalformedDirective(line.to_string()))?
        .to_string();
    let price_str = tokens.collect::<Vec<_>>().join(" ");
    if price_str.is_empty() {
        return Err(ParseError::MalformedDirective(line.to_string()));
    }
    let price = parse_amount(price_str.trim(), amt)?;
    Ok(PriceDirective {
        date: normalize_date(&date, default_year)?,
        commodity: Commodity(commodity),
        price,
    })
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
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(ParseError::MalformedDate(format!("'{token}'")));
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
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
    account: String,
    amount: Option<Amount>,
    balance_assertion: Option<BalanceAssertion>,
    comment: String,
    tags: Vec<(String, String)>,
}

fn parse_transaction(
    lines: &[&str],
    start: usize,
    tindex: u32,
    amt: AmountCtx,
    source_name: &str,
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

    let mut raw_postings: Vec<RawPosting> = Vec::new();
    let mut last_posting_line = header_no;
    let mut j = start + 1;
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || !line.starts_with([' ', '\t']) {
            break;
        }
        if line.trim_start().starts_with(';') {
            // Transaction-level comment line inside the body; skip without
            // treating it as a posting.
            j += 1;
            continue;
        }
        let posting_no = to_u32(j + 1);
        let posting = parse_posting(line, posting_no, amt)
            .map_err(|e| locate(source_name, posting_no, line, e))?;
        raw_postings.push(posting);
        last_posting_line = posting_no;
        j += 1;
    }

    let postings = balance_postings(raw_postings, header_no, amt.styles)
        .map_err(|e| locate(source_name, header_no, lines[start], e))?;
    let source_span = (
        SourcePos {
            line: header_no,
            column: 1,
        },
        SourcePos {
            line: last_posting_line.saturating_add(1),
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
    };
    Ok((transaction, j))
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
    let account = account_part.trim().to_string();
    let amount_expr = amount_part.trim();

    let (amount, balance_assertion) = if amount_expr.is_empty() {
        (None, None)
    } else {
        parse_amount_and_assertion(amount_expr, main, line_no, amt)?
    };

    Ok(RawPosting {
        status,
        account,
        amount,
        balance_assertion,
        comment: comment_text,
        tags,
    })
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

        let assertion_amount = parse_amount(assertion_str, amt)?;
        let column = main
            .chars()
            .position(|c| c == '=')
            .map_or(1, |p| to_u32(p + 1));
        let amount = parse_primary_and_cost(amount_str, amt)?;
        let assertion = BalanceAssertion {
            amount: assertion_amount,
            inclusive,
            total,
            position: SourcePos {
                line: line_no,
                column,
            },
        };
        Ok((Some(amount), Some(assertion)))
    } else {
        Ok((Some(parse_primary_and_cost(expr, amt)?), None))
    }
}

/// Parse `AMOUNT [@ PRICE | @@ PRICE]` into an amount with an optional cost.
fn parse_primary_and_cost(expr: &str, amt: AmountCtx) -> Result<Amount, ParseError> {
    if let Some((primary, price)) = expr.split_once("@@") {
        let mut amount = parse_amount(primary.trim(), amt)?;
        let cost_amount = parse_amount(price.trim(), amt)?;
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
        parse_amount(expr.trim(), amt)
    }
}

/// Parse a single commodity+quantity token, applying the commodity's canonical
/// style (with as-written precision). Undeclared commodities honor a journal
/// `decimal-mark` default before falling back to literal inference.
fn parse_amount(token: &str, amt: AmountCtx) -> Result<Amount, ParseError> {
    let (symbol, number, side, spaced) = split_commodity_number(token)?;
    let commodity = Commodity(symbol);

    let canonical = amt.styles.get(&commodity);
    let decimal_mark = canonical
        .map(|style| style.decimal_mark)
        .or(amt.default_mark)
        .unwrap_or('.');
    let quantity = Dec::parse(&number, decimal_mark)?;
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
            let (mark, digit_groups, _) = analyze_number(&number, amt.default_mark);
            AmountStyle {
                side,
                spaced,
                decimal_mark: mark.unwrap_or('.'),
                digit_groups,
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

/// Fill in the single elided posting (if any) so the transaction balances per
/// commodity, then finalize all postings.
fn balance_postings(
    raw: Vec<RawPosting>,
    line_no: u32,
    styles: &Styles,
) -> Result<Vec<Posting>, ParseError> {
    let elided = raw.iter().filter(|p| p.amount.is_none()).count();
    if elided > 1 {
        return Err(ParseError::MultipleElidedPostings(line_no));
    }

    // Accumulate each explicit posting's cost value, per commodity, preserving
    // first-seen order and the maximum contributing precision.
    let mut sums: Vec<(Commodity, Dec, u32)> = Vec::new();
    for posting in &raw {
        if let Some(amount) = &posting.amount {
            let (commodity, quantity, precision) = cost_contribution(amount)?;
            match sums.iter_mut().find(|(c, _, _)| *c == commodity) {
                Some(entry) => {
                    entry.1 = entry.1.add(quantity)?;
                    entry.2 = entry.2.max(precision);
                }
                None => sums.push((commodity, quantity, precision)),
            }
        }
    }

    raw.into_iter()
        .map(|posting| finalize_posting(posting, &sums, styles))
        .collect()
}

/// A posting's contribution to the transaction balance: its cost value if
/// priced, otherwise the amount itself.
fn cost_contribution(amount: &Amount) -> Result<(Commodity, Dec, u32), ParseError> {
    match &amount.cost {
        None => Ok((
            amount.commodity.clone(),
            amount.quantity,
            amount.style.precision,
        )),
        Some(cost) => {
            let price = &cost.amount;
            let quantity = match cost.kind {
                CostKind::Unit => amount.quantity.mul(price.quantity)?,
                CostKind::Total => {
                    let magnitude = price.quantity.abs()?;
                    if amount.quantity.mantissa < 0 {
                        magnitude.neg()?
                    } else {
                        magnitude
                    }
                }
            };
            Ok((price.commodity.clone(), quantity, price.style.precision))
        }
    }
}

fn finalize_posting(
    raw: RawPosting,
    sums: &[(Commodity, Dec, u32)],
    styles: &Styles,
) -> Result<Posting, ParseError> {
    let amounts = match raw.amount {
        Some(amount) => vec![amount],
        None => sums
            .iter()
            .filter(|(_, value, _)| !value.is_zero())
            .map(|(commodity, value, precision)| {
                Ok(Amount {
                    commodity: commodity.clone(),
                    quantity: value.neg()?,
                    style: inferred_style(commodity, *precision, styles),
                    cost: None,
                })
            })
            .collect::<Result<Vec<_>, ParseError>>()?,
    };

    Ok(Posting {
        status: raw.status,
        account: AccountName(raw.account),
        amounts,
        balance_assertion: raw.balance_assertion,
        comment: raw.comment,
        tags: raw.tags,
    })
}

/// The style for an inferred amount: canonical format bits plus the precision
/// carried through from the contributing amounts.
fn inferred_style(commodity: &Commodity, precision: u32, styles: &Styles) -> AmountStyle {
    match styles.get(commodity) {
        Some(style) => AmountStyle {
            side: style.side,
            spaced: style.spaced,
            decimal_mark: style.decimal_mark,
            digit_groups: style.digit_groups.clone(),
            precision,
        },
        None => AmountStyle {
            side: CommoditySide::Left,
            spaced: false,
            decimal_mark: '.',
            digit_groups: None,
            precision,
        },
    }
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
fn is_commodity_char(c: char) -> bool {
    !c.is_ascii_digit()
        && !c.is_whitespace()
        && !matches!(c, '-' | '+' | '.' | ',' | '@' | '=' | ';' | '(' | ')')
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

        let mut end = 0;
        for (idx, ch) in token.char_indices() {
            let is_sign = idx == 0 && matches!(ch, '-' | '+');
            if is_sign || ch.is_ascii_digit() || ch == '.' || ch == ',' {
                end = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        let number = token[..end].to_string();
        let rest = &token[end..];
        let spaced = rest.starts_with(char::is_whitespace);
        let commodity = rest.trim().to_string();
        if number.is_empty() || commodity.is_empty() {
            return Err(ParseError::MalformedAmount(token.to_string()));
        }
        Ok((commodity, number, CommoditySide::Right, spaced))
    }
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
            .skip(1)
            .map(|segment| u8::try_from(segment.len()).unwrap_or(u8::MAX))
            .collect();
        sizes.reverse();
        DigitGroups { mark, sizes }
    });

    (decimal_mark, digit_groups, precision)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eur_styles() -> Styles {
        let (commodity, style) = parse_commodity_directive("commodity 1.000,00 EUR").unwrap();
        let mut styles = HashMap::new();
        styles.insert(commodity, style);
        styles
    }

    /// Build an [`AmountCtx`] over `styles` with no `decimal-mark` default.
    fn ctx(styles: &Styles) -> AmountCtx<'_> {
        AmountCtx {
            styles,
            default_mark: None,
        }
    }

    #[test]
    fn commodity_directive_dollar_style() {
        let (commodity, style) = parse_commodity_directive("commodity $1,000.00").unwrap();
        assert_eq!(commodity, Commodity("$".to_string()));
        assert_eq!(style.side, CommoditySide::Left);
        assert!(!style.spaced);
        assert_eq!(style.decimal_mark, '.');
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
        let (commodity, style) = parse_commodity_directive("commodity 1.000,00 EUR").unwrap();
        assert_eq!(commodity, Commodity("EUR".to_string()));
        assert_eq!(style.side, CommoditySide::Right);
        assert!(style.spaced);
        assert_eq!(style.decimal_mark, ',');
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
        assert_eq!(amount.style.decimal_mark, ',');
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
        assert_eq!(amount.style.decimal_mark, ',');
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
    fn periodic_and_auto_posting_blocks_are_skipped() {
        let text = concat!(
            "~ monthly budget\n",
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
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].description, "real");
        assert_eq!(journal.transactions[0].index, Tindex(1));
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
    }

    #[test]
    fn unsupported_directive_still_errors_with_location() {
        let err = parse_journal("apply account assets:foo\n", "t.journal").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("t.journal:1"), "{msg}");
        assert!(msg.contains("unsupported directive: 'apply'"), "{msg}");
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
