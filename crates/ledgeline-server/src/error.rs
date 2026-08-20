//! One error type for the native `/api/*` surface (CLEANUP.md DRY-4).
//!
//! Both native API modules used to declare their own
//! `type ApiError = (StatusCode, String)` and build the tuple ad hoc at ~15
//! sites, with the status→condition mapping living in two private functions
//! (`reports_api::report_error`, `edit_api::edit_error`) that nothing connected.
//! Every fallible engine call therefore carried a `.map_err(…)` closure whose
//! only job was to re-state that mapping.
//!
//! [`AppError`] replaces both. It names the failure CLASS rather than an HTTP
//! constant, implements [`IntoResponse`], and — because it has
//! `From<ReportError>` and `From<EditError>` — lets every one of those call
//! sites collapse to a bare `?`.
//!
//! # The body stays `text/plain`, deliberately
//!
//! The obvious next step would be a JSON error body carrying a machine-readable
//! code. That would be a BREAKING change to a contract the SPA already depends
//! on: `web/src/lib/api/native.ts` reads the write path's error body with
//! `response.text()` and hands the string, verbatim and unparsed, to
//! `ValidationError` / `NotFoundError` / `ConflictError` /
//! `NativeApiUnavailableError` — whose `.message` the edit popup and the setup
//! modal display to the user. Switching to JSON would show people a raw
//! `{"code":…}` blob.
//!
//! So [`into_response`](AppError::into_response) delegates to the exact
//! `(StatusCode, String)` rendering the tuple alias produced, and the status +
//! media type + body bytes of every error are pinned by
//! `tests/error_surface.rs`. The enum discriminant is the machine-readable code
//! the finding asked for; it is simply not on the wire yet. Putting it there is
//! a one-line change here plus a matching change in `native.ts` — which is a
//! coordinated Rust+SPA change, not a refactor.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ledgeline_core::EditError;
use ledgeline_core::reports::ReportError;
use ledgeline_core::rules::RulesError;
use thiserror::Error;

/// A failed `/api/*` request: the class of failure plus the sentence the client
/// (and, on the write path, the user) sees.
///
/// The variants are named for the CONDITION, not the status code — the
/// status is derived in exactly one place ([`AppError::status`]), so the
/// mapping can no longer drift between modules the way `report_error` and
/// `edit_error` could.
///
/// `Display` is the response body verbatim, so a message is written once, at
/// the site that knows what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum AppError {
    /// The request asked something we cannot answer: a malformed date, an
    /// unknown interval, an out-of-range count, an amount the journal must
    /// never be asked to hold. `400`.
    #[error("{0}")]
    BadRequest(String),
    /// The addressed transaction (or posting) does not exist. `404`.
    #[error("{0}")]
    NotFound(String),
    /// The journal file changed on disk under us; the client should re-fetch
    /// and retry. `409` — the SPA's `ConflictError`.
    #[error("{0}")]
    Conflict(String),
    /// This server has no editor bound, so the write path is unavailable.
    /// `501` — the SPA's `NativeApiUnavailableError`.
    #[error("{0}")]
    EditingDisabled(String),
    /// The report scheduler is shutting down. `503`.
    #[error("{0}")]
    Unavailable(String),
    /// An invariant broke, or an operation that cannot fail for realistic data
    /// did. `500`.
    #[error("{0}")]
    Internal(String),
}

/// The `501` returned when this state has no editor bound (it was built from a
/// parsed journal with no backing file, or the editor was unbound after a
/// failure it could not recover from).
///
/// It lives here rather than in [`edit_api`](crate::edit_api) because BOTH write
/// surfaces answer with it — the transaction endpoints and the rules-file `PUT`
/// — and the SPA matches on the sentence, not the status: `native.ts` turns a
/// `501` into a `NativeApiUnavailableError` whose `.message` the setup modal
/// shows verbatim. Two copies of this string is two copies of a user-facing
/// sentence, and `tests/error_surface.rs` pins it.
pub(crate) fn editing_disabled() -> AppError {
    AppError::EditingDisabled(
        "editing is not enabled: this server was started without a journal file bound to an editor"
            .to_string(),
    )
}

impl AppError {
    /// The HTTP status this failure is reported with — the single source of
    /// truth for the mapping.
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::EditingDisabled(_) => StatusCode::NOT_IMPLEMENTED,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    /// Renders exactly as the `(StatusCode, String)` tuple this type replaced:
    /// the status, `Content-Type: text/plain; charset=utf-8`, and the message
    /// as the body. See the module docs for why it is not JSON.
    fn into_response(self) -> Response {
        (self.status(), self.to_string()).into_response()
    }
}

/// A bad bucket key is a client error; a decimal overflow is ours. Both are
/// unreachable for realistic journals, but neither is unwrapped.
///
/// # Why a bad `issection:` or `holdings:` is a `400` and not a `500`
///
/// They are the variants whose cause is the JOURNAL rather than the request or
/// an invariant, so neither status is a perfect fit. `400` is chosen because the
/// two things that actually matter about the response are that it FAILS rather
/// than serving a plausible statement with a box reading zero, and that the
/// sentence reaches a human — `EditError::Unbalanced` is a `400` on the same
/// reasoning, and `500` is reserved here for "our bug, nothing you can do".
/// The message names the account, the value and the seven codes, so it is
/// actionable exactly as an unknown `interval` or `value` param is.
///
/// The alternative — a finding in the Problems drawer — is the better home for
/// it and is deliberately not built here: `wire::WireDiagnostic` anchors every
/// finding to a `txnIndex`, and an `account` DIRECTIVE has no transaction to
/// anchor to, so it would need a wider wire struct plus a matching entry in the
/// SPA's `normalize.ts` allow-list (which silently drops unknown rules). That is
/// a coordinated Rust+SPA change, not this one.
impl From<ReportError> for AppError {
    fn from(error: ReportError) -> Self {
        let message = error.to_string();
        match error {
            ReportError::InvalidBucketKey(_)
            | ReportError::UnknownIsSection { .. }
            | ReportError::UnknownHoldingsClass { .. }
            | ReportError::UnknownValuationRole { .. }
            | ReportError::UnknownBsTerm { .. } => Self::BadRequest(message),
            ReportError::Decimal(_) => Self::Internal(message),
        }
    }
}

/// The `EditError` → HTTP table, moved here verbatim from `edit_api`.
///
/// A `409` is the one the client must act on: it means the file changed under
/// us and the edit did NOT land.
impl From<EditError> for AppError {
    fn from(error: EditError) -> Self {
        let message = error.to_string();
        match error {
            EditError::ExternalChange => Self::Conflict(message),
            EditError::Unbalanced
            | EditError::Unsupported(_)
            | EditError::ParseInvalidAfterEdit(_)
            | EditError::RoundTripMismatch => Self::BadRequest(message),
            EditError::TransactionNotFound(_) | EditError::PostingNotFound { .. } => {
                Self::NotFound(message)
            }
            EditError::Io(_)
            | EditError::Parse(_)
            | EditError::Decimal(_)
            | EditError::Internal(_) => Self::Internal(message),
        }
    }
}

/// The `RulesError` → HTTP table, mirroring [`From<EditError>`] above.
///
/// Every variant but one is the client's fault: a stale item id, a duplicated
/// or omitted one, a construct the edit policy will not rewrite, a value this
/// engine will not write, an arrangement in which two constructs would re-parse
/// as one. The exception is `RoundTripMismatch`, which stays a `500`.
///
/// # Why `RoundTripMismatch` is still a `500`
///
/// It was ROUTINELY reachable, and that was the bug rather than the mapping.
/// A conditional table's extent ends at a blank line, so a table written at EOF
/// carried no terminator; appending a rule after one produced text in which the
/// new block re-parsed as further table rows, `verify` refused (correctly), and
/// an ordinary "add a rule" answered `500` with a sentence naming nothing the
/// user could do. The engine now supplies that blank line the moment the table
/// stops being last, exactly as it already supplied a missing final newline.
///
/// What remains behind `RoundTripMismatch` is genuinely ours: the rendered text
/// did not match what the plan renders, or the re-parse did not tile.
/// [`rules_api`](crate::rules_api) verifies [`RulesDoc::apply`]'s own output —
/// the only supported way to use the pair — so no request body can reach either
/// without a bug in this codebase. `500` is the honest answer to that.
///
/// The caller-caused half moved to `WouldMergeConstructs`, a `400`: some extents
/// are ended by the KIND of the next line (a bare `if` with no assignments reads
/// any column-1 line beneath it as another matcher), where there is no
/// terminator to supply and the arrangement really is the caller's to change.
/// Its sentence names the offending position and what to do instead.
///
/// There is deliberately no `409` row: `RulesError` never observes the
/// filesystem (it is `Clone`/`PartialEq` precisely because it carries no
/// [`std::io::Error`]), so optimistic-concurrency conflicts are raised by
/// [`rules_api`](crate::rules_api), which is the layer that read the bytes.
///
/// [`RulesDoc::apply`]: ledgeline_core::rules::RulesDoc::apply
impl From<RulesError> for AppError {
    fn from(error: RulesError) -> Self {
        let message = error.to_string();
        match error {
            RulesError::UnknownItem(_)
            | RulesError::DuplicateItem(_)
            | RulesError::MissingItems(_)
            | RulesError::NotEditable { .. }
            | RulesError::Invalid(_)
            | RulesError::BomMustLeadDocument
            | RulesError::WouldMergeConstructs(_) => Self::BadRequest(message),
            RulesError::RoundTripMismatch => Self::Internal(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::decimal::DecError;
    use ledgeline_core::rules::ItemId;

    /// The status table in full. It used to live in two private functions in
    /// two modules; this is the test neither of them had.
    #[test]
    fn each_variant_maps_to_its_status() {
        let cases = [
            (AppError::BadRequest(String::new()), StatusCode::BAD_REQUEST),
            (AppError::NotFound(String::new()), StatusCode::NOT_FOUND),
            (AppError::Conflict(String::new()), StatusCode::CONFLICT),
            (
                AppError::EditingDisabled(String::new()),
                StatusCode::NOT_IMPLEMENTED,
            ),
            (
                AppError::Unavailable(String::new()),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                AppError::Internal(String::new()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.status(), expected, "{error:?}");
        }
    }

    /// A converted error keeps the source's own sentence — the response body is
    /// the engine's message, not a re-worded one.
    #[test]
    fn conversions_preserve_the_status_and_the_message() {
        let bucket = ReportError::InvalidBucketKey("2026-00".to_string());
        let expected = bucket.to_string();
        let converted = AppError::from(bucket);
        assert_eq!(converted.status(), StatusCode::BAD_REQUEST);
        assert_eq!(converted.to_string(), expected);

        assert_eq!(
            AppError::from(ReportError::Decimal(DecError::Overflow)).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        // A journal-content error the user can act on, with the engine's own
        // naming sentence carried through verbatim.
        let bad_tag = ReportError::UnknownIsSection {
            account: "cogs".to_string(),
            value: "cost-of-goods-sold".to_string(),
        };
        let expected = bad_tag.to_string();
        let converted = AppError::from(bad_tag);
        assert_eq!(converted.status(), StatusCode::BAD_REQUEST);
        assert_eq!(converted.to_string(), expected);
        assert!(expected.contains("cost-of-goods-sold"), "{expected}");

        let missing = EditError::TransactionNotFound(7);
        let expected = missing.to_string();
        let converted = AppError::from(missing);
        assert_eq!(converted.status(), StatusCode::NOT_FOUND);
        assert_eq!(converted.to_string(), expected);

        assert_eq!(
            AppError::from(EditError::ExternalChange).status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::from(EditError::Unbalanced).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::from(EditError::Internal("x".to_string())).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// One row per [`RulesError`] variant, so adding a variant to the engine
    /// without deciding its status fails here rather than defaulting to a `500`
    /// in front of a user who typed a bad account name.
    #[test]
    fn every_rules_error_variant_maps_to_its_status() {
        let cases = [
            (RulesError::UnknownItem(3), StatusCode::BAD_REQUEST),
            (RulesError::DuplicateItem(3), StatusCode::BAD_REQUEST),
            (
                RulesError::MissingItems("2, 5".to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (
                RulesError::NotEditable {
                    id: Some(ItemId(4)),
                    why: "item 4 is an `include`".to_string(),
                },
                StatusCode::BAD_REQUEST,
            ),
            (
                RulesError::Invalid("a matcher pattern may not be empty".to_string()),
                StatusCode::BAD_REQUEST,
            ),
            (RulesError::BomMustLeadDocument, StatusCode::BAD_REQUEST),
            // The caller's arrangement, so the caller's error — see the
            // conversion's docs for what stayed a `500` and why.
            (RulesError::WouldMergeConstructs(2), StatusCode::BAD_REQUEST),
            (
                RulesError::RoundTripMismatch,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (error, expected) in cases {
            let message = error.to_string();
            let converted = AppError::from(error);
            assert_eq!(converted.status(), expected, "{message}");
            assert_eq!(
                converted.to_string(),
                message,
                "a converted error must keep the engine's own sentence"
            );
        }
    }
}
