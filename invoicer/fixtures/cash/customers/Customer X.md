---
title: Customer X
type: organization
tags: [customer]
emails:
  - billing: accounts_receivable@customerx.com
  - work: hello@customerx.com
phones:
  - work: "+1 (303) 555-0100"
  - billing: "+1 (303) 555-0142"
addresses:
  - billing: |
      1234 Main St. #555
      Springfield, CO 80000
terms: 30
revenue_account: revenue:consulting
invoice_template: /invoices/tpl/myco-default.html
ledger_customer: Customer X         # the customer: tag value in the books
contracts:
  - "[[Customer X MSA 2026]]"
  - "/customers/contracts/Customer X SOW 3 renewal (signed).pdf"
open_pos:
  - reference: "PO #PY75334"
    amount: $40,000
    expires: 2027-06-30
billing_portal:
  name: Coupa
  url: https://customerx.coupahost.com/
  login: finance@mycompany.com
  supplier_id: SUP-44821
  requires_reference: true
  instructions: |
    Upload the PDF under Invoices → Create from PO, then attach the timesheet.
snippets: ["[[Net 30 ACH]]"]
---

Fred Smith is the champion.
