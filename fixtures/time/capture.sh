#!/usr/bin/env bash
# Capture hledger's own figures for the time fixture into expected/.
#
# These are the parity oracle for crates/ledgeline-core/tests/time_fixture.rs.
# They come from hledger (1.52), never from Ledgeline: re-run this script after
# editing any file here, and commit both together. hledger 1.52 rounds each
# unit amount (20m) and each timeclock session to 0.01 h; Ledgeline keeps the
# exact time, so the tests compare to within that rounding.
#
# The commands are the ones the roll-up's header teaches (plan 46 F-files).
set -euo pipefail
cd "$(dirname "$0")"

R=time.journal
mkdir -p expected

# Strict mode passes: `commodity ""` for timedot, `commodity h` for timeclock,
# every account declared (plan 46 fact 10).
hledger -s -f "$R" check

# Where the time went: the account tree with roll-ups.
hledger -f "$R" bal --tree -O csv > expected/bal-tree.csv
# Billable or not, resolved nearest-ancestor (fact 7: --pivot, not tag:).
hledger -f "$R" bal --pivot billable -O csv > expected/bal-pivot-billable.csv
# Per customer and per person (the person: tag only; hledger can't see folders).
hledger -f "$R" bal --pivot customer -O csv > expected/bal-pivot-customer.csv
hledger -f "$R" bal --pivot person -O csv > expected/bal-pivot-person.csv
# What went on invoice 101, in the time book and across books and time.
hledger -f "$R" reg tag:invoice=101 -O csv > expected/reg-invoice-101.csv
hledger -f with-time.journal reg tag:invoice=101 -O csv > expected/reg-invoice-101-with-books.csv
# Every transaction hledger makes of the time files.
hledger -f "$R" print > expected/print.journal
# The books alone, and with the time roll-up included: money balances match.
hledger -f books.journal bal -O csv > expected/books-bal.csv
hledger -f with-time.journal bal not:expr:'cur:"" or cur:h' -O csv > expected/with-time-money-bal.csv
