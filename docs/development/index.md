---
title: Development
description: "Maintainer documentation: building, releasing, the docs site, import internals, performance and research notes."
order: 90
tags: [development]
---

# Development

These pages are for people working on Ledgeline itself. If you only want to use the app, start at the [home page](../index.md).

- [Architecture](architecture.md) describes the layering: the core library, the axum server around it, the hosts that run it (desktop, headless, iOS in-process), and where a native UI would plug in.
- [Building and CI](building.md) covers the Nix dev shell, the pre-push hook, crane caching, how the SPA is embedded in the binary, the macOS app bundle, the Cachix cache and CI.
- [Releasing](releasing.md) explains how a `v*` tag becomes signed, notarized DMGs, how the crates are published, and how to bump the bundled hledger.
- [The docs site](docs-site.md) explains how this manual is built with mbr and published to GitHub Pages.
- [Imports internals](imports-internals.md) explains why the import pipeline is built the way it is, including the subprocess rules, alias handling and security layers.
- [Journal view internals](journal-view.md) explains how the Journal tab reads a large journal a window at a time: the engine routes, their caches, and the SPA stores that follow them.
- [Performance baseline](perf-baseline.md) has the benchmark corpus, the recorded numbers, and how to compare after a change.
- [PDF statement extraction](pdf-extraction.md) holds the research notes for a feature that hasn't been built.
- [Nix binary cache playbook](nix-binary-cache-playbook.md) is a general guide to making a Nix binary cache useful to other people.

Design plans for each feature live in [`plans/`](https://github.com/zmre/ledgeline/tree/main/plans) in the repository.
