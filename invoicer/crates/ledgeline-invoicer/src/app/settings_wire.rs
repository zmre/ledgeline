//! The Settings page's wire shapes (`/api/settings`, settings plan §5).
//! `invoicer/web/src/lib/settings.ts` mirrors them, and
//! `tests/snapshots/settings-wire.json` is their golden: change all three
//! together.
//!
//! Envelope fields are camelCase. Setting names stay as the file spells
//! them (`invoices-dir`, `words.quantity`): they are map keys and list
//! entries the page looks labels up by, never text it shows.

// Until the routes that use them land (the wire is published first).
#![allow(dead_code)]

use invoicer_core::settings_edit::{Effect, Refusal, SettingProblem};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Whether the books' `ledgeline.toml` is there and readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FileState {
    /// No file: Save asks before creating one (`409 no-file`).
    Missing,
    /// A TOML file the app can read (its `[invoicer]` may still be invalid:
    /// see [`SettingsPage::problems`]).
    Present,
    /// A symbolic link, over 64 KiB, or not TOML: never written.
    Unusable,
}

/// `GET /api/settings`. Read from the file itself, so it answers even when
/// `[invoicer]` is invalid and the books couldn't open.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsPage {
    pub file: FileState,
    /// Why the page is read-only, in plain words: the file is unusable, or
    /// its `[invoicer]` has problems (listed in `problems`). `None` when
    /// the page can save.
    pub problem: Option<String>,
    /// The problems in `[invoicer]` as it is on disk, by setting.
    pub problems: Vec<SettingProblem>,
    /// `true` when nothing may be saved (`problem` says why).
    pub read_only: bool,
    /// What Save must send back: a hash of `[invoicer]` only, so a write to
    /// `[ui]`, `[books]` or `[sync]` never conflicts with an open form.
    /// `None` when the file is unusable.
    pub revision: Option<String>,
    /// Every setting's effective value (the file's, else the default),
    /// keyed as in the file; `words` nested (`values.words.quantity`).
    pub values: Map<String, Value>,
    /// Every setting's default, in the same shape.
    pub defaults: Map<String, Value>,
    /// The settings the file sets (`words.quantity` for a word list): the
    /// rows with "Use the default".
    pub set: Vec<String>,
    /// What the books would use for an account left unset ("Found for
    /// you: …"); `None` when nothing is found or the books aren't open.
    pub inferred: Inferred,
    /// The next invoice number and what it follows; `None` without books.
    pub next_number: Option<NextNumber>,
    /// Ledgeline's `[books]`, shown read-only.
    pub books: BooksFacts,
    /// Why the books couldn't open (a problem in this file, say); `None`
    /// when they are open.
    pub books_error: Option<String>,
    /// What the pickers offer.
    pub choices: Choices,
    /// Settings that change new bookings: Save asks to confirm them.
    pub affects_bookings: Vec<&'static str>,
    /// Choices shown disabled, as "Coming later".
    pub coming: Vec<ComingLater>,
}

/// The accounts the books would use when the setting is unset.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Inferred {
    #[serde(rename = "receivable-account")]
    pub receivable_account: Option<String>,
    #[serde(rename = "revenue-account")]
    pub revenue_account: Option<String>,
    #[serde(rename = "deposit-account")]
    pub deposit_account: Option<String>,
}

/// The next number: `number`, following `after`, which is in `source`
/// (`document`, `books` or `start`).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct NextNumber {
    pub number: String,
    pub after: Option<String>,
    pub source: &'static str,
}

/// `[books]`, as Ledgeline reads it.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BooksFacts {
    /// `cash` or `accrual`; `None` when the books aren't open.
    pub basis: Option<&'static str>,
    /// `[books] closed-through`, when set.
    pub closed_through: Option<String>,
}

/// What the pickers offer.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Choices {
    /// The books' templates, then the starters not copied in yet.
    pub templates: Vec<TemplateChoice>,
    /// Organization notes, for "Your company".
    pub companies: Vec<CompanyChoice>,
    /// The journal's accounts.
    pub accounts: Vec<String>,
}

/// A template the default may name.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TemplateChoice {
    /// What the setting holds: `/invoices/tpl/classic.html`.
    pub value: String,
    /// Its name.
    pub name: String,
    /// The document types it is for (`invoice`, `estimate`, …).
    pub kinds: Vec<String>,
    /// A starter not copied in yet (copied in when an invoice first uses
    /// it).
    pub starter: bool,
}

/// A note "Your company" may name.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompanyChoice {
    /// The note's title, as shown.
    pub title: String,
    /// What the setting holds: `[[Title]]`.
    pub link: String,
}

/// A choice not built yet, shown disabled.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ComingLater {
    /// A stable name for it.
    pub id: &'static str,
    /// The setting it will be a choice of, when it is one.
    pub setting: Option<&'static str>,
    /// The page section: `payments` or `editing`.
    pub section: &'static str,
    /// Its label, in plain words.
    pub label: &'static str,
}

/// `POST /api/settings/check` and `PUT /api/settings`: a change to
/// `[invoicer]`. Only the settings that changed are sent.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SettingsChange {
    /// The [`SettingsPage::revision`] the form was built from.
    pub revision: Option<String>,
    /// New values, by setting (`words.quantity` for a word list).
    #[serde(default)]
    pub set: Map<String, Value>,
    /// Settings to remove ("Use the default").
    #[serde(default)]
    pub unset: Vec<String>,
    /// Create `ledgeline.toml` if there is none (after `409 no-file`).
    #[serde(default)]
    pub create_file: bool,
    /// The [`Effect::key`]s the user confirmed (after `409 confirm`).
    #[serde(default)]
    pub confirmed: Vec<String>,
}

/// `POST /api/settings/check`: what Save would do. Writes nothing.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SettingsCheck {
    /// What is wrong with the values, by setting (Save answers `422
    /// invalid` while there are any).
    pub problems: Vec<SettingProblem>,
    /// What each booking-related change does (Save answers `409 confirm`
    /// until each is confirmed).
    pub effects: Vec<Effect>,
    /// A change refused whatever is confirmed (`422 refused`).
    pub refused: Option<Refusal>,
}

/// `PUT /api/settings`'s `200`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsSaved {
    /// The new revision.
    pub revision: String,
    /// Every setting's effective value now, as [`SettingsPage::values`].
    pub values: Map<String, Value>,
    /// The settings the file sets now.
    pub set: Vec<String>,
    /// `true` when `ledgeline.toml` was created.
    pub created: bool,
}

/// The settings that change what new bookings do.
pub(crate) const AFFECTS_BOOKINGS: [&str; 6] = [
    "receivable-account",
    "revenue-account",
    "deposit-account",
    "discount-account",
    "cash-deposits",
    "payment-allocation",
];

/// The choices shown as "Coming later" (settings plan D3).
pub(crate) const COMING: [ComingLater; 4] = [
    ComingLater {
        id: "oldest-row-first",
        setting: Some("payment-allocation"),
        section: "payments",
        label: "Oldest row first",
    },
    ComingLater {
        id: "keep-deferral",
        setting: Some("cash-deposits"),
        section: "payments",
        label: "Cash books: keep a deposit as a deferral",
    },
    ComingLater {
        id: "first-open-day",
        setting: None,
        section: "editing",
        label: "Sending into a closed period: book it on the first open day",
    },
    ComingLater {
        id: "reverse-and-rebook",
        setting: None,
        section: "editing",
        label: "Editing a sent invoice: reverse it and book it again",
    },
];

#[cfg(test)]
mod tests {
    //! The golden: one of each shape, as the server writes it.
    //!
    //! ```sh
    //! UPDATE_SNAPSHOTS=1 cargo test -p ledgeline-invoicer settings_wire   # re-record
    //! ```

    use super::*;
    use invoicer_core::books::WaitingDeposit;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn page() -> SettingsPage {
        SettingsPage {
            file: FileState::Present,
            problem: None,
            problems: Vec::new(),
            read_only: false,
            revision: Some("1a2b3c4d5e6f7081-42".to_string()),
            values: object(json!({
                "invoices-dir": "invoices",
                "default-terms": 30,
                "default-template": null,
                "words": {"quantity": ["Qty", "Hours"]},
            })),
            defaults: object(json!({
                "invoices-dir": "invoices",
                "default-terms": null,
                "default-template": null,
                "words": {"quantity": ["Qty", "Quantity", "Hours", "Units"]},
            })),
            set: vec!["default-terms".to_string(), "words.quantity".to_string()],
            inferred: Inferred {
                receivable_account: Some("assets:receivable".to_string()),
                revenue_account: Some("revenue:consulting".to_string()),
                deposit_account: Some("liabilities:customer-deposits".to_string()),
            },
            next_number: Some(NextNumber {
                number: "208".to_string(),
                after: Some("207".to_string()),
                source: "document",
            }),
            books: BooksFacts {
                basis: Some("cash"),
                closed_through: Some("2026-06-30".to_string()),
            },
            books_error: None,
            choices: Choices {
                templates: vec![TemplateChoice {
                    value: "/invoices/tpl/classic.html".to_string(),
                    name: "Classic".to_string(),
                    kinds: vec!["invoice".to_string()],
                    starter: true,
                }],
                companies: vec![CompanyChoice {
                    title: "My Company".to_string(),
                    link: "[[My Company]]".to_string(),
                }],
                accounts: vec!["assets:checking".to_string()],
            },
            affects_bookings: AFFECTS_BOOKINGS.to_vec(),
            coming: COMING.to_vec(),
        }
    }

    fn unreadable_page() -> SettingsPage {
        SettingsPage {
            file: FileState::Present,
            problem: Some(
                "The invoicer settings in ledgeline.toml have a problem, so they can't be \
                 saved here. Fix it in the file."
                    .to_string(),
            ),
            problems: vec![SettingProblem {
                key: Some("filename".to_string()),
                message: "The new invoice file name must include the invoice number, or two \
                          invoices could share a file."
                    .to_string(),
            }],
            read_only: true,
            revision: Some("0-0".to_string()),
            books_error: Some("The books couldn't open: …".to_string()),
            next_number: None,
            books: BooksFacts::default(),
            inferred: Inferred::default(),
            choices: Choices::default(),
            ..page()
        }
    }

    fn check() -> SettingsCheck {
        SettingsCheck {
            problems: vec![SettingProblem {
                key: Some("dso-days".to_string()),
                message: "The days-sales-outstanding window must be a whole number from 1 to \
                          3660."
                    .to_string(),
            }],
            effects: vec![Effect {
                key: "payment-allocation".to_string(),
                message: "Payments you apply from now on are split pro rata. Payments already \
                          in your books keep the split they were booked with."
                    .to_string(),
            }],
            refused: Some(Refusal {
                key: "cash-deposits".to_string(),
                message: "1 deposit is waiting for its final invoice. Change how deposit \
                          payments book once its final invoice is paid."
                    .to_string(),
                deposits: vec![WaitingDeposit {
                    number: "205".to_string(),
                    customer: "Contoso".to_string(),
                    date: "2026-10-01".to_string(),
                    waiting: "$1,500.00".to_string(),
                    booked_to: vec!["revenue:consulting".to_string()],
                }],
            }),
        }
    }

    fn saved() -> SettingsSaved {
        SettingsSaved {
            revision: "1a2b3c4d5e6f7082-43".to_string(),
            values: page().values,
            set: page().set,
            created: false,
        }
    }

    #[test]
    fn the_wire_is_its_golden() {
        let change: SettingsChange = serde_json::from_value(json!({
            "revision": "1a2b3c4d5e6f7081-42",
            "set": {"default-terms": 15},
            "unset": ["words.quantity"],
            "createFile": false,
            "confirmed": ["payment-allocation"],
        }))
        .expect("reads");
        assert_eq!(change.unset, ["words.quantity"]);
        assert_eq!(change.confirmed, ["payment-allocation"]);
        assert!(
            serde_json::from_value::<SettingsChange>(json!({"revision": null, "sets": {}}))
                .is_err(),
            "unknown fields are refused"
        );
        let golden = json!({
            "page": page(),
            "pageReadOnly": unreadable_page(),
            "check": check(),
            "saved": saved(),
            "errors": {
                "changed": {"error": "The settings file changed since this page read it.", "code": "changed", "revision": "1a2b3c4d5e6f7090-44"},
                "noFile": {"error": "These books have no settings file yet.", "code": "no-file"},
                "unusable": {"error": "ledgeline.toml is a symbolic link, so the app won't write to it.", "code": "unusable"},
                "confirm": {"error": "Some changes affect new bookings.", "code": "confirm", "effects": check().effects},
                "invalid": {"error": "Some settings have problems.", "code": "invalid", "problems": check().problems},
                "refused": {"error": "1 deposit is waiting for its final invoice.", "code": "refused", "refused": check().refused},
                "badRequest": {"error": "There is no setting called “invoice-dir”.", "code": "bad-request"},
            },
        });
        super::super::dashboard_tests::snapshot("settings-wire.json", &golden);
    }
}
