---
title: Templates
description: "Change how your invoices look in Ledgeline Invoicer without writing a page from scratch: the starter templates, Customize and Duplicate, the invoice base and its blocks with an example each, your logo, and what to do when a template shows an error."
order: 30
tags: [invoicer]
---

# Templates

A template decides how an invoice looks: in the app's preview, in the PDF you send, and in [mbr](https://github.com/zmre/mbr-markdown-browser) if you browse your books with it. The app's own theme never changes an invoice. Your customer sees what the template says, whether you use the app in dark mode or not.

You don't need to know HTML to change the colours, the font, the paper size or the logo. This page shows the few lines each change takes. Copy them, change the values, and watch the preview.

## The starters

The app comes with two templates:

| Starter | Look                                                                   |
|---------|------------------------------------------------------------------------|
| Classic | A serif letterhead, ruled lines and a right-aligned bill table.        |
| Modern  | A sans-serif page with one accent colour and the amount due in a band. |

A starter is copied into your books folder, under `invoices/tpl/`, the first time an invoice uses it. From then on the copy is yours: the app never overwrites it. Until then, the Templates page lists the starter as read-only.

## Choosing a template

- **For one invoice.** In the invoice editor, the Template field (<kbd>t</kbd> jumps to it). On a saved invoice, the template picker previews another template, and **Use this template** switches to it.
- **For a customer.** The customer's default template is what their new invoices start with.
- **For these books.** A new invoice for a customer with no default uses the Default template [setting](settings.md#new-invoices), in **Settings → These books → New invoices**. It can be one of your templates or a starter.
- **For everything else.** With no Default template set, a new invoice uses the first template in your books, or Classic when there is none. The same happens when the Default template is no longer in your books, and the app says so.

## The Templates page

The Templates page lists your templates and the starters you haven't used yet. For each one it shows what it is for, what it builds on, which parts it changes and which invoices and customers use it. Click a name to open it.

A template's page has:

- **Name**, how pickers show it, and **For**: invoices, estimates, credit notes. The file keeps its own name when you rename it.
- **Blocks**: the parts of the base this template changes, and **Override block** to change another.
- The template's code.
- A live **Preview with** one of your invoices, the latest by default. It updates as you type.
- **Save**, **Discard changes**, **Duplicate…** and **Delete…** at the top right. <kbd>Cmd</kbd>+<kbd>S</kbd> (<kbd>Ctrl</kbd>+<kbd>S</kbd> on Linux) saves.

### Customize and Duplicate

A starter you haven't used opens read-only, with two buttons:

- **Customize** copies the starter into your books under its own name. You then edit the copy, and every invoice using that starter uses your copy.
- **Duplicate…** asks for a name and makes a new template from it, leaving the starter as it is. Use this to keep Classic and have "Classic, blue" beside it.

**Duplicate…** works on your own templates too. Save first: it copies the saved file, not unsaved changes.

**New template** on the Templates page asks for a name and what to start from: the invoice base as it comes, a starter, or a copy of one of your templates.

**Delete…** removes a template from your books. It is refused while any invoice or customer uses it; switch them to another template first. The base stays.

## How a template is built

Every template the app makes starts with two lines:

```text
{# invoicer: name="Ocean blue" kinds="invoice" #}
{% extends "_invoice_base.v1.html" %}
```

The first line is the name and the kinds of document, which the editor shows as fields. The second says "start from the invoice base". With nothing more, the template looks exactly like the base: your company and the customer side by side with their addresses, emails and phones, a line of dates, then the bill table with ruled cells, and a Draft, Void or Paid mark beside the title.

The base, `_invoice_base.v1.html`, sits beside your templates in `invoices/tpl/`. It is divided into named **blocks**. A template changes the look by giving its own version of a block. Everything it doesn't mention comes from the base.

A block looks like this:

```text
{% block footer %}
  <div class="footer">Thank you for your business.</div>
{% endblock footer %}
```

Inside a block, `{{ super() }}` stands for what the base has there. Write it first to keep the base's version and add to it; leave it out to replace the block.

**Override block** in the editor lists the blocks you haven't changed yet, with what each is for. Pick one and it adds the block, with `{{ super() }}` inside, where your cursor is.

The rest of this section takes the blocks in page order, with an example for each. Put any of them under the two first lines, in any order.

### settings

Choices made before anything is drawn. Two are useful:

```text
{% block settings %}
  {% set kind = "Quote" %}
{% endblock settings %}
```

`kind` is the word before the number in the title: "Invoice 105". It is "Invoice", "Estimate" or "Credit note" by itself.

```text
{% block settings %}
  {% set chrome = false %}
{% endblock settings %}
```

`chrome = false` shows the invoice alone even when mbr serves the page, without mbr's sidebar and search around it.

### ready

Tells the PDF export when the page is ready to print, after fonts and images have loaded. Leave it alone. An empty `ready` block prints as soon as the page loads, which can print before a font or image has loaded.

### head

Anything extra for the top of the page, such as a stylesheet with your fonts:

```text
{% block head %}
  <link rel="stylesheet" href="/assets/fonts.css">
{% endblock head %}
```

A path starting with `/` is from your books folder.

### theme

The look of the sheet: colours, font, spacing, paper. This is the block most templates change. The base names its colours so you can change them in one place:

| Name      | Colours                        |
|-----------|--------------------------------|
| `--ink`   | the text                       |
| `--muted` | addresses and other quiet text |
| `--rule`  | lines and table borders        |
| `--paper` | the sheet                      |
| `--desk`  | around the sheet on screen     |

To keep the base's layout and change only colours, font and paper, write `{{ super() }}` first and your changes after it:

```text
{% block theme %}
  {{ super() }}
  :root {
    --ink: #1b2a41;
    --muted: #5b6b80;
    --rule: #b8c4d4;
  }
  .sheet { font-family: Georgia, "Times New Roman", serif; }
  .sheet h1 { color: #0b5394; }
  @page { size: A4; margin: 18mm; }
{% endblock theme %}
```

- Colours are written as `#` and six hex digits, as in any colour picker.
- `@page` sets the paper and margins of the PDF: `size: letter`, `size: A4`, `size: legal`.
- Without `{{ super() }}`, the base's look is gone and you style the whole sheet yourself. Classic and Modern do that; open one to see a complete theme.

### header

The top of the sheet: your logo, then the title with the number, and the Draft, Void or Paid mark. This version adds a line under the title:

```text
{% block header %}
  {{ super() }}
  <p class="tagline">Architecture and engineering since 2009</p>
{% endblock header %}
```

Style the new line in your `theme` block, for example `.tagline { color: var(--muted); margin-top: 0; }`.

### parties

Who the invoice is from and who pays: name, address, email and phone. This version shows only the customer:

```text
{% block parties %}
  <div class="parties">
    <div class="bill-to">
      <p class="party-head">Bill to {{ fm.invoicee_name }}</p>
      <p class="party-line">{{ fm.invoicee_address | default(value="") | trim | escape | linebreaksbr | safe }}</p>
    </div>
  </div>
{% endblock parties %}
```

The long line after the address keeps the address's line breaks and makes it safe to print. Copy it as it is.

### details

The line of facts under the parties: date, due date, terms, reference, and what a deposit is for. To add a field of your own, such as `project: Harbor bridge` in an invoice, keep the base's line and add yours under it:

```text
{% block details %}
  {{ super() }}
  {% if fm.project %}<p class="details"><b>Project</b> {{ fm.project }}</p>{% endif %}
{% endblock details %}
```

`{% if fm.project %}` leaves the line out of invoices that have no project.

### body

The invoice's own text, the bill table included. You rarely change it, but you can add something to every invoice:

```text
{% block body %}
  {{ super() }}
  <p>Questions about this invoice? Reply to {{ fm.invoicer_email }}.</p>
{% endblock body %}
```

### footer

The bottom of the sheet. The base leaves it empty.

```text
{% block footer %}
  <div class="footer">{{ fm.invoicer_name }} · Registered in Colorado · Tax ID 12-3456789</div>
{% endblock footer %}
```

## What a template can show

A template reads the invoice's fields through `fm`. Write a field as `{{ fm.name }}`:

| Write                    | Shows                                                             |
|--------------------------|-------------------------------------------------------------------|
| `{{ fm.invoice_num }}`   | the number                                                        |
| `{{ fm.date }}`          | the issue date                                                    |
| `{{ fm.due_date }}`      | the due date                                                      |
| `{{ fm.terms }}`         | the payment terms, in days                                        |
| `{{ fm.reference }}`     | the customer's reference, such as a PO                            |
| `{{ fm.invoicee_name }}` | the customer's billing name; also `_email`, `_phone`, `_address`  |
| `{{ fm.invoicer_name }}` | your company's name; also `_email`, `_phone`, `_address`, `_logo` |
| `{{ fm.status }}`        | `draft`, `sent` or `void`                                         |
| `{{ fm.deposit_for }}`   | on a deposit invoice, what it is for                              |
| `{{ fm.anything }}`      | any field you add to your invoices yourself                       |
| `{{ kind }}`             | "Invoice", "Estimate" or "Credit note"                            |
| `{{ paid }}`             | the paid status as text: `no`, `partial …`, or a date             |
| `{{ markdown \| safe }}` | the invoice's text and bill table                                 |

[Files and formats](files.md#frontmatter-fields) lists every field. Two habits avoid most errors:

- A field some invoices don't have needs a fallback: `{{ fm.reference | default(value="") }}`, or wrap the line in `{% if fm.reference %}…{% endif %}`.
- Addresses go through `| trim | escape | linebreaksbr | safe`, as in the parties example, so their lines stay apart.

Templates use the [Tera](https://keats.github.io/tera/docs/) template language. Its documentation lists everything else you can write.

## What a template can't do

Templates are files people share, so a template can only show the invoice:

- Images, stylesheets and fonts load from your books folder only. One from a web address doesn't load, in the app or in the PDF, so a template can't send an invoice's details anywhere. Put the file in the books folder and write its path, such as `/assets/fonts.css`.
- A `<script>` written in a template doesn't run, and the preview runs no scripts at all.
- Tera's `get_env`, which reads the computer's environment variables, is turned off.
- HTML in an invoice's text or in a bill-table cell is cleaned before it shows. Formatting such as `<br>` and `<b>` stays; scripts, `style` attributes, forms and frames are removed. To show such HTML as text instead, put `raw_html = "escape"` in `.mbr/config.toml` in your books folder.

## Your logo

Put the logo on your company's note, the organization note named by the Your company [setting](settings.md#company):

```yaml
logo: logo.png
```

The image can be a PNG, JPEG, GIF, SVG or WebP file. `logo.png` and `"[[logo.png]]"` look beside the company note first, then from the books folder. `/assets/logo.png` is from the books folder only. A web address, a file outside the books folder or a missing file means no logo.

Each new invoice copies the logo's path into its `invoicer_logo` field, as it copies your address, so an old invoice keeps the logo it was sent with. Invoices written before you set the logo have no logo until you add that field to them.

The base's `header` block shows the logo above the title, no larger than 2.5 inches wide and 0.8 inches tall, with your company's name as its description for screen readers. Classic and Modern put it above your company's name. To place it yourself in your own `header` block, write:

```text
{% if fm.invoicer_logo %}
  <img class="logo" src="{{ fm.invoicer_logo | urlencode }}" alt="{{ fm.invoicer_name }}">
{% endif %}
```

## When a template shows an error

If the code can't be read or refers to something that isn't there, the preview keeps showing the last version that worked, and a yellow notice above the code says what is wrong and on which line. **Go to line** puts your cursor there.

| The notice says                                                       | It usually means                                                           | Do this                                                                                                  |
|-----------------------------------------------------------------------|----------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| Nothing called fm.project is available to the template                | a field name is misspelled, or the preview invoice doesn't have that field | Check the spelling. For a field some invoices lack, add `\| default(value="")` or wrap it in `{% if %}`. |
| The template doesn't read here: expected …                            | a `{% %}` or `{{ }}` isn't finished, or an `if` or `block` isn't closed    | Look at the line, and the line above it, for a missing `%}`, `}}`, `{% endif %}` or `{% endblock %}`.    |
| There's no filter called …                                            | a misspelled word after a `\|`                                             | Fix the spelling. `default`, `trim`, `escape`, `upper` and `date` are common ones.                       |
| It extends _invoice_base.v1.html, which isn't in the templates folder | the base file was moved or deleted                                         | Copy a starter in with **Customize**; that brings the base back. Or put the file back from git.          |
| There's no file … beside the template to include                      | an `include` names a file that isn't in the templates folder               | Fix the name or add the file.                                                                            |

The `extends` line must be the second line, right after the name line. Tera, the template language, allows only one comment before it. Your own comments go after it.

A template only changes how invoices look. Nothing you do in the template editor changes an invoice file or your books.

## When the base changes

The base's name carries a version, `v1`, because your copy is yours to change. If a later version of the app fixes the base, it ships as a new file, `_invoice_base.v2.html`, and your templates keep using `v1` until you change their `extends` line.

A template doesn't have to use the base at all. A whole HTML page that mentions `invoice_num` works as a template too, as long as it hides the books column. This rule does it:

```css
table:has(thead th:last-child > em:only-child) :is(th, td):last-child { display: none; }
```
