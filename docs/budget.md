# The Budget tab, and the `~` rules behind it

The Budget tab shows two things about the same subject: how you are doing
against your plan (the bars), and what the plan actually says (the goals). The
second half is editable — Ledgeline writes your budget back into your journal as
ordinary hledger `~` periodic rules, which `hledger balance --budget` reads
exactly as it always has.

Design notes and the reasoning behind each decision live in
[`plans/15-budget-editor.md`](../plans/15-budget-editor.md).

## What a budget is, in the file

A budget goal is one posting inside a `~` periodic transaction rule:

```journal
~ monthly  household budget
    (expenses:food)      $400
    (expenses:bus)        $20

~ yearly  annual budget
    (income:interest)  $-1200
```

`~ monthly` says how often the goal recurs. Each indented line is one goal: an
account and an amount. The parentheses make it an *unbalanced virtual* posting,
which is what lets a rule state a food goal without also stating where the money
comes from. This is the form every hledger budget example uses, and it is the
form Ledgeline writes.

Everything in the tab is a reading of these lines. There is no separate budget
database, no sidecar file of Ledgeline's own, and nothing here that hledger
cannot read.

## Picking a category

The category field is a combobox over your chart of accounts. Start typing and it
suggests matches — segment-aware, so `ex:gr` finds `expenses:groceries` and `xpg`
does not have to be in order. Tab completes to the longest shared prefix, the
arrow keys walk the list, Enter picks, and Escape closes the list without
discarding what you were typing.

It opens on typing rather than on focus, deliberately: the field is focused for
you when the dialog opens, and a list opened at that moment would be positioned
against a dialog that is still animating into place.

Any account is allowed. Most goals are expenses, but budgeting an income account
is the same gesture — see § Income.

## Where your goals get written

**Wherever they already are.** Ledgeline finds every `~` rule in every file your
journal includes, and edits a goal in the file it is already written in. It never
moves one.

If you have no goals at all yet, the tab offers to start you off:

> Create **budget.journal**

Taking it does exactly two things, and says so before you do:

1. writes a new `budget.journal` beside your main journal, holding a comment and
   nothing else;
2. appends `include budget.journal` to the **end** of your main journal.

The end, specifically, because that is the one position that provably cannot
change the meaning of anything already in the file — an `include` placed
mid-file changes which directives are in force for everything after it.

Two refusals, both about not surprising you:

- If a `budget.journal` already exists beside your journal, it is **never**
  written over, appended to, or included. Move it aside or include it yourself.
- If your journal already declares `~` rules, no second file is created — you
  already have a home for goals, and splitting them across files is not something
  Ledgeline will do to you unasked.

Once the file exists, new goals go there by default. You are free to move them
anywhere hledger can read them; Ledgeline will find them again.

## Weekly, monthly, quarterly, annual

The tab groups your goals by how often they recur, because that is how you think
about them: "my monthly budget", "what I expect to earn this year". Which `~`
block a goal lives in, and which file that block sits in, are storage details —
shown in each goal's tooltip, not used as the organising idea.

Adding a goal puts it in the existing rule of that period — the first writable
one, in file order, **whatever it is called** — and opens a new rule
(`~ monthly  monthly budget`) only when that period has no rule at all. Your
monthly rule can be called `household budget`; goals still go in it. A rule shown
read-only (§ What Ledgeline will not rewrite) is never joined; a new one is
opened instead. So a journal edited through Ledgeline tends toward one block per
interval, which is legible.

The tab can say which rule it means, because it has the listing in front of it.
The engine, asked for a goal under a period *and* a name — which is what an API
client does, and all the tab can do when it is opening a rule rather than joining
one — joins only a rule matching **both**. `--budget=DESCPAT` filters on the
description, so folding a goal into a rule of the right period but another name
would quietly change which filtered report it turns up in.

Spacing is not part of that identity: `~ monthly   monthly budget` and
`~ monthly  monthly budget` are the same rule, compared with their whitespace
collapsed. Only the comparison is normalised — the header itself is never
rewritten, and a goal joining a rule adds its line and touches nothing else.

One goal per account per rule. Adding a second goal for an account a rule already
budgets is refused, naming the goal to edit instead: hledger would add the two
lines together, so a second one is not another goal but an unreadable way of
writing the first.

hledger's other two intervals, `daily` and `quarterly`, are read and edited
normally; the tab just does not offer *daily* when creating a new rule.

## Recent activity, and why it is subaccount-inclusive

When you set a goal, the last four periods of that account's actual activity
appear above the amount box, plus the period now running, plus their average:

```
expenses:food — monthly

  Apr 2026   May 2026   Jun 2026   Jul 2026   Aug 2026 so far │ Average of 4
   $577       $612       $548       $701       $389           │   $609.50
```

These figures **include subaccounts**: `expenses:food` counts `food:dining` and
`food:groceries` too. That is not a convenience — it is what makes the number
comparable to the goal you are about to set it against, because the budget report
aggregates a parent's goal from its children and shows the parent's inclusive
actual. A reference figure that excluded subaccounts would disagree with the very
bar it exists to inform, quietly, and only for people who have subaccounts.

The period still running is labelled "so far" rather than shown as a whole one.

**The average covers the complete periods only**, and says how many — "Average of
4" above, not of 5. This is the number most people will actually budget from, so
it has to mean something stable: folding in a month that is four days old would
drag it down by however far through the month you happen to be, which changes
every day for reasons that have nothing to do with spending. If no period has
finished yet, no average is shown at all — that is a different fact from an
average of zero, and printing `$0.00` for it would be a confident answer to a
question nobody can answer.

Each commodity is averaged on its own, over the same period count. A month in
which a commodity did not appear still counts toward its mean, because "nothing"
is a real month of spending in it.

## Income, and the sign

hledger records income as negative. Ledgeline does not make you think about that:
you type `1200` for an annual interest goal and the journal gets `$-1200`. The
recent-activity figures and their average are shown the same way up, so the strip
and the box never disagree about which direction the numbers run. The modal says
so, and shows you the exact line before you save:

```
(income:interest)  $-1200
```

The same flip applies in reverse everywhere: an income goal reads back as `1200`
in the box, and its recent-activity figures are shown positive, so the strip and
the box agree about which way round the numbers go.

Whether an account is income is taken from its declared type — `account
income:interest ; type: R`, or hledger's own inference from the name. If you have
an income account under an unusual name, declare its type and the tab will follow.

## What Ledgeline will not rewrite

Some rules are shown read-only, with a sentence saying why. They still count
toward the report; they simply cannot be edited from here. This is deliberate:
a rule Ledgeline cannot promise to rewrite safely is one you should edit
yourself, in your own editor, rather than have it guessed at.

| You will see                                    | Because                                                                                     |
|--------------------------------------------------|---------------------------------------------------------------------------------------------|
| the whole rule locked                            | its period is not one of hledger's five fixed intervals (`~ every 2 weeks`, `~ monthly from …`) |
| the whole rule locked                            | it uses balanced-virtual `[account]` postings, which balance as a second group                |
| the whole rule locked                            | its postings are not all in one commodity                                                    |
| one goal locked                                  | it has no written amount — hledger works it out from the other lines, so it changes when they do |
| one goal locked                                  | its amount carries an `@` cost annotation or a `=` balance assertion                          |

Note the fourth row. In a rule like

```journal
~ monthly  budget
    expenses:food   $400
    assets:checking
```

the `assets:checking` leg has no number of its own — hledger derives it. Editing
the food goal is all it takes; the leg follows on its own, and Ledgeline writes
nothing to it.

## Rules that balance explicitly

If every amount in a rule is written down, changing one would leave the rule
unbalanced — so Ledgeline changes the counter-leg too, by exactly the difference:

```journal
~ monthly  budget
    expenses:food      $400     ← raise this to $450
    expenses:rent     $1500
    assets:checking  $-1900     ← and this becomes $-1950
```

The counter-leg is the one posting, other than the one you changed, whose amount
is signed the opposite way. If there is not exactly one — as when you try to edit
`assets:checking` itself above, where both food and rent are candidates —
Ledgeline refuses rather than picking, and tells you to edit that rule in your
journal. Nothing in the file says whether food or rent should absorb the
difference, and guessing would silently re-point somebody's budget.

## What an edit does to the rest of your file

Nothing. An edit rewrites bytes only inside the amount it names; every other byte
comes out of the file exactly as it went in. Column alignment survives, comments
survive, blank lines survive — not because anything reformats them back, but
because they are never touched. A new goal is padded to line up with the block it
joins.

Before anything is written, Ledgeline re-parses your whole journal with the
edited text in memory and requires that it still reads — and that the goal reads
back as the number you typed. If either check fails, nothing is written.

If the file changed on disk since the tab loaded it (you edited it in vim, or
another window saved), the save is refused with "reload and re-apply" rather than
clobbering the other change.

## Removing a goal

Removing a goal removes its line. Removing a rule's *last* goal removes the whole
rule, header and all — a bare `~` header with no postings is not something
hledger accepts, and not something worth leaving behind.

## Reading the bars

The top half is `hledger balance --budget` in envelope form: one bullet bar per
category, split into Income and Expenses, with the goal marker where your budget
sits and the fill showing what has actually happened.

The bars cover **whole months**. The engine walks month buckets backwards from
the range end, so a range starting mid-month still starts its first bar on the
1st; the tab shows the real span under the heading, and a category's journal link
uses that same span, so the transactions you drill into always add up to the bar
you clicked.

See [`docs/income-statement.md`](income-statement.md) for how account types are
resolved — the same `type:` rules decide which side of the budget an account
lands on.
