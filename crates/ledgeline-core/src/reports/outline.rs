//! Readable account outlines for statement lines (plan 50 D1–D5).
//!
//! A statement line (an income-statement group, a cash-flow line, a balance
//! sheet group) holds a set of MEMBER accounts. Printed as raw paths they
//! repeat themselves: `expenses:travel:airfare`, `expenses:travel:hotel`, …
//! under a "Travel" heading. This module turns that set into an indented
//! outline a bookkeeper would recognise:
//!
//! - Members are first clipped to the report's depth, so a deep account folds
//!   into its ancestor (hledger's `--depth`).
//! - An ancestor becomes a HEADING only when two or more shown accounts sit
//!   under it. A lone chain collapses into its one account.
//! - The node covering the whole line is never a heading: the line's own
//!   heading already says it, so "Travel" isn't printed under "Travel".
//! - A heading that also has its own postings gets a first child row,
//!   `<label> (other)`, so the children add up.
//! - Labels come from the account's OWN `name:` tag (never inherited: a child
//!   doesn't borrow its parent's name), else the last segment, title-cased
//!   only when it looks code-like (`ground-transport` → `Ground Transport`).
//! - Two siblings with the same label are told apart by their parent's label.
//!
//! The outline is presentation only. It never decides membership and never
//! computes money: each node lists the members whose figures it shows, and the
//! caller sums them with the same window arithmetic as the line total.

use std::collections::{BTreeMap, BTreeSet};

use super::account_groups::humanized_segment;

/// What an outline row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineKind {
    /// A grouping row: label only, no amount of its own.
    Heading,
    /// An account row (possibly a depth-clipped ancestor standing for its
    /// subtree).
    Account,
    /// A heading account's own postings, listed as its first child.
    Other,
}

/// One row of an outline, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    pub kind: OutlineKind,
    /// What to print.
    pub label: String,
    /// The full account path the row stands for (shown on hover).
    pub account: String,
    /// Indent level within the line, 0 = directly under the line heading.
    pub level: usize,
    /// The line's member accounts whose figures this row shows. For a
    /// heading, every member beneath it (what a budget Total row would sum).
    pub members: Vec<String>,
}

/// `account`'s first `depth` segments (`depth` ≥ 1).
pub(super) fn clip(account: &str, depth: usize) -> &str {
    match account.match_indices(':').nth(depth.max(1) - 1) {
        Some((cut, _)) => &account[..cut],
        None => account,
    }
}

/// The D2 label of `account`: its OWN `name:` tag (never an ancestor's), else
/// its last segment, title-cased only when it looks code-like. For a line's
/// own heading, labelled the same way as the rows beneath it.
#[must_use]
pub fn label_for(account: &str, names: &BTreeMap<String, String>) -> String {
    names.get(account).map_or_else(
        || label_segment(account.rsplit(':').next().unwrap_or(account)),
        |name| name.trim().to_string(),
    )
}

/// The outline for one line's `members`, clipped to `depth` (`None` = no
/// clip, `Some(0)` = no rows, as hledger's totals-only). `names` holds the
/// accounts' OWN declared `name:` tags.
#[must_use]
pub fn outline(
    members: &BTreeSet<String>,
    depth: Option<usize>,
    names: &BTreeMap<String, String>,
) -> Vec<OutlineNode> {
    let shown: BTreeMap<&str, Vec<&str>> = match depth {
        Some(0) => BTreeMap::new(),
        _ => members.iter().fold(BTreeMap::new(), |mut acc, member| {
            let name = depth.map_or(member.as_str(), |d| clip(member, d));
            acc.entry(name)
                .or_insert_with(Vec::new)
                .push(member.as_str());
            acc
        }),
    };
    let ctx = Ctx {
        shown: &shown,
        names,
    };
    let accounts: Vec<&str> = shown.keys().copied().collect();
    let roots = match accounts.as_slice() {
        [] => Vec::new(),
        [only] => vec![ctx.leaf(only)],
        _ => {
            // The node spanning the whole line is absorbed by the line's own
            // heading: its children come up a level.
            let top = ctx.spine("", &accounts);
            if top.is_empty() {
                ctx.children("", &accounts)
            } else {
                ctx.node(&top, &accounts).children
            }
        }
    };
    let mut out = Vec::new();
    flatten(&ctx, relabel(&ctx, roots), 0, &mut out);
    out
}

/// A node of the outline tree before flattening.
struct Tree {
    kind: OutlineKind,
    account: String,
    label: String,
    children: Vec<Tree>,
}

struct Ctx<'a> {
    /// Shown (clipped) account → the members folded into it.
    shown: &'a BTreeMap<&'a str, Vec<&'a str>>,
    names: &'a BTreeMap<String, String>,
}

impl Ctx<'_> {
    fn label(&self, account: &str) -> String {
        label_for(account, self.names)
    }

    fn leaf(&self, account: &str) -> Tree {
        Tree {
            kind: OutlineKind::Account,
            account: account.to_string(),
            label: self.label(account),
            children: Vec::new(),
        }
    }

    /// Walk down from `prefix` while the node isn't itself shown and has a
    /// single child branch: the first node that would print as a heading
    /// ("" when the virtual root already has several branches).
    fn spine(&self, prefix: &str, accounts: &[&str]) -> String {
        match branches(prefix, accounts).as_slice() {
            [(branch, _)] if !self.shown.contains_key(prefix) => self.spine(branch, accounts),
            _ => prefix.to_string(),
        }
    }

    /// The subtree at `account`, whose shown accounts are `accounts`.
    fn node(&self, account: &str, accounts: &[&str]) -> Tree {
        match accounts {
            [only] => self.leaf(only),
            _ => {
                let spine = self.spine(account, accounts);
                if spine != account {
                    return self.node(&spine, accounts);
                }
                let label = self.label(account);
                let other = self.shown.contains_key(account).then(|| Tree {
                    kind: OutlineKind::Other,
                    account: account.to_string(),
                    label: format!("{label} (other)"),
                    children: Vec::new(),
                });
                Tree {
                    kind: OutlineKind::Heading,
                    account: account.to_string(),
                    label,
                    children: other
                        .into_iter()
                        .chain(self.children(account, accounts))
                        .collect(),
                }
            }
        }
    }

    fn children(&self, prefix: &str, accounts: &[&str]) -> Vec<Tree> {
        branches(prefix, accounts)
            .into_iter()
            .map(|(branch, within)| self.node(&branch, &within))
            .collect()
    }

    /// Every member at or below `account` (for a heading's `members`).
    fn members_under(&self, account: &str) -> Vec<String> {
        let prefix = format!("{account}:");
        self.shown
            .iter()
            .filter(|(name, _)| **name == account || name.starts_with(&prefix))
            .flat_map(|(_, members)| members.iter().map(|m| (*m).to_string()))
            .collect()
    }
}

/// `accounts` strictly below `prefix` ("" = the virtual root), split by their
/// next segment, in order.
fn branches<'a>(prefix: &str, accounts: &[&'a str]) -> Vec<(String, Vec<&'a str>)> {
    accounts
        .iter()
        .filter_map(|account| {
            let rest = if prefix.is_empty() {
                Some(*account)
            } else {
                account
                    .strip_prefix(prefix)
                    .and_then(|r| r.strip_prefix(':'))
            }?;
            let segment = rest.split(':').next()?;
            let branch = if prefix.is_empty() {
                segment.to_string()
            } else {
                format!("{prefix}:{segment}")
            };
            Some((branch, *account))
        })
        .fold(
            Vec::<(String, Vec<&str>)>::new(),
            |mut acc, (branch, account)| {
                match acc.last_mut() {
                    Some((last, within)) if *last == branch => within.push(account),
                    _ => acc.push((branch, vec![account])),
                }
                acc
            },
        )
}

/// D3: siblings whose labels collide are prefixed with their parent's label.
fn relabel(ctx: &Ctx<'_>, trees: Vec<Tree>) -> Vec<Tree> {
    let counts = trees
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut acc, t| {
            *acc.entry(t.label.clone()).or_default() += 1;
            acc
        });
    trees
        .into_iter()
        .map(|tree| {
            let collides = tree.kind != OutlineKind::Other
                && counts.get(&tree.label).copied().unwrap_or(0) > 1;
            let label = match tree.account.rfind(':') {
                Some(cut) if collides => {
                    format!("{} · {}", ctx.label(&tree.account[..cut]), tree.label)
                }
                _ => tree.label,
            };
            Tree {
                label,
                children: relabel(ctx, tree.children),
                ..tree
            }
        })
        .collect()
}

fn flatten(ctx: &Ctx<'_>, trees: Vec<Tree>, level: usize, out: &mut Vec<OutlineNode>) {
    trees.into_iter().for_each(|tree| {
        let members = match tree.kind {
            OutlineKind::Heading | OutlineKind::Account => ctx.members_under(&tree.account),
            OutlineKind::Other => ctx
                .shown
                .get(tree.account.as_str())
                .map(|m| m.iter().map(|s| (*s).to_string()).collect())
                .unwrap_or_default(),
        };
        out.push(OutlineNode {
            kind: tree.kind,
            label: tree.label,
            account: tree.account,
            level,
            members,
        });
        flatten(ctx, tree.children, level + 1, out);
    });
}

/// One segment as a row label: `ground-transport` → `Ground Transport` when
/// the segment looks code-like (lowercase ASCII with `-`/`_`), else the
/// group-label humanizer (alias table, first letter up-cased), which leaves
/// `4110 Rental Income` exactly as written.
fn label_segment(segment: &str) -> String {
    let code_like = segment.contains(['-', '_'])
        && segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if code_like {
        segment
            .split(['-', '_'])
            .filter(|word| !word.is_empty())
            .map(humanized_segment)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        humanized_segment(segment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(accounts: &[&str]) -> BTreeSet<String> {
        accounts.iter().map(|a| (*a).to_string()).collect()
    }

    fn rows(nodes: &[OutlineNode]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| {
                let marker = match n.kind {
                    OutlineKind::Heading => "#",
                    OutlineKind::Account => "-",
                    OutlineKind::Other => "~",
                };
                format!("{}{marker} {}", "  ".repeat(n.level), n.label)
            })
            .collect()
    }

    fn plain(members: &[&str]) -> Vec<String> {
        rows(&outline(&set(members), None, &BTreeMap::new()))
    }

    #[test]
    fn shared_root_is_absorbed_by_the_line_heading() {
        assert_eq!(
            plain(&[
                "expenses:travel:airfare",
                "expenses:travel:ground-transport"
            ]),
            vec!["- Airfare", "- Ground Transport"]
        );
    }

    #[test]
    fn nested_siblings_get_a_heading() {
        assert_eq!(
            plain(&[
                "expenses:travel:airfare",
                "expenses:travel:lodging:hotels",
                "expenses:travel:lodging:short-term-rentals",
            ]),
            vec![
                "- Airfare",
                "# Lodging",
                "  - Hotels",
                "  - Short Term Rentals"
            ]
        );
    }

    #[test]
    fn lone_chain_collapses_to_its_leaf() {
        assert_eq!(
            plain(&[
                "expenses:people-costs:payroll-processing-fees",
                "expenses:professional-fees:accounting",
                "expenses:professional-fees:legal"
            ]),
            vec![
                "- Payroll Processing Fees",
                "# Professional Fees",
                "  - Accounting",
                "  - Legal"
            ]
        );
    }

    #[test]
    fn single_member_is_one_leaf() {
        assert_eq!(plain(&["expenses:travel:airfare"]), vec!["- Airfare"]);
    }

    #[test]
    fn parent_with_postings_gets_an_other_row() {
        assert_eq!(
            plain(&[
                "revenue:sales",
                "revenue:sales:hybrid-cloud",
                "revenue:services"
            ]),
            vec![
                "# Sales",
                "  ~ Sales (other)",
                "  - Hybrid Cloud",
                "- Services"
            ]
        );
    }

    #[test]
    fn absorbed_root_with_postings_keeps_its_other_row() {
        assert_eq!(
            plain(&["revenue:sales", "revenue:sales:hybrid-cloud"]),
            vec!["~ Sales (other)", "- Hybrid Cloud"]
        );
    }

    #[test]
    fn written_names_are_kept_verbatim() {
        assert_eq!(
            plain(&[
                "4000 Income:Rental Income:4110 Rental Income",
                "4000 Income:Rental Income:4120 RV Rental Income"
            ]),
            vec!["- 4110 Rental Income", "- 4120 RV Rental Income"]
        );
    }

    #[test]
    fn own_name_tag_wins_and_is_not_inherited() {
        let names = BTreeMap::from([(
            "assets:bank:fcb".to_string(),
            "First Community Bank".to_string(),
        )]);
        let got = rows(&outline(
            &set(&[
                "assets:bank:fcb:checking",
                "assets:bank:fcb:savings",
                "assets:bank:ally:savings",
                "assets:bank:ally:checking",
            ]),
            None,
            &names,
        ));
        assert_eq!(
            got,
            vec![
                "# Ally",
                "  - Checking",
                "  - Savings",
                "# First Community Bank",
                "  - Checking",
                "  - Savings"
            ]
        );
    }

    #[test]
    fn colliding_siblings_take_their_parent_label() {
        assert_eq!(
            plain(&["expenses:office:other", "expenses:travel:other"]),
            vec!["- Office · Other", "- Travel · Other"]
        );
    }

    #[test]
    fn depth_folds_leaves_into_ancestors() {
        let members = set(&[
            "expenses:travel:airfare",
            "expenses:travel:lodging:hotels",
            "expenses:travel:lodging:short-term-rentals",
        ]);
        let got = outline(&members, Some(3), &BTreeMap::new());
        assert_eq!(rows(&got), vec!["- Airfare", "- Lodging"]);
        assert_eq!(
            got[1].members,
            vec![
                "expenses:travel:lodging:hotels",
                "expenses:travel:lodging:short-term-rentals"
            ]
        );
        assert!(outline(&members, Some(0), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn heading_members_cover_its_subtree_and_other_only_its_own() {
        let got = outline(
            &set(&[
                "revenue:sales",
                "revenue:sales:hybrid-cloud",
                "revenue:services",
            ]),
            None,
            &BTreeMap::new(),
        );
        assert_eq!(got[0].kind, OutlineKind::Heading);
        assert_eq!(
            got[0].members,
            vec!["revenue:sales", "revenue:sales:hybrid-cloud"]
        );
        assert_eq!(got[1].members, vec!["revenue:sales"]);
        assert_eq!(got[0].account, "revenue:sales");
    }

    #[test]
    fn label_for_reads_the_accounts_own_name_else_its_last_segment() {
        let names = BTreeMap::from([("liabilities:cc".to_string(), " Credit cards ".to_string())]);
        assert_eq!(label_for("liabilities:cc", &names), "Credit cards");
        assert_eq!(label_for("liabilities:cc:visa", &names), "Visa");
        assert_eq!(label_for("assets:petty-cash", &names), "Petty Cash");
    }

    #[test]
    fn several_roots_stay_side_by_side() {
        assert_eq!(
            plain(&["income:grants", "expenses:settlements"]),
            vec!["- Settlements", "- Grants"]
        );
    }
}
