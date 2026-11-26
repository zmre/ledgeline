//! Editing `ledgeline.toml [invoicer]` from the Settings page (settings
//! plan §3, option C): Save is the explicit action, and writes only what
//! changed.
//!
//! This module holds what the page and the server agree on: a problem
//! with one setting ([`SettingProblem`]), what a booking-related change
//! does ([`Effect`]), and a change refused outright ([`Refusal`], plan D5).

use crate::books::deposits::WaitingDeposit;
use serde::Serialize;

/// A problem with one setting, or with the file (`key` `None`). `key` is
/// the setting's name in the file (`filename`, `words.quantity`); the page
/// shows its label. `message` is a plain sentence that never names a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingProblem {
    /// The setting, as in the file; `None` for the file as a whole.
    pub key: Option<String>,
    /// Why, in plain words.
    pub message: String,
}

/// What a change to a setting that affects bookings does, for the
/// confirmation Save asks for. The page sends `key` back in `confirmed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Effect {
    /// The setting.
    pub key: String,
    /// What changes, and what stays as it was, in plain words.
    pub message: String,
}

/// A change refused whatever is confirmed (plan D5: deposit payments in
/// cash books while a deposit waits for its final invoice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// The setting.
    pub key: String,
    /// Why, in plain words.
    pub message: String,
    /// The deposits it waits on.
    pub deposits: Vec<WaitingDeposit>,
}
