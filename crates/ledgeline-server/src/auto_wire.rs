//! An `= QUERY` auto-posting rule's FORM on the wire, both ways, and the
//! checks every inbound rule passes before the engine sees it — shared by the
//! two places a rule is edited: a projection scenario
//! ([`crate::projections_api`]) and the main journal ([`crate::auto_api`]).

use ledgeline_core::auto_form::{AutoRuleForm, FormAmount, FormPosting};
use ledgeline_core::model::AutoPostingRule;
use ledgeline_core::parse::{JournalStyles, parse_journal_under};
use serde::{Deserialize, Serialize};

use crate::edit_api::{WireDecIn, dec_from_wire};
use crate::error::AppError;
use crate::reports_api::WireDec;

/// The longest one rule's text may be, in bytes. A rule is a query and a few
/// posting lines; this is fifty times that. Also the cap on a form's extra
/// query and its comment.
pub(crate) const MAX_AUTO_RULE_BYTES: usize = 16 * 1024;
/// The most postings one rule FORM may carry. A rule text is bounded by its
/// bytes; a form's postings are counted before any is decoded.
const MAX_AUTO_POSTINGS: usize = 50;

/// [`AutoRuleForm`] on the wire. Percentages and amounts are as the user
/// enters them: POSITIVE for "the account grows" (the sign inference is the
/// engine's, `auto_form`'s module docs).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAutoRuleForm {
    match_account: String,
    extra_query: String,
    postings: Vec<WireFormPosting>,
    counter: Option<String>,
    comment: String,
    /// [`AutoRuleForm::forecast_only`].
    forecast_only: bool,
}

/// One posting of a form: `kind` is `percent` (with `percent` set, `3.5` for
/// 3.5%) or `fixed` (with `amount` set); the other field is `null`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireFormPosting {
    account: String,
    kind: &'static str,
    percent: Option<WireDec>,
    amount: Option<WireFormAmount>,
}

/// A fixed posting's amount: the quantity as entered, and its places.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFormAmount {
    commodity: String,
    quantity: WireDec,
    precision: u32,
}

impl From<&AutoRuleForm> for WireAutoRuleForm {
    fn from(form: &AutoRuleForm) -> Self {
        Self {
            match_account: form.match_account.clone(),
            extra_query: form.extra_query.clone(),
            postings: form
                .postings
                .iter()
                .map(|posting| match &posting.amount {
                    FormAmount::Percent(percent) => WireFormPosting {
                        account: posting.account.clone(),
                        kind: "percent",
                        percent: Some(WireDec::from(*percent)),
                        amount: None,
                    },
                    FormAmount::Fixed {
                        commodity,
                        quantity,
                    } => WireFormPosting {
                        account: posting.account.clone(),
                        kind: "fixed",
                        percent: None,
                        amount: Some(WireFormAmount {
                            commodity: commodity.clone(),
                            quantity: WireDec::from(*quantity),
                            precision: quantity.places,
                        }),
                    },
                })
                .collect(),
            counter: form.counter.clone(),
            comment: form.comment.clone(),
            forecast_only: form.forecast_only,
        }
    }
}

/// [`WireAutoRuleForm`], inbound.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AutoRuleFormIn {
    match_account: String,
    #[serde(default)]
    extra_query: String,
    postings: Vec<FormPostingIn>,
    #[serde(default)]
    counter: Option<String>,
    #[serde(default)]
    comment: String,
    /// Absent is `false`. A scenario's rules ignore it: they are always
    /// forecast-only ([`form_from_wire`]'s caller decides).
    #[serde(default)]
    forecast_only: bool,
}

/// [`WireFormPosting`], inbound.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FormPostingIn {
    account: String,
    kind: String,
    #[serde(default)]
    percent: Option<WireDecIn>,
    #[serde(default)]
    amount: Option<FormAmountIn>,
}

/// A form's fixed amount. Unlike [`AmountIn`] the commodity may be empty: a
/// bare number in a rule is a commodity-less amount, as hledger writes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FormAmountIn {
    commodity: String,
    quantity: WireDecIn,
    /// Echoed back; the quantity's own places are what is written.
    #[serde(default)]
    #[allow(dead_code)]
    precision: Option<u32>,
}

/// One rule's text, read back as exactly one `=` block.
///
/// The shape is checked BEFORE the text reaches the journal parser: an `=`
/// line, then only indented lines. That is what keeps a rule body from
/// smuggling in a directive — `include`, `alias`, a transaction — that the
/// parser would otherwise act on. A rendered form goes through here too.
///
/// Amounts are read as the journal the rule goes into reads them (see
/// [`parse_journal_under`]): a scenario's hand-written `$1,500` under the
/// main journal's `commodity $1,000.00` is 1500, as the file's GET reads it,
/// and a form rendered under `styles` reads back as the figure that was
/// rendered. Always pass the same `styles` the render used.
pub(crate) fn rule_from_text(
    raw: &str,
    styles: &JournalStyles<'_>,
) -> Result<AutoPostingRule, AppError> {
    let refuse = |why: &str| AppError::BadRequest(format!("an auto rule {why}"));
    if raw.len() > MAX_AUTO_RULE_BYTES {
        return Err(refuse(&format!(
            "may be at most {MAX_AUTO_RULE_BYTES} bytes"
        )));
    }
    // A blank line ends a block, so none may appear inside one.
    let body = raw.trim_matches('\n');
    let mut lines = body.lines();
    let header = lines.next().unwrap_or("");
    if !header.starts_with('=')
        || !lines.all(|line| !line.trim().is_empty() && line.starts_with([' ', '\t']))
    {
        return Err(refuse(
            "must be one `= QUERY` line followed by indented posting lines, with no blank line between",
        ));
    }
    let text = format!("{body}\n");
    let journal = parse_journal_under(&text, "auto-rule.journal", styles)
        .map_err(|error| refuse(&format!("could not be read: {error}")))?;
    match journal.auto_rules.as_slice() {
        [only] if journal.transactions.is_empty() && journal.periodic_transactions.is_empty() => {
            Ok(only.clone())
        }
        _ => Err(refuse("must be exactly one `= QUERY` block")),
    }
}

/// A form's fields, bounded and decoded. Accounts are trimmed; whether each
/// can be written is [`render`]'s to say.
pub(crate) fn form_from_wire(form: &AutoRuleFormIn) -> Result<AutoRuleForm, AppError> {
    let refuse = |why: String| AppError::BadRequest(format!("an auto rule {why}"));
    if form.postings.len() > MAX_AUTO_POSTINGS {
        return Err(refuse(format!(
            "may have at most {MAX_AUTO_POSTINGS} postings (received {})",
            form.postings.len()
        )));
    }
    for (what, text) in [
        ("match account", &form.match_account),
        ("extra query", &form.extra_query),
        ("comment", &form.comment),
    ] {
        if text.len() > MAX_AUTO_RULE_BYTES {
            return Err(refuse(format!(
                "{what} may be at most {MAX_AUTO_RULE_BYTES} bytes"
            )));
        }
    }
    if form.match_account.trim().is_empty() {
        return Err(refuse("needs an account to match".to_string()));
    }
    let postings = form
        .postings
        .iter()
        .map(|posting| {
            if posting.account.trim().is_empty() {
                return Err(refuse("posting needs an account".to_string()));
            }
            let amount = match (posting.kind.as_str(), &posting.percent, &posting.amount) {
                ("percent", Some(percent), None) => FormAmount::Percent(dec_from_wire(percent)?),
                ("fixed", None, Some(amount)) => FormAmount::Fixed {
                    commodity: amount.commodity.trim().to_string(),
                    quantity: dec_from_wire(&amount.quantity)?,
                },
                (kind, _, _) => {
                    return Err(refuse(format!(
                        "posting of kind '{kind}' must be `percent` with a percent, or `fixed` \
                         with an amount"
                    )));
                }
            };
            Ok(FormPosting {
                account: posting.account.trim().to_string(),
                amount,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(AutoRuleForm {
        match_account: form.match_account.trim().to_string(),
        extra_query: form.extra_query.clone(),
        postings,
        counter: form
            .counter
            .as_deref()
            .map(str::trim)
            .filter(|counter| !counter.is_empty())
            .map(str::to_string),
        comment: form.comment.trim().to_string(),
        forecast_only: form.forecast_only,
    })
}
