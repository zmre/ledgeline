---
title: 2026-09-15 Fabrikam Invoice 102
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 102
date: 2026-09-15
terms: 30
due_date: 2026-10-15
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
sent_date: 2026-09-15
sent_via: email
paid: 2026-10-20
---

Re-issued after the scope change (re-booked).

| Code  | Description        | Qty | Rate   | Amount    | *Account*                   |
|:------|:-------------------|----:|-------:|----------:|:----------------------------|
| WKSHP | Discovery workshop | 1   | $2,000 | $2,000.00 | revenue:consulting          |
| JR    | Implementation     | 6   | $200   | $1,200.00 | revenue:consulting:delivery |
|       |                    |     | **Subtotal** | **$3,200.00** |                   |
|       | Loyalty discount   |     |        | -$300.00  | revenue:discounts           |
|       |                    |     | **Total due** | **$2,900.00** |                  |
