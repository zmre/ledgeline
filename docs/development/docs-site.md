---
title: The docs site
description: "How this manual is built with mbr, previewed and link-checked with just, and published to GitHub Pages on each release."
order: 30
tags: [development]
---

# The docs site

This manual is the `docs/` folder, built into a static site by [mbr](https://github.com/zmre/mbr-markdown-browser) and published at <https://zmre.github.io/ledgeline/>. The same Markdown files read fine on GitHub.

## Previewing and building

Both recipes run mbr v0.6.2 through `nix run`, the same pinned version CI uses, so nothing has to be installed:

```sh
just docs          # live preview server; mbr prints the URL
just docs-build    # static build into /tmp/ledgeline-docs-build, failing on any broken link
just docs-build ./site       # the same, into another folder
```

Run `just docs-build` before you push a docs change. It is the check CI runs, and it fails on a link to a page that doesn't exist.

mbr doesn't check `#anchor` fragments, so a link to a renamed heading still builds. mbr's heading ids are its own: lowercase, spaces become `-`, and punctuation becomes an extra `-`. `## Income, and the sign` gets the id `income--and-the-sign`, which is not what GitHub produces. Links in this manual use mbr's ids, so a few anchors don't jump when the file is read on GitHub.

## Screenshots

```sh
just docs-screenshots
```

This regenerates every PNG under `docs/**/images/` from the Playwright harness in `web/docs-screenshots/`. The harness starts the same engine and preview servers as the e2e suite, pins the clock to 2026-07-08, and loads the fixture journals under `fixtures/`, so the images come out the same on every machine. Circles, arrows and boxes are drawn in the page as an SVG overlay before the shot is taken, so they move with the UI when it changes. Commit the regenerated images.

Images live next to the page that uses them, in an `images/` folder, and are linked with relative paths.

## How the site is laid out

The site root is `docs/`, marked by `docs/.mbr/config.toml`. Without that file, mbr would look further up and publish the whole repository.

- Each folder's landing page is its `index.md`. A `README.md` would be an ordinary page.
- Pages and folders are sorted by the `order` frontmatter field, then by `title`. A folder's position comes from its `index.md`.
- The sidebar is a single tree (`sidebar_style = "single"`), and every page title is prefixed with "Ledgeline | ".

Every page starts with YAML frontmatter:

```yaml
---
title: Importing from QuickBooks
description: "Export the Journal report from QuickBooks Online and bring it into your hledger journal."
order: 40
tags: [imports, quickbooks]
---
```

Invalid YAML makes mbr drop the whole block without an error, so quote any value that contains a colon. Tags come from a small fixed list: `imports`, `reports`, `holdings`, `budget`, `projections`, `journal`, `account-tags`, `export`, `quickbooks`, `prices` and `development`. `account-tags` marks every page that explains an account tag Ledgeline reads, such as `bsgroup:` or `holdings:`.

A few more rules come from mbr:

- Links between pages are relative Markdown links (`[x](../reports/index.md)`). A link that leaves `docs/`, to a plan or a source file, counts as broken, so those use full `https://github.com/zmre/ledgeline/blob/main/...` URLs.
- Callouts can be `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]` or `[!CAUTION]`. `[!INFO]` doesn't exist in mbr or on GitHub.
- mbr's default ignore list skips files such as `*.sh`, `*.ts` and `*.css`, and folders such as `build` and `templates`. Don't put pages or images under those names.

## Publishing

[`.github/workflows/docs.yml`](https://github.com/zmre/ledgeline/blob/main/.github/workflows/docs.yml) runs in three cases:

| Trigger | What happens |
| --- | --- |
| A `v*` tag is pushed | Build, link-check, deploy to GitHub Pages |
| Run by hand (workflow dispatch) | Build, link-check, deploy |
| A pull request touches `docs/**` | Build and link-check only |

The job builds mbr with Nix, because the release binary links into `/nix/store` and won't run on a stock runner. It then runs `mbr -b --fail-on-broken-links`. The output is tarred by hand with `--dereference`, because mbr symlinks its assets and `upload-pages-artifact` would drop the `.mbr/` folder that holds the theme and the search index.

### One-time repository setup

Two settings have to be changed by hand before the first deploy works:

1. In **Settings → Pages**, set the source to "GitHub Actions".
2. In **Settings → Environments → github-pages**, add a deployment tag rule for `v*`. The default rule only allows the default branch, so a deploy from a tag is rejected without it.
