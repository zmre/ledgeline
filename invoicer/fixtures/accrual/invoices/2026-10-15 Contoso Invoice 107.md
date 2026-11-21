---
title: 2026-10-15 Contoso Invoice 107
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 107
date: 2026-10-15
terms: 30
due_date: 2026-11-14
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
sent_date: 2026-10-15
sent_via: email
paid: partial 2026-11-05 $1,000
---

| Code  | Description        | Qty | Rate   | Amount    | *Account*          |
|:------|:-------------------|----:|-------:|----------:|:-------------------|
| WKSHP | Follow-up workshop | 1.5 | $2,000 | $3,000.00 | revenue:consulting |
|       |                    |     | **Total due** | **$3,000.00** |      |
