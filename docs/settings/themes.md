---
title: Themes
description: "Ledgeline's colour themes: Default, GitHub, Catppuccin, Gruvbox, Tokyo Night, Solarized, One Dark Pro, Dracula, Monokai and Nord. Which ones follow your system's light or dark setting, and the theme key that saves your choice."
order: 50
---

# Themes

Ledgeline comes with sixteen colour themes based on popular editor themes. Each one sets the colours of the whole app, charts included. Every theme has its own chart colours, checked so that neighbouring colours stay distinct for colour-blind readers and every chart mark stands out from the background.

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
