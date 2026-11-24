---
title: 2026-11-10 Customer X Invoice 105
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 105
date: 2026-11-10
terms: 30
due_date: 2026-12-10
reference: "PO #PY75334, rev. 2"
customer: "[[Customer X]]"
invoicee_name: Customer X
invoicee_email: accounts_receivable@customerx.com
invoicee_phone: +1 (303) 555-0142
invoicee_address: |
  1234 Main St. #555
  Springfield, CO 80000
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_phone: +1 (555) 010-2000
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
sent_date: 2026-11-10
sent_via: portal
paid: no
---

Hi Fred, this is the work we discussed, less the deposit you already paid.

| Description                  | Qty | Rate          | Amount       | *Account*                     |
|:-----------------------------|----:|--------------:|-------------:|:------------------------------|
| Senior architect hours       | 10  | $800          | $8,000       | revenue:consulting            |
| Friends and family discount  |     | -10%          | -$800        |                               |
| Junior engineer              | 50  | $200          | $10,000      | revenue:consulting:delivery   |
| Friends and family discount  |     | -5%           | -$500        |                               |
|                              |     | **Subtotal**  | **$16,700**  |                               |
| Less deposit (Invoice 104)   |     |               | -$8,350      | liabilities:customer-deposits |
|                              |     | **Total due** | **$8,350**   |                               |

## Payment

Pay by ACH to account 123456789, routing 987654321, within 30 days.

## Timesheet

| Date       | Who    | Hours |
|:-----------|:-------|------:|
| 2026-11-03 | Senior | 10    |
| 2026-11-04 | Junior | 50    |
