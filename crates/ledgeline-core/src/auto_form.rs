//! An `= QUERY` auto-posting rule as a FORM: the shape the rule editor edits,
//! and its exact mapping to and from the text hledger reads (plan 30, Phase 4).
//!
//! # The form
//!
//! A form says "when a posting to MATCH ACCOUNT (or under it) is generated,
//! add these postings": each one a percentage of the matched amount or a
//! fixed amount, plus an optional COUNTER-ACCOUNT that takes the balancing
//! amount. [`render`] writes it as text; [`form_from_rule`] reads a parsed rule
//! back, or says `None` when the rule says something the form cannot — the
//! editor then shows the raw text, locked (the "still parsed, still carried,
//! locked" contract plan 21 set for period expressions).
//!
//! # Form ↔ text
//!
//! ```text
//! = ^revenue:sales(:|$) tag:_generated-transaction EXTRA  ; COMMENT
//!     expenses:sales-commission  *-0.035
//!     assets:checking            *0.035
//! ```
//!
//! - **The query** is the match account as an anchored, escaped regex,
//!   `^ACCOUNT(:|$)` — the account and its subtree, never a sibling such as
//!   `revenue:sales2` (hledger's account terms are infix regexes, so a bare
//!   `revenue:sales` matches that too) — then, when the form is
//!   [`forecast_only`](AutoRuleForm::forecast_only), `tag:_generated-transaction`,
//!   which limits the rule to forecast transactions in stock hledger exactly
//!   as Ledgeline applies a scenario's rules (Decision 6), then any extra terms
//!   as typed. An account with a space is quoted.
//! - **A percentage** is a `*N` multiplier: `3.5` is `*0.035`, signed by
//!   [`sign inference`](#sign-inference).
//! - **A fixed amount** is written as an amount, signed by its account.
//! - **The counter-account** is the OPPOSITE of the other legs, never an
//!   amountless posting: `*-(sum of the multipliers)`, or the negated sum of
//!   the fixed amounts. hledger infers an amountless auto posting per
//!   TRANSACTION, so a rule with one that fires twice in one transaction makes
//!   hledger refuse the file (plan 30 amendment 8); the explicit multiplier
//!   always loads and means the same. A counter therefore needs legs of one
//!   kind: all percentages, or fixed amounts in one commodity.
//!
//! Reading back, an amountless last posting is also a counter, so a
//! hand-written rule in the manual's style is editable — and is written in the
//! explicit form once edited. A plain account word (`revenue:sales`) is read as
//! the match account too (an edit writes it anchored), and the forecast-only
//! tag, present or not, is [`AutoRuleForm::forecast_only`]. Anything else — virtual or
//! status-marked postings, posting comments, a balance assertion, `*N CUR`
//! re-denomination, a cost,
//! `%account`, a multi-line comment, a query whose account term is not one of
//! those two shapes — is not representable.
//!
//! # Sign inference
//!
//! The user types a POSITIVE percentage, meaning "the target account grows by
//! this share of the matched amount"; a negative one means it shrinks.
//!
//! Every account has a normal side. Asset and expense accounts (and anything
//! untyped) grow with a positive amount; liability, equity and revenue
//! accounts (gains and conversion included) grow with a negative one —
//! hledger writes income negative. So the multiplier is
//!
//! ```text
//! factor = percent / 100 × side(matched account) × side(target account)
//! ```
//!
//! where `side` is `+1` for the debit-normal types and `-1` for the
//! credit-normal ones. The matched posting already carries its own side's sign,
//! and multiplying by both cancels it and applies the target's:
//!
//! - 3% commission on a sale: revenue (−) into an expense (+) → `*-0.03`, so a
//!   `$-40000` sale books `$1200` of commission.
//! - 10% payroll tax on wages: expense (+) into an expense (+) → `*0.1`.
//! - 10% of wages accrued as a tax liability: expense (+) into a liability (−)
//!   → `*-0.1`, so `$1000` of wages owes `$-100` more.
//!
//! A fixed amount takes its target account's side alone: `$50` into an expense
//! is `$50`, into a liability `$-50`.
//!
//! This deliberately differs from the projections table's own entry rule
//! ([`crate::reports::account_types::signed_entry`], which negates revenue
//! only): a table line is the movement of ONE account, while a rule relates two,
//! and "grows" is the only reading under which typing `3` means the same thing
//! whichever way round the two accounts are.
//!
//! Type resolution is [`AccountTypes::resolve`]: declared types first, then
//! the name inference every report uses.

use std::collections::BTreeSet;

use crate::decimal::Dec;
use crate::model::{
    Amount, AmountStyle, AutoAmount, AutoPostingRule, Commodity, CommoditySide, PostingType, Status,
};
use crate::parse::JournalStyles;
use crate::query::Query;
use crate::reports::account_types::{AccountType, AccountTypes};

/// The query term that limits a rule to forecast transactions in stock hledger
/// (plan 30 semantics 5). A [`forecast_only`](AutoRuleForm::forecast_only)
/// form's query carries it.
pub const FORECAST_ONLY_TERM: &str = "tag:_generated-transaction";

/// Its visible twin, which hledger also honours; read back as the same thing.
const FORECAST_ONLY_VISIBLE: &str = "tag:generated-transaction";

/// The suffix that makes an anchored account regex match the account and its
/// subtree, and nothing else.
const SUBTREE: &str = "(:|$)";

/// Characters a POSIX ERE (hledger's) and the `regex` crate (Ledgeline's) both
/// read as syntax. An account name holding one is escaped; verified with
/// hledger 1.52 that the escaped form matches the literal name.
const REGEX_META: &[char] = &[
    '\\', '.', '^', '$', '|', '(', ')', '[', ']', '{', '}', '*', '+', '?',
];

/// The query prefixes hledger knows. A plain account word whose first segment
/// is one of these is a different term, not an account.
const QUERY_PREFIXES: &[&str] = &[
    "acct", "desc", "payee", "note", "code", "cur", "tag", "amt", "status", "real", "depth",
    "date", "date2", "type", "expr", "any", "all", "not",
];

/// An editable auto-posting rule. See the [module docs](self) for how each
/// field maps to text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRuleForm {
    /// The account whose postings (and whose subaccounts' postings) the rule
    /// fires on.
    pub match_account: String,
    /// Further query terms, ANDed or ORed as hledger combines them; empty for
    /// none. Normalized to single spaces between terms.
    pub extra_query: String,
    /// The postings added below each match, in order. At least one.
    pub postings: Vec<FormPosting>,
    /// The account that takes the opposite of [`Self::postings`], or `None`
    /// when the postings balance by themselves or are virtual-free and
    /// unbalanced on purpose (hledger refuses an unbalanced regular rule; the
    /// server validates).
    pub counter: Option<String>,
    /// The `=` line's comment, without the `;`. Empty for none.
    pub comment: String,
    /// Whether the query carries [`FORECAST_ONLY_TERM`], so stock hledger
    /// applies the rule to forecast transactions only. A scenario's rules
    /// always do (Ledgeline applies them to nothing else); a main-journal
    /// rule usually does not, since Ledgeline forecasts nothing there and a
    /// forecast-only rule in it never fires.
    pub forecast_only: bool,
}

/// One posting of a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormPosting {
    /// The account it books to.
    pub account: String,
    /// What it books.
    pub amount: FormAmount,
}

/// What one posting books, as the user enters it — before sign inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormAmount {
    /// A percentage of the matched amount: `3.5` is 3.5%.
    Percent(Dec),
    /// A fixed amount per match. An empty commodity is a bare number, which
    /// hledger 1.52 leaves commodity-less (plan 30 amendment 9).
    Fixed {
        /// The commodity symbol, `$` or `USD`, or empty.
        commodity: String,
        /// The quantity, as entered.
        quantity: Dec,
    },
}

/// Why a form cannot be written. The message is a sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct FormError(pub String);

fn refuse<T>(why: impl Into<String>) -> Result<T, FormError> {
    Err(FormError(why.into()))
}

// ===========================================================================
// Sign inference
// ===========================================================================

/// Whether `account` grows with a NEGATIVE amount: liability, equity and
/// revenue (with their subtypes). Asset, expense and untyped accounts grow
/// with a positive one. See [sign inference](self#sign-inference).
#[must_use]
pub fn grows_negative(account: &str, types: &AccountTypes) -> bool {
    matches!(
        types.resolve(account),
        Some(
            AccountType::Liability
                | AccountType::Equity
                | AccountType::Revenue
                | AccountType::Gain
                | AccountType::Conversion
        )
    )
}

/// `value` negated when `negate`.
fn signed(value: Dec, negate: bool) -> Result<Dec, FormError> {
    if negate {
        value
            .neg()
            .map_err(|error| FormError(format!("an amount is out of range: {error}")))
    } else {
        Ok(value)
    }
}

/// A percentage as a multiplier: `3.5` → `0.035`, `10` → `0.1` — exactly, and
/// without trailing zeros (a multiplier has no display precision to keep).
fn factor_of(percent: Dec) -> Dec {
    Dec::new(percent.mantissa, percent.places.saturating_add(2)).normalized()
}

/// A multiplier as a percentage: `0.035` → `3.5`, exactly.
fn percent_of(factor: Dec) -> Option<Dec> {
    if factor.places >= 2 {
        Some(Dec::new(factor.mantissa, factor.places - 2))
    } else {
        let scale = 10_i128.pow(2 - factor.places);
        factor.mantissa.checked_mul(scale).map(|m| Dec::new(m, 0))
    }
}

// ===========================================================================
// Rendering
// ===========================================================================

/// `form` as one `=` block, each line ending in `\n`, its amounts written in
/// `styles` — those of the journal the rule is written into, so `10%` is
/// `*0,1` under `decimal-mark ,`.
///
/// The result is checked to read back, through the journal parser under those
/// same styles and [`form_from_rule`], as a form that renders to the very same
/// text, so what is written is what was asked for.
///
/// # Errors
/// [`FormError`] for a field that cannot be written, with the reason.
pub fn render(
    form: &AutoRuleForm,
    types: &AccountTypes,
    styles: &JournalStyles<'_>,
) -> Result<String, FormError> {
    let text = render_unchecked(form, types, styles)?;
    let journal = crate::parse::parse_journal_under(&text, "auto-rule.journal", styles)
        .map_err(|error| FormError(format!("the rule would not read back: {error}")))?;
    let [rule] = journal.auto_rules.as_slice() else {
        return refuse("the rule would not read back as one rule");
    };
    match form_from_rule(rule, types).map(|back| render_unchecked(&back, types, styles)) {
        Some(Ok(again)) if again == text => Ok(text),
        _ => refuse(format!(
            "the rule would not read back as written: {}",
            text.lines().next().unwrap_or_default()
        )),
    }
}

/// [`render`] without the read-back check.
fn render_unchecked(
    form: &AutoRuleForm,
    types: &AccountTypes,
    styles: &JournalStyles<'_>,
) -> Result<String, FormError> {
    if form.postings.is_empty() {
        return refuse("an automatic posting rule needs at least one posting");
    }
    let query = query_text(&form.match_account, &form.extra_query, form.forecast_only)?;
    let comment = form.comment.trim();
    if comment.chars().any(|c| c.is_control()) {
        return refuse("a rule's comment may not contain a line break or a control character");
    }

    let match_negative = grows_negative(form.match_account.trim(), types);
    let mut rows: Vec<(String, String)> = Vec::with_capacity(form.postings.len() + 1);
    let mut factors: Vec<Dec> = Vec::new();
    let mut fixed: Vec<(String, Dec)> = Vec::new();
    for posting in &form.postings {
        let account = posting.account.trim();
        check_account(account)?;
        let target_negative = grows_negative(account, types);
        let text = match &posting.amount {
            FormAmount::Percent(percent) => {
                let factor = signed(factor_of(*percent), match_negative != target_negative)?;
                factors.push(factor);
                format!("*{}", plain(factor, styles))
            }
            FormAmount::Fixed {
                commodity,
                quantity,
            } => {
                let quantity = signed(*quantity, target_negative)?;
                let commodity = commodity.trim();
                fixed.push((commodity.to_string(), quantity));
                fixed_text(commodity, quantity, styles)?
            }
        };
        rows.push((account.to_string(), text));
    }
    if let Some(counter) = form
        .counter
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        check_account(counter)?;
        let text = if fixed.is_empty() {
            format!("*{}", plain(negated_sum(&factors)?.normalized(), styles))
        } else if factors.is_empty() && fixed.iter().all(|(c, _)| *c == fixed[0].0) {
            let quantities: Vec<Dec> = fixed.iter().map(|(_, q)| *q).collect();
            fixed_text(&fixed[0].0, negated_sum(&quantities)?, styles)?
        } else {
            return refuse(
                "a counter-account balances percentages, or fixed amounts in one commodity, \
                 but not a mix: give the rule only one kind, or post the balance yourself",
            );
        };
        rows.push((counter.to_string(), text));
    }

    let width = rows
        .iter()
        .map(|(account, _)| account.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = format!("= {query}");
    if !comment.is_empty() {
        out.push_str("  ; ");
        out.push_str(comment);
    }
    out.push('\n');
    for (account, amount) in rows {
        out.push_str(&format!("    {account:<width$}  {amount}\n"));
    }
    Ok(out)
}

/// `-(a + b + …)`.
fn negated_sum(values: &[Dec]) -> Result<Dec, FormError> {
    let overflow = |error| FormError(format!("an amount is out of range: {error}"));
    values
        .iter()
        .try_fold(Dec::zero(), |sum, value| sum.add(*value))
        .and_then(Dec::neg)
        .map_err(overflow)
}

/// A number exactly as written, no commodity, with the mark `styles` reads a
/// bare number with: `-0.035`, or `-0,035`.
fn plain(value: Dec, styles: &JournalStyles<'_>) -> String {
    let commodity = Commodity(String::new());
    let mark = styles.decimal_mark_for(&commodity);
    crate::edit::render_amount(&Amount {
        commodity,
        quantity: value,
        style: style(CommoditySide::Left, false, mark, value.places),
        cost: None,
    })
}

fn style(side: CommoditySide, spaced: bool, decimal_mark: char, precision: u32) -> AmountStyle {
    AmountStyle {
        side,
        spaced,
        decimal_mark: Some(decimal_mark),
        digit_groups: None,
        precision,
    }
}

/// A fixed amount in its declared style when `styles` has one (its side,
/// spacing and decimal mark; digit groups are never written); otherwise a
/// symbol such as `$` on the left with no space, a word such as `USD` on the
/// right after one, nothing for an empty commodity, with the mark `styles`
/// reads it with.
fn fixed_text(
    commodity: &str,
    quantity: Dec,
    styles: &JournalStyles<'_>,
) -> Result<String, FormError> {
    if commodity.chars().any(|c| {
        c.is_whitespace() || c.is_control() || c.is_ascii_digit() || "\"';#-+.,*@=()[]".contains(c)
    }) {
        return refuse(format!(
            "the commodity `{commodity}` cannot be written in a rule without quoting; use a plain \
             symbol or word"
        ));
    }
    let commodity = Commodity(commodity.to_string());
    let mark = styles.decimal_mark_for(&commodity);
    let style = match styles.declared_style(&commodity) {
        Some(declared) => AmountStyle {
            decimal_mark: Some(mark),
            ..declared.clone()
        },
        None => {
            let wordy = commodity.0.chars().any(char::is_alphabetic);
            let side = if wordy {
                CommoditySide::Right
            } else {
                CommoditySide::Left
            };
            style(side, wordy, mark, quantity.places)
        }
    };
    Ok(crate::edit::render_amount(&Amount {
        commodity,
        quantity,
        style,
        cost: None,
    }))
}

/// Refuse an account name a posting line cannot carry — the projections
/// writer's own rule, so the two cannot disagree — or one holding `%`, which an
/// `=` rule reads as `%account`.
fn check_account(account: &str) -> Result<(), FormError> {
    crate::projections::serialize::check_account(account)
        .map_err(|error| FormError(error.to_string()))?;
    if account.contains('%') {
        return refuse("an account name in a rule may not contain `%`, which hledger expands");
    }
    Ok(())
}

/// The query a form writes: `^ACCOUNT(:|$) tag:_generated-transaction EXTRA`,
/// or without the tag when `forecast_only` is false.
///
/// # Errors
/// [`FormError`] for an account or term that cannot be written, or a query
/// this engine cannot evaluate (it would be locked the moment it was saved).
pub fn query_text(
    match_account: &str,
    extra_query: &str,
    forecast_only: bool,
) -> Result<String, FormError> {
    let account = match_account.trim();
    check_account(account)?;
    let mut pattern = String::from("^");
    for c in account.chars() {
        if REGEX_META.contains(&c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push_str(SUBTREE);
    let term = if pattern.contains(char::is_whitespace) {
        if !pattern.contains('\'') {
            format!("'{pattern}'")
        } else if !pattern.contains('"') {
            format!("\"{pattern}\"")
        } else {
            return refuse("an account name with a space may not hold both kinds of quote");
        }
    } else {
        pattern
    };
    let mut query = if forecast_only {
        format!("{term} {FORECAST_ONLY_TERM}")
    } else {
        term
    };
    let extra = normalized_extra(extra_query)?;
    if !extra.is_empty() {
        query.push(' ');
        query.push_str(&extra);
    }
    if let Some(why) = Query::parse(&query).unsupported() {
        return refuse(format!("the query `{query}` cannot be used: {why}"));
    }
    Ok(query)
}

/// The extra terms, one space apart, without the forecast-only term (which
/// [`AutoRuleForm::forecast_only`] states).
fn normalized_extra(extra: &str) -> Result<String, FormError> {
    if extra.contains([';', '#']) || extra.chars().any(char::is_control) {
        return refuse(
            "extra query terms may not contain `;`, `#` or a line break, which would end the \
             rule's query",
        );
    }
    let Some(terms) = raw_terms(extra) else {
        return refuse("a quote in the extra query terms is never closed");
    };
    Ok(terms
        .into_iter()
        .filter(|term| !is_forecast_only(term))
        .collect::<Vec<_>>()
        .join(" "))
}

fn is_forecast_only(term: &str) -> bool {
    term == FORECAST_ONLY_TERM || term == FORECAST_ONLY_VISIBLE
}

/// A query split into its terms the way [`crate::query`] splits it — on
/// whitespace outside quotes — but with the quotes KEPT, so each term can be
/// written back as it was. `None` for an unclosed quote.
fn raw_terms(query: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in query.chars() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                current.push(c);
            }
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        out.push(current);
    }
    Some(out)
}

// ===========================================================================
// Reading back
// ===========================================================================

/// The form `rule` states, or `None` when it states something a form cannot
/// (see the [module docs](self)). Uses the same `types` [`render`] signs by.
#[must_use]
pub fn form_from_rule(rule: &AutoPostingRule, types: &AccountTypes) -> Option<AutoRuleForm> {
    if rule.query.unsupported().is_some() {
        return None;
    }
    let (match_account, extra_query, forecast_only) = split_query(rule.query.raw())?;
    // One comment line on the `=` line, or none.
    let comment = match rule.comment.strip_suffix('\n') {
        None if rule.comment.is_empty() => String::new(),
        Some(line) if !line.contains('\n') => line.to_string(),
        _ => return None,
    };

    for posting in &rule.postings {
        let plain = posting.ptype == PostingType::Regular
            && posting.status == Status::Unmarked
            && posting.comment.is_empty()
            && posting.tags.is_empty()
            && posting.balance_assertion.is_none()
            && check_account(&posting.account).is_ok();
        if !plain {
            return None;
        }
    }
    let (legs, counter) = match rule.postings.split_last()? {
        (last, legs) if !legs.is_empty() && balances(&last.amount, legs) => {
            (legs, Some(last.account.clone()))
        }
        _ => (rule.postings.as_slice(), None),
    };

    let match_negative = grows_negative(&match_account, types);
    let postings = legs
        .iter()
        .map(|posting| {
            let target_negative = grows_negative(&posting.account, types);
            let amount = match &posting.amount {
                AutoAmount::Scale {
                    factor,
                    commodity: None,
                } => FormAmount::Percent(
                    signed(percent_of(*factor)?, match_negative != target_negative).ok()?,
                ),
                AutoAmount::Fixed(amount) if amount.cost.is_none() => FormAmount::Fixed {
                    commodity: amount.commodity.0.clone(),
                    quantity: signed(amount.quantity, target_negative).ok()?,
                },
                _ => return None,
            };
            Some(FormPosting {
                account: posting.account.clone(),
                amount,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    Some(AutoRuleForm {
        match_account,
        extra_query,
        postings,
        counter,
        comment,
        forecast_only,
    })
}

/// Whether `last` is the counter of `legs`: amountless, or exactly their
/// opposite.
fn balances(last: &AutoAmount, legs: &[crate::model::AutoPosting]) -> bool {
    match last {
        AutoAmount::Inferred => true,
        AutoAmount::Scale {
            factor,
            commodity: None,
        } => {
            let factors: Option<Vec<Dec>> = legs
                .iter()
                .map(|leg| match &leg.amount {
                    AutoAmount::Scale {
                        factor,
                        commodity: None,
                    } => Some(*factor),
                    _ => None,
                })
                .collect();
            factors.is_some_and(|factors| negated_sum(&factors).is_ok_and(|sum| sum == *factor))
        }
        AutoAmount::Fixed(amount) if amount.cost.is_none() => {
            let quantities: Option<Vec<Dec>> = legs
                .iter()
                .map(|leg| match &leg.amount {
                    AutoAmount::Fixed(leg)
                        if leg.cost.is_none() && leg.commodity == amount.commodity =>
                    {
                        Some(leg.quantity)
                    }
                    _ => None,
                })
                .collect();
            quantities.is_some_and(|quantities| {
                negated_sum(&quantities).is_ok_and(|sum| sum == amount.quantity)
            })
        }
        _ => false,
    }
}

/// `(match account, extra terms, forecast-only)` of a query a form can state.
fn split_query(query: &str) -> Option<(String, String, bool)> {
    let terms = raw_terms(query)?;
    let at = terms.iter().position(|term| account_term(term).is_some())?;
    let account = account_term(&terms[at])?;
    let extra = terms
        .iter()
        .enumerate()
        .filter(|(index, term)| *index != at && !is_forecast_only(term))
        .map(|(_, term)| term.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let forecast_only = terms.iter().any(|term| is_forecast_only(term));
    Some((account, extra, forecast_only))
}

/// The account a query term names, when it is `^ESCAPED(:|$)` or a plain
/// account word, optionally quoted and optionally `acct:`-prefixed.
fn account_term(term: &str) -> Option<String> {
    let unquoted = match term.chars().next() {
        Some(q @ ('\'' | '"')) => term.strip_prefix(q)?.strip_suffix(q)?,
        _ => term,
    };
    if unquoted.contains(['\'', '"']) {
        return None;
    }
    let body = unquoted.strip_prefix("acct:").unwrap_or(unquoted);
    let account = if let Some(anchored) = body.strip_prefix('^') {
        unescape(anchored.strip_suffix(SUBTREE)?)?
    } else {
        if body.contains(REGEX_META) {
            return None;
        }
        let first = body.split(':').next().unwrap_or_default();
        if body.split(':').count() > 1 && QUERY_PREFIXES.contains(&first) {
            return None;
        }
        body.to_string()
    };
    check_account(&account).ok()?;
    Some(account)
}

/// A regex that is an escaped literal, as the literal; `None` when it holds
/// syntax.
fn unescape(pattern: &str) -> Option<String> {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let escaped = chars.next()?;
            if !REGEX_META.contains(&escaped) {
                return None;
            }
            out.push(escaped);
        } else if REGEX_META.contains(&c) {
            return None;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

// ===========================================================================
// Unknown accounts (Decision 11)
// ===========================================================================

/// One warning per (rule, account) where `rule` posts to an account `known`
/// does not hold — neither declared nor used, by itself or by a subaccount.
/// hledger accepts such an account (it simply creates it), so this warns
/// rather than refuses: it is almost always a typo such as
/// `expesnses:people-costs:payroll-tax`.
///
/// `known` is every declared account and every account posted to. Its
/// ANCESTORS count as known too, as in hledger's account list. An account
/// holding `%account` is skipped: it names the matched account.
#[must_use]
pub fn unknown_account_warnings(
    rules: &[AutoPostingRule],
    known: &BTreeSet<String>,
) -> Vec<String> {
    // Known itself, or the parent of something known: a child of `account`
    // is the first name at or after `account:` (one starting `account-`, which
    // sorts between the two, cannot be).
    let is_known = |account: &str| {
        let child = format!("{account}:");
        known.contains(account)
            || known
                .range::<str, _>((
                    std::ops::Bound::Included(child.as_str()),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .is_some_and(|next| next.starts_with(&child))
    };
    let mut out = Vec::new();
    for rule in rules {
        let mut seen = BTreeSet::new();
        for posting in &rule.postings {
            let account = posting.account.as_str();
            if account.contains('%') || is_known(account) || !seen.insert(account) {
                continue;
            }
            out.push(format!(
                "auto rule `= {}` posts to `{account}`, which no account directive declares and \
                 no transaction uses — check the spelling",
                rule.query.raw()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_journal;
    use std::collections::BTreeMap;

    fn types() -> AccountTypes {
        AccountTypes::from_declared(BTreeMap::from([
            ("revenue".to_string(), AccountType::Revenue),
            ("expenses".to_string(), AccountType::Expense),
            ("liabilities".to_string(), AccountType::Liability),
            ("assets".to_string(), AccountType::Asset),
            ("assets:checking".to_string(), AccountType::Cash),
            ("equity".to_string(), AccountType::Equity),
        ]))
    }

    fn pct(mantissa: i128, places: u32) -> FormAmount {
        FormAmount::Percent(Dec::new(mantissa, places))
    }

    fn form(
        match_account: &str,
        postings: &[(&str, FormAmount)],
        counter: Option<&str>,
    ) -> AutoRuleForm {
        AutoRuleForm {
            match_account: match_account.to_string(),
            extra_query: String::new(),
            postings: postings
                .iter()
                .map(|(account, amount)| FormPosting {
                    account: (*account).to_string(),
                    amount: amount.clone(),
                })
                .collect(),
            counter: counter.map(str::to_string),
            comment: String::new(),
            forecast_only: true,
        }
    }

    fn rule_of(text: &str) -> AutoPostingRule {
        let journal = parse_journal(text, "t.journal").expect("parses");
        journal.auto_rules.into_iter().next().expect("one rule")
    }

    /// 3% commission on a sale: the revenue posting is negative and the
    /// expense must come out positive, so the multiplier is negative.
    #[test]
    fn a_commission_on_revenue_into_an_expense_is_a_negative_multiplier() {
        let text = render(
            &form(
                "revenue:sales",
                &[("expenses:sales-commission", pct(3, 0))],
                Some("assets:checking"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert_eq!(
            text,
            "= ^revenue:sales(:|$) tag:_generated-transaction\n\
             \x20   expenses:sales-commission  *-0.03\n\
             \x20   assets:checking            *0.03\n"
        );
    }

    /// Payroll tax on wages: expense into expense keeps the sign.
    #[test]
    fn payroll_tax_on_wages_is_a_positive_multiplier() {
        let text = render(
            &form(
                "expenses:people-costs:wages-salaries",
                &[("expenses:people-costs:payroll-tax", pct(10, 0))],
                Some("assets:checking"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert!(
            text.contains("expenses:people-costs:payroll-tax  *0.1\n"),
            "{text}"
        );
        assert!(
            text.contains("assets:checking                    *-0.1\n"),
            "{text}"
        );
    }

    /// An expense accrued into a liability: the liability grows negative.
    #[test]
    fn an_expense_accrued_into_a_liability_is_a_negative_multiplier() {
        let text = render(
            &form(
                "expenses:wages",
                &[("liabilities:payroll-tax", pct(10, 0))],
                Some("expenses:payroll-tax"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert!(text.contains("liabilities:payroll-tax  *-0.1\n"), "{text}");
        assert!(text.contains("expenses:payroll-tax     *0.1\n"), "{text}");
    }

    /// What those multipliers DO, through the engine that applies them: the
    /// commission is $1200 on a $40,000 sale and checking pays it.
    #[test]
    fn the_rendered_commission_books_what_the_user_meant() {
        let rule = render(
            &form(
                "revenue:sales",
                &[("expenses:sales-commission", pct(3, 0))],
                Some("assets:checking"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        let journal = parse_journal(
            &format!(
                "{rule}\n2027-04-01 sale  ; _generated-transaction: ~ x\n    revenue:sales  $-40000\n    assets:checking\n"
            ),
            "c.journal",
        )
        .expect("parses");
        let txn =
            crate::auto::apply(&journal.auto_rules, &journal.transactions[0]).expect("applies");
        let amounts: Vec<(String, Dec)> = txn
            .postings
            .iter()
            .map(|p| (p.account.0.clone(), p.amounts[0].quantity))
            .collect();
        assert_eq!(
            amounts[1],
            ("expenses:sales-commission".into(), Dec::new(1200, 0))
        );
        assert_eq!(amounts[2], ("assets:checking".into(), Dec::new(-1200, 0)));
    }

    /// Fixed amounts take the target's side, and the counter is their
    /// negated sum in one commodity.
    #[test]
    fn fixed_amounts_and_their_counter() {
        let text = render(
            &form(
                "revenue:sales",
                &[
                    (
                        "expenses:fees",
                        FormAmount::Fixed {
                            commodity: "$".into(),
                            quantity: Dec::new(50, 0),
                        },
                    ),
                    (
                        "liabilities:levy",
                        FormAmount::Fixed {
                            commodity: "$".into(),
                            quantity: Dec::new(1050, 2),
                        },
                    ),
                ],
                Some("assets:checking"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert!(text.contains("expenses:fees     $50\n"), "{text}");
        assert!(text.contains("liabilities:levy  $-10.50\n"), "{text}");
        assert!(text.contains("assets:checking   $-39.50\n"), "{text}");
        let words = render(
            &form(
                "revenue",
                &[(
                    "expenses:x",
                    FormAmount::Fixed {
                        commodity: "USD".into(),
                        quantity: Dec::new(2, 0),
                    },
                )],
                None,
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert!(words.contains("expenses:x  2 USD\n"), "{words}");
    }

    #[test]
    fn a_counter_over_mixed_kinds_is_refused() {
        let error = render(
            &form(
                "revenue:sales",
                &[
                    ("expenses:a", pct(1, 0)),
                    (
                        "expenses:b",
                        FormAmount::Fixed {
                            commodity: "$".into(),
                            quantity: Dec::new(5, 0),
                        },
                    ),
                ],
                Some("assets:checking"),
            ),
            &types(),
            &JournalStyles::default(),
        )
        .expect_err("refused");
        assert!(error.0.contains("not a mix"), "{error}");
    }

    /// Escaping, quoting, the extra terms and the comment, all read back.
    #[test]
    fn odd_account_names_extra_terms_and_a_comment_round_trip() {
        let mut f = form(
            "revenue:my sales.co (x)",
            &[("expenses:comm", pct(35, 1))],
            Some("assets:checking"),
        );
        // Brackets are refused in a posting account, and so in a match.
        assert!(render(&f, &types(), &JournalStyles::default()).is_err());
        f.match_account = "revenue:my sales.co+1".into();
        f.extra_query = "  desc:acme   'note:big deal'  ".into();
        f.comment = "commission, sales: yes".into();
        let text = render(&f, &types(), &JournalStyles::default()).expect("renders");
        assert_eq!(
            text.lines().next(),
            Some(
                "= '^revenue:my sales\\.co\\+1(:|$)' tag:_generated-transaction desc:acme 'note:big deal'  ; commission, sales: yes"
            )
        );
        let back = form_from_rule(&rule_of(&text), &types()).expect("representable");
        assert_eq!(back.match_account, "revenue:my sales.co+1");
        assert_eq!(back.extra_query, "desc:acme 'note:big deal'");
        assert_eq!(back.comment, "commission, sales: yes");
        assert_eq!(back.postings, f.postings);
        assert_eq!(back.counter.as_deref(), Some("assets:checking"));
    }

    /// The user's own hand-written rules read as forms: a plain account
    /// word, an amountless counter, no tag at all.
    #[test]
    fn hand_written_rules_in_the_manual_s_style_are_forms() {
        let rule = rule_of(
            "= revenue:sales tag:_generated-transaction\n    expenses:people-costs:sales-commission  *-0.035\n    assets:mercury:checking\n",
        );
        let f = form_from_rule(&rule, &types()).expect("representable");
        assert_eq!(f.match_account, "revenue:sales");
        assert_eq!(f.extra_query, "");
        assert_eq!(
            f.postings,
            vec![FormPosting {
                account: "expenses:people-costs:sales-commission".into(),
                amount: pct(35, 1)
            }]
        );
        assert_eq!(f.counter.as_deref(), Some("assets:mercury:checking"));
        // Once edited it is written in the explicit form hledger always loads.
        let text = render(&f, &types(), &JournalStyles::default()).expect("renders");
        assert!(
            text.starts_with("= ^revenue:sales(:|$) tag:_generated-transaction\n"),
            "{text}"
        );
        assert!(
            text.ends_with("assets:mercury:checking                 *0.035\n"),
            "{text}"
        );

        let untagged = rule_of("= expenses:rent\n    expenses:rent-tax  *0.1\n");
        let f = form_from_rule(&untagged, &types()).expect("representable");
        assert_eq!(f.counter, None);
        assert_eq!(f.postings[0].amount, pct(10, 0));
        assert!(!f.forecast_only);
    }

    /// A main-journal rule applies to real transactions: without the tag the
    /// form says so, and writes no tag back.
    #[test]
    fn a_rule_for_every_transaction_carries_no_forecast_tag() {
        let f = AutoRuleForm {
            forecast_only: false,
            ..form("expenses:rent", &[("expenses:rent-tax", pct(10, 0))], None)
        };
        let text = render(&f, &types(), &JournalStyles::default()).expect("renders");
        assert_eq!(text, "= ^expenses:rent(:|$)\n    expenses:rent-tax  *0.1\n");
        assert_eq!(form_from_rule(&rule_of(&text), &types()), Some(f.clone()));

        let tagged = render(
            &AutoRuleForm {
                forecast_only: true,
                ..f
            },
            &types(),
            &JournalStyles::default(),
        )
        .expect("renders");
        assert_eq!(
            tagged.lines().next(),
            Some("= ^expenses:rent(:|$) tag:_generated-transaction")
        );
        // The visible spelling hledger also honours reads as forecast-only too.
        let visible =
            rule_of("= expenses:rent tag:generated-transaction\n    expenses:rent-tax  *0.1\n");
        assert!(
            form_from_rule(&visible, &types())
                .expect("representable")
                .forecast_only
        );
    }

    #[test]
    fn rules_a_form_cannot_state_are_none() {
        for text in [
            "= expr:'food and not lunch'\n    (x)  1\n",
            "= revenue:sales\n    (liabilities:tax)  *0.05\n",
            "= revenue:sales\n    expenses:x  *0.05 USD\n    assets:checking\n",
            "= revenue:sales\n    %account:fee  *0.05\n",
            "= revenue:sales\n    * expenses:x  *0.05\n",
            "= revenue:sales\n    expenses:x  *0.05  ; note\n",
            "= revenue:sales\n    ; a second comment line\n    expenses:x  *0.05\n",
            "= ^revenue:sales$\n    expenses:x  *0.05\n",
            "= reven.e:sales\n    expenses:x  *0.05\n",
            "= desc:acme\n    expenses:x  *0.05\n",
            "= revenue:sales\n    expenses:x\n    assets:checking\n",
            "= revenue:sales\n    expenses:x  $5 @ €4\n",
        ] {
            assert_eq!(form_from_rule(&rule_of(text), &types()), None, "{text}");
        }
    }

    /// A rule is written in the styles of the journal it goes into. Under a
    /// comma decimal mark — `decimal-mark ,`, `D 1.000,00 EUR`, or (for a
    /// fixed amount) `commodity 1.000,00 EUR` — a `.`-written `*0.1` reads as
    /// a 100% multiplier and `1500.50 EUR` as `150050 EUR`, in this parser and
    /// in hledger 1.52. Rendered under the journal's styles, the rule reads
    /// back as 10% and `1500.50`, which `hledger print --auto` confirms for
    /// each text below: a `1000 EUR` rent books `100 EUR` and `1500.50 EUR`
    /// (2026-10-28).
    #[test]
    fn a_rule_is_written_in_the_decimal_mark_of_the_journal_it_goes_into() {
        let f = AutoRuleForm {
            forecast_only: false,
            ..form(
                "expenses:rent",
                &[
                    ("expenses:fee", pct(10, 0)),
                    (
                        "expenses:fixed",
                        FormAmount::Fixed {
                            commodity: "EUR".into(),
                            quantity: Dec::new(150_050, 2),
                        },
                    ),
                ],
                None,
            )
        };
        for (header, factor, fixed) in [
            ("decimal-mark ,", "*0,1", "1500,50 EUR"),
            ("D 1.000,00 EUR", "*0,1", "1500,50 EUR"),
            ("commodity 1.000,00 EUR", "*0.1", "1500,50 EUR"),
        ] {
            let journal = parse_journal(&format!("{header}\n"), "j.journal").expect("parses");
            let text = render(&f, &types(), &JournalStyles::of(&journal)).expect("renders");
            assert_eq!(
                text,
                format!(
                    "= ^expenses:rent(:|$)\n    expenses:fee    {factor}\n    expenses:fixed  {fixed}\n"
                ),
                "{header}"
            );
            // Read where it is written: after the journal's own directives.
            let rule = rule_of(&format!("{header}\n\n{text}"));
            assert_eq!(
                form_from_rule(&rule, &types()).map(|back| back.postings),
                Some(f.postings.clone()),
                "{header}"
            );
        }
        // With no such directive, `.` it is, as before.
        let text = render(&f, &types(), &JournalStyles::default()).expect("renders");
        assert!(
            text.contains("*0.1\n") && text.contains("1500.50 EUR\n"),
            "{text}"
        );
    }

    /// A form has no place for a balance assertion, so a rule carrying one
    /// opens as text: an Edit → Save through the form would re-render it
    /// without the assertion and silently delete it from the journal.
    #[test]
    fn a_rule_with_a_balance_assertion_opens_as_text_so_a_save_cannot_drop_it() {
        for (plain, asserted) in [
            (
                "= expenses:food\n    assets:x  $1\n    assets:cash  $-1\n",
                "= expenses:food\n    assets:x  $1 = $5\n    assets:cash  $-1\n",
            ),
            (
                "= expenses:food\n    assets:x  *0.1\n    assets:cash\n",
                "= expenses:food\n    assets:x  *0.1 = $5\n    assets:cash\n",
            ),
        ] {
            assert!(
                form_from_rule(&rule_of(plain), &types()).is_some(),
                "{plain}"
            );
            assert_eq!(
                form_from_rule(&rule_of(asserted), &types()),
                None,
                "{asserted}"
            );
        }
    }

    #[test]
    fn a_query_this_engine_cannot_evaluate_is_refused_at_render() {
        let mut f = form("revenue", &[("expenses:x", pct(1, 0))], None);
        f.extra_query = "type:X".into();
        assert!(
            render(&f, &types(), &JournalStyles::default())
                .unwrap_err()
                .0
                .contains("cannot be used")
        );
        f.extra_query = "desc:'open".into();
        assert!(render(&f, &types(), &JournalStyles::default()).is_err());
        f.extra_query = "desc:a ; include x".into();
        assert!(render(&f, &types(), &JournalStyles::default()).is_err());
    }

    #[test]
    fn the_typo_in_the_user_s_rule_is_warned_about() {
        let rule = rule_of(
            "= expenses:people-costs:wages-salaries tag:_generated-transaction\n    expesnses:people-costs:payroll-tax  *0.1\n    assets:mercury:checking\n    (%account:x)  1\n    expenses:people-costs  *0\n",
        );
        let known: BTreeSet<String> = [
            "assets:mercury:checking",
            "expenses:people-costs:payroll-tax",
            "expenses:people-costs-x",
            "expenses:people-costs:wages-salaries:george",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let warnings = unknown_account_warnings(&[rule], &known);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("`expesnses:people-costs:payroll-tax`"),
            "{warnings:?}"
        );
    }

    mod round_trip {
        use super::*;
        use proptest::prelude::*;

        const ACCOUNTS: &[&str] = &[
            "revenue:sales",
            "revenue:my sales",
            "expenses:people-costs:payroll-tax",
            "expenses:a.b+c",
            "liabilities:tax",
            "assets:checking",
            "equity:owner",
            "untyped:thing",
        ];

        fn account() -> impl Strategy<Value = String> {
            proptest::sample::select(ACCOUNTS).prop_map(str::to_string)
        }

        fn dec() -> impl Strategy<Value = Dec> {
            (-1_000_000_i128..1_000_000, 0_u32..5).prop_map(|(m, p)| Dec::new(m, p))
        }

        fn amount() -> impl Strategy<Value = FormAmount> {
            prop_oneof![
                dec().prop_map(FormAmount::Percent),
                (proptest::sample::select(&["$", "USD", "€", ""][..]), dec()).prop_map(
                    |(commodity, quantity)| FormAmount::Fixed {
                        commodity: commodity.to_string(),
                        quantity
                    }
                ),
            ]
        }

        fn arb_form() -> impl Strategy<Value = AutoRuleForm> {
            (
                account(),
                proptest::sample::select(
                    &[
                        "",
                        "desc:acme",
                        "amt:>100",
                        "not:desc:refund 'payee:big co'",
                    ][..],
                ),
                proptest::collection::vec((account(), amount()), 1..4),
                proptest::option::of(account()),
                proptest::sample::select(&["", "commission", "tax: payroll"][..]),
                any::<bool>(),
            )
                .prop_map(
                    |(match_account, extra, postings, counter, comment, forecast_only)| {
                        AutoRuleForm {
                            match_account,
                            extra_query: extra.to_string(),
                            postings: postings
                                .into_iter()
                                .map(|(account, amount)| FormPosting { account, amount })
                                .collect(),
                            counter,
                            comment: comment.to_string(),
                            forecast_only,
                        }
                    },
                )
        }

        proptest! {
            /// `form_from_rule(parse(render(form, &JournalStyles::default()))) == form`, for every form
            /// that renders and is not ambiguous (postings whose last leg
            /// happens to balance the rest read back as having a counter).
            #[test]
            fn render_then_read_is_the_identity(f in arb_form()) {
                let Ok(text) = render(&f, &types(), &JournalStyles::default()) else {
                    // Only a counter over mixed kinds or commodities refuses.
                    prop_assert!(f.counter.is_some());
                    return Ok(());
                };
                let back = form_from_rule(&rule_of(&text), &types());
                if f.counter.is_none() && f.postings.len() > 1 {
                    let rule = rule_of(&text);
                    let (last, legs) = rule.postings.split_last().expect("postings");
                    prop_assume!(!balances(&last.amount, legs));
                }
                prop_assert_eq!(back, Some(f));
            }
        }
    }
}
