---
title: Invoicer keyboard shortcuts
description: "Every keyboard shortcut in Ledgeline Invoicer, page by page: going to a section, moving through lists, sorting, writing and sending an invoice, and the bill table's row keys."
order: 60
tags: [invoicer]
---

# Keyboard shortcuts

You can write, send and export an invoice without touching the mouse. The invoicer uses the same keys as [Ledgeline](../journal/keyboard.md): <kbd>j</kbd> and <kbd>k</kbd> move, <kbd>/</kbd> searches, and <kbd>g</kbd> followed by a letter goes somewhere.

## Press `?` for help

<kbd>?</kbd> opens a sheet of the shortcuts that work on the page you are on, in the order of the sections below. It lists only keys that do something right now: the Send key doesn't appear on a sent invoice. Press <kbd>Esc</kbd> or <kbd>?</kbd> again to close it.

## Typing and shortcuts

Single-letter shortcuts are off while you type in a field, so <kbd>j</kbd> in the search box types a "j". Press <kbd>Esc</kbd> to leave the field and get the shortcuts back. Saving and the bill table's row keys work from inside a field too.

A capital letter means Shift: <kbd>N</kbd> is <kbd>Shift</kbd>+<kbd>N</kbd>. A sequence such as <kbd>g</kbd> <kbd>i</kbd> means press <kbd>g</kbd>, let go, then press <kbd>i</kbd>.

<kbd>Cmd</kbd> on macOS is <kbd>Ctrl</kbd> on Linux, and <kbd>Option</kbd> is <kbd>Alt</kbd>.

## Everywhere

| Keys                        | Does                                                                           |
|-----------------------------|--------------------------------------------------------------------------------|
| <kbd>?</kbd>                | Show the keyboard shortcuts                                                    |
| <kbd>n</kbd>                | New invoice                                                                    |
| <kbd>d</kbd>                | New deposit invoice                                                            |
| <kbd>N</kbd>                | Open the New menu (deposit invoice, customer, catalog item, snippet, template) |
| <kbd>/</kbd>                | Search this list (on pages with a search box)                                  |
| <kbd>Esc</kbd>              | Leave the field you are typing in                                              |
| <kbd>Cmd</kbd>+<kbd>,</kbd> | Open Settings, from inside a field too                                         |
| <kbd>Cmd</kbd>+<kbd>+</kbd> | Larger text                                                                    |
| <kbd>Cmd</kbd>+<kbd>−</kbd> | Smaller text                                                                   |
| <kbd>Cmd</kbd>+<kbd>0</kbd> | Actual size                                                                    |

The text size keys work from inside a field too, and over a dialog. The desktop app's View menu has the same three. See [Appearance](settings.md#appearance).

## Go to

| Keys                      | Goes to   |
|---------------------------|-----------|
| <kbd>g</kbd> <kbd>d</kbd> | Dashboard |
| <kbd>g</kbd> <kbd>i</kbd> | Invoices  |
| <kbd>g</kbd> <kbd>c</kbd> | Customers |
| <kbd>g</kbd> <kbd>k</kbd> | Catalog   |
| <kbd>g</kbd> <kbd>s</kbd> | Snippets  |
| <kbd>g</kbd> <kbd>t</kbd> | Templates |
| <kbd>g</kbd> <kbd>,</kbd> | Settings  |

## Dashboard

| Keys                                   | Does                                                                |
|----------------------------------------|---------------------------------------------------------------------|
| <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> | Open Drafts, Outstanding or Paid                                    |
| <kbd>w</kbd> / <kbd>W</kbd>            | A longer / shorter paid period                                      |
| <kbd>j</kbd> / <kbd>k</kbd>            | Next / previous recently updated invoice; <kbd>Enter</kbd> opens it |

## Lists

These work on the Invoices, Customers, Snippets and Templates lists.

| Keys                                     | Does                                                                |
|------------------------------------------|---------------------------------------------------------------------|
| <kbd>j</kbd> / <kbd>k</kbd>              | Next / previous row                                                 |
| <kbd>g</kbd> <kbd>g</kbd> / <kbd>G</kbd> | First / last row                                                    |
| <kbd>o</kbd>                             | Open the row                                                        |
| <kbd>Enter</kbd>                         | Open the row (with focus in the list)                               |
| <kbd>Esc</kbd>                           | Clear the row cursor                                                |
| <kbd>s</kbd> / <kbd>S</kbd>              | Sort by the next column / reverse the sort (Invoices and Customers) |

In the search box, <kbd>Esc</kbd> clears what you typed, and a second <kbd>Esc</kbd> leaves the box. <kbd>Enter</kbd> in the invoice search opens the invoice whose number you typed, or the only one left.

## An invoice

| Keys         | Does             |
|--------------|------------------|
| <kbd>e</kbd> | Edit the invoice |
| <kbd>P</kbd> | Export the PDF   |

## The invoice editor

| Keys                        | Does                                                       |
|-----------------------------|------------------------------------------------------------|
| <kbd>Cmd</kbd>+<kbd>S</kbd> | Save                                                       |
| <kbd>c</kbd>                | Go to the customer field                                   |
| <kbd>t</kbd>                | Go to the template field                                   |
| <kbd>S</kbd>                | Send… (a draft, once saved)                                |
| <kbd>R</kbd>                | Re-book… (a sent invoice, once saved)                      |
| <kbd>V</kbd>                | Void…                                                      |
| <kbd>a</kbd>                | Apply a payment… (cash books, a sent invoice not yet paid) |
| <kbd>D</kbd>                | Discard the changes, after asking                          |
| <kbd>P</kbd>                | Export the PDF                                             |

Send, Re-book, Void and Discard each open a confirmation with the focus on Cancel, so a stray key does nothing until you choose. Apply payment opens on the best-matching deposit: <kbd>↑</kbd> and <kbd>↓</kbd> choose another, and the payment is applied only from the Apply payment button, which waits for the preview.

In the customer field, <kbd>↓</kbd> opens the list of customers; <kbd>↑</kbd> and <kbd>↓</kbd> move, <kbd>Enter</kbd> chooses.

## The bill table

These work from any field on the editor page.

| Keys                                            | Adds                  |
|-------------------------------------------------|-----------------------|
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> | An item row           |
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | A percentage discount |
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> | A fixed discount      |
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | A subtotal            |
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | A tax row             |
| <kbd>Option</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> | A blank spacer row    |

Inside the table, <kbd>Option</kbd>+<kbd>↑</kbd> and <kbd>Option</kbd>+<kbd>↓</kbd> move the row you are in. In a description or code cell, <kbd>↓</kbd> opens the catalog.

## A customer

| Keys                        | Does                                               |
|-----------------------------|----------------------------------------------------|
| <kbd>e</kbd>                | Edit the customer                                  |
| <kbd>Cmd</kbd>+<kbd>S</kbd> | Save, while editing                                |
| <kbd>Esc</kbd>              | Cancel editing; asks first if something is unsaved |

## Catalog, snippets and templates

| Keys                                                            | Does                            |
|-----------------------------------------------------------------|---------------------------------|
| <kbd>Cmd</kbd>+<kbd>S</kbd>                                     | Save                            |
| <kbd>Option</kbd>+<kbd>↑</kbd> / <kbd>Option</kbd>+<kbd>↓</kbd> | Move the catalog row you are in |

In a template's code, <kbd>Tab</kbd> indents. To move on to the next control, press <kbd>Esc</kbd> and then <kbd>Tab</kbd>.

## Settings

| Keys                        | Does                                   |
|-----------------------------|----------------------------------------|
| <kbd>Cmd</kbd>+<kbd>S</kbd> | Save the changes on These books        |
| <kbd>1</kbd> / <kbd>2</kbd> | These books / Appearance               |
| <kbd>←</kbd> / <kbd>→</kbd> | The other tab, with the focus on a tab |

<kbd>Tab</kbd> moves through every setting in screen order. In a list of choices, <kbd>↑</kbd> and <kbd>↓</kbd> pick one. In a word list, <kbd>Enter</kbd> or <kbd>,</kbd> adds the word you typed. See [Settings](settings.md).

## Dialogs

While a confirmation is open, the page's shortcuts are off. <kbd>Esc</kbd> cancels, and <kbd>Tab</kbd> stays inside the dialog.
