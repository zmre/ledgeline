//! `ledgeline sync …` — bank sync without a window, for dogfooding (plan 43
//! Phase 1). The same engine and the same routes' functions: connect claims
//! and keeps the key in the keychain, link writes `[sync]` and the rules
//! file and runs the first sync, `now` runs Sync now.
//!
//! Every answer is printed as the JSON the matching route returns, so what a
//! terminal shows and what the Accounts tab shows can be compared directly.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::auto::Mode;
use super::{engine, link, now_seconds, read_config, status};
use crate::AppState;

/// `ledgeline sync`.
#[derive(clap::Args, Debug, Clone)]
pub struct CliSync {
    /// The journal whose `ledgeline.toml` holds the links.
    #[arg(short = 'j', long)]
    pub journal: PathBuf,

    #[command(subcommand)]
    pub command: CliSyncCommand,
}

/// What `ledgeline sync` does.
#[derive(clap::Subcommand, Debug, Clone)]
pub enum CliSyncCommand {
    /// Claim a SimpleFIN setup token, keep its access key in the keychain,
    /// and list the accounts it can see (one request).
    Connect {
        /// The setup token from SimpleFIN ("New app connection").
        token: String,
        /// A name for the connection.
        #[arg(long)]
        name: Option<String>,
    },
    /// Replace a connection's access with a new setup token, keeping its id
    /// and links (after the token was revoked or disabled).
    Reconnect {
        /// The connection id.
        connection: String,
        /// The new setup token.
        token: String,
    },
    /// The connection's redacted diagnostic report, for a bug report.
    Diagnostics {
        /// The connection id.
        connection: String,
    },
    /// List a connection's accounts with suggested mappings (one request).
    Accounts {
        /// The connection id (`ledgeline sync status` lists them).
        connection: String,
    },
    /// Link one account and run its first sync.
    Link {
        /// The connection id.
        connection: String,
        /// The provider's account id (`ledgeline sync accounts` lists them).
        remote_id: String,
        /// The journal account its transactions post to.
        #[arg(long)]
        account: String,
        /// A rules file: an existing one's id (journal-relative), or a new
        /// name such as `imports/checking.sync.csv.rules`.
        #[arg(long)]
        rules: String,
        /// The journal file new transactions are written to
        /// (journal-relative, one this journal includes).
        #[arg(long)]
        target: String,
        /// The first date to import, YYYY-MM-DD.
        #[arg(long)]
        start: String,
        /// The account's display label.
        #[arg(long)]
        label: Option<String>,
        /// `-1` for a bank that reports this balance with the wrong sign.
        #[arg(long, default_value_t = 1, allow_hyphen_values = true)]
        balance_sign: i8,
    },
    /// The links, their last outcomes, problems and quota.
    Status,
    /// Sync now: every linked account, or one connection's.
    Now {
        /// Only this connection.
        #[arg(long)]
        connection: Option<String>,
    },
}

/// Run one `ledgeline sync` command against `state` and answer the text to
/// print.
///
/// # Errors
/// One sentence, ready to print.
pub fn run_cli_sync(state: &AppState, args: &CliSync) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start: {error}"))?;
    let answer: Value = runtime.block_on(async {
        match &args.command {
            CliSyncCommand::Connect { token, name } => {
                link::connect(state, token, name.as_deref(), None)
                    .await
                    .map_err(describe)
            }
            CliSyncCommand::Reconnect { connection, token } => {
                link::connect(state, token, None, Some(connection))
                    .await
                    .map_err(describe)
            }
            CliSyncCommand::Diagnostics { connection } => {
                let state = state.clone();
                let connection = connection.clone();
                super::blocking(move || engine::diagnostics(&state, &connection))
                    .await
                    .map(Value::String)
                    .map_err(|error| error.to_string())
            }
            CliSyncCommand::Accounts { connection } => link::remote_accounts(state, connection)
                .await
                .map(|accounts| json!({ "accounts": accounts }))
                .map_err(describe),
            CliSyncCommand::Link {
                connection,
                remote_id,
                account,
                rules,
                target,
                start,
                label,
                balance_sign,
            } => {
                let existing = super::blocking({
                    let state = state.clone();
                    let rules = rules.clone();
                    move || {
                        let snapshot = state.snapshot();
                        Ok(link::Context::of(&state, &snapshot)
                            .discovery
                            .is_some_and(|discovery| discovery.resolve(&rules).is_some()))
                    }
                })
                .await
                .unwrap_or(false);
                let rules = if existing {
                    json!({"mode": "existing", "id": rules})
                } else {
                    json!({"mode": "new", "name": rules})
                };
                let request: link::LinkRequest = serde_json::from_value(json!({
                    "connectionId": connection,
                    "remoteId": remote_id,
                    "label": label.clone().unwrap_or_else(|| remote_id.clone()),
                    "journalAccount": account,
                    "startDate": start,
                    "target": target,
                    "balanceSign": balance_sign,
                    "rules": rules,
                }))
                .map_err(|error| format!("bad arguments: {error}"))?;
                link::link(state, vec![request])
                    .await
                    .map(|run| serde_json::to_value(run).unwrap_or_default())
                    .map_err(describe)
            }
            CliSyncCommand::Status => {
                let state = state.clone();
                super::blocking(move || status(&state, now_seconds()))
                    .await
                    .map(|status| serde_json::to_value(status).unwrap_or_default())
                    .map_err(|error| error.to_string())
            }
            CliSyncCommand::Now { connection } => {
                if let Some(id) = connection {
                    let (config, _) = read_config(state).map_err(|error| error.to_string())?;
                    if config.connection(id).is_none() {
                        return Err(format!(
                            "no connection {id:?}; `ledgeline sync status` lists them"
                        ));
                    }
                }
                let run = engine::run(
                    state,
                    now_seconds(),
                    &engine::Request {
                        mode: Mode::Manual,
                        connection: connection.clone(),
                        accounts: Vec::new(),
                    },
                )
                .await;
                Ok(serde_json::to_value(run).unwrap_or_default())
            }
        }
    })?;
    match answer {
        Value::String(text) => Ok(text),
        other => serde_json::to_string_pretty(&other).map_err(|error| error.to_string()),
    }
}

/// A route failure as one sentence.
fn describe(reply: super::SyncReply) -> String {
    match reply {
        super::SyncReply::App(error) => error.to_string(),
        super::SyncReply::Problem(problem) => {
            let detail = problem
                .detail
                .as_deref()
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default();
            format!("{}{detail}", problem.message)
        }
        super::SyncReply::Forbidden => "this journal is open read-only".to_string(),
    }
}
