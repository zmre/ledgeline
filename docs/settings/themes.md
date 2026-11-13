---
title: Themes
description: "Ledgeline's colour themes: Default, GitHub, Catppuccin, Gruvbox, Tokyo Night, Solarized, One Dark Pro, Dracula, Monokai and Nord. Which ones follow your system's light or dark setting, and the theme key that saves your choice."
order: 50
---

# Themes

Ledgeline comes with sixteen colour themes based on popular editor themes. Each one sets the colours of the whole app, charts included. Every theme has its own chart colours, checked so that neighbouring colours stay distinct for colour-blind readers and every chart mark stands out from the background.

## Choosing a theme

Open **Settings → Appearance**. Each theme's card is drawn in its own colours. Click a card to use that theme. On a card with **Auto · Light · Dark** buttons, Auto follows your system, and Light or Dark keeps that version.

Tick **Use for this ledger only** to give the open ledger its own theme, so you can tell your books apart at a glance. Ledgeline saves it in a `ledgeline.toml` beside the journal, creating the file if needed. Untick it to go back to your usual theme.

## Follow the system, or pick one

Six themes come in a light and a dark version. Choose one of these by its family name and Ledgeline follows your system: the light version while your computer is in light mode, the dark one in dark mode, switching when the system does. Choose a version by its own name to keep it whatever the system says.

| Theme        | Follows the system | Light version      | Dark version        |
| ------------ | ------------------ | ------------------ | ------------------- |
| Default      | `default`          | `default-light`    | `default-dark`      |
| GitHub       | `github`           | `github-light`     | `github-dark`       |
| Catppuccin   | `catppuccin`       | `catppuccin-latte` | `catppuccin-mocha`  |
| Gruvbox      | `gruvbox`          | `gruvbox-light`    | `gruvbox-dark`      |
| Tokyo Night  | `tokyo-night`      | `tokyo-night-day`  | `tokyo-night-night` |
| Solarized    | `solarized`        | `solarized-light`  | `solarized-dark`    |
| One Dark Pro |                    |                    | `one-dark-pro`      |
| Dracula      |                    |                    | `dracula`           |
| Monokai      |                    |                    | `monokai`           |
| Nord         |                    |                    | `nord`              |

The last four are dark only, so they stay dark when your system is in light mode.

`default` is the starting theme. Its dark version is the look Ledgeline has always had.

## Text size

**Settings → Appearance** also sets the text size, from 70 % to 200 %. All text grows or shrinks together, and the layout around it follows. The keyboard does the same from anywhere in the app:

| Keys (Mac) | Keys (Windows, Linux) | Does        |
| ---------- | --------------------- | ----------- |
| ⌘ +        | Ctrl +                | Larger      |
| ⌘ −        | Ctrl −                | Smaller     |
| ⌘ 0        | Ctrl 0                | Actual size |

In the desktop app the same commands are in the **View** menu. The size is saved as `uiScale` (a percentage, `100` by default) and can be kept for one ledger like the theme.

## The `theme` key

Your choice is saved as `theme` in your view settings, using one of the names in the table. To set it by hand, add it to `ui-prefs.toml`:

```toml
theme = "catppuccin"
```

Put the same line under `[ui]` in a ledger's `ledgeline.toml` to give that ledger its own theme. [Settings files](settings-files.md) explains which file wins and where changes are saved. A name Ledgeline doesn't know is read as `default`.

## Colours that differ from the original

The themes use their authors' published colours. In a few places Ledgeline adjusts them:

- Some light themes' accent colours are too pale to read as a chart line on their background. Those chart colours are a shade darker.
- Themes with only a handful of accent colours get extra chart colours: darker or lighter shades of their own colours.
- Solarized and Tokyo Night Day keep their own text colours, which have less contrast than the other themes' text. Solarized was designed that way.
