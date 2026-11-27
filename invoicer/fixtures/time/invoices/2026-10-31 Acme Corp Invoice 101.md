---
title: 2026-10-31 Acme Corp Invoice 101
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 101
date: 2026-10-31
terms: 30
due_date: 2026-11-30
customer: "[[Acme Corp]]"
invoicee_name: Acme Corp
invoicee_email: ap@acme.example
invoicee_address: |
  1 Acme Plaza
  Springfield, OR 97477
invoicer: "[[My Company]]"
invoicer_name: My Company
invoicer_email: finance@mycompany.example
invoicer_address: |
  1234 Background Blvd.
  Anywhere, CA 90000
sent_date: 2026-10-31
sent_via: email
paid: no
---

| Code | Description          | Qty | Unit | Rate | Amount    | *Account*   |
|:-----|:---------------------|----:|:-----|-----:|----------:|:------------|
| TAX  | Tax return, Oct 2026 | 5   | h    | $200 | $1,000.00 | revenue:tax |
|      |                      |     |      | **Total due** | **$1,000.00** |             |
