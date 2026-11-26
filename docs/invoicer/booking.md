---
title: Booking
description: "How Ledgeline Invoicer books invoices into your hledger journal: drafts stay out, sending books, payments, voids, deposits, editing after sending, and the paid status, in accrual and cash books."
order: 20
tags: [invoicer, accrual]
---

# Booking

An invoice file says what you billed. The journal says what happened to the money. This page explains how the two connect: what the invoicer writes into your journal, when, and how it reads payments back.

The invoicer writes only ordinary hledger transactions, through the same code Ledgeline uses to edit your journal. If Ledgeline has the books open and saves at the same moment, neither app overwrites the other's change.

## Accrual or cash

What gets booked depends on whether your books are kept on an accrual or a cash basis. The invoicer treats your books as accrual books exactly when Ledgeline shows its [recognition helpers](../journal/accrual-recognition.md#when-the-helpers-appear) for them:

- `basis = "cash"` or `basis = "accrual"` under `[books]` in your [`ledgeline.toml`](../settings/settings-files.md#books) settles it;
- without that setting, books with a receivable, payable, deferred revenue or prepaid account are accrual books, and others are cash books.

Both work fully. Keep whichever basis your books already use; the invoicer follows it.

## What happens when

| Step                    | Accrual books                                                                                    | Cash books                                                                                                     |
|-------------------------|--------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------|
| Draft written or edited | Nothing.                                                                                         | Nothing.                                                                                                       |
| Send                    | One transaction on the invoice date: the total to accounts receivable, each row to its account.  | Nothing in the journal. The invoice is tracked from its file.                                                  |
| Payment arrives         | You import or sync it in Ledgeline and link it to the invoice there.                             | You import or sync it in Ledgeline, then apply it to the invoice, which splits it into the invoice's accounts. |
| Edit after sending      | The transaction is replaced to match, after you confirm.                                         | Only the file changes.                                                                                         |
| Void                    | A reversing transaction on the void date, linked to the original so the receivable nets to zero. | The file is marked void and its `paid:` line removed.                                                          |

### Drafts never touch the books

A draft is only a file. You can write, change and delete drafts as you like, and your reports never see them. The dashboard still counts them, so you can see what you are about to bill.

## Sending

**Send…** on an invoice, or `ledgeline-invoicer send`, does two things in order. It books the invoice, then marks the file sent with today's date and, if you say, how you delivered it (email, portal, mail).

The invoicer refuses to send an invoice dated on or before the date your books are closed through. Ledgeline sets that date with **Close books through…**; see [Closed periods](../journal/accrual-recognition.md#closed-periods). It also refuses an invoice for a customer whose portal requires a reference, when the invoice has none.

### What an accrual booking looks like

Sending invoice 105 from the [example](files.md#an-invoice) writes this:

```journal
2026-11-10 Customer X | Invoice 105  ; invoice: 105, document: invoices/2026-11-10 Customer X Invoice 105.md, due: 2026-12-10, reference: PO #PY75334 rev. 2
    assets:accounts receivable          $8,350.00  ; customer: Customer X
    revenue:consulting                 $-7,200.00  ; customer: Customer X, item: Senior architect hours
    revenue:consulting:delivery        $-9,500.00  ; customer: Customer X, item: Junior engineer
    liabilities:customer-deposits       $8,350.00  ; customer: Customer X, item: Less deposit (Invoice 104)
```

- Each account gets one posting. The architect hours' 10% discount is netted into its row's account: $8,000 less $800 is $7,200.
- A discount on a subtotal is shared over the item rows above it, in proportion to their amounts.
- The transaction goes in date order, so a journal split into one file per year gets it in the right file.

The tags are how everything else finds the invoice:

| Tag          | Where                            | What it is for                                                                             |
|--------------|----------------------------------|--------------------------------------------------------------------------------------------|
| `invoice:`   | the transaction                  | The invoice number. Ledgeline's invoice matching uses it to pair the payment.              |
| `document:`  | the transaction                  | The invoice file's path, so the books point back at it.                                    |
| `due:`       | the transaction                  | The due date, for aging reports.                                                           |
| `reference:` | the transaction                  | The customer's reference, such as a purchase order.                                        |
| `customer:`  | every posting                    | The customer's name in the books, so revenue by customer works in Ledgeline and `hledger`. |
| `item:`      | every posting but the receivable | The rows the posting came from.                                                            |

Tag values can't hold commas. The invoicer drops a comma inside a number (`$16,700` becomes `$16700`) and turns any other comma into a space. That is why the reference above reads `PO #PY75334 rev. 2`.

A row booked to a deferral also gets its recognition schedule, which Ledgeline posts month by month. See [Revenue and expense recognition](../journal/accrual-recognition.md).

## Getting paid

### Accrual books

You record the payment the way you already do: import your bank statement or sync your bank in Ledgeline. Then link the payment to the invoice with Ledgeline's [invoice matching](../journal/invoice-matching.md), which ranks the invoice first because the `invoice:` tag and customer match.

The invoicer never writes the payment itself. That would duplicate the transaction your bank import brings in.

### Cash books

In cash books nothing was booked at send, so the payment is where the revenue lands. After the deposit is in your journal, apply it to the invoice:

```sh
ledgeline-invoicer apply-payment 207              # list the deposits that might pay it
ledgeline-invoicer apply-payment 207 --txn REF    # apply one, by the reference the list shows
ledgeline-invoicer apply-payment 207 --undo       # put the deposit back as it was
```

Applying replaces the deposit's income side with the invoice's accounts, each tagged with the customer, and tags the transaction with the invoice number. A $3,000 payment that arrived as a $2,910 payout and a $90 card fee becomes:

```journal
2026-11-03 * STRIPE PAYOUT CONTOSO  ; id: st-1, invoice:201
    assets:bank:checking       $2,910.00
    expenses:payment fees         $90.00
    revenue:consulting           $-1800.00  ; customer: Contoso
    revenue:consulting:delivery  $-1200.00  ; customer: Contoso
```

When one payment doesn't cover the whole invoice, the `payment-allocation` [setting](settings.md) decides how it is split over the rows. The default moves each account toward its row's amount, so the payment that settles the invoice leaves every account exactly at its row. `pro-rata` splits each payment in the rows' proportions instead. Each share is rounded half away from zero, and the largest row takes the cent or two left over, so the shares add up exactly. A discount spread over rows at booking uses the same rule, so both put the odd cent on the same row.

In the app, a sent invoice that isn't paid in full has **Apply payment…** in its Books panel (or press <kbd>a</kbd>). It lists the deposits that might pay it, best match first, and shows how the one you choose would be split and what would still be due before you apply it. To undo an applied payment, use `apply-payment N --undo`.

## The paid status

Each sent invoice shows whether it is paid; a void one has no paid status. The invoicer works that out from the books and writes it into the invoice file, so `grep` and templates can see it:

| Books say    | The invoice says                               |
|--------------|------------------------------------------------|
| Nothing paid | `no`                                           |
| Some paid    | `partial 2026-10-28 $1,500, 2026-11-19 $3,500` |
| Paid in full | the date of the payment that completed it      |

In accrual books, every payment and credit linked to the invoice's receivable counts. In cash books, the payments applied to it count. An invoice with a total of zero is paid on its own date.

The app refreshes this when it opens your books and whenever the journal or an invoice changes on disk, including when you link a payment in Ledgeline. `ledgeline-invoicer reconcile` does it from the command line. Nothing else rewrites the field, and you can't set it by hand with `set`.

## Voiding

**Void…** asks for a date (today unless you choose another) and reverses the booking on that date:

```journal
2026-10-10 Tailspin | Invoice 103 (void)  ; invoice: 103, void: 2026-10-10
    assets:accounts receivable  $-800.00  ; customer: Tailspin, match:in0103
    revenue:consulting          $800.00  ; customer: Tailspin, item: Site visit
```

The reversal is linked to the original, so Ledgeline treats the receivable as settled. The file keeps its rows, is marked void with the date, and loses its `paid:` line. Voiding a draft only marks the file, unless a send stopped partway and the draft is already booked; then it is reversed like a sent invoice.

In cash books, an invoice with a payment applied can't be voided until you undo the payment.

## Editing after sending

You can edit a sent invoice. Saving it asks to re-book: the invoicer replaces the original transaction with one that matches the file. If a payment was already linked, the link stays and the paid status is worked out again.

Rarely, the file saves but the books can't be written. Another program might change the journal at the same moment, for example. The editor then says the invoice was saved but its books weren't updated. Fix the cause and use **Re-book…**, or run `ledgeline-invoicer rebook N`.

Two cases work differently:

- **The invoice is in a closed period.** If its date, or its booking's date, is on or before your closed-through date, re-booking is refused. Void it and write a new invoice instead.
- **The edit can't be booked.** Before anything is saved, the invoicer works out what the edited file would book. If that re-booking would be refused, the file stays as it was and the message says why. That happens if you remove a deferred row whose schedule is running, or give a row no account. Recalculate checks the same way.
- **The file changed outside the app.** If you edit a sent invoice in another editor, the invoicer doesn't re-book it silently. The Books panel in its editor says the books are out of date and shows what re-booking would write, and **Re-book…** does it. `ledgeline-invoicer rebook` does the same, and `ledgeline-invoicer check` lists every invoice in that state.

## Deposits

Billing part of a job up front is a separate **deposit invoice**, not a discount. Start one from **New → Deposit invoice** (or press <kbd>d</kbd>). Its one row books to a deposit account:

| Description                             | Amount    | *Account*                     |
|:----------------------------------------|----------:|:------------------------------|
| Deposit: 50% of proposal P-12 ($16,700) | $8,350.00 | liabilities:customer-deposits |

The final invoice bills the whole job and takes the deposit off with a row booked to the same account: `Less deposit (Invoice 104)` in invoice 105 above. **Apply deposit** in the invoice editor lists the customer's deposits not yet used and writes that row for you, just above the total.

The deposit account is the `deposit-account` [setting](settings.md). Without it, the invoicer uses the account declared `kpi: deferred-revenue`, else `liabilities:deferred-revenue`.

### In accrual books

The deposit invoice books a receivable against the deposit account when you send it. The final invoice's deposit row clears that account. If the deposit covers the whole final invoice, its total is zero and it books no receivable, only the rows against the deposit account.

### In cash books

By default a deposit is income the day it arrives. Applying its payment books it to revenue (the row's account, or the customer's revenue account when the row names the deposit account), and the final invoice's deposit row books nothing.

If you prefer to hold deposits as a liability until the work is billed, set `cash-deposits = "liability-until-final"`. The deposit's payment then goes to the deposit account, and the final invoice's payment clears it.

Voiding a final invoice gives its deposit back, so a reissued invoice can apply it again.

## Checking that the books and the files agree

`ledgeline-invoicer check` compares every invoice with the journal and reports what doesn't line up, each with the command that fixes it when there is one:

| Problem                                              | How bad | Fix                                                     |
|------------------------------------------------------|---------|---------------------------------------------------------|
| An invoice number booked twice                       | error   | Remove the extra transaction.                           |
| A sent invoice with no booking                       | error   | `send N` books it.                                      |
| A booking with no invoice file                       | warning | Restore the file, or leave it if another app booked it. |
| A booking that points at another file                | warning | `rebook N`.                                             |
| A draft that is already booked (a send that stopped) | error   | `send N` finishes it.                                   |
| Void in the file but not reversed in the books       | error   | Set it back to sent by hand, then `void N`.             |
| Reversed in the books but sent in the file           | error   | `void N --date D` finishes it.                          |
| Booked differently from what the file now says       | warning | `rebook N`, or void it in a closed period.              |
| A row whose account can't be worked out              | error   | Fix the row.                                            |
| The paid status disagrees with the books             | warning | `reconcile`.                                            |

Every write to the journal is checked first and tried once more if the journal changed between reading and writing, as it can when Ledgeline saves at the same time.
