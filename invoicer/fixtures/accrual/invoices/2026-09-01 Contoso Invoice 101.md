---
title: 2026-09-01 Contoso Invoice 101
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 101
date: 2026-09-01
terms: 30
due_date: 2026-10-01
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
sent_date: 2026-09-01
sent_via: email
paid: 2026-10-05
---

| Description         | Qty | Rate   | Amount    | *Account*          |
|:--------------------|----:|-------:|----------:|:-------------------|
| Architecture review | 5   | $800   | $4,000.00 | revenue:consulting |
|                     |     | **Total due** | **$4,000.00** |  |

Paid in two parts, on 2026-09-20 and 2026-10-05.
