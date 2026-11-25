---
title: Scripting recipes
description: "Working with Ledgeline Invoicer's files from the shell: finding unpaid invoices and drafts with grep, revenue by customer and by invoice with hledger, and the command line's JSON with jq."
order: 80
tags: [invoicer]
---

# Scripting recipes

Your invoices are text files and their money is in an hledger journal, so the usual tools work on them. These recipes were run on the sample books in the repository, [`invoicer/fixtures/`](https://github.com/zmre/ledgeline/tree/main/invoicer/fixtures), and the output shown is what they print there.

Run them from your books folder. They assume the default folders and file names; see [Settings](settings.md) if you changed them. The [file format](files.md) is the reference for every field.

## Finding invoices with grep

Invoice fields are one per line, in lowercase, so a line-anchored search is reliable.

**Drafts waiting to be sent:**

```sh
grep -l '^status: draft' invoices/*.md
```

```text
invoices/2026-11-17 Customer X Invoice 108.md
```

**Sent and not paid at all.** Voiding removes an invoice's `paid:` line, but invoices voided by older versions may still read `paid: no`, so look among the sent ones only. File names have spaces, so pass them along with `--null` and `xargs -0`:

```sh
grep -l --null '^status: sent' invoices/*.md | xargs -0 grep -l '^paid: no'
```

```text
invoices/2026-08-01 Wingtip Invoice 106.md
invoices/2026-11-10 Customer X Invoice 105.md
```

**Partly paid, with the payments so far:**

```sh
grep -H '^paid: partial' invoices/*.md
```

```text
invoices/2026-10-15 Contoso Invoice 107.md:paid: partial 2026-11-05 $1,000
```

**Every invoice for one customer:**

```sh
grep -l '^customer: "\[\[Contoso\]\]"' invoices/*.md
```

**Due dates, soonest first:**

```sh
grep -H '^due_date:' invoices/*.md | sort -t: -k3
```

The paid status in the files is refreshed whenever the app is open and your books change, or when you run `ledgeline-invoicer reconcile`. Run that first if the app hasn't been open since the last payment came in.

## Reporting with hledger

Every booked invoice carries an `invoice:` tag and every posting a `customer:` tag (see [Booking](booking.md#what-an-accrual-booking-looks-like)), so hledger can slice by them.

**One invoice's booking**, and its void if it has one:

```sh
hledger print 'tag:invoice=^105$'
```

```journal
2026-11-10 Customer X | Invoice 105  ; invoice: 105, document: invoices/2026-11-10 Customer X Invoice 105.md, due: 2026-12-10, reference: PO #PY75334 rev. 2
    assets:accounts receivable          $8,350.00  ; customer: Customer X
    revenue:consulting                 $-7,200.00  ; customer: Customer X, item: Senior architect hours
    revenue:consulting:delivery        $-9,500.00  ; customer: Customer X, item: Junior engineer
    liabilities:customer-deposits       $8,350.00  ; customer: Customer X, item: Less deposit (Invoice 104)
```

The `^` and `$` make `105` match exactly, not `1050` too.

**Revenue by customer:**

```sh
hledger bal '^revenue' --pivot customer
```

**Revenue by invoice.** A voided invoice nets to zero and drops out.

```sh
hledger bal '^revenue' tag:invoice --pivot invoice
```

```text
          $-4,000.00  101
          $-2,900.00  102
         $-16,700.00  105
          $-1,200.00  106
          $-3,000.00  107
          $-1,500.00  2002
--------------------
         $-29,300.00
```

In cash books this shows the revenue each invoice's applied payments brought in.

**Invoices and their payments through accounts receivable** (accrual books):

```sh
hledger reg 'assets:accounts receivable' tag:invoice
```

## The command line with jq

`ledgeline-invoicer` prints JSON with `--json`, which [jq](https://jqlang.org) can pick apart. See [Command line](cli.md) for every command.

**Overdue invoices, one per line:**

```sh
ledgeline-invoicer list --overdue --json | jq -r '.[] | "\(.number)\t\(.due_date)\t\(.total)"'
```

```text
107	2026-11-14	$3,000.00
106	2026-08-31	$1,200.00
```

**Everything owed, with its aging range.** Unlike `list`, this includes receivables booked without an invoice file:

```sh
ledgeline-invoicer receivables --json | jq -r '.open[] | "\(.number // "-")\t\(.customer)\t\(.outstanding)\t\(.bucket)"'
```

```text
2001	Tailspin	3000.00	91+
106	Wingtip	1200.00	61–90
2002	Wingtip	1500.00	31–60
107	Contoso	2000.00	1–30
2003	Adatum	5000.00	1–30
2004	Fourth Coffee	1800.00	Current
105	Customer X	8350.00	Current
```

Both outputs above were made with `--today 2026-11-17`, the sample books' date.

## Writing invoices from a script

**Draft an invoice** from rows and catalog items, checking it first with `--dry-run`:

```sh
ledgeline-invoicer new --customer Contoso --date 2026-11-20 \
  --row 'Strategy workshop|1|$2,500' \
  --row 'Discount||-10%' \
  --from-catalog WKSHP:2 \
  --dry-run
```

The bill table it prints:

```markdown
| Code  | Description        | Qty | Unit | Rate          | Amount        | *Account*          |
|:------|:-------------------|----:|:-----|--------------:|--------------:|:-------------------|
|       | Strategy workshop  | 1   |      | $2,500        | $2,500.00     |                    |
|       | Discount           |     |      | -10%          | -$250.00      |                    |
| WKSHP | Discovery workshop | 2   | ea   | $2,000        | $4,000.00     | revenue:consulting |
|       |                    |     |      | **Total due** | **$6,250.00** |                    |
```

Drop `--dry-run` to write it. The blank account cells get the [default account](files.md#the-books-column) when the invoice is booked.

**Write the file yourself.** Any markdown file with `type: invoice`, `status: draft` and the fields in [Files and formats](files.md#frontmatter-fields) is a draft the app opens. `ledgeline-invoicer check` tells you what is missing, `recalc` fills in the amounts, and `send` books it.

**Change a field** without touching anything else in the file:

```sh
ledgeline-invoicer set 108 reference="PO #PY75334" sent_via=portal
```

## Checking before you commit

If you keep your books in git, a pre-commit hook can refuse a commit while an invoice and the books disagree. Git runs hooks from the top of the repository, so name the journal from there:

```sh
#!/bin/sh
# .git/hooks/pre-commit
exec ledgeline-invoicer -f books.journal check
```

`check` exits 1 when it finds an error. Add `--strict` to stop on warnings too.
