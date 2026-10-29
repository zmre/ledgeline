//! Plan 31 C5's guarantees, over a generator of scenario files built from
//! pieces: rules with one to four postings (growth, opening and gains among
//! them), one-offs, `=` rules, notes, section headings, `comment` blocks,
//! disabled versions of each entry, fixed `account` directives, attached
//! comments, comments inside blocks, random gaps, LF or CRLF, with or without
//! a final newline.
//!
//! 1. **Identity.** A save of the file as read, with its layout or without
//!    one, gives back the file byte for byte.
//! 2. **Locality.** Whatever moves or is deleted, every piece that is written
//!    unchanged appears verbatim in the output.
//! 3. **Round trip.** The written file reads back as the layout asked for
//!    (each write runs `confirm_round_trip`; the tests also re-read).
//! 4. **Toggle involution.** Disabling then enabling an entry restores the
//!    file's bytes.
//! 5. **hledger blindness.** The file's active scenario equals the scenario
//!    of the same file with its disabled blocks deleted.
//!
//! Plus: a permutation of the non-fixed items reads back in that order;
//! deleting items; editing one amount changes only that piece's bytes; and
//! disabling an entry projects exactly as deleting it.

use std::collections::BTreeMap;

use ledgeline_core::Journal;
use ledgeline_core::auto_edit::block_spans;
use ledgeline_core::decimal::Dec;
use ledgeline_core::parse_journal;
use ledgeline_core::periodic::PeriodicDoc;
use ledgeline_core::projections::serialize::{ProjectionDoc, scenario_from_text, write_scenario};
use ledgeline_core::projections::{
    DEFAULT_GAINS_ACCOUNT, ItemKind, JournalFacts, ProjectionOpts, RealSide, Scenario, project,
    project_cached,
};
use ledgeline_core::reports::Interval;
use ledgeline_core::reports::account_types::AccountType;
use proptest::prelude::*;

const NAME: &str = "projection-gen.journal";

fn declared() -> BTreeMap<String, AccountType> {
    BTreeMap::new()
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// One posting of a generated rule.
#[derive(Debug, Clone)]
enum GenPosting {
    /// An expense flow, maybe growing, maybe with a comment of its own.
    Flow {
        cents: i64,
        growth: bool,
        comment: bool,
    },
    /// An asset row: growth, an opening override, maybe a gains account.
    Asset { cents: i64, gains: bool },
}

/// What one generated piece is.
#[derive(Debug, Clone)]
enum Shape {
    Rule {
        postings: Vec<GenPosting>,
        header_comment: bool,
        from: bool,
    },
    Event {
        month: u8,
        cents: i64,
    },
    Auto,
    Note {
        lines: u8,
    },
    Section,
    Block,
    Fixed,
}

impl Shape {
    fn is_entry(&self) -> bool {
        matches!(self, Shape::Rule { .. } | Shape::Event { .. } | Shape::Auto)
    }

    fn is_free_comment(&self) -> bool {
        matches!(self, Shape::Note { .. } | Shape::Section | Shape::Block)
    }
}

/// One generated piece and what follows it.
#[derive(Debug, Clone)]
struct GenPiece {
    shape: Shape,
    /// Commented out (entries only).
    disabled: bool,
    /// A comment directly above it (entries only).
    attached: bool,
    /// Blank lines after it.
    gap: u8,
}

/// A whole generated file.
#[derive(Debug, Clone)]
struct GenFile {
    pieces: Vec<GenPiece>,
    crlf: bool,
    final_newline: bool,
}

fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    format!("${sign}{}.{:02}", cents / 100, cents % 100)
}

impl GenPiece {
    /// The piece's lines, with `n` making every name in it unique.
    fn lines(&self, n: usize) -> Vec<String> {
        let mut content: Vec<String> = match &self.shape {
            Shape::Rule {
                postings,
                header_comment,
                from,
            } => {
                let mut lines = vec![format!(
                    "~ monthly{}  rule {n}{}",
                    if *from { " from 2027-01-01" } else { "" },
                    if *header_comment {
                        format!("  ; about rule {n}")
                    } else {
                        String::new()
                    }
                )];
                for (j, posting) in postings.iter().enumerate() {
                    lines.push(match posting {
                        GenPosting::Flow {
                            cents,
                            growth,
                            comment,
                        } => {
                            let mut tags = Vec::new();
                            if *growth {
                                tags.push("growth: 3%/yr".to_string());
                            }
                            if *comment {
                                tags.push(format!("posting {j} of rule {n}"));
                            }
                            let comment = if tags.is_empty() {
                                String::new()
                            } else {
                                format!("  ; {}", tags.join(", "))
                            };
                            format!("    expenses:g{n}:p{j}    {}{comment}", money(*cents))
                        }
                        GenPosting::Asset { cents, gains } => format!(
                            "    assets:invest:g{n}p{j}    {}  ; growth: 5%/yr, opening: 1000.00{}",
                            money(*cents),
                            if *gains { ", gains: equity:gains" } else { "" }
                        ),
                    });
                }
                lines.push("    assets:checking".to_string());
                lines
            }
            Shape::Event { month, cents } => vec![
                format!("~ 2027-{month:02}-15  event {n}"),
                format!("    expenses:e{n}    {}", money(*cents)),
                "    assets:checking".to_string(),
            ],
            Shape::Auto => vec![
                format!("= expenses:g{n}"),
                format!("    (liabilities:tax{n})  *0.05"),
            ],
            Shape::Note { lines } => (0..*lines)
                .map(|k| format!("; note {n}, line {k}"))
                .collect(),
            Shape::Section => vec![";".repeat(22), format!(";; Section {n}")],
            Shape::Block => vec![
                "comment".to_string(),
                format!("block {n} ~ monthly"),
                "end comment".to_string(),
            ],
            Shape::Fixed => vec![format!("account expenses:fixed{n}  ; type: X")],
        };
        if self.disabled {
            content = content
                .into_iter()
                .map(|line| format!("; {line}"))
                .collect();
        }
        if self.attached {
            content.insert(0, format!("; about entry {n}"));
        }
        content
    }
}

impl GenFile {
    /// The file's text; `keep` decides which pieces are written at all.
    fn render_with(&self, keep: &dyn Fn(&GenPiece) -> bool) -> String {
        let mut lines = vec![
            "; Ledgeline projection".to_string(),
            "; projection: Generated".to_string(),
            "; created: 2026-01-01".to_string(),
            "; updated: 2026-01-01".to_string(),
            String::new(),
            // The gains accounts every asset row may use, declared, so a save
            // has nothing to declare under the header.
            "account equity:gains  ; type: E".to_string(),
            format!("account {DEFAULT_GAINS_ACCOUNT}  ; type: E"),
            String::new(),
        ];
        for (n, piece) in self.pieces.iter().enumerate() {
            if !keep(piece) {
                continue;
            }
            lines.extend(piece.lines(n));
            lines.extend((0..piece.gap).map(|_| String::new()));
        }
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut text = lines.join(nl);
        text.push_str(nl);
        let ends_bare = self.pieces.last().is_some_and(|piece| piece.gap == 0);
        if !self.final_newline && ends_bare {
            text.truncate(text.len() - nl.len());
        }
        text
    }

    fn render(&self) -> String {
        self.render_with(&|_| true)
    }
}

fn posting() -> impl Strategy<Value = GenPosting> {
    prop_oneof![
        3 => (1i64..2_000_000, any::<bool>(), any::<bool>()).prop_map(|(cents, growth, comment)| {
            GenPosting::Flow { cents, growth, comment }
        }),
        1 => (0i64..100_000, any::<bool>()).prop_map(|(cents, gains)| GenPosting::Asset { cents, gains }),
    ]
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        4 => (prop::collection::vec(posting(), 1..=4), any::<bool>(), any::<bool>()).prop_map(
            |(postings, header_comment, from)| Shape::Rule { postings, header_comment, from }
        ),
        2 => (1u8..=12, 1i64..5_000_000).prop_map(|(month, cents)| Shape::Event { month, cents }),
        2 => Just(Shape::Auto),
        2 => (1u8..=2).prop_map(|lines| Shape::Note { lines }),
        1 => Just(Shape::Section),
        1 => Just(Shape::Block),
        1 => Just(Shape::Fixed),
    ]
}

fn piece() -> impl Strategy<Value = GenPiece> {
    (shape(), any::<bool>(), any::<bool>(), 0u8..=2).prop_map(|(shape, disabled, attached, gap)| {
        let entry = shape.is_entry();
        // Never no gap after a free comment: it would attach to what follows.
        let gap = if shape.is_free_comment() {
            gap.max(1)
        } else {
            gap
        };
        GenPiece {
            shape,
            disabled: entry && disabled,
            attached: entry && attached,
            gap,
        }
    })
}

fn file() -> impl Strategy<Value = GenFile> {
    (
        prop::collection::vec(piece(), 1..10),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(pieces, crlf, final_newline)| GenFile {
            pieces,
            crlf,
            final_newline,
        })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read(text: &str) -> Scenario {
    scenario_from_text(text, NAME, "Generated", &declared())
        .unwrap_or_else(|error| panic!("{error}:\n{text}"))
}

fn doc(text: &str) -> ProjectionDoc {
    ProjectionDoc::parse(text, NAME, &declared()).unwrap_or_else(|error| panic!("{error}:\n{text}"))
}

fn save(text: &str, scenario: &Scenario) -> String {
    write_scenario(&doc(text), scenario, NAME)
        .unwrap_or_else(|error| panic!("{error}\n--- before ---\n{text}"))
}

/// The scenario without the entry layout item `at` names.
fn without(scenario: &Scenario, at: usize) -> Scenario {
    let item = &scenario.layout[at];
    let mut out = scenario.clone();
    out.layout.remove(at);
    match item.kind {
        ItemKind::Rule => out.lines.retain(|line| line.group != item.key),
        ItemKind::Event => out.events.retain(|event| event.id != item.key),
        ItemKind::Auto => out.auto_rules.retain(|rule| rule.key != item.key),
        ItemKind::Comment => out.comments.retain(|comment| comment.key != item.key),
        ItemKind::Fixed => out.fixed.retain(|fixed| fixed.key != item.key),
    }
    out
}

/// What each layout item states, independent of keys and origins: its kind,
/// its disabled flag, its attached comment, and the text its piece has once
/// written (for an entry, the canonical text of what it states).
fn stated(scenario: &Scenario) -> Vec<String> {
    scenario
        .layout
        .iter()
        .map(|item| {
            let content = match item.kind {
                ItemKind::Rule => format!(
                    "{:?}",
                    scenario
                        .lines
                        .iter()
                        .filter(|line| line.group == item.key)
                        .map(|line| (
                            &line.account,
                            line.amount.quantity,
                            &line.period.raw,
                            &line.growth,
                            line.opening.as_ref().map(|opening| opening.quantity),
                            &line.counter,
                            &line.gains,
                            &line.note,
                            &line.comment,
                            &line.rule_comment,
                        ))
                        .collect::<Vec<_>>()
                ),
                ItemKind::Event => format!(
                    "{:?}",
                    scenario
                        .events
                        .iter()
                        .find(|event| event.id == item.key)
                        .map(|event| (
                            &event.date,
                            &event.description,
                            &event.postings,
                            &event.counter,
                            &event.comment,
                            &event.posting_comments
                        ))
                ),
                ItemKind::Auto => scenario
                    .auto_rules
                    .iter()
                    .find(|rule| rule.key == item.key)
                    .map(|rule| rule.rule.raw.clone())
                    .unwrap_or_default(),
                ItemKind::Comment => scenario
                    .comments
                    .iter()
                    .find(|comment| comment.key == item.key)
                    .map(|comment| format!("{:?} {}", comment.style, comment.raw))
                    .unwrap_or_default(),
                ItemKind::Fixed => scenario
                    .fixed
                    .iter()
                    .find(|fixed| fixed.key == item.key)
                    .map(|fixed| fixed.raw.clone())
                    .unwrap_or_default(),
            };
            format!(
                "{:?} {} {:?} {content}",
                item.kind, item.disabled, item.attached
            )
        })
        .collect()
}

/// A deterministic shuffle of `items` driven by `seed`.
fn shuffled<T>(mut items: Vec<T>, seed: u64) -> Vec<T> {
    let mut state = seed | 1;
    for at in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let other = usize::try_from(state % (at as u64 + 1)).unwrap_or(0);
        items.swap(at, other);
    }
    items
}

/// `scenario` with its non-fixed layout items permuted by `seed`, every fixed
/// item kept in its own slot.
fn permuted(scenario: &Scenario, seed: u64) -> Scenario {
    let movable: Vec<_> = scenario
        .layout
        .iter()
        .filter(|item| item.kind != ItemKind::Fixed)
        .cloned()
        .collect();
    let mut moved = shuffled(movable, seed).into_iter();
    let layout = scenario
        .layout
        .iter()
        .map(|item| {
            if item.kind == ItemKind::Fixed {
                item.clone()
            } else {
                moved.next().expect("as many as were taken")
            }
        })
        .collect();
    Scenario {
        layout,
        ..scenario.clone()
    }
}

/// Every piece's exact bytes, attached comment included.
fn piece_bytes(text: &str) -> Vec<String> {
    doc(text)
        .pieces()
        .into_iter()
        .map(|piece| text[piece.span].to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The reader sees the pieces the generator wrote: kind for kind, with
    /// their disabled flags and attached comments. (Fixed text with no gap
    /// before more fixed text is one piece.)
    #[test]
    fn a_generated_file_reads_as_its_pieces(file in file()) {
        let text = file.render();
        let mut want: Vec<(ItemKind, bool, bool)> = vec![(ItemKind::Fixed, false, false)];
        let mut glued = false;
        for piece in &file.pieces {
            let kind = match &piece.shape {
                Shape::Rule { .. } => ItemKind::Rule,
                Shape::Event { .. } => ItemKind::Event,
                Shape::Auto => ItemKind::Auto,
                Shape::Note { .. } | Shape::Section | Shape::Block => ItemKind::Comment,
                Shape::Fixed => ItemKind::Fixed,
            };
            if !(glued && kind == ItemKind::Fixed) {
                want.push((kind, piece.disabled, piece.attached));
            }
            glued = kind == ItemKind::Fixed && piece.gap == 0;
        }
        let got: Vec<(ItemKind, bool, bool)> = read(&text)
            .layout
            .iter()
            .map(|item| (item.kind, item.disabled, !item.attached.is_empty()))
            .collect();
        prop_assert_eq!(got, want, "{}", text);
    }

    /// The extents agree with the parser's: an active `~` piece's content is
    /// `PeriodicDoc`'s block, an `=` piece's is `auto_edit::block_spans`'.
    #[test]
    fn block_extents_are_the_parser_s(file in file()) {
        let text = file.render();
        let pieces = doc(&text).pieces();
        let content = |kinds: &[ItemKind]| -> Vec<std::ops::Range<usize>> {
            pieces
                .iter()
                .filter(|piece| !piece.disabled && kinds.contains(&piece.kind))
                .map(|piece| piece.attached.end..piece.span.end)
                .collect()
        };
        let periodic: Vec<std::ops::Range<usize>> = PeriodicDoc::parse(&text)
            .blocks()
            .iter()
            .map(|block| block.full.clone())
            .collect();
        prop_assert_eq!(content(&[ItemKind::Rule, ItemKind::Event]), periodic);
        let journal = parse_journal(&text, NAME).expect("parses");
        prop_assert_eq!(
            content(&[ItemKind::Auto]),
            block_spans(&text, &journal.auto_rules)
        );
    }

    /// Guarantee 1, with the read layout and with none (the legacy path).
    #[test]
    fn identity(file in file()) {
        let text = file.render();
        let scenario = read(&text);
        prop_assert_eq!(save(&text, &scenario), text.clone());
        let mut legacy = scenario;
        legacy.layout.clear();
        prop_assert_eq!(save(&text, &legacy), text);
    }

    /// Guarantees 2 and 3 under a permutation of the non-fixed items.
    #[test]
    fn a_permutation_reads_back_in_that_order_and_keeps_every_piece(file in file(), seed in any::<u64>()) {
        let text = file.render();
        let scenario = permuted(&read(&text), seed);
        let out = save(&text, &scenario);
        prop_assert_eq!(stated(&read(&out)), stated(&scenario), "{}", out);
        for bytes in piece_bytes(&text) {
            let bytes = bytes.trim_end_matches(['\r', '\n']);
            prop_assert!(out.contains(bytes), "lost {:?} from\n{}", bytes, out);
        }
    }

    /// Deleting items: the rest read back in order, their bytes untouched.
    #[test]
    fn deleting_items_keeps_the_rest(file in file(), seed in any::<u64>(), count in 1usize..4) {
        let text = file.render();
        let mut scenario = read(&text);
        let mut deletable: Vec<usize> = scenario
            .layout
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind != ItemKind::Fixed)
            .map(|(at, _)| at)
            .collect();
        deletable = shuffled(deletable, seed);
        let mut doomed: Vec<usize> = deletable.into_iter().take(count).collect();
        doomed.sort_unstable();
        let pieces = doc(&text).pieces();
        let kept_bytes: Vec<String> = scenario
            .layout
            .iter()
            .enumerate()
            .filter(|(at, _)| !doomed.contains(at))
            .filter_map(|(_, item)| item.origin)
            .map(|origin| text[pieces[origin].span.clone()].to_string())
            .collect();
        for at in doomed.into_iter().rev() {
            scenario = without(&scenario, at);
        }
        let out = save(&text, &scenario);
        prop_assert_eq!(stated(&read(&out)), stated(&scenario), "{}", out);
        for bytes in kept_bytes {
            let bytes = bytes.trim_end_matches(['\r', '\n']);
            prop_assert!(out.contains(bytes), "lost {:?} from\n{}", bytes, out);
        }
    }

    /// Editing one amount changes only that piece's content bytes.
    #[test]
    fn an_edit_changes_only_its_own_piece(file in file(), pick in any::<prop::sample::Index>()) {
        let text = file.render();
        let mut scenario = read(&text);
        prop_assume!(!scenario.lines.is_empty());
        let at = pick.index(scenario.lines.len());
        let group = scenario.lines[at].group.clone();
        let amount = &mut scenario.lines[at].amount;
        amount.quantity = amount.quantity.add(Dec::new(1, 2)).expect("small");
        let origin = scenario
            .layout
            .iter()
            .find(|item| item.kind == ItemKind::Rule && item.key == group)
            .and_then(|item| item.origin)
            .expect("the rule's piece");
        let piece = &doc(&text).pieces()[origin];
        let out = save(&text, &scenario);
        let (before, after) = (&text[..piece.attached.end], &text[piece.span.end..]);
        prop_assert!(out.starts_with(before), "{}", out);
        prop_assert!(out.ends_with(after), "{}", out);
        prop_assert_eq!(stated(&read(&out)), stated(&scenario));
    }

    /// Guarantee 4: off then on (or on then off) restores the bytes.
    #[test]
    fn toggling_twice_restores_the_bytes(file in file(), pick in any::<prop::sample::Index>()) {
        let text = file.render();
        let scenario = read(&text);
        let entries: Vec<usize> = scenario
            .layout
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.kind, ItemKind::Rule | ItemKind::Event | ItemKind::Auto))
            .map(|(at, _)| at)
            .collect();
        prop_assume!(!entries.is_empty());
        let at = entries[pick.index(entries.len())];
        let mut once = scenario.clone();
        once.layout[at].disabled = !once.layout[at].disabled;
        let toggled = save(&text, &once);
        let mut back = read(&toggled);
        prop_assert_eq!(back.layout[at].disabled, once.layout[at].disabled, "{}", toggled);
        back.layout[at].disabled = !back.layout[at].disabled;
        prop_assert_eq!(save(&toggled, &back), text);
    }

    /// Guarantee 5: the active scenario is the file with its disabled blocks
    /// deleted, which is all hledger reads.
    #[test]
    fn disabled_blocks_are_invisible_to_the_active_scenario(file in file()) {
        let with = read(&file.render()).active();
        let without = read(&file.render_with(&|piece| !piece.disabled)).active();
        prop_assert_eq!(&with.lines, &without.lines);
        prop_assert_eq!(&with.events, &without.events);
        let raws = |scenario: &Scenario| -> Vec<(String, String)> {
            scenario
                .auto_rules
                .iter()
                .map(|rule| (rule.key.clone(), rule.rule.raw.clone()))
                .collect()
        };
        prop_assert_eq!(raws(&with), raws(&without));
    }
}

// ---------------------------------------------------------------------------
// Disabling equals deleting, for the numbers
// ---------------------------------------------------------------------------

fn main_journal() -> Journal {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/projections-parity/main.journal");
    let text = std::fs::read_to_string(&path).expect("the parity main journal reads");
    parse_journal(&text, "main.journal").expect("it parses")
}

/// A projection's every figure, `Dec` scales included.
fn exact<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:#?}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// `project(s with item i disabled) == project(s without item i)`, both by
    /// the full scan and from a cached real side.
    #[test]
    fn disabling_an_entry_projects_exactly_as_deleting_it(
        file in file(),
        pick in any::<prop::sample::Index>(),
    ) {
        let journal = main_journal();
        let facts = JournalFacts::new(&journal).expect("facts");
        let scenario = read(&file.render());
        let entries: Vec<usize> = scenario
            .layout
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !item.disabled && matches!(item.kind, ItemKind::Rule | ItemKind::Event | ItemKind::Auto)
            })
            .map(|(at, _)| at)
            .collect();
        prop_assume!(!entries.is_empty());
        let at = entries[pick.index(entries.len())];
        let mut disabled = scenario.clone();
        disabled.layout[at].disabled = true;
        let deleted = without(&scenario, at);

        let today = "2026-10-20";
        let opts = ProjectionOpts {
            today,
            interval: Interval::Monthly,
            count: 15,
            lookback: 2,
            depth: usize::MAX,
            declared: &facts.shape().declared,
            value_in: None,
        };
        let run = |scenario: &Scenario| {
            project(scenario, &journal.transactions, &journal.prices, &opts).expect("projects")
        };
        prop_assert_eq!(exact(&run(&disabled)), exact(&run(&deleted)));

        let key = facts.key(today, Interval::Monthly, 2, None).expect("key");
        let real = RealSide::build(&journal, &facts, &key).expect("real side");
        let cached = |scenario: &Scenario| {
            project_cached(scenario, &real, 15, usize::MAX).expect("projects cached")
        };
        prop_assert_eq!(exact(&cached(&disabled)), exact(&cached(&deleted)));
        prop_assert_eq!(exact(&cached(&disabled)), exact(&run(&disabled)));
    }
}
