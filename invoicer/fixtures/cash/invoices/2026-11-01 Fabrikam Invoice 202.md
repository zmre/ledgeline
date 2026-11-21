---
title: 2026-11-01 Fabrikam Invoice 202
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 202
date: 2026-11-01
terms: 30
due_date: 2026-12-01
customer: "[[Fabrikam]]"
invoicee_name: Fabrikam
invoicee_email: ap@fabrikam.example
invoicee_address: |
  1 Fabrikam Way
  Springfield, IL 62701
  USA
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.com
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
sent_date: 2026-11-01
sent_via: email
paid: partial 2026-11-12 $500
---

| Description        | Qty | Rate   | Amount    | *Account*                   |
|:-------------------|----:|-------:|----------:|:----------------------------|
| Discovery workshop | 1   | $1,500 | $1,500.00 | revenue:consulting          |
| Implementation     | 2.5 | $200   | $500.00   | revenue:consulting:delivery |
|                    |     | **Total due** | **$2,000.00** |                 |
