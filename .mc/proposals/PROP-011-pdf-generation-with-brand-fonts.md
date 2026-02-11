---
id: PROP-011
title: PDF Generation with Brand Fonts
status: accepted
type: feature
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- pdf
- branding
- export
created: 2026-02-09
updated: 2026-02-09
---

# PDF Generation with Brand Fonts

## Context

MissionControl stores meetings and research as Markdown files, but these often need to be shared externally as professionally formatted PDFs. The PDF output should reflect organizational branding—custom fonts, colors, and cover pages—without requiring external tools like LaTeX or headless browsers.

## Options Considered

### Option 1: Markdown-to-HTML-to-PDF via headless browser

Render Markdown to HTML, then use a headless Chrome/Chromium to print to PDF. High fidelity but requires a browser installation, is slow, and adds a large external dependency.

### Option 2: LaTeX-based PDF generation

Convert Markdown to LaTeX and compile with pdflatex/xelatex. Excellent typographic quality but requires a full TeX installation (gigabytes), complex templating, and slow compilation.

### Option 3: Pure Rust PDF generation with genpdf

Use the `genpdf` crate to generate PDFs directly from parsed Markdown. Load brand fonts from a configurable directory. No external dependencies beyond TTF font files.

## Decision

Use pure Rust PDF generation (Option 3). The `commands/print.rs` module uses `genpdf` to produce branded PDFs with:

- **Configurable fonts** — loaded from a brand fonts directory specified in `config.yml` (`brand.fonts_dir` and `brand.font_name`)
- **Brand colors** — primary and accent colors from config, used for headings, cover page elements, and decorative rules
- **Cover pages** — professional title pages with brand name, document title, tagline, and metadata (entity ID, date, status)
- **Markdown rendering** — a custom `pulldown_cmark` event walker that maps headings, paragraphs, lists, code blocks, and tables to `genpdf` elements
- **Page decoration** — headers with document title, footers with page numbers and brand name

## Consequences

- PDF generation works offline with zero external tools — only TTF font files are needed
- Brand identity is maintained across all exported documents via config-driven fonts and colors
- The `genpdf` crate handles page layout and font embedding, keeping the implementation manageable
- Typography is adequate for business documents but less refined than LaTeX output
- Only meetings and research entities support PDF export currently — the pattern is extensible to other entity types
- Font loading errors produce clear `McError::Pdf` messages guiding users to the font setup documentation
