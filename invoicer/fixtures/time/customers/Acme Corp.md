---
title: Acme Corp
type: organization
tags: [customer]
emails:
  - billing: ap@acme.example
addresses:
  - billing: |
      1 Acme Plaza
      Springfield, OR 97477
terms: 30
cadence: monthly
time_detail: daily
time_increment: 6
time_rounding: up
time_items:
  - account: clients:acme
    person: sam
    item: JR
---

Billed monthly in six-minute increments, rounded up per row. Sam's time on
Acme bills as a junior engineer (`time_items`).
