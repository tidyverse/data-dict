# Render report UI

This directory implements the client-side UI for the self-contained `render-report` HTML page. It uses classic scripts and Preact globals, not ES modules; asset order is defined in `crates/data-dict-cli/src/assets.rs` and is significant.

## Structure

- `report.html` is the page template. It embeds CSS, JavaScript, and JSON payloads.
- `report.js` owns hash routing, page layout, sidebar navigation, and table/problem/step views.
- `steps.js` renders the check roster, including filtering, failures-only visibility, sorting, and check verdicts.
- `rows.js` renders problem and dataset-level row evidence, including highlighted cells and tooltips.
- `diagnostic.js`, `yaml-excerpt.js`, and `suggestion.js` render problem details and source context.
- `report.css` contains report-local layout and components. It loads after `render/shared/app.css` and `render/shared/tables.css`, so use shared semantic tokens and primitives rather than duplicating them locally.

## UI behavior

- Routes are `#table/<name>`, `#step/<id>`, and `#problem/<index>`; `#rows/<table>` redirects to the table route.
- The desktop layout has a sticky table sidebar and a report panel. At narrow widths the sidebar becomes wrapping pills above an edge-to-edge panel.
- Table verdicts are pass/fail: warnings do not currently create a warning table state.
- The checks filter matches columns only, not check codes, labels, or messages.
- Failed check rows link to step details. Unevaluated checks are non-linking.

## Data and safety

The report is a renderer, not a data source: it must not add or reconstruct data. Optional payload fields are commonly absent and must be handled gracefully.

Columns marked `display: restricted` must never reveal data-derived values in rendered HTML, UI controls, tooltips, or diagnostics. Preserve existing upstream redaction behavior; do not infer restricted values from adjacent fields.

Problem-level evidence and dataset-level failed rows have different payload shapes. Count-only findings intentionally use prose rather than a fabricated row table.

## Testing and workflow

There is no browser/UI test suite. Rust tests cover asset composition, generated HTML, offline payload embedding, and redaction in `crates/data-dict-cli/tests/cli.rs`; report JSON contracts and snapshots live in `crates/data-dict/tests/report.rs` and its snapshots.

For manual UI work, `render-report --live` supports CSS hot-swapping. `playground/data-dict.yaml` with `playground/generate.R` provides representative clean and failing data.

The authoritative report payload contract is `site/report.md`; validation and redaction behavior is documented in `site/dev-validation.md`.
