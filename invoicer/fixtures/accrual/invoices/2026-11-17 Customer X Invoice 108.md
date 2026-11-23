---
title: 2026-11-17 Customer X Invoice 108
type: invoice
template: /invoices/tpl/myco-default.html
status: draft
invoice_num: 108
date: 2026-11-17
terms: 30
due_date: 2026-12-17
reference: "PO #PY75334"
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
support_tier: gold   # a custom field, kept byte for byte
---

Next month's platform subscription, invoiced ahead.

| Code   | Description                   | Qty | Unit | Rate    | Amount     | *Account*                                                                  |
|:-------|:------------------------------|----:|:-----|--------:|-----------:|:---------------------------------------------------------------------------|
| SAAS-Y | Platform subscription, 1 year | 1   | ea   | $12,000 | $12,000.00 | liabilities:deferred-revenue → revenue:saas, 12 months from 2026-12-01     |
| ARCH   | Senior architect hours        | 2   | h    | $800    | $1,600.00  | revenue:consulting                                                         |
|        |                               |     |      | **Total due** | **$13,600.00** |                                                                 |

## Payment

Pay by ACH to account 123456789, routing 987654321, within 30 days.
