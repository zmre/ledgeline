//! `hledger.conf` — reading the `--alias` options it declares and the settings
//! Ledgeline honours (`--auto`, `--depth`), and writing them back surgically.
//!
//! # Why this module exists: two homes for one mapping
//!
//! An `alias` directive in a journal and an `--alias` option in a config file
//! look like the same thing and are not. Verified against hledger 1.52:
//!
//! | | applies when hledger READS the journal | applies to an `import`'s CSV |
//! | --- | --- | --- |
//! | `alias` directive in the journal | yes | **no** |
//! | `--alias` in `hledger.conf` | yes | **yes** |
//!
//! [`crate::aliases`] exists because of the first row's second column: Ledgeline
//! reads the journal's directives and hands them to hledger as `--alias`, which
//! is the only way they can reach a statement. This module exists because of the
//! consequence — a plain command-line `hledger import`, run by the same user over
//! the same CSV with the same rules file, gets no such help and writes different
//! account names. Verified: `PW Roth IRA - 3077:cash` from the terminal where
//! Ledgeline writes `assets:morganstanley:pw-roth-ira:cash`. Same inputs, two
//! journals.
//!
//! A config file closes that, because it applies to *every* hledger command. So
//! this module (a) reads one, so an import Ledgeline runs honours what the
//! user's terminal already honours, and (b) writes one, so the divergence has a
//! fix rather than only a warning.
//!
//! # We read a config file; we never delegate to one
//!
//! Note what this module does **not** do: pass `--conf`. Every invocation
//! `hledger.rs` builds carries `--no-conf`, because a config file can replace the
//! command hledger runs (its docs say so and the binary agrees), and because a
//! config may hold `--depth`, `-b`/`-e` or `--forecast`, any of which would
//! silently change output this crate parses and appends to a journal. Reading the
//! file ourselves and forwarding only the `--alias` values we understand is the
//! narrow version of the same benefit: the mapping travels, the rest does not.
//!
//! # The file format, as the binary actually reads it
//!
//! Each fact below was checked against hledger 1.52 rather than read off the
//! manual, because two of them are undocumented and one is a trap.
//!
//! * **Tokens are split on whitespace and QUOTES ARE NOT HONOURED.** Both
//!   `--alias="…"` and `--alias='…'` fail with a `parse error`; the quote
//!   character ends up inside the value. This is the trap, and [`conf_argument`]
//!   is the whole of this module's answer to it.
//! * `#` starts a comment, anywhere on a line. **`;` does not** — a leading `;`
//!   is read as the command word, which hijacks the invocation exactly as a bare
//!   `balance` would.
//! * **The FIRST token of the general section, if it does not begin with a dash,
//!   replaces the command.** Only the first: a bare word appearing after any
//!   option is passed through as an ordinary argument and hijacks nothing (both
//!   verified). A hijacked config file breaks every hledger command the user runs,
//!   so none of its options reach anything — which is why [`alias_arguments`]
//!   answers with none, and [`hijacks_command`] exists to say why.
//! * `[name]` opens a section. Everything before the first heading is the
//!   *general* section and applies to every command; a `[name]` section applies
//!   only to that command. An import therefore sees general + `[import]`, and
//!   [`Section::applies_to`] is that rule.
//! * The general section's options are applied **before** a command section's,
//!   and `--alias` options compose in order, so the first one to match an account
//!   wins.
//! * hledger searches for `hledger.conf` in the working directory and **every
//!   directory above it**, nearest first, then `$HOME/.hledger.conf`, then
//!   `$XDG_CONFIG_HOME/hledger/hledger.conf` — and uses exactly one file.
//!   [`locate_conf_for`] is that order; see "Deliberate differences" below.
//! * **Section headings are case-sensitive FULL command names.** `[balancesheet]`
//!   applies to `hledger bs`; `[bs]` and `[BalanceSheet]` are silently ignored.
//!   When one heading appears twice, hledger reads only the **last** one.
//!   (All verified.) [`ConfSettings`] reads the same way and warns about both.
//! * `--depth N` and `--depth REGEX=N` combine; the last plain `N` wins, the last
//!   value for one REGEX wins, `-N` means `--depth N`, and a command section's
//!   options apply after the general section's. See [`DepthSpec`].
//!
//! # Deliberate differences from hledger
//!
//! Only one remains: **the search starts at the JOURNAL's directory**, not at
//! the process's working directory. hledger searches from wherever it was
//! launched, and where a user launches hledger for these books is beside these
//! books; a server process's working directory is an accident of how it was
//! started. (A sandboxed vault also has no "above" and no home directory, so
//! its search ends at its root — not a difference in behaviour, just a smaller
//! filesystem.) `$HOME` and `$XDG_CONFIG_HOME` are parameters
//! ([`UserConfDirs`]), never read from the environment here.
//!
//! # Settings
//!
//! [`ConfSettings`] is the part of a config file Ledgeline itself honours:
//! general-section `--auto` (plan 30, Decision 7) and `--depth` in the general,
//! `[balancesheet]`, `[incomestatement]`, `[cashflow]` and `[balance]` sections.
//! [`with_option`] and [`without_option`] write exactly one option into exactly
//! one section and leave every other byte alone.
//!
//! # Writing: the escaping rule
//!
//! Because there is no quoting and no escape for a space, an alias whose pattern
//! contains whitespace cannot be written literally. [`conf_argument`] converts it
//! or refuses; it never writes a mapping that does not match. See that function
//! for the two deliberate widenings and the four refusals.

use crate::vault::{EntryKind, LocalVault, Vault, VaultPath};
use std::path::{Path, PathBuf};

/// The file name hledger looks for in a directory.
pub const CONF_NAME: &str = "hledger.conf";

/// The command section an import's options may come from, besides the general
/// one.
pub const IMPORT_COMMAND: &str = "import";

/// Largest config file this module will read.
///
/// A config file is a handful of option lines. The cap exists so a `hledger.conf`
/// that is secretly a disk image cannot be pulled into memory by a page load, in
/// the same spirit as the rules scan's own budgets.
pub const MAX_CONF_BYTES: u64 = 256 * 1024;

/// How many directories, starting with the journal's own, [`locate_conf`] (and
/// [`locate_in`], which never passes the vault root) will look at.
///
/// Deep enough for any real layout, finite so a pathological symlinked tree
/// cannot make the walk unbounded.
pub const MAX_ANCESTORS: usize = 64;

/// How many of one config file's `--alias` options are forwarded to an import.
///
/// The same bound, and the same reasoning, as [`crate::aliases::MAX_FORWARDED`]:
/// it limits an `argv`, not a user — and, like that one, aliases past it are
/// counted and reported ([`forwarded`]) rather than quietly dropped.
pub const MAX_CONF_ALIASES: usize = 200;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Which part of a config file an option was declared in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// Before the first `[heading]`. Applies to every command.
    General,
    /// Inside `[name]`. Applies to `name` only.
    Command(String),
}

impl Section {
    /// Does an option in this section reach `command`?
    #[must_use]
    pub fn applies_to(&self, command: &str) -> bool {
        match self {
            Self::General => true,
            Self::Command(name) => name.eq_ignore_ascii_case(command),
        }
    }
}

/// One option read out of a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfOption {
    /// Where it was declared.
    pub section: Section,
    /// The flag, with its leading dashes (`--alias`).
    pub name: String,
    /// Its value, when it had one.
    pub value: Option<String>,
}

/// Every option a config file declares, in file order, each tagged with its
/// section.
///
/// Deliberately tolerant: a line this module does not understand contributes
/// nothing and stops nothing. The result is used for exactly one thing — finding
/// `--alias` values — and refusing to read a file because of an option we have no
/// opinion about would turn an unrelated setting into a broken import.
#[must_use]
pub fn parse(text: &str) -> Vec<ConfOption> {
    text.lines()
        .scan(Section::General, |section, line| {
            // `#` comments run to end of line, wherever they start. `;` does NOT
            // start one — see the module docs.
            let content = line.split('#').next().unwrap_or_default().trim();
            if let Some(name) = heading(content) {
                *section = Section::Command(name);
                return Some(Vec::new());
            }
            Some(options(content, section))
        })
        .flatten()
        .collect()
}

/// The section name if `line` is a `[heading]`, trimmed.
fn heading(line: &str) -> Option<String> {
    line.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(|name| name.trim().to_string())
}

/// The options on one already-decommented line.
///
/// Whitespace-separated, exactly as hledger splits it, and both spellings of a
/// valued option are read: `--alias=X=Y` (one token, and the only one this module
/// ever *writes*) and `--alias X=Y` (two tokens, which a hand-written file may
/// well use). A token that is not a flag is a positional — the command word, or
/// an option's value already consumed — and is skipped.
fn options(line: &str, section: &Section) -> Vec<ConfOption> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let token = tokens[at];
        at += 1;
        if !token.starts_with('-') {
            continue;
        }
        let (name, value) = match token.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => {
                // A following token that is not itself a flag is this one's
                // value. A flag with no value (`--debug`) simply has none.
                let next = tokens.get(at).filter(|next| !next.starts_with('-'));
                if next.is_some() {
                    at += 1;
                }
                (token, next.map(|value| (*value).to_string()))
            }
        };
        out.push(ConfOption {
            section: section.clone(),
            name: name.to_string(),
            value,
        });
    }
    out
}

/// The command word this config file forces on every hledger invocation, if it
/// forces one.
///
/// hledger's own manual: "If the first word in a config file's top (general)
/// section does not begin with a dash (eg: print), it is treated as the command
/// argument (overriding any argument on the command line)." Verified, including
/// the narrowness of it — a bare word after any option is an ordinary argument
/// and overrides nothing.
///
/// A config in this state makes **every** hledger command fail or run as
/// something else, so its options never reach anything. Worth naming rather than
/// silently reading around, because a user whose terminal hledger stopped working
/// is owed the reason.
#[must_use]
pub fn hijacks_command(text: &str) -> Option<String> {
    parse_tokens(text)
        .into_iter()
        .find(|(section, _)| *section == Section::General)
        .map(|(_, token)| token)
        .filter(|token| !token.starts_with('-'))
}

/// Every whitespace-separated token, decommented, tagged with its section.
fn parse_tokens(text: &str) -> Vec<(Section, String)> {
    text.lines()
        .scan(Section::General, |section, line| {
            let content = line.split('#').next().unwrap_or_default().trim();
            if let Some(name) = heading(content) {
                *section = Section::Command(name);
                return Some(Vec::new());
            }
            Some(
                content
                    .split_whitespace()
                    .map(|token| (section.clone(), token.to_string()))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

/// The `--alias` values a config file would give `command`, in the order hledger
/// applies them.
///
/// General-section options first, then the command section's, because that is the
/// order hledger assembles them and `--alias` options compose in order — the
/// first one to match an account is the one that rewrites it.
///
/// **None at all when [`hijacks_command`] answers**: a config that replaces the
/// command breaks the invocation outright, so nothing in it applies to anything.
///
/// **Every one of them**, not the first [`MAX_CONF_ALIASES`]: this is also the
/// answer to "is this alias already in the file?", and a list cut at the cap
/// answered "no" for every alias past it, so a write appended them again. The
/// cap belongs where an `argv` is built — see [`forwarded`].
#[must_use]
pub fn alias_arguments(text: &str, command: &str) -> Vec<String> {
    if hijacks_command(text).is_some() {
        return Vec::new();
    }
    let parsed = parse(text);
    let pick = |wanted: &Section| -> Vec<String> {
        parsed
            .iter()
            .filter(|option| &option.section == wanted && option.name == "--alias")
            .filter_map(|option| option.value.clone())
            .filter(|value| !value.is_empty())
            .collect()
    };
    pick(&Section::General)
        .into_iter()
        .chain(pick(&Section::Command(command.to_string())))
        .collect()
}

/// Split [`alias_arguments`]' answer into the ones forwarded to an hledger
/// invocation and how many past [`MAX_CONF_ALIASES`] are not.
///
/// The count is returned rather than dropped for the reason
/// [`crate::aliases::AliasRefusal::Limit`] exists: a terminal `hledger` applies
/// every one of them, so an alias past the cap is a mapping the two disagree
/// about, and the person reading the import screen has to be told.
#[must_use]
pub fn forwarded(mut arguments: Vec<String>) -> (Vec<String>, usize) {
    let unforwarded = arguments.len().saturating_sub(MAX_CONF_ALIASES);
    arguments.truncate(MAX_CONF_ALIASES);
    (arguments, unforwarded)
}

/// The nearest `hledger.conf` at or above `directory` **within the vault**, if
/// there is one — the first part of hledger's lookup.
///
/// The walk starts at the JOURNAL's directory (the module docs' one deliberate
/// difference) and stops at the vault root. [`locate_conf`] continues above the
/// root on a vault with a real directory behind it, and [`locate_conf_for`]
/// adds hledger's `$HOME` and XDG fallbacks after that.
///
/// Only a regular file counts. A `hledger.conf` that is a directory or a FIFO is
/// skipped rather than opened, because opening the second one never returns. A
/// symlink counts only when it resolves to a regular file inside the vault, and
/// then its resolved path is what is returned (and later read).
#[must_use]
pub fn locate_in(vault: &dyn Vault, directory: &VaultPath) -> Option<VaultPath> {
    directory.ancestors().take(MAX_ANCESTORS).find_map(|dir| {
        let candidate = dir.child(CONF_NAME).ok()?;
        let entry = vault.stat(&candidate).ok()??;
        let resolved = match entry.kind {
            EntryKind::File => candidate,
            EntryKind::Symlink => {
                let target = vault.canonicalize(&candidate).ok()?;
                vault.is_file(&target).then_some(target)?
            }
            EntryKind::Dir | EntryKind::Other => return None,
        };
        let size = vault.stat(&resolved).ok()??.size;
        (size <= MAX_CONF_BYTES).then_some(resolved)
    })
}

/// The nearest `hledger.conf` at or above the real directory `directory`: the
/// path form of [`locate_conf`] over a vault rooted at `directory`, so the walk
/// continues above it exactly as it always did.
#[must_use]
pub fn locate(directory: &Path) -> Option<PathBuf> {
    let vault = LocalVault::new(directory);
    locate_conf(&vault, &VaultPath::root()).map(|found| match found {
        ConfFile::InVault(path) => vault.native(&path),
        ConfFile::AboveVault { path, .. } => path,
    })
}

/// Read a config file located by [`locate_in`].
///
/// Lossy rather than strict UTF-8: a config file is read to find `--alias`
/// values, and refusing the whole file — and with it the account mapping an
/// import depends on — over one stray byte in a comment would be the wrong trade.
///
/// Bounded by [`MAX_CONF_BYTES`] here, at the read, and not only in
/// [`locate_in`]: the write path finds its file by name rather than through
/// `locate_in`, and a file that grows between a `stat` and the read would get
/// past a size check made beforehand anyway.
///
/// # Errors
/// Whatever the vault reports, or [`std::io::ErrorKind::InvalidData`] when the
/// file is larger than [`MAX_CONF_BYTES`].
pub fn read_in(vault: &dyn Vault, path: &VaultPath) -> std::io::Result<String> {
    match vault.read_pinned(path, Some(MAX_CONF_BYTES)) {
        Ok((bytes, _)) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the config file is larger than Ledgeline reads",
        )),
        Err(error) => Err(std::io::Error::from(error)),
    }
}

/// Read a config file named by a real path — one [`locate_conf`] found ABOVE a
/// local vault's root. Plain filesystem I/O, exactly as before the vault seam:
/// such a file is outside the vault by definition. Same lossy decode and the
/// same [`MAX_CONF_BYTES`] bound at the read as [`read_in`].
///
/// # Errors
/// Whatever opening or reading the file reports, or
/// [`std::io::ErrorKind::InvalidData`] when it is larger than [`MAX_CONF_BYTES`].
pub fn read(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_CONF_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONF_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the config file is larger than Ledgeline reads",
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Where [`locate_conf`] found the config file in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfFile {
    /// Inside the vault: read it with [`read_in`].
    InVault(VaultPath),
    /// Above the vault root, on a vault with a real directory behind it: read it
    /// with [`read`]. `levels` is how many directories above the starting
    /// directory it sits — what a relative `../hledger.conf` handle needs.
    AboveVault { path: PathBuf, levels: usize },
}

/// The config file in force for `directory`, the way a terminal `hledger` run
/// beside the journal finds it: [`locate_in`] inside the vault, then — only
/// when the vault has a real directory behind it ([`Vault::local_path`] of its
/// root, i.e. the desktop) — the real directories ABOVE the vault root, nearest
/// first, by plain filesystem `metadata` as before the vault seam. A vault with
/// no local path (in-memory, a sandboxed iOS folder) has no "above" and stops
/// at its root.
///
/// Above the root only a regular file of at most [`MAX_CONF_BYTES`] counts, and
/// the whole walk is bounded by [`MAX_ANCESTORS`].
///
/// The upward walk only: `~/.hledger.conf` and the XDG file are not consulted
/// here. [`locate_conf_for`] is the full lookup, and what the server uses.
#[must_use]
pub fn locate_conf(vault: &dyn Vault, directory: &VaultPath) -> Option<ConfFile> {
    if let Some(found) = locate_in(vault, directory) {
        return Some(ConfFile::InVault(found));
    }
    let native_root = vault.local_path(&VaultPath::root())?;
    let depth = directory.segments().count();
    native_root
        .ancestors()
        .skip(1)
        .take(MAX_ANCESTORS.saturating_sub(depth + 1))
        .enumerate()
        .map(|(above, dir)| (depth + above + 1, dir.join(CONF_NAME)))
        .find(|(_, candidate)| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_CONF_BYTES)
        })
        .map(|(levels, path)| ConfFile::AboveVault { path, levels })
}

/// The user-level directories hledger falls back to when no `hledger.conf`
/// sits at or above the journal: `$HOME` and `$XDG_CONFIG_HOME`.
///
/// **Passed in, never read from the environment here**, so core stays pure and
/// a test can point both at a temp dir instead of somebody's real dotfiles. The
/// desktop server fills it from its `AppDirs` seam; a sandboxed host leaves it
/// empty, which is also [`Default`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserConfDirs {
    /// `$HOME`. `~/.hledger.conf` is looked for here, and the XDG default
    /// (`~/.config`) derives from it.
    pub home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` when it is set and non-empty. `None` means hledger's
    /// default, `$HOME/.config`.
    pub xdg_config_home: Option<PathBuf>,
}

impl UserConfDirs {
    /// `~/.hledger.conf`, the second place hledger looks.
    #[must_use]
    pub fn home_conf(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|home| home.join(".hledger.conf"))
    }

    /// `$XDG_CONFIG_HOME/hledger/hledger.conf`, the third and last, with its
    /// `~/.config` default.
    #[must_use]
    pub fn xdg_conf(&self) -> Option<PathBuf> {
        self.xdg_base()
            .map(|base| base.join("hledger").join(CONF_NAME))
    }

    fn xdg_base(&self) -> Option<PathBuf> {
        self.xdg_config_home
            .clone()
            .or_else(|| self.home.as_ref().map(|home| home.join(".config")))
    }

    /// Whether the XDG directory is somewhere other than `~/.config`, which
    /// decides how its handle is spelled.
    fn custom_xdg(&self) -> bool {
        match (&self.xdg_config_home, &self.home) {
            (Some(xdg), Some(home)) => *xdg != home.join(".config"),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// Which tier of hledger's lookup order a config file was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfOrigin {
    /// In the journal's directory (`levels == 0`) or `levels` directories above
    /// it — the upward walk.
    Ledger { levels: usize },
    /// `~/.hledger.conf`.
    Home,
    /// `$XDG_CONFIG_HOME/hledger/hledger.conf`. `custom` when that is not the
    /// `~/.config` default, so the handle names the variable instead.
    Xdg { custom: bool },
}

impl ConfOrigin {
    /// The handle a client is shown for a file from this tier: relative to the
    /// journal's directory, or `~`-relative, and **never an absolute path**
    /// (security layer 5, as [`ConfFile`]'s relative handle always was).
    #[must_use]
    pub fn handle(self) -> String {
        match self {
            Self::Ledger { levels } => format!("{}{CONF_NAME}", "../".repeat(levels)),
            Self::Home => "~/.hledger.conf".to_string(),
            Self::Xdg { custom: false } => format!("~/.config/hledger/{CONF_NAME}"),
            Self::Xdg { custom: true } => format!("$XDG_CONFIG_HOME/hledger/{CONF_NAME}"),
        }
    }

    /// A short machine-readable name for the wire.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Ledger { .. } => "ledger",
            Self::Home => "home",
            Self::Xdg { .. } => "xdg",
        }
    }
}

/// How a located config file is read and written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfSource {
    /// Inside the journal's vault: [`read_in`].
    InVault(VaultPath),
    /// A real path outside it — above the vault root, or a user-level file:
    /// [`read`].
    Native(PathBuf),
}

/// The config file in force, from [`locate_conf_for`]: where it is, and which
/// tier of the lookup found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedConf {
    pub source: ConfSource,
    pub origin: ConfOrigin,
}

impl LocatedConf {
    /// The client-safe handle; see [`ConfOrigin::handle`].
    #[must_use]
    pub fn handle(&self) -> String {
        self.origin.handle()
    }

    /// Read it, through `vault` when it is inside it.
    ///
    /// # Errors
    /// As [`read_in`] / [`read`].
    pub fn read(&self, vault: &dyn Vault) -> std::io::Result<String> {
        match &self.source {
            ConfSource::InVault(path) => read_in(vault, path),
            ConfSource::Native(path) => read(path),
        }
    }
}

/// The config file in force for `directory`, in hledger's own lookup order
/// (Decision 10 of plan 30), nearest first and exactly one file:
///
/// 1. the journal's directory, then each parent upward ([`locate_conf`]: inside
///    the vault, then above its root on a vault with a real directory);
/// 2. `~/.hledger.conf`;
/// 3. `$XDG_CONFIG_HOME/hledger/hledger.conf` (default
///    `~/.config/hledger/hledger.conf`).
///
/// Tiers 2 and 3 are consulted **only on a vault with a real directory behind
/// it** (the desktop), for the reason [`locate_conf`] stops at a sandboxed
/// vault's root: such a host has no home directory that is the user's. Same
/// file rules as above the root — a regular file (a symlink to one counts, as
/// it does for hledger and for every dotfile manager) of at most
/// [`MAX_CONF_BYTES`].
#[must_use]
pub fn locate_conf_for(
    vault: &dyn Vault,
    directory: &VaultPath,
    user: &UserConfDirs,
) -> Option<LocatedConf> {
    if let Some(found) = locate_conf(vault, directory) {
        return Some(match found {
            ConfFile::InVault(path) => {
                let below = path.parent().map_or(0, |parent| parent.segments().count());
                LocatedConf {
                    origin: ConfOrigin::Ledger {
                        levels: directory.segments().count().saturating_sub(below),
                    },
                    source: ConfSource::InVault(path),
                }
            }
            ConfFile::AboveVault { path, levels } => LocatedConf {
                source: ConfSource::Native(path),
                origin: ConfOrigin::Ledger { levels },
            },
        });
    }
    vault.local_path(&VaultPath::root())?;
    [
        (user.home_conf(), ConfOrigin::Home),
        (
            user.xdg_conf(),
            ConfOrigin::Xdg {
                custom: user.custom_xdg(),
            },
        ),
    ]
    .into_iter()
    .find_map(|(candidate, origin)| {
        let candidate = candidate?;
        std::fs::metadata(&candidate)
            .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_CONF_BYTES)
            .then_some(LocatedConf {
                source: ConfSource::Native(candidate),
                origin,
            })
    })
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Why an alias cannot be written into a config file.
///
/// Every variant is a case where the value *could* be written and would then
/// mean something other than it says. A refusal is reported to the user beside
/// the alias it belongs to; none of them is a silent drop, because a mapping that
/// looks installed and does not match is worse than one that is visibly missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfRefusal {
    /// A `#` on either side. It starts a comment in a config file, so everything
    /// after it would be discarded.
    Comment,
    /// Whitespace in the replacement. The pattern's whitespace has a workaround
    /// (`.` matches a space); an account NAME has none — a config file has no
    /// quoting and no escape, so the name would simply be cut short.
    ReplacementWhitespace,
    /// A backslash in the replacement of an alias that must be converted to a
    /// regular expression, where `\1` means a capture group rather than a
    /// literal backslash.
    ReplacementBackslash,
    /// A bracket expression in a regular-expression pattern that also contains
    /// whitespace. Inside `[...]` a `.` is an ordinary dot, so the substitution
    /// this module performs would change what the pattern matches.
    PatternBracket,
    /// A backslash in a regular-expression pattern that also contains whitespace.
    /// Substituting next to an escape somebody else wrote is a guess.
    PatternBackslash,
    /// A `/` in a plain pattern that must be converted to a regular expression,
    /// where `/` ends the pattern and would have to be re-escaped.
    PatternSlash,
}

impl ConfRefusal {
    /// A sentence completing "this alias cannot be added to your config file
    /// because …", written for the person reading the screen.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Comment => {
                "it contains a `#`, which starts a comment in a config file — everything after it \
                 would be thrown away"
            }
            Self::ReplacementWhitespace => {
                "the account it maps to contains a space, and a config file has no quoting and no \
                 escape for one, so hledger would read only the first word"
            }
            Self::ReplacementBackslash => {
                "the account it maps to contains a backslash, which would become a capture-group \
                 reference once this alias is written as a regular expression"
            }
            Self::PatternBracket => {
                "its pattern contains both a space and a `[...]` group, and inside such a group a \
                 `.` is an ordinary dot rather than the any-character stand-in this rewrite needs"
            }
            Self::PatternBackslash => {
                "its pattern contains both a space and a backslash escape, and Ledgeline will not \
                 rewrite next to an escape it did not write"
            }
            Self::PatternSlash => {
                "its pattern contains both a space and a `/`, and the `/` would have to be \
                 re-escaped to survive being written as a regular expression"
            }
        }
    }

    /// The machine-readable half, for the wire. Spelled out rather than derived
    /// from `Debug`, so a rename here cannot silently change a wire value.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::ReplacementWhitespace => "replacementWhitespace",
            Self::ReplacementBackslash => "replacementBackslash",
            Self::PatternBracket => "patternBracket",
            Self::PatternBackslash => "patternBackslash",
            Self::PatternSlash => "patternSlash",
        }
    }
}

/// The `--alias` **value** for one alias, in a form a config file can hold.
///
/// # The problem
///
/// hledger's config parser splits on whitespace and ignores quotes, so
/// `--alias="/^PW Roth IRA - 3077/=assets:x"` is a parse error and so is the
/// unquoted spelling. Both verified. There is no escape for a space.
///
/// # The rule, in order
///
/// 1. **No whitespace anywhere ⇒ write it verbatim**, in whichever form the user
///    declared. This is the common case and it is provably identical to what
///    Ledgeline forwards on the command line, because it is the same string.
/// 2. **Whitespace in the replacement ⇒ refuse.** An account name is matched
///    literally; there is no wildcard to stand in for its space.
/// 3. **Whitespace in a REGEX pattern ⇒ substitute.** Each whitespace character
///    becomes `.`, which matches it. Refused when the pattern also holds `[` or
///    `\`, where the substitution could mean something else.
/// 4. **Whitespace in a PLAIN pattern ⇒ convert to a regex.** A plain alias
///    matches the whole account name or a prefix ending at a `:` (verified: it
///    rewrites `a` and `a:sub` and leaves `abc` alone), which is exactly
///    `/^PATTERN($|:)/` with `\1` carrying the boundary into the replacement.
///    Regex metacharacters in the original are escaped first, then its whitespace
///    becomes `.`.
///
/// # Two widenings, both deliberate and both shown to the user
///
/// A `.` matches *any* character, not only a space, and hledger's regex aliases
/// are **case-insensitive** where plain ones are not (both verified). So a
/// converted alias matches everything the original did and can match a little
/// more. That is the trade for being expressible at all, and it is why the
/// resulting line is displayed before it is written rather than after.
///
/// # Errors
/// [`ConfRefusal`], one variant per case above.
pub fn conf_argument(pattern: &str, replacement: &str, regex: bool) -> Result<String, ConfRefusal> {
    if pattern.contains('#') || replacement.contains('#') {
        return Err(ConfRefusal::Comment);
    }
    if replacement.chars().any(char::is_whitespace) {
        return Err(ConfRefusal::ReplacementWhitespace);
    }
    if !pattern.chars().any(char::is_whitespace) {
        // Rule 1: the same bytes we put on the command line.
        let rendered = if regex {
            format!("/{pattern}/")
        } else {
            pattern.to_string()
        };
        return Ok(format!("{rendered}={replacement}"));
    }
    if regex {
        if pattern.contains('[') {
            return Err(ConfRefusal::PatternBracket);
        }
        if pattern.contains('\\') {
            return Err(ConfRefusal::PatternBackslash);
        }
        return Ok(format!("/{}/={replacement}", dotted(pattern)));
    }
    if pattern.contains('/') {
        return Err(ConfRefusal::PatternSlash);
    }
    if replacement.contains('\\') {
        return Err(ConfRefusal::ReplacementBackslash);
    }
    Ok(format!(
        "/^{}($|:)/={replacement}\\1",
        dotted(&escaped(pattern))
    ))
}

/// Every whitespace character replaced by `.`, the regex any-character atom.
///
/// One `.` per character, never one per run: `.` matches exactly one character,
/// so collapsing two spaces into one dot would produce a pattern that no longer
/// matches the name it came from.
fn dotted(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_whitespace() { '.' } else { c })
        .collect()
}

/// Every POSIX-ERE metacharacter escaped, so a literal pattern stays literal.
///
/// hledger's alias regexes are Haskell `regex-tdfa`, i.e. POSIX ERE. The set
/// below is that dialect's specials plus `/`, which ends an alias pattern, and
/// `\` itself. Whitespace is deliberately NOT escaped here — [`dotted`] runs
/// afterwards and turns it into the one metacharacter this conversion wants.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '.' | '^'
                | '$'
                | '*'
                | '+'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | '\\'
                | '/'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// One complete config-file line for an alias value.
#[must_use]
pub fn conf_line(argument: &str) -> String {
    format!("--alias={argument}")
}

/// Where a new general-section option has to go, as a byte offset into `text`.
///
/// **Not the end of the file**, and that distinction is load-bearing. Everything
/// after the first `[heading]` belongs to that command's section, so a line
/// appended at EOF of a file ending in `[balance]` would be a balance-only option
/// — present, plausible, and never applied to an import. So the insertion point
/// is immediately before the first heading, or the end of the file when there is
/// none.
#[must_use]
pub fn general_section_end(text: &str) -> usize {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.split('#').next().unwrap_or_default().trim();
        if heading(content).is_some() {
            return offset;
        }
        offset += line.len();
    }
    text.len()
}

/// `text` with `arguments` added as `--alias` lines in its general section.
///
/// Pure, and every byte outside the inserted run comes out of `text` unchanged —
/// the same discipline `aliases::AliasDoc` holds to, for the same reason: this is
/// a file the user may well have written by hand.
///
/// Arguments already present are skipped, so pressing the button twice adds
/// nothing the second time. A missing final newline before the insertion point is
/// supplied, because without one the new option would be glued onto whatever line
/// was there and become part of its value.
#[must_use]
pub fn with_aliases(text: &str, arguments: &[String]) -> String {
    let existing = alias_arguments(text, IMPORT_COMMAND);
    let wanted: Vec<&String> = arguments
        .iter()
        .filter(|argument| !existing.contains(argument))
        .collect();
    if wanted.is_empty() {
        return text.to_string();
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = wanted
        .iter()
        .map(|argument| conf_line(argument))
        .collect::<Vec<_>>()
        .join(newline);
    insert_line(text, &scan(text), &Section::General, &body)
}

/// The header a config file Ledgeline creates from scratch opens with.
///
/// Only for a file that did not exist. An existing config gets its option lines
/// and nothing else — a comment block inserted into somebody's hand-written file
/// is bytes they did not ask for.
#[must_use]
pub fn new_file_header() -> String {
    "# hledger's config file: every hledger command run for these books reads it.\n\
     # Created by Ledgeline, which changes only the lines you edit from its\n\
     # Settings › hledger config tab or its import screen's alias fix. (An `alias`\n\
     # directive in a journal is NOT applied to an imported CSV; an --alias here is.)\n"
        .to_string()
}

/// The sentence explaining a hijacked config file, completing nothing: shown on
/// its own beside the file, and as the reason a write was refused.
#[must_use]
pub fn hijack_sentence(word: &str) -> String {
    format!(
        "This config file starts with `{word}`, which hledger reads as the command to run: it \
         replaces whatever command you type, so every hledger command is affected and none of the \
         file's options reach anything. Ledgeline will not edit it until that first word is \
         removed or turned into a `#` comment."
    )
}

// ---------------------------------------------------------------------------
// Settings Ledgeline honours: `--auto` and `--depth`
// ---------------------------------------------------------------------------

/// The report commands whose `--depth` Ledgeline honours, each by hledger's
/// FULL command name — the only spelling a section heading may use (verified,
/// hledger 1.52: `[balancesheet]` applies, `[bs]` is silently ignored, and so
/// is `[BalanceSheet]`, because heading names are case-sensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepthCommand {
    /// `[balancesheet]` — the Balance Sheet tab's grouped tree.
    BalanceSheet,
    /// `[incomestatement]` — the P&L tab's grouped tree.
    IncomeStatement,
    /// `[cashflow]` — the Cash Flow tab's depth control.
    CashFlow,
    /// `[balance]` — the Net Worth tab's depth control (net worth is hledger's
    /// `balance --historical` over assets and liabilities).
    Balance,
}

impl DepthCommand {
    /// Every one, in the order the settings screen lists them.
    pub const ALL: [Self; 4] = [
        Self::BalanceSheet,
        Self::IncomeStatement,
        Self::CashFlow,
        Self::Balance,
    ];

    /// hledger's full command name, which is also the section heading.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::BalanceSheet => "balancesheet",
            Self::IncomeStatement => "incomestatement",
            Self::CashFlow => "cashflow",
            Self::Balance => "balance",
        }
    }

    /// The command a full name names, exactly as hledger matches it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.name() == name)
    }

    /// The short names hledger accepts on the command line and ignores as a
    /// section heading.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::BalanceSheet => &["bs"],
            Self::IncomeStatement => &["is"],
            Self::CashFlow => &["cf"],
            Self::Balance => &["bal", "b"],
        }
    }

    /// The command a heading was probably MEANT to name — an alias, or the full
    /// name in the wrong case — when it is not that name exactly.
    fn intended_by(heading: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| {
            heading != command.name()
                && (heading.eq_ignore_ascii_case(command.name())
                    || command
                        .aliases()
                        .iter()
                        .any(|alias| heading.eq_ignore_ascii_case(alias)))
        })
    }
}

/// What one section's `--depth` options say: a plain limit and any per-account
/// ones.
///
/// Verified against hledger 1.52, and the reason this is not one number:
/// `--depth N` and `--depth REGEX=N` COMBINE (the plain one limits every
/// account no regex matches); repeating either kind replaces it — the last
/// plain `N` wins, and the last value for one REGEX wins; and `-N` is a plain
/// `--depth N`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepthSpec {
    /// The plain limit, if one is set.
    pub all: Option<u32>,
    /// `(REGEX, N)` pairs, in the order first declared, one per regex.
    pub per_regex: Vec<(String, u32)>,
}

impl DepthSpec {
    /// Nothing set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.all.is_none() && self.per_regex.is_empty()
    }

    fn apply(&mut self, value: &DepthValue) {
        match value {
            DepthValue::All(depth) => self.all = Some(*depth),
            DepthValue::Regex(regex, depth) => {
                match self.per_regex.iter_mut().find(|(known, _)| known == regex) {
                    Some(slot) => slot.1 = *depth,
                    None => self.per_regex.push((regex.clone(), *depth)),
                }
            }
            DepthValue::Bad(_) => {}
        }
    }

    /// `self` with `later`'s options applied after it — how hledger combines a
    /// general section with a command's (general first, verified): a command
    /// section's plain limit replaces the general one, and their per-regex
    /// limits accumulate.
    #[must_use]
    pub fn then(&self, later: &Self) -> Self {
        let mut out = self.clone();
        if let Some(depth) = later.all {
            out.apply(&DepthValue::All(depth));
        }
        for (regex, depth) in &later.per_regex {
            out.apply(&DepthValue::Regex(regex.clone(), *depth));
        }
        out
    }
}

/// One `--depth` value as written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DepthValue {
    All(u32),
    Regex(String, u32),
    /// Unreadable — including a regex hledger would refuse, which makes it
    /// refuse the whole file. Carried so it can be warned about.
    Bad(String),
}

impl DepthValue {
    fn parse(raw: &str) -> Self {
        if let Ok(depth) = raw.parse::<u32>() {
            return Self::All(depth);
        }
        match raw.rsplit_once('=') {
            Some((regex, depth)) if !regex.is_empty() => match depth.parse::<u32>() {
                Ok(depth) if regex_refusal(regex).is_none() => {
                    Self::Regex(regex.to_string(), depth)
                }
                _ => Self::Bad(raw.to_string()),
            },
            _ => Self::Bad(raw.to_string()),
        }
    }
}

/// Something in a config file a reader should be told about, because hledger
/// will not do what it looks like it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfWarning {
    /// `[bs]`, `[BalanceSheet]` …: hledger ignores a heading that is not a full
    /// command name exactly, so every option under it does nothing.
    AliasSection {
        found: String,
        command: DepthCommand,
    },
    /// The same command heading twice. hledger reads only the LAST one
    /// (verified): options under the earlier ones do nothing.
    DuplicateSection { command: DepthCommand },
    /// A `--depth` value that is not `N` or `REGEX=N`, or whose regex is
    /// invalid — which makes hledger refuse every command.
    BadDepth { section: Section, value: String },
}

impl ConfWarning {
    /// The machine-readable half, for the wire.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AliasSection { .. } => "aliasSection",
            Self::DuplicateSection { .. } => "duplicateSection",
            Self::BadDepth { .. } => "badDepth",
        }
    }

    /// A sentence for the person reading the settings screen.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::AliasSection { found, command } => format!(
                "hledger ignores the `[{found}]` section: a section heading must be the full command \
                 name, exactly, so its options do nothing. Rename it `[{}]`.",
                command.name()
            ),
            Self::DuplicateSection { command } => format!(
                "`[{0}]` appears more than once. hledger reads only the last `[{0}]` section, so \
                 options in the earlier ones do nothing.",
                command.name()
            ),
            Self::BadDepth { section, value } => {
                let place = match section {
                    Section::General => "the general section".to_string(),
                    Section::Command(name) => format!("`[{name}]`"),
                };
                format!(
                    "`--depth {value}` in {place} is not `N` or `ACCOUNTREGEX=N` with a regular \
                     expression Ledgeline can read. If hledger cannot read it either, every \
                     hledger command will fail until it is fixed."
                )
            }
        }
    }
}

/// The settings Ledgeline honours from a config file (plan 30, Decision 10):
/// `--auto` in the general section, and `--depth` in the general section and
/// the four report sections. Everything else in the file is left to hledger.
///
/// A pure reading of the text. A file whose first word hijacks the command
/// ([`hijacks_command`]) reads as no settings at all, as [`alias_arguments`]
/// does: hledger cannot run any command with it, so none of its options apply
/// to anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfSettings {
    /// `--auto` in the general section: apply the journal's `=` rules to its
    /// real transactions (plan 30, Decision 7).
    pub auto: bool,
    /// The general section's `--depth` options, as written there.
    pub general: DepthSpec,
    /// Each report section's own `--depth` options, as written in the LAST
    /// heading of that name (the one hledger reads). Absent: no such section,
    /// or nothing set in it.
    pub sections: std::collections::BTreeMap<DepthCommand, DepthSpec>,
    /// What hledger will not do as written; see [`ConfWarning`].
    pub warnings: Vec<ConfWarning>,
}

impl ConfSettings {
    /// Read the settings out of a config file's text.
    #[must_use]
    pub fn read(text: &str) -> Self {
        if hijacks_command(text).is_some() {
            return Self::default();
        }
        let lines = scan(text);
        let last = last_occurrences(&lines);
        let mut settings = Self::default();
        for found in occurrences(text, &lines) {
            let at = &lines[found.line].section;
            let command = match &at.section {
                Section::General => None,
                Section::Command(name) => match DepthCommand::from_name(name) {
                    Some(command) if last.get(name) == Some(&at.occurrence) => Some(command),
                    _ => continue,
                },
            };
            match (&found.what, command) {
                (Found::Auto, None) => settings.auto = true,
                (Found::Auto, Some(_)) => {}
                (Found::Depth(value), command) => {
                    if let DepthValue::Bad(raw) = value {
                        settings.warnings.push(ConfWarning::BadDepth {
                            section: at.section.clone(),
                            value: raw.clone(),
                        });
                    }
                    let spec = match command {
                        None => &mut settings.general,
                        Some(command) => settings.sections.entry(command).or_default(),
                    };
                    spec.apply(value);
                }
            }
        }
        settings.sections.retain(|_, spec| !spec.is_empty());
        let mut headings: Vec<&str> = Vec::new();
        for line in lines.iter().filter(|line| line.heading) {
            let Section::Command(name) = &line.section.section else {
                continue;
            };
            if headings.contains(&name.as_str()) {
                continue;
            }
            headings.push(name);
            if let Some(command) = DepthCommand::intended_by(name) {
                settings.warnings.push(ConfWarning::AliasSection {
                    found: name.clone(),
                    command,
                });
            }
            if let Some(command) = DepthCommand::from_name(name)
                && last.get(name).is_some_and(|count| *count > 1)
            {
                settings
                    .warnings
                    .push(ConfWarning::DuplicateSection { command });
            }
        }
        settings
    }

    /// The `--depth` hledger applies to `command`: the general section's
    /// options, then that command's section's.
    #[must_use]
    pub fn depth(&self, command: DepthCommand) -> DepthSpec {
        self.sections
            .get(&command)
            .map_or_else(|| self.general.clone(), |own| self.general.then(own))
    }

    /// The options written in `section`, when it is the general section or one
    /// of the four this module reads.
    #[must_use]
    pub fn written(&self, section: Option<DepthCommand>) -> DepthSpec {
        match section {
            None => self.general.clone(),
            Some(command) => self.sections.get(&command).cloned().unwrap_or_default(),
        }
    }
}

/// Is `--auto` in force in this config file? The one question Phase 5 of plan
/// 30 asks of it.
#[must_use]
pub fn auto_enabled(text: &str) -> bool {
    ConfSettings::read(text).auto
}

/// Why a regex cannot be written as a `--depth REGEX=N` value, or `None` when
/// it can.
///
/// A config file has no quoting, so whitespace and `#` cannot be written; a
/// leading `-` would read as another option; an `=` makes the split ambiguous;
/// and a regex hledger cannot compile makes it refuse every command — so one
/// the `regex` crate rejects is refused here too.
#[must_use]
pub fn regex_refusal(regex: &str) -> Option<&'static str> {
    if regex.is_empty() {
        return Some("the account pattern is empty");
    }
    if regex.chars().any(char::is_whitespace) {
        return Some(
            "the account pattern contains a space, and a config file has no quoting for one — use `.` to match it",
        );
    }
    if regex.contains('#') {
        return Some("the account pattern contains a `#`, which starts a comment in a config file");
    }
    if regex.starts_with('-') {
        return Some("the account pattern starts with `-`, which hledger would read as an option");
    }
    if regex.contains('=') {
        return Some("the account pattern contains `=`, which separates it from the depth");
    }
    if regex::RegexBuilder::new(regex)
        .case_insensitive(true)
        .build()
        .is_err()
    {
        return Some("the account pattern is not a valid regular expression");
    }
    None
}

// ---------------------------------------------------------------------------
// Surgical writes: one option, in one section
// ---------------------------------------------------------------------------

/// An option Ledgeline knows how to find, write and remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedOption {
    /// `--auto`.
    Auto,
    /// `--depth N`.
    Depth(u32),
    /// `--depth REGEX=N`.
    DepthFor { regex: String, depth: u32 },
}

/// Which occurrence an option replaces: its identity, without its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfSlot {
    /// `--auto`.
    Auto,
    /// The plain `--depth N` (also `--depth=N` and `-N`).
    Depth,
    /// `--depth REGEX=N` for this REGEX, compared exactly.
    DepthFor(String),
}

impl ManagedOption {
    /// The slot this option fills.
    #[must_use]
    pub fn slot(&self) -> ConfSlot {
        match self {
            Self::Auto => ConfSlot::Auto,
            Self::Depth(_) => ConfSlot::Depth,
            Self::DepthFor { regex, .. } => ConfSlot::DepthFor(regex.clone()),
        }
    }

    /// The option as Ledgeline writes it: two tokens for a valued option, the
    /// spelling hledger's own manual uses.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Auto => "--auto".to_string(),
            Self::Depth(depth) => format!("--depth {depth}"),
            Self::DepthFor { regex, depth } => format!("--depth {regex}={depth}"),
        }
    }

    fn is(&self, found: &Found) -> bool {
        match (self, found) {
            (Self::Auto, Found::Auto) => true,
            (Self::Depth(depth), Found::Depth(DepthValue::All(other))) => depth == other,
            (Self::DepthFor { regex, depth }, Found::Depth(DepthValue::Regex(r, d))) => {
                regex == r && depth == d
            }
            _ => false,
        }
    }
}

impl ConfSlot {
    fn holds(&self, found: &Found) -> bool {
        match (self, found) {
            (Self::Auto, Found::Auto) => true,
            (Self::Depth, Found::Depth(DepthValue::All(_))) => true,
            (Self::DepthFor(regex), Found::Depth(DepthValue::Regex(r, _))) => regex == r,
            _ => false,
        }
    }
}

/// Why an option cannot be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfWriteError {
    /// The file's first word replaces the command; see [`hijack_sentence`].
    Hijacked(String),
    /// A section name that is not a plain lower-case command name.
    BadSection(String),
    /// A value a config file cannot hold as written; the reason completes "it
    /// cannot be written because …".
    Inexpressible(&'static str),
}

impl std::fmt::Display for ConfWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hijacked(word) => f.write_str(&hijack_sentence(word)),
            Self::BadSection(name) => {
                write!(f, "`[{name}]` is not a command name Ledgeline writes")
            }
            Self::Inexpressible(reason) => write!(f, "that setting cannot be written: {reason}"),
        }
    }
}

impl std::error::Error for ConfWriteError {}

/// `text` with `option` set in `section`: the last occurrence of its slot
/// there rewritten in place, or a new line inserted — at the end of the general
/// section (never at EOF, see [`general_section_end`]), after the last option of
/// a command section, or under a new `[command]` heading appended at EOF.
///
/// A command section means its LAST heading, the one hledger reads. Every byte
/// outside the rewritten or inserted run comes out unchanged — comments,
/// unknown options, blank lines, CRLF line endings — and a value that is
/// already in force returns `text` byte-identical, so a caller can compare and
/// skip the write.
///
/// # Errors
/// [`ConfWriteError`]: a hijacked file, a bad section name, or a regex a config
/// file cannot hold ([`regex_refusal`]).
pub fn with_option(
    text: &str,
    section: &Section,
    option: &ManagedOption,
) -> Result<String, ConfWriteError> {
    guard(text, section)?;
    if let ManagedOption::DepthFor { regex, .. } = option
        && let Some(reason) = regex_refusal(regex)
    {
        return Err(ConfWriteError::Inexpressible(reason));
    }
    let lines = scan(text);
    let slot = option.slot();
    let present = in_section(text, &lines, section)
        .into_iter()
        .rfind(|found| slot.holds(&found.what));
    Ok(match present {
        Some(found) if option.is(&found.what) => text.to_string(),
        Some(found) => format!(
            "{}{}{}",
            &text[..found.span.start],
            option.render(),
            &text[found.span.end..]
        ),
        None => insert_line(text, &lines, section, &option.render()),
    })
}

/// `text` with every occurrence of `slot` removed from `section` (its last
/// heading, for a command). A line left with nothing but whitespace is removed
/// whole, its trailing comment with it; otherwise only the option's tokens go.
/// Nothing to remove returns `text` byte-identical.
///
/// # Errors
/// [`ConfWriteError::Hijacked`] or [`ConfWriteError::BadSection`].
pub fn without_option(
    text: &str,
    section: &Section,
    slot: &ConfSlot,
) -> Result<String, ConfWriteError> {
    guard(text, section)?;
    let lines = scan(text);
    let mut out = text.to_string();
    // Back to front, so every earlier span is still where it was.
    for found in in_section(text, &lines, section)
        .into_iter()
        .filter(|found| slot.holds(&found.what))
        .rev()
    {
        let line = &lines[found.line];
        let body_end = line.start + line.body.len();
        let rest_blank = text[line.start..found.span.start].trim().is_empty()
            && text[found.span.end..body_end].trim().is_empty();
        let range = if rest_blank {
            line.start..line.end
        } else {
            // The option and the whitespace after it — or, at the end of the
            // body, the whitespace before it.
            let after = text[found.span.end..body_end].len()
                - text[found.span.end..body_end].trim_start().len();
            if after > 0 && found.span.end + after < body_end {
                found.span.start..found.span.end + after
            } else {
                let before = text[line.start..found.span.start].len()
                    - text[line.start..found.span.start].trim_end().len();
                found.span.start - before..found.span.end
            }
        };
        out.replace_range(range, "");
    }
    Ok(out)
}

fn guard(text: &str, section: &Section) -> Result<(), ConfWriteError> {
    if let Some(word) = hijacks_command(text) {
        return Err(ConfWriteError::Hijacked(word));
    }
    if let Section::Command(name) = section
        && (name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    {
        return Err(ConfWriteError::BadSection(name.clone()));
    }
    Ok(())
}

/// The options found in `section` as hledger reads it: the general section, or
/// the last heading of a command's name.
fn in_section(text: &str, lines: &[Line<'_>], section: &Section) -> Vec<Occurrence> {
    let target = target_of(lines, section);
    occurrences(text, lines)
        .into_iter()
        .filter(|found| Some(&lines[found.line].section) == target.as_ref())
        .collect()
}

fn target_of(lines: &[Line<'_>], section: &Section) -> Option<SectionAt> {
    match section {
        Section::General => Some(SectionAt {
            section: Section::General,
            occurrence: 0,
        }),
        Section::Command(name) => last_occurrences(lines)
            .get(name)
            .map(|occurrence| SectionAt {
                section: section.clone(),
                occurrence: *occurrence,
            }),
    }
}

/// `text` with `option_line` inserted into `section` (see [`with_option`]).
fn insert_line(text: &str, lines: &[Line<'_>], section: &Section, option_line: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let at = match (section, target_of(lines, section)) {
        // After the general section's last option, so a new one joins its
        // peers rather than the blank line before the first heading; with no
        // option yet, just before that heading (never at EOF — see
        // `general_section_end`).
        (Section::General, Some(target)) => lines
            .iter()
            .rfind(|line| line.section == target && !line.body.trim().is_empty())
            .map_or_else(|| general_section_end(text), |line| line.end),
        (Section::General, None) => general_section_end(text),
        (Section::Command(_), Some(target)) => lines
            .iter()
            .rfind(|line| line.section == target && (line.heading || !line.body.trim().is_empty()))
            .map_or(text.len(), |line| line.end),
        (Section::Command(name), None) => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(newline);
            }
            if !out.is_empty() && !out.ends_with(&format!("{newline}{newline}")) {
                out.push_str(newline);
            }
            return format!("{out}[{name}]{newline}{option_line}{newline}");
        }
    };
    let head = &text[..at];
    let lead = if head.is_empty() || head.ends_with('\n') {
        ""
    } else {
        newline
    };
    format!("{head}{lead}{option_line}{newline}{}", &text[at..])
}

/// Which heading a line sits under: the section, and which occurrence of that
/// heading (1 for the first `[balance]`, 2 for a second …; 0 for general).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionAt {
    section: Section,
    occurrence: usize,
}

/// One line of a config file, by byte offsets into the text.
struct Line<'a> {
    /// Offset of the line's first byte.
    start: usize,
    /// Offset just past its terminator (the text's length for an unterminated
    /// last line).
    end: usize,
    /// The part before any `#` comment, without the terminator.
    body: &'a str,
    section: SectionAt,
    /// It is a `[heading]` (and belongs to the section it opens).
    heading: bool,
}

fn scan(text: &str) -> Vec<Line<'_>> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut current = SectionAt {
        section: Section::General,
        occurrence: 0,
    };
    let mut offset = 0;
    text.split_inclusive('\n')
        .map(|chunk| {
            let start = offset;
            offset += chunk.len();
            let raw = chunk.strip_suffix('\n').unwrap_or(chunk);
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let body = raw.split('#').next().unwrap_or_default();
            let opened = heading(body.trim());
            if let Some(name) = &opened {
                let count = seen.entry(name.clone()).or_insert(0);
                *count += 1;
                current = SectionAt {
                    section: Section::Command(name.clone()),
                    occurrence: *count,
                };
            }
            Line {
                start,
                end: offset,
                body,
                section: current.clone(),
                heading: opened.is_some(),
            }
        })
        .collect()
}

/// How many times each heading appears, which is also the occurrence number
/// of its last one — the one hledger reads.
fn last_occurrences(lines: &[Line<'_>]) -> std::collections::HashMap<String, usize> {
    lines
        .iter()
        .filter(|line| line.heading)
        .filter_map(|line| match &line.section.section {
            Section::Command(name) => Some((name.clone(), line.section.occurrence)),
            Section::General => None,
        })
        .collect()
}

/// An option this module manages, as found in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    Auto,
    Depth(DepthValue),
}

struct Occurrence {
    /// Index into the scanned lines.
    line: usize,
    /// The bytes of its token, or of both tokens for `--depth N`.
    span: std::ops::Range<usize>,
    what: Found,
}

/// Every `--auto` and `--depth` in the file, in order, with its bytes.
///
/// Tokenised as hledger does — whitespace-separated, no quoting — with one
/// narrowing: a `--depth` whose value is on the NEXT line (hledger reads one;
/// nobody writes one) is reported as unreadable rather than followed.
fn occurrences(text: &str, lines: &[Line<'_>]) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.heading {
            continue;
        }
        let tokens: Vec<(usize, &str)> = line
            .body
            .split_whitespace()
            .map(|token| (token.as_ptr() as usize - text.as_ptr() as usize, token))
            .collect();
        let mut at = 0;
        while at < tokens.len() {
            let (start, token) = tokens[at];
            at += 1;
            let mut end = start + token.len();
            let what = if token == "--auto" {
                Some(Found::Auto)
            } else if let Some(value) = token.strip_prefix("--depth=") {
                Some(Found::Depth(DepthValue::parse(value)))
            } else if token == "--depth" {
                match tokens.get(at).filter(|(_, next)| !next.starts_with('-')) {
                    Some((next_start, next)) => {
                        at += 1;
                        end = next_start + next.len();
                        Some(Found::Depth(DepthValue::parse(next)))
                    }
                    None => Some(Found::Depth(DepthValue::Bad(String::new()))),
                }
            } else if let Some(depth) = token
                .strip_prefix('-')
                .filter(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
                .and_then(|digits| digits.parse::<u32>().ok())
            {
                Some(Found::Depth(DepthValue::All(depth)))
            } else {
                // Another option: a following non-flag token is its value, as
                // `options` reads it.
                if token.starts_with('-')
                    && !token.contains('=')
                    && tokens
                        .get(at)
                        .is_some_and(|(_, next)| !next.starts_with('-'))
                {
                    at += 1;
                }
                None
            };
            if let Some(what) = what {
                out.push(Occurrence {
                    line: index,
                    span: start..end,
                    what,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_and_import_sections_apply_and_others_do_not() {
        let text = "--alias=/^A/=general\n\
                    \n\
                    [balance]\n\
                    --alias=/^A/=balanceonly\n\
                    \n\
                    [import]\n\
                    --alias=/^A/=importonly\n";
        assert_eq!(
            alias_arguments(text, IMPORT_COMMAND),
            vec!["/^A/=general", "/^A/=importonly"]
        );
        // And the general one is FIRST, which is the order hledger applies them
        // and therefore the one that decides which alias wins.
        assert_eq!(alias_arguments(text, "balance")[1], "/^A/=balanceonly");
    }

    #[test]
    fn a_hash_is_a_comment_and_a_semicolon_is_not() {
        // Verified against the binary: `#` is the comment character, inline as
        // well as leading. `;` is not one — as the first token it is read as the
        // COMMAND, which is why the file below hijacks rather than comments.
        assert_eq!(
            alias_arguments(
                "# --alias=/^A/=nope\n--alias=/^A/=yes # trailing\n",
                IMPORT_COMMAND
            ),
            vec!["/^A/=yes"]
        );
        assert_eq!(hijacks_command("; --alias=/^A/=B\n").as_deref(), Some(";"));
        assert!(alias_arguments("; --alias=/^A/=B\n", IMPORT_COMMAND).is_empty());
    }

    #[test]
    fn only_the_first_token_of_the_general_section_hijacks() {
        // The single most dangerous thing a config file can do, and it is exactly
        // this narrow — verified against hledger 1.52 in both directions.
        assert_eq!(hijacks_command("balance\n").as_deref(), Some("balance"));
        assert_eq!(hijacks_command("--alias=a=b\nfoo\n"), None);
        // A bare word inside a COMMAND section is that section's argument and
        // overrides nothing in general.
        assert_eq!(hijacks_command("[import]\nfoo\n--alias=a=b\n"), None);
        assert_eq!(
            alias_arguments("[import]\nfoo\n--alias=a=b\n", IMPORT_COMMAND),
            vec!["a=b"]
        );
    }

    #[test]
    fn aliases_past_the_cap_are_counted_not_lost() {
        let text: String = (0..=MAX_CONF_ALIASES)
            .map(|n| format!("--alias=a{n}=b{n}\n"))
            .collect();
        let all = alias_arguments(&text, IMPORT_COMMAND);
        assert_eq!(all.len(), MAX_CONF_ALIASES + 1);
        let last = format!("a{MAX_CONF_ALIASES}=b{MAX_CONF_ALIASES}");
        // Already present, so a write adds nothing for it.
        assert_eq!(with_aliases(&text, std::slice::from_ref(&last)), text);
        let (forwarded, unforwarded) = forwarded(all);
        assert_eq!(forwarded.len(), MAX_CONF_ALIASES);
        assert_eq!(unforwarded, 1);
    }

    /// On the desktop the search continues ABOVE the vault root, as a terminal
    /// `hledger` would, and reports how far up the file sits.
    #[test]
    fn a_local_vault_finds_a_config_above_its_root() {
        let outer = std::env::temp_dir().join(format!(
            "ledgeline-conf-above-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let books = outer.join("books");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::write(outer.join(CONF_NAME), "--alias=a=b\n").unwrap();
        let vault = LocalVault::new(&books);
        match locate_conf(&vault, &VaultPath::root()) {
            Some(ConfFile::AboveVault { path, levels }) => {
                assert_eq!(levels, 1);
                assert_eq!(read(&path).unwrap(), "--alias=a=b\n");
            }
            other => panic!("expected the parent's config, got {other:?}"),
        }
        assert_eq!(
            locate_in(&vault, &VaultPath::root()),
            None,
            "not in the vault"
        );
        // One beside the journal wins over the one above.
        std::fs::write(books.join(CONF_NAME), "").unwrap();
        assert_eq!(
            locate_conf(&vault, &VaultPath::root()),
            Some(ConfFile::InVault(VaultPath::new(CONF_NAME).unwrap()))
        );
        std::fs::remove_dir_all(&outer).unwrap();
    }

    /// A vault with no local path (in-memory here; a sandboxed iOS folder later)
    /// has no "above": the search stops at its root.
    #[test]
    fn a_vault_without_a_local_path_stops_at_its_root() {
        let vault = crate::vault::MemVault::new()
            .with_file("hledger.conf", "--alias=x=y\n")
            .with_file("books/main.journal", "");
        let books = VaultPath::new("books").unwrap();
        assert_eq!(
            locate_conf(&vault, &books),
            Some(ConfFile::InVault(VaultPath::new(CONF_NAME).unwrap()))
        );
        let inner = crate::vault::MemVault::new().with_file("books/main.journal", "");
        assert_eq!(locate_conf(&inner, &books), None);
    }

    #[test]
    fn read_refuses_a_file_past_the_cap() {
        let dir = std::env::temp_dir().join(format!("ledgeline-conf-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONF_NAME);
        let cap = usize::try_from(MAX_CONF_BYTES).unwrap();
        std::fs::write(&path, "x".repeat(cap)).unwrap();
        assert_eq!(read(&path).unwrap().len(), cap);
        std::fs::write(&path, "x".repeat(cap + 1)).unwrap();
        assert_eq!(
            read(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn both_spellings_of_a_valued_option_are_read() {
        assert_eq!(
            alias_arguments("--alias=a=b\n--alias c=d\n", IMPORT_COMMAND),
            vec!["a=b", "c=d"]
        );
    }

    #[test]
    fn an_option_this_module_has_no_opinion_about_stops_nothing() {
        let text = "--depth 3\n--forecast\n--alias=a=b\n";
        assert_eq!(alias_arguments(text, IMPORT_COMMAND), vec!["a=b"]);
    }

    #[test]
    fn a_value_with_no_whitespace_is_written_verbatim_in_its_own_form() {
        assert_eq!(conf_argument("a:b", "c:d", false).unwrap(), "a:b=c:d");
        assert_eq!(conf_argument("^a.*", "c:d", true).unwrap(), "/^a.*/=c:d");
    }

    #[test]
    fn a_plain_pattern_with_spaces_becomes_the_equivalent_anchored_regex() {
        // The exact string this was all for, and the exact string a real
        // `hledger import` was verified to honour.
        assert_eq!(
            conf_argument(
                "PW Roth IRA - 3077",
                "assets:morganstanley:pw-roth-ira",
                false
            )
            .unwrap(),
            "/^PW.Roth.IRA.-.3077($|:)/=assets:morganstanley:pw-roth-ira\\1"
        );
    }

    #[test]
    fn metacharacters_are_escaped_before_whitespace_becomes_a_dot() {
        // A literal `.` in the bank's name must stay literal, or the mapping
        // silently matches accounts it was never meant to.
        assert_eq!(
            conf_argument("A.B C", "x", false).unwrap(),
            "/^A\\.B.C($|:)/=x\\1"
        );
        // Every space becomes its OWN dot: `.` matches exactly one character.
        assert_eq!(
            conf_argument("a  b", "x", false).unwrap(),
            "/^a..b($|:)/=x\\1"
        );
    }

    #[test]
    fn a_regex_pattern_with_spaces_only_has_its_spaces_substituted() {
        assert_eq!(
            conf_argument("^PW Roth (.+)$", "assets:\\1", true).unwrap(),
            "/^PW.Roth.(.+)$/=assets:\\1"
        );
    }

    #[test]
    fn every_inexpressible_alias_is_refused_by_name() {
        for (pattern, replacement, regex, expected) in [
            ("a", "b#c", false, ConfRefusal::Comment),
            ("a b", "c d", false, ConfRefusal::ReplacementWhitespace),
            ("a b", "c\\1", false, ConfRefusal::ReplacementBackslash),
            ("a [xy] b", "c", true, ConfRefusal::PatternBracket),
            ("a \\d b", "c", true, ConfRefusal::PatternBackslash),
            ("a/b c", "d", false, ConfRefusal::PatternSlash),
        ] {
            assert_eq!(
                conf_argument(pattern, replacement, regex),
                Err(expected),
                "{pattern:?} → {replacement:?}"
            );
        }
    }

    #[test]
    fn a_new_option_lands_in_the_general_section_not_at_eof() {
        // The trap this function exists for: appended at EOF, the option would
        // be inside `[balance]` and would never reach an import.
        let text = "--depth 3\n\n[balance]\n--no-total\n";
        assert_eq!(
            with_aliases(text, &["a=b".to_string()]),
            "--depth 3\n--alias=a=b\n\n[balance]\n--no-total\n"
        );
    }

    #[test]
    fn adding_the_same_alias_twice_adds_it_once() {
        let text = "--alias=a=b\n";
        assert_eq!(with_aliases(text, &["a=b".to_string()]), text);
    }

    #[test]
    fn a_file_with_no_final_newline_gets_one_before_the_new_option() {
        assert_eq!(
            with_aliases("--depth 3", &["a=b".to_string()]),
            "--depth 3\n--alias=a=b\n"
        );
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        assert_eq!(
            with_aliases("--depth 3\r\n", &["a=b".to_string()]),
            "--depth 3\r\n--alias=a=b\r\n"
        );
    }

    #[test]
    fn an_empty_file_is_written_without_a_leading_blank_line() {
        assert_eq!(with_aliases("", &["a=b".to_string()]), "--alias=a=b\n");
    }

    // -----------------------------------------------------------------------
    // The full lookup: upward walk, then ~/.hledger.conf, then XDG
    // -----------------------------------------------------------------------

    /// A fresh directory under the OS temp dir, removed on drop. Never the real
    /// `$HOME`: every user-level path these tests use is inside one of these.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ledgeline-conf-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_lookup_runs_ledger_then_home_then_xdg_and_uses_one_file() {
        let scratch = Scratch::new("tiers");
        let books = scratch.0.join("books");
        let home = scratch.0.join("home");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::create_dir_all(home.join(".config/hledger")).unwrap();
        let user = UserConfDirs {
            home: Some(home.clone()),
            xdg_config_home: None,
        };
        let vault = LocalVault::new(&books);
        let root = VaultPath::root();
        assert_eq!(locate_conf_for(&vault, &root, &user), None);

        // Tier 3: the XDG default under ~/.config.
        std::fs::write(home.join(".config/hledger").join(CONF_NAME), "--auto\n").unwrap();
        let found = locate_conf_for(&vault, &root, &user).unwrap();
        assert_eq!(found.origin, ConfOrigin::Xdg { custom: false });
        assert_eq!(found.handle(), "~/.config/hledger/hledger.conf");
        assert_eq!(found.read(&vault).unwrap(), "--auto\n");

        // Tier 2 beats it.
        std::fs::write(home.join(".hledger.conf"), "--depth 2\n").unwrap();
        let found = locate_conf_for(&vault, &root, &user).unwrap();
        assert_eq!(found.origin, ConfOrigin::Home);
        assert_eq!(found.handle(), "~/.hledger.conf");

        // Tier 1 — beside the journal — beats both.
        std::fs::write(books.join(CONF_NAME), "").unwrap();
        let found = locate_conf_for(&vault, &root, &user).unwrap();
        assert_eq!(found.origin, ConfOrigin::Ledger { levels: 0 });
        assert_eq!(found.handle(), "hledger.conf");
        assert_eq!(
            found.source,
            ConfSource::InVault(VaultPath::new(CONF_NAME).unwrap())
        );
    }

    #[test]
    fn a_custom_xdg_dir_is_named_by_its_variable_and_never_by_its_path() {
        let scratch = Scratch::new("xdg");
        let books = scratch.0.join("books");
        let xdg = scratch.0.join("xdg");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::create_dir_all(xdg.join("hledger")).unwrap();
        std::fs::write(xdg.join("hledger").join(CONF_NAME), "").unwrap();
        let user = UserConfDirs {
            home: Some(scratch.0.join("home")),
            xdg_config_home: Some(xdg),
        };
        let found = locate_conf_for(&LocalVault::new(&books), &VaultPath::root(), &user).unwrap();
        assert_eq!(found.handle(), "$XDG_CONFIG_HOME/hledger/hledger.conf");
        assert!(!found.handle().contains(&*scratch.0.to_string_lossy()));
    }

    #[test]
    fn a_vault_without_a_local_path_never_reads_the_user_files() {
        let scratch = Scratch::new("sandboxed");
        std::fs::write(scratch.0.join(".hledger.conf"), "--auto\n").unwrap();
        let user = UserConfDirs {
            home: Some(scratch.0.clone()),
            xdg_config_home: None,
        };
        let vault = crate::vault::MemVault::new().with_file("main.journal", "");
        assert_eq!(locate_conf_for(&vault, &VaultPath::root(), &user), None);
    }

    #[test]
    fn an_above_root_file_reports_its_levels_as_a_relative_handle() {
        let scratch = Scratch::new("levels");
        let books = scratch.0.join("books");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::write(scratch.0.join(CONF_NAME), "").unwrap();
        let found = locate_conf_for(
            &LocalVault::new(&books),
            &VaultPath::root(),
            &UserConfDirs::default(),
        )
        .unwrap();
        assert_eq!(found.handle(), "../hledger.conf");
    }

    // -----------------------------------------------------------------------
    // Reading settings
    // -----------------------------------------------------------------------

    fn spec(all: Option<u32>, per_regex: &[(&str, u32)]) -> DepthSpec {
        DepthSpec {
            all,
            per_regex: per_regex
                .iter()
                .map(|(regex, depth)| ((*regex).to_string(), *depth))
                .collect(),
        }
    }

    #[test]
    fn auto_counts_only_in_the_general_section() {
        assert!(auto_enabled("--auto\n"));
        assert!(auto_enabled("# rules\n--depth 2 --auto # on\n[balance]\n"));
        assert!(!auto_enabled("[balance]\n--auto\n"));
        assert!(!auto_enabled("# --auto\n"));
        // A hijacked file applies nothing at all.
        assert!(!auto_enabled("print\n--auto\n"));
    }

    #[test]
    fn a_command_section_overrides_the_general_depth_and_regexes_accumulate() {
        // Each case below was run through hledger 1.52 first.
        let settings = ConfSettings::read(
            "--depth 2\n--depth assets:inv=3\n\n[balancesheet]\n--depth 1\n--depth cash=4\n",
        );
        assert_eq!(settings.general, spec(Some(2), &[("assets:inv", 3)]));
        assert_eq!(
            settings.depth(DepthCommand::BalanceSheet),
            spec(Some(1), &[("assets:inv", 3), ("cash", 4)])
        );
        // A command with no section of its own gets the general one.
        assert_eq!(
            settings.depth(DepthCommand::CashFlow),
            spec(Some(2), &[("assets:inv", 3)])
        );
    }

    #[test]
    fn every_spelling_of_a_plain_depth_is_read_and_the_last_wins() {
        for (text, expected) in [
            ("--depth 3\n", 3),
            ("--depth=3\n", 3),
            ("-3\n", 3),
            ("--depth 1 -2\n", 2),
            ("--depth 2\n--depth 1\n", 1),
        ] {
            assert_eq!(
                ConfSettings::read(text).general.all,
                Some(expected),
                "{text:?}"
            );
        }
        assert_eq!(
            ConfSettings::read("--depth inv=3\n--depth inv=4\n").general,
            spec(None, &[("inv", 4)])
        );
    }

    #[test]
    fn an_alias_named_section_is_ignored_and_warned_about() {
        let settings = ConfSettings::read("[bs]\n--depth 1\n[BalanceSheet]\n--depth 2\n[bal]\n");
        assert_eq!(
            settings.depth(DepthCommand::BalanceSheet),
            DepthSpec::default()
        );
        assert_eq!(
            settings.warnings,
            vec![
                ConfWarning::AliasSection {
                    found: "bs".to_string(),
                    command: DepthCommand::BalanceSheet
                },
                ConfWarning::AliasSection {
                    found: "BalanceSheet".to_string(),
                    command: DepthCommand::BalanceSheet
                },
                ConfWarning::AliasSection {
                    found: "bal".to_string(),
                    command: DepthCommand::Balance
                },
            ]
        );
        assert!(settings.warnings[0].message().contains("[balancesheet]"));
    }

    #[test]
    fn only_the_last_heading_of_a_name_is_read() {
        // Verified: `[balance]\n--depth 3\n[balance]\n--no-total` applies no
        // depth to `hledger balance`.
        let settings = ConfSettings::read("[balance]\n--depth 3\n\n[balance]\n--no-total\n");
        assert_eq!(settings.depth(DepthCommand::Balance), DepthSpec::default());
        assert_eq!(
            settings.warnings,
            vec![ConfWarning::DuplicateSection {
                command: DepthCommand::Balance
            }]
        );
    }

    #[test]
    fn an_unreadable_depth_is_warned_and_ignored() {
        let settings = ConfSettings::read("--depth (=2\n--depth x\n--depth 2\n");
        assert_eq!(settings.general, spec(Some(2), &[]));
        assert_eq!(settings.warnings.len(), 2);
        assert!(
            settings
                .warnings
                .iter()
                .all(|warning| warning.code() == "badDepth")
        );
    }

    #[test]
    fn options_of_other_kinds_and_their_values_are_not_mistaken_for_depth() {
        let settings = ConfSettings::read("--alias -2=x --begin 2026 --forecast\n");
        assert_eq!(settings, ConfSettings::default());
    }

    // -----------------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------------

    fn general() -> Section {
        Section::General
    }

    fn command(name: &str) -> Section {
        Section::Command(name.to_string())
    }

    #[test]
    fn setting_auto_inserts_into_the_general_section_not_at_eof() {
        assert_eq!(
            with_option(
                "--depth 3\n\n[balance]\n--no-total\n",
                &general(),
                &ManagedOption::Auto
            )
            .unwrap(),
            "--depth 3\n--auto\n\n[balance]\n--no-total\n"
        );
        // No option in it yet: just before the first heading.
        assert_eq!(
            with_option("# notes\n\n[balance]\n", &general(), &ManagedOption::Auto).unwrap(),
            "# notes\n\n--auto\n[balance]\n"
        );
        assert_eq!(
            with_option("", &general(), &ManagedOption::Auto).unwrap(),
            "--auto\n"
        );
    }

    #[test]
    fn a_value_already_in_force_is_a_byte_identical_no_op() {
        for text in [
            "--auto\n",
            "# c\r\n--depth=2   # mine\r\n",
            "[balancesheet]\n-2\n",
        ] {
            let section = if text.contains("[balancesheet]") {
                command("balancesheet")
            } else {
                general()
            };
            let option = if text.contains("auto") {
                ManagedOption::Auto
            } else {
                ManagedOption::Depth(2)
            };
            assert_eq!(with_option(text, &section, &option).unwrap(), text);
        }
        assert_eq!(
            without_option("--depth 2\n", &general(), &ConfSlot::Auto).unwrap(),
            "--depth 2\n"
        );
    }

    #[test]
    fn an_existing_option_is_replaced_in_place_and_nothing_else_moves() {
        let text = "# my settings\r\n--forecast --depth=4 # deep\r\n--unknown thing\r\n";
        assert_eq!(
            with_option(text, &general(), &ManagedOption::Depth(2)).unwrap(),
            "# my settings\r\n--forecast --depth 2 # deep\r\n--unknown thing\r\n"
        );
        // `-N` and the two-token form are replaced too; the LAST is the one in force.
        assert_eq!(
            with_option("-1\n--depth 5\n", &general(), &ManagedOption::Depth(2)).unwrap(),
            "-1\n--depth 2\n"
        );
        // A regex slot is matched by its regex, not by its value.
        assert_eq!(
            with_option(
                "--depth cash=1 --depth inv=2\n",
                &general(),
                &ManagedOption::DepthFor {
                    regex: "inv".into(),
                    depth: 3
                }
            )
            .unwrap(),
            "--depth cash=1 --depth inv=3\n"
        );
    }

    #[test]
    fn a_missing_command_section_is_created_at_eof_under_its_full_name() {
        assert_eq!(
            with_option(
                "--auto\n",
                &command("balancesheet"),
                &ManagedOption::Depth(2)
            )
            .unwrap(),
            "--auto\n\n[balancesheet]\n--depth 2\n"
        );
        assert_eq!(
            with_option("--auto", &command("cashflow"), &ManagedOption::Depth(2)).unwrap(),
            "--auto\n\n[cashflow]\n--depth 2\n"
        );
        assert_eq!(
            with_option(
                "--auto\r\n\r\n",
                &command("cashflow"),
                &ManagedOption::Depth(2)
            )
            .unwrap(),
            "--auto\r\n\r\n[cashflow]\r\n--depth 2\r\n"
        );
        assert_eq!(
            with_option("", &command("balance"), &ManagedOption::Depth(2)).unwrap(),
            "[balance]\n--depth 2\n"
        );
    }

    #[test]
    fn an_insert_into_an_existing_section_lands_after_its_last_option() {
        let text = "[balance]\n--no-total\n# about register\n\n[register]\n--wide\n";
        assert_eq!(
            with_option(text, &command("balance"), &ManagedOption::Depth(2)).unwrap(),
            "[balance]\n--no-total\n--depth 2\n# about register\n\n[register]\n--wide\n"
        );
        // Into the LAST heading of that name — the one hledger reads.
        assert_eq!(
            with_option(
                "[balance]\n\n[balance]\n",
                &command("balance"),
                &ManagedOption::Depth(2)
            )
            .unwrap(),
            "[balance]\n\n[balance]\n--depth 2\n"
        );
        // An empty section with no newline after its heading.
        assert_eq!(
            with_option("[balance]", &command("balance"), &ManagedOption::Depth(2)).unwrap(),
            "[balance]\n--depth 2\n"
        );
    }

    #[test]
    fn removing_takes_the_whole_line_only_when_nothing_else_is_on_it() {
        assert_eq!(
            without_option(
                "# keep\n--auto # on\n--depth 2\n",
                &general(),
                &ConfSlot::Auto
            )
            .unwrap(),
            "# keep\n--depth 2\n"
        );
        assert_eq!(
            without_option("--auto --depth 2\n", &general(), &ConfSlot::Auto).unwrap(),
            "--depth 2\n"
        );
        assert_eq!(
            without_option("--depth 2 --auto  # c\n", &general(), &ConfSlot::Auto).unwrap(),
            "--depth 2  # c\n"
        );
        // Every occurrence goes, or an earlier one would come into force.
        assert_eq!(
            without_option("-1\n--depth 3 --depth x=1\n", &general(), &ConfSlot::Depth).unwrap(),
            "--depth x=1\n"
        );
        // Only from the section asked about.
        assert_eq!(
            without_option(
                "--auto\n[balance]\n--auto\n",
                &command("balance"),
                &ConfSlot::Auto
            )
            .unwrap(),
            "--auto\n[balance]\n"
        );
        assert_eq!(
            without_option("--auto\r\n--depth 2\r\n", &general(), &ConfSlot::Auto).unwrap(),
            "--depth 2\r\n"
        );
    }

    #[test]
    fn a_hijacked_file_and_inexpressible_values_are_refused() {
        assert_eq!(
            with_option("balance\n", &general(), &ManagedOption::Auto),
            Err(ConfWriteError::Hijacked("balance".into()))
        );
        assert!(matches!(
            without_option("print\n--auto\n", &general(), &ConfSlot::Auto),
            Err(ConfWriteError::Hijacked(_))
        ));
        for regex in ["", "a b", "a#b", "-a", "a=b", "("] {
            assert!(
                matches!(
                    with_option(
                        "",
                        &general(),
                        &ManagedOption::DepthFor {
                            regex: regex.into(),
                            depth: 1
                        }
                    ),
                    Err(ConfWriteError::Inexpressible(_))
                ),
                "{regex:?}"
            );
        }
        assert!(matches!(
            with_option("", &command("Balance"), &ManagedOption::Auto),
            Err(ConfWriteError::BadSection(_))
        ));
    }

    #[test]
    fn every_write_reads_back_as_what_was_written() {
        let mut text = "# hand-written\n--forecast\n[register]\n--wide\n".to_string();
        let edits: [(Section, ManagedOption); 4] = [
            (general(), ManagedOption::Auto),
            (general(), ManagedOption::Depth(3)),
            (command("balancesheet"), ManagedOption::Depth(1)),
            (
                command("balancesheet"),
                ManagedOption::DepthFor {
                    regex: "^assets:inv".into(),
                    depth: 3,
                },
            ),
        ];
        for (section, option) in &edits {
            text = with_option(&text, section, option).unwrap();
        }
        let settings = ConfSettings::read(&text);
        assert!(settings.auto);
        assert_eq!(settings.general, spec(Some(3), &[]));
        assert_eq!(
            settings.written(Some(DepthCommand::BalanceSheet)),
            spec(Some(1), &[("^assets:inv", 3)])
        );
        assert!(text.starts_with("# hand-written\n--forecast\n"));
        assert!(text.contains("[register]\n--wide\n"));
        assert!(settings.warnings.is_empty());
    }
}
