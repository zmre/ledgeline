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
impl From<ReportError> for AppError {
    fn from(error: ReportError) -> Self {
        let message = error.to_string();
        match error {
            ReportError::InvalidBucketKey(_) => Self::BadRequest(message),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ledgeline_core::decimal::DecError;

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
}
