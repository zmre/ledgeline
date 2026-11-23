---
title: 2026-11-16 Customer X Invoice 207
type: invoice
template: /invoices/tpl/myco-default.html
status: sent
invoice_num: 207
date: 2026-11-16
terms: 30
due_date: 2026-12-16
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
sent_date: 2026-11-16
sent_via: portal
paid: no
---

| Code | Description                | Qty | Rate          | Amount         | *Account*                     |
|:-----|:---------------------------|----:|--------------:|---------------:|:------------------------------|
| ARCH | Senior architect hours     | 10  | $800          | $8,000.00      | revenue:consulting            |
| JR   | Junior engineer            | 20  | $200          | $4,000.00      | revenue:consulting:delivery   |
|      |                            |     | **Subtotal**  | **$12,000.00** |                               |
|      | Less deposit (Invoice 206) |     |               | -$5,000.00     | liabilities:customer-deposits |
|      |                            |     | **Total due** | **$7,000.00**  |                               |
