#!/usr/bin/env python3
"""Regenerate the *binary* import fixtures.

The delimited fixtures that are plain UTF-8 text are committed as text and edited
by hand. The ones below cannot be: a workbook is a zip or a BIFF stream, and the
two non-UTF-8 CSVs exist precisely because their bytes are not text an editor
would preserve. They are committed as binaries and this script is how they are
rebuilt, so a reviewer can see what is in them without a hex editor.

Run from the repository root:

    nix-shell -p python3Packages.openpyxl python3Packages.xlwt python3Packages.odfpy \
        --run "python3 fixtures/import/generate.py"

Every value here is synthetic. See fixtures/import/README.md for what each file
proves.
"""

from __future__ import annotations

import datetime
import pathlib

import openpyxl
import xlwt
from odf.opendocument import OpenDocumentSpreadsheet
from odf.table import Table, TableCell, TableRow
from odf.text import P

ROOT = pathlib.Path(__file__).resolve().parent
DELIMITED = ROOT / "delimited"
SPREADSHEET = ROOT / "spreadsheet"

# One statement, shared by every spreadsheet fixture, so a test that compares two
# backends is comparing the backends and not the data.
STATEMENT = [
    ["Date", "Description", "Amount", "Balance"],
    [datetime.date(2026, 1, 5), "GROCERY STORE", -54.20, 1200.00],
    [datetime.date(2026, 1, 6), "ATM WITHDRAWAL", -100.00, 1100.00],
    [datetime.date(2026, 1, 7), "EMPLOYER PAYROLL", 2500.00, 3600.00],
    [datetime.date(2026, 1, 9), "CORNER MARKET", -31.18, 3568.82],
]

# A currency number format on the Amount column. It is here to be IGNORED:
# calamine exposes no number formats at all, so the fixture proves we emit
# `-54.2` and never `($54.20)`.
CURRENCY = '"$"#,##0.00_);("$"#,##0.00)'


def simple_xlsx() -> None:
    """Header row plus date cells stored as serial numbers with a date format."""
    book = openpyxl.Workbook()
    sheet = book.active
    sheet.title = "Statement"
    for row in STATEMENT:
        sheet.append(row)
    for row in sheet.iter_rows(min_row=2, min_col=1, max_col=1):
        for cell in row:
            cell.number_format = "yyyy-mm-dd"
    for row in sheet.iter_rows(min_row=2, min_col=3, max_col=4):
        for cell in row:
            cell.number_format = CURRENCY
    book.save(SPREADSHEET / "simple.xlsx")


def multi_sheet_xlsx() -> None:
    """Three sheets: one that is not a table, and two that are.

    `Cover` is a single populated cell, so sheet selection must walk past it.
    `Transactions` starts at C4, so the blank leading rows and columns must be
    trimmed before it looks like a table at all. `Summary` is a second genuine
    candidate, which is what makes `SheetChosen` owed to the user.
    """
    book = openpyxl.Workbook()
    cover = book.active
    cover.title = "Cover"
    cover["A1"] = "Acme Bank - statement export"

    transactions = book.create_sheet("Transactions")
    for offset, row in enumerate(STATEMENT):
        for column, value in enumerate(row):
            transactions.cell(row=4 + offset, column=3 + column, value=value)
    for offset in range(1, len(STATEMENT)):
        transactions.cell(row=4 + offset, column=3).number_format = "yyyy-mm-dd"

    summary = book.create_sheet("Summary")
    for row in [["Opening", 1254.20], ["Closing", 3568.82]]:
        summary.append(row)

    book.save(SPREADSHEET / "multi-sheet.xlsx")


def preamble_xlsx() -> None:
    """A floating title block above the header, which is what a real export ships.

    The shape is taken cell-for-cell from a real brokerage "All Activity" export:
    two empty rows, a one-cell title, an empty row, a second one-cell title,
    another empty row, and only then the column labels. Trimming the *blank*
    edges is not enough here — every one of those rows is inside the trimmed
    rectangle, so "the first populated row is the header" reads `All Activity
    Types` as the header and the entire statement as its body.

    The table below the preamble is the shared STATEMENT, so the conversion must
    produce a `Tabular` equal to `simple.xlsx`'s — differing only by
    `PreambleSkipped { lines: 4 }`.
    """
    book = openpyxl.Workbook()
    sheet = book.active
    sheet.title = "AllActivity"
    sheet["A3"] = "All Activity Types"
    sheet["A5"] = "Account Activity for All Accounts from Last 30 Days"

    first = 7
    for offset, row in enumerate(STATEMENT):
        for column, value in enumerate(row, start=1):
            sheet.cell(row=first + offset, column=column, value=value)
    for offset in range(1, len(STATEMENT)):
        sheet.cell(row=first + offset, column=1).number_format = "yyyy-mm-dd"
        for column in (3, 4):
            sheet.cell(row=first + offset, column=column).number_format = CURRENCY

    book.save(SPREADSHEET / "preamble.xlsx")


def trailer_xlsx() -> None:
    """A disclaimer block BELOW the transactions, which is what a real export ships.

    The synthetic twin of `real-brokerage-preamble.xlsx`'s other end, and it
    carries three traps in one sheet on purpose:

    1. A **trailer** of blank rows and one-cell paragraphs under the last
       transaction. Left in place, `to_csv` renders each of them as a record of
       empty fields, and hledger abandons the *entire* file on the first one —
       so a correct rules file reports that the data will not parse.
    2. A **blank row inside** the transactions. Same failure, different place, so
       trimming only the end is not enough.
    3. The last transaction has an **empty final column**, so it reaches column
       three of four. A rule spelled "narrower than the header" trims it and the
       user silently loses a transaction; the rule has to be "too narrow to hold
       a date and an amount". This is the row the trim must stop at.

    The four surviving rows are the shared STATEMENT's, so the conversion must
    produce `simple.xlsx`'s table with its last Balance blanked, plus
    `TrailerSkipped {lines: 4}` and `BlankRowsDropped {count: 1}`.
    """
    book = openpyxl.Workbook()
    sheet = book.active
    sheet.title = "Statement"

    # Row 4 is left empty on purpose: the blank row inside the transactions.
    layout = [(1, STATEMENT[0]), (2, STATEMENT[1]), (3, STATEMENT[2]), (5, STATEMENT[3])]
    # The last transaction, with its Balance cleared.
    layout.append((6, STATEMENT[4][:3] + [None]))
    for at, row in layout:
        for column, value in enumerate(row, start=1):
            if value is not None:
                sheet.cell(row=at, column=column, value=value)
    for at in (2, 3, 5, 6):
        sheet.cell(row=at, column=1).number_format = "yyyy-mm-dd"
        for column in (3, 4):
            sheet.cell(row=at, column=column).number_format = CURRENCY

    # Rows 7 and 9 are blank; 8 and 10 are the disclaimer paragraphs. The block
    # has to END on a populated row, or trimming the sheet's blank edges would
    # dispose of it before the rule under test ever sees it.
    sheet["A8"] = "*Balances shown are as of the statement date and may not reflect pending activity."
    sheet["A10"] = "Acme Bank is a member FDIC. Please retain this statement for your records."

    book.save(SPREADSHEET / "trailer.xlsx")


def single_column_xlsx() -> None:
    """A genuine one-column sheet, under a title of its own.

    The counter-example to the preamble rule. Every row here holds exactly one
    populated cell, so a rule spelled "a row with one cell in it is a title"
    discards the whole sheet — and a rule that instead looked for the first row
    matching the modal width would find it on line one and skip nothing, which is
    the answer we want. A one-wide table carries no signal either way, so this
    file must come back as `NoTable`: a sheet with no date and no amount is not a
    statement, and it must not be *reshaped* into one.
    """
    book = openpyxl.Workbook()
    sheet = book.active
    sheet.title = "Balances"
    sheet["A1"] = "Daily Closing Balance"
    sheet["A3"] = "Balance"
    for offset, value in enumerate([1200.00, 1100.00, 3600.00, 3568.82, 3510.00]):
        sheet.cell(row=4 + offset, column=1, value=value)
    book.save(SPREADSHEET / "single-column.xlsx")


def no_table_xlsx() -> None:
    """A workbook that opens cleanly and contains no table.

    A cover note and an empty sheet. This is `ConvertError::NoTable` — a specific
    answer about a perfectly valid file, not a parse failure — and it exists so a
    test can tell the two apart.
    """
    book = openpyxl.Workbook()
    cover = book.active
    cover.title = "Cover"
    cover["A1"] = "Acme Bank"
    cover["A2"] = "Please retain for your records."
    book.create_sheet("Blank")
    book.save(SPREADSHEET / "no-table.xlsx")


def legacy_xls() -> None:
    """The BIFF8 path, which is a completely different reader inside calamine."""
    book = xlwt.Workbook()
    sheet = book.add_sheet("Statement")
    date_style = xlwt.easyxf(num_format_str="YYYY-MM-DD")
    money_style = xlwt.easyxf(num_format_str=CURRENCY)
    for at, row in enumerate(STATEMENT):
        for column, value in enumerate(row):
            if isinstance(value, datetime.date):
                sheet.write(at, column, value, date_style)
            elif isinstance(value, float):
                sheet.write(at, column, value, money_style)
            else:
                sheet.write(at, column, value)
    book.save(str(SPREADSHEET / "legacy.xls"))


def sheet_ods() -> None:
    """ODS, where a date is ISO 8601 text in an attribute and never a serial."""
    document = OpenDocumentSpreadsheet()
    table = Table(name="Statement")
    for row in STATEMENT:
        line = TableRow()
        for value in row:
            if isinstance(value, datetime.date):
                cell = TableCell(valuetype="date", datevalue=value.isoformat())
                cell.addElement(P(text=value.isoformat()))
            elif isinstance(value, float):
                cell = TableCell(valuetype="float", value=value)
                cell.addElement(P(text=f"{value:.2f}"))
            else:
                cell = TableCell(valuetype="string")
                cell.addElement(P(text=str(value)))
            line.addElement(cell)
        table.addElement(line)
    document.spreadsheet.addElement(table)
    document.save(str(SPREADSHEET / "sheet.ods"))


def latin1_csv() -> None:
    """Windows-1252, with every byte that separates it from ISO-8859-1.

    0x92, 0x93, 0x94 and 0x80 are a right single quote, a pair of double quotes
    and the euro sign in Windows-1252, and unassigned C1 control characters in
    ISO-8859-1. None of them is valid UTF-8, so chardetng is the only thing that
    can decode this file, and only one of its two plausible answers is right.
    """
    # Written as the characters, encoded to the bytes: U+2019 -> 0x92,
    # U+201C/U+201D -> 0x93/0x94, U+20AC -> 0x80, U+00C9 -> 0xC9.
    text = (
        "Date,Description,Amount\r\n"
        "2026-01-05,MCDONALD’S RESTAURANT,-8.40\r\n"
        "2026-01-06,“THE CORNER” DELICATESSEN,-12.25\r\n"
        "2026-01-07,CAFÉ RÉPUBLIQUE PARIS,-31.00\r\n"
        "2026-01-08,ACME GMBH INVOICE €50 FEE,-50.00\r\n"
    )
    encoded = text.encode("cp1252")
    for byte in (0x92, 0x93, 0x94, 0x80, 0xC9):
        assert byte in encoded, f"0x{byte:02X} must survive into the fixture"
    (DELIMITED / "latin1.csv").write_bytes(encoded)


def utf16le_bom_csv() -> None:
    """What Excel's "Unicode Text" export writes, saved under a .csv name.

    UTF-16LE with a byte-order mark and CRLF terminators. Handed to chardetng
    first this decodes as windows-1252 and every cell comes back with a NUL after
    every character; the BOM has to be believed before any detector is consulted.
    """
    text = (
        "Date,Description,Amount\r\n"
        "2026-01-05,CAFÉ RÉPUBLIQUE,-8.40\r\n"
        "2026-01-06,BÜCHER MÜNCHEN,-12.25\r\n"
        "2026-01-07,EMPLOYER PAYROLL,2500.00\r\n"
    )
    (DELIMITED / "utf16le-bom.csv").write_bytes(b"\xff\xfe" + text.encode("utf-16-le"))


def main() -> None:
    SPREADSHEET.mkdir(parents=True, exist_ok=True)
    DELIMITED.mkdir(parents=True, exist_ok=True)
    for build in (
        simple_xlsx,
        multi_sheet_xlsx,
        preamble_xlsx,
        trailer_xlsx,
        single_column_xlsx,
        no_table_xlsx,
        legacy_xls,
        sheet_ods,
        latin1_csv,
        utf16le_bom_csv,
    ):
        build()
        print(f"wrote {build.__name__}")


if __name__ == "__main__":
    main()
