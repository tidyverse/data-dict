# Landing page review: data-dict.tidyverse.org

## Steps

- [x] 1. Reconsider homepage content: move Inspirations + semantic-model discussion to `design.md`
- [ ] 2. Fix naming and navigation drift: project name is `data-dict` everywhere
- [ ] 3. Quickstart page (sketch): AI-prompt-driven otters example; Hadley writes the prose
- [ ] 4. Screenshot automation: light + dark otters captures, theme-aware via `.light-content`/`.dark-content`
- [ ] 5. Hero: tagline, positioning sentence, two buttons, SCSS
- [ ] 6. Install tabs: Shell / uv / pipx / R above the fold, `uvx` zero-install line
- [ ] 7. Show the artefact: YAML excerpt + screenshot + three-command quickstart teaser
- [ ] 8. Feature grid: six tiles, longer prose below
- [ ] 9. Built for the agent era: retitle, simulated agent conversation, two workflows
- [ ] 10. Example cards: description, table count, YAML + rendered links
- [ ] 11. Status and trust strip: version from `Cargo.toml` at render time, early-preview badge, GitHub star link
- [ ] 12. Polish pass: render, dark mode, mobile, `llms-txt`, proofread

*Reviewed against the "marketing landing page" archetype used by Polars, DuckDB, Apache Spark, ClickHouse and dbt.*

## Verdict

Right now this is a well-written **essay**, not a landing page. It reads like a design rationale — thorough, thoughtful, and almost entirely in prose — but it is missing nearly every block that marketing-archetype sites use to convert a visitor in the first ten seconds: no hero CTA, no install snippet, no code or YAML example, no picture, no version number, no feature grid. The raw material all exists; it is in the wrong order and the wrong form.

## What works already

- The opening sentence is strong and should stay: `data-dict.yaml` is two things, a specification for data dictionaries and a validator that enforces it. That is a clean positioning statement.
- The "Why now? AI" section is a differentiator no other dictionary standard has. It deserves more prominence, not less.
- There is a real visual asset — the rendered otters dictionary — but it is mentioned in passing in a bullet near the bottom.
- The install story is excellent (single binary, curl script, uv/pipx, R wrapper), but it lives on a separate page and the overview never shows a command.

## Prioritised recommendations

### 1. Add a hero with a tagline and two buttons

Every marketing-style site opens with one noun phrase plus a verb. Draft:

> **A data dictionary your data can't disagree with.**
> A lightweight YAML spec for documenting related tables, and a CLI that validates your data against it. Built for humans and AI agents.
> [Get started] [See an example]

"Get started" goes to a 60-second quickstart (see recommendation 3); "See an example" goes to the rendered otters site.

### 2. Put an install one-liner above the fold, tabbed by method

DuckDB, ClickHouse and Polars all do this. Four install paths already exist; show them as tabs on the homepage — Shell / uv / pipx / R — with the curl line as the default:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tidyverse/data-dict/releases/latest/download/data-dict-cli-installer.sh | sh
```

The zero-install line is a gift — feature it as "Try without installing":

```sh
uvx --from data-dict-yaml data-dict validate-spec data-dict.yaml
```

### 3. Show the artefact

Marketing sites lead with code (DuckDB, Spark) or a picture (D3, scikit-learn). Do both, because the product *is* a file and a rendered site.

- Add a side-by-side: a 15–20 line `data-dict.yaml` excerpt (one table, two columns, a constraint, a relationship, one glossary entry) next to a screenshot of the rendered otters page.

- Follow it with a simulated AI conversation that says "Create a data dictionary for otters.parquet with a the data-dict cli". Make sure it includes reading the `data-dict skill-read` skill
- Follow it with a three-command quickstart: `data-dict draft` → `data-dict validate-data` → `data-dict render`.

A visitor can currently finish the page without ever seeing what a data dictionary looks like.

### 4. Convert the "Why use" bullets into a feature grid

Nine prose bullets across spec and CLI collapse into six icon tiles in the Polars/DuckDB pattern — bold two-word label, one sentence each:

| Label | Sentence |
| --- | --- |
| **Validated, not just documented** | The CLI checks names, types, ranges and uniqueness against real data. |
| **Single binary** | No language run-times, no cloud; runs locally and in CI. |
| **Polyglot** | Language-neutral spec; works across R, Python and SQL. |
| **Diffable YAML** | Plain text you can version and review. |
| **Beautiful websites** | Browse tables, columns, relationships and glossary without reading YAML. |
| **Agent-ready** | Gives LLMs the context that currently lives in your head. |

Keep the longer prose below the fold for people who scroll.

### 5. Promote the AI angle to a named section with a concrete demo

"Why now? (*cough* AI *cough*)" is charming but buries the lede in a joke. Retitle to something like **"Built for the agent era"** and show the two workflows the text already describes:

1. An agent drafting a dictionary from existing docs (`.doc`, `.html`, `.pdf`).
2. An agent reading the dictionary to answer questions more accurately.

Even a short transcript or prompt snippet makes this tangible.

### 6. Make the examples browsable, not a list of slugs

`dabstep`, `elevators`, `foodbank`, `loan-application`, `otters` mean nothing to a newcomer. Give each a card with:

- a one-line description
- table count
- two links: "YAML" and "Rendered site"

This mirrors how scikit-learn pairs a description with a thumbnail.

### 7. Show version and status

DuckDB, D3 and pandas put the version in the header. The project is at v0.0.x with no formal governance — say so plainly with a "Status: early preview, under active development" badge and the current version. It manages expectations and signals the project is alive.

### 8. Add the trust strip

There are no logo walls or star counts yet, so use what exists:

- "Supported by Posit" with the logo
- GitHub link with a star button
- "Discuss on GitHub issues" call to contribute

Move the Posit sentence out of a bullet and into a footer band.

### 9. Fix naming and navigation drift

- The header says `data-dict` on the overview and `data-dict.yaml` on the install page; the browser title is "data-dict.yaml – data-dict". Pick one project name. State once that the spec is `data-dict.yaml` and the CLI is `data-dict`, then use the project name consistently.

### 10. Reconsider what belongs on the homepage

"Inspirations" and the semantic-model distinction are valuable but are for people already convinced. Move them to a "Design" page and link to it. The homepage should end with the CTA repeated, not with a bibliography.

## Decisions (2026-09-09)

- **Project name is `data-dict`.** State once that the spec is `data-dict.yaml` and the CLI is `data-dict`; use `data-dict` everywhere else.
- **Quickstart is a new page**, not just a homepage section. Assistant sketches it; Hadley writes most of it. The motivating example starts from an AI prompt: "Use the data-dict cli to document the otters dataset. You can find an existing dictionary in seot_morphometricsReproStatus_ak_monson_metadata.html"
- **Posit support stays in the header** (existing badge script); no footer band for it. The trust strip is status/version + GitHub only.
- **Screenshots are automated** via a script so they can be regenerated later. Work from the otters example. Produce both light and dark images and switch between them to match the site's active theme (Quarto's `.light-content` / `.dark-content`).
- **Version number is pulled from `Cargo.toml` at render time.**
- `data-dict draft` and `data-dict render` exist, so the quickstart commands are real.

## Suggested page order

1. Hero (tagline + two buttons)
2. Install tabs
3. YAML excerpt + rendered screenshot + three-command quickstart
4. Six-tile feature grid
5. "Built for the agent era"
6. "When to use it" (keep as is — it is good)
7. Example cards
8. Status/version + Posit + GitHub band

## Implementation note

Quarto can do all of this (tabsets, grid layout, `.hero` divs), so none of it requires leaving the current stack. Open the Polars and DuckDB homepages side by side with yours while rewriting — the gap in form is much larger than the gap in substance.\
