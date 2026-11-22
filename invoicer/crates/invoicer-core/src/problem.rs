//! What `check` reports (ARCHITECTURE §4.3, §5.6): a problem in one file,
//! with a line and a fix-it where there is one.

use ledgeline_core::vault::VaultPath;
use serde::Serialize;
use serde_json::Value;
use std::fmt;

/// How bad a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// The document can't be booked or sent as it is.
    Error,
    /// Worth fixing; nothing is blocked.
    Warning,
}

/// What kind of problem, stable for scripts (`check --json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemCode {
    /// The frontmatter isn't YAML, or isn't a mapping.
    Frontmatter,
    /// A required field is missing.
    MissingField,
    /// A field has the wrong shape (a list where text belongs, …).
    BadField,
    /// A date field isn't `YYYY-MM-DD`.
    BadDate,
    /// `status` isn't one of the document type's statuses.
    BadStatus,
    /// `invoice_num` can't be a journal tag value (a comma, a newline).
    BadNumber,
    /// `terms` isn't a whole number of days.
    BadTerms,
    /// `due_date` is before `date`.
    DueBeforeDate,
    /// An address written with `>`, which folds its lines.
    FoldedText,
    /// A key from the first sketch (`po_num`, `term`).
    LegacyField,
    /// `paid:` doesn't follow the mirror's grammar.
    BadPaid,
    /// A mirror (`paid`, `sent_date`, `void_date`) the books disagree with
    /// (Phase 2's reconciler).
    MissingMirror,
    /// No bill table in the body.
    NoBillTable,
    /// The bill table can't be read.
    Table,
    /// A written amount disagrees with the table's arithmetic.
    Arithmetic,
    /// Rows below the final total (refused at booking).
    RowsAfterTotal,
    /// `customer:` names no note, or a note that isn't a customer.
    UnknownCustomer,
    /// `template:` names no template.
    UnknownTemplate,
    /// The customer's portal needs a `reference` and there is none.
    ReferenceRequired,
    /// Two documents share a number.
    DuplicateNumber,
    /// A `contracts:` entry that doesn't exist.
    MissingContract,
    /// A `billing_portal:` that can't be read, or holds a secret.
    BadPortal,
    /// A catalog row that can't be read, or a duplicate code.
    Catalog,
    /// A `[[wikilink]]` that names nothing.
    BrokenLink,
    /// A sent invoice the books have no transaction for.
    MissingTransaction,
    /// A transaction in the books whose invoice has no file.
    OrphanTransaction,
    /// A booking whose `document:` tag doesn't name its file.
    DocumentPath,
    /// A draft the books already hold (a send stopped after the journal).
    BookedDraft,
    /// A sent invoice booked differently from what its file now says.
    BooksOutOfDate,
    /// A void in the file or the books, but not both.
    VoidMismatch,
    /// The invoice can't be booked as written (an account that doesn't
    /// resolve, a total that can't be split).
    Booking,
    /// The books can't be read for the books checks.
    Books,
}

/// A change that fixes a problem, for the CLI and the editor to offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FixEdit {
    /// Set a frontmatter field.
    SetField {
        /// The key.
        key: String,
        /// The value.
        value: Value,
    },
    /// Rename a frontmatter field, keeping its value.
    RenameField {
        /// The old key.
        from: String,
        /// The new key.
        to: String,
    },
    /// Rewrite the bill table's computed amounts (`recalc`).
    Recalc,
    /// Run a `ledgeline-invoicer` command (`send 105`, `rebook 105`).
    Command {
        /// The command and its arguments, as typed after the binary.
        command: String,
    },
}

/// How to fix a problem.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fix {
    /// What to do, in words.
    pub message: String,
    /// The edit, when it can be made mechanically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<FixEdit>,
}

/// One problem in one file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    /// The file, vault-relative.
    pub file: Option<VaultPath>,
    /// The 1-based line, when one can be named.
    pub line: Option<usize>,
    /// How bad it is.
    pub severity: Severity,
    /// What kind of problem.
    pub code: ProblemCode,
    /// What is wrong, in words.
    pub message: String,
    /// How to fix it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

impl Problem {
    /// An error with no file or line yet.
    #[must_use]
    pub fn error(code: ProblemCode, message: impl Into<String>) -> Self {
        Self {
            file: None,
            line: None,
            severity: Severity::Error,
            code,
            message: message.into(),
            fix: None,
        }
    }

    /// A warning with no file or line yet.
    #[must_use]
    pub fn warning(code: ProblemCode, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message)
        }
    }

    /// This problem at `line`.
    #[must_use]
    pub fn at_line(self, line: Option<usize>) -> Self {
        Self { line, ..self }
    }

    /// This problem in `file`.
    #[must_use]
    pub fn in_file(self, file: &VaultPath) -> Self {
        Self {
            file: Some(file.clone()),
            ..self
        }
    }

    /// This problem with a fix.
    #[must_use]
    pub fn with_fix(self, message: impl Into<String>, edit: Option<FixEdit>) -> Self {
        Self {
            fix: Some(Fix {
                message: message.into(),
                edit,
            }),
            ..self
        }
    }

    /// Whether it is an error.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match (&self.file, self.line) {
            (Some(file), Some(line)) => write!(f, "{file}:{line}: ")?,
            (Some(file), None) => write!(f, "{file}: ")?,
            (None, Some(line)) => write!(f, "line {line}: ")?,
            (None, None) => {}
        }
        write!(f, "{level}: {}", self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " ({})", fix.message)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_file_line_and_fix() {
        let file = VaultPath::new("invoices/105.md").expect("path");
        let problem = Problem::error(ProblemCode::BadDate, "`date` is not a date")
            .at_line(Some(7))
            .in_file(&file)
            .with_fix("write it as YYYY-MM-DD", None);
        assert_eq!(
            problem.to_string(),
            "invoices/105.md:7: error: `date` is not a date (write it as YYYY-MM-DD)"
        );
        assert!(problem.is_error());
        assert!(!Problem::warning(ProblemCode::BadField, "x").is_error());
    }

    #[test]
    fn serializes_for_scripts() {
        let problem = Problem::warning(ProblemCode::LegacyField, "po_num").with_fix(
            "rename it",
            Some(FixEdit::RenameField {
                from: "po_num".into(),
                to: "reference".into(),
            }),
        );
        let json = serde_json::to_value(&problem).expect("json");
        assert_eq!(json["code"], "legacy-field");
        assert_eq!(json["severity"], "warning");
        assert_eq!(json["fix"]["edit"]["kind"], "rename-field");
    }
}
