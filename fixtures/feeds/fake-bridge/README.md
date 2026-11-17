# Fake SimpleFIN bridge (plan 43, WP-S)

What `LEDGELINE_FEED_FAKE=<dir>` serves, for e2e and docs screenshots: a
`ledgeline` built with `--features test-fakes` answers every `/accounts`
request from `<dir>/accounts.json` (filtered by the request's dates and
accounts, as the Bridge does) and keeps credentials in memory. Nothing touches
the network or the keychain.

Shaped to `fixtures/sample.journal`, whose last transactions are dated
2026-07-03. Everything here is synthetic.

| Account | Maps to | What it shows |
| --- | --- | --- |
| Lakeside Credit Union · Checking ••4821 | `assets:bank:checking` | 4 new rows (Trader Joe's has a transacted date, a merchant code and an `extra` object), 1 pending row that is never imported; the balance matches once they land |
| Harbor Card Services · Visa Signature ••3310 | `liabilities:cc:visa` | `SAFEWAY` matches the journal's 2026-07-03 Safeway purchase, so it is **held**; 2 new rows; balance matches |
| Summit Online Bank · High-Yield Savings ••0077 | nothing yet | suggested by name as `assets:bank:savings`; link it as a **new** account (`assets:bank:summit-online-bank:high-yield-savings`, opening balance $20,000.00 on 2026-06-30, start 2026-07-01) to show that path — its balance then matches |

`fake-bridge-broken/` is the same bridge with `con.auth` on Harbor (sign in
again at SimpleFIN) and `act.failed` on the savings account.

A setup token the fake accepts is any base64 of a `https://fake.simplefin.test/claim/…`
URL, for example:

    aHR0cHM6Ly9mYWtlLnNpbXBsZWZpbi50ZXN0L2NsYWltL0RFTU8=

which is `https://fake.simplefin.test/claim/DEMO`.
