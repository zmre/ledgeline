---
title: 2026-10-20 Contoso Invoice 201
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 201
date: 2026-10-20
terms: 30
due_date: 2026-11-19
customer: "[[Contoso]]"
invoicee_name: Contoso
invoicee_email: ap@contoso.example
invoicee_address: |
  1 Contoso Way
  Springfield, IL 62701
  USA
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
sent_date: 2026-10-20
sent_via: email
paid: 2026-11-03
---

| Description         | Qty | Rate   | Amount    | *Account*                   |
|:--------------------|----:|-------:|----------:|:----------------------------|
| Architecture sprint | 1   | $1,800 | $1,800.00 | revenue:consulting          |
| Implementation      | 6   | $200   | $1,200.00 | revenue:consulting:delivery |
|                     |     | **Total due** | **$3,000.00** |                 |

Paid through Stripe: the payout's income side was re-split into these rows.
