# data-dict.yaml

`data-dict.yaml` is a YAML-based data dictionary specification and validator. It describes collections of related tables — their columns, types, constraints, relationships, and domain vocabulary — and is designed to be co-written by humans and AI agents. The main deliverable is a CLI tool that validates data dictionary YAML files against the spec.

The repo contains:

- `site/spec.md`: the full specification (v0.1.0)
- `README.md`: project overview, CLI install/build instructions, and a pointer to the site.
- `site/`: the [Quarto](https://quarto.org) website published to data-dict.tidyverse.org. Holds the spec and design docs (`spec.md`, `semantic-models.md`), as well as example data dictionaries downloaded from other repos (see `download-examples.R`). Built and deployed by `.github/workflows/publish-site.yaml`.
- `crates/`: Rust workspace (see crate architecture below)
- `schema.yaml`: JSON Schema for structural validation of data dictionary files

## Code principles

* Reserve comments for explaining why, not what or how.
* User facing code should be accompanied by a test.

## Spec and implementation must stay in sync

The spec (`site/spec.md`) and the implementation (the crates + `schema.yaml`) are two views of the same thing and must never drift apart.

- **New features start in the spec, and REQUIRE human sign-off.** This is the single most important rule in this file. Any new feature is a two-phase process with a hard stop between the phases:
    1. **Write the spec.** Draft and iterate the change in `site/spec.md` *only*. Do not touch `schema.yaml`, the crates, the tests, or any other file in this phase.
    2. **Stop and get an explicit "yes" from a human on the spec text.** Asking clarifying questions is not sign-off. Presenting a plan is not sign-off. You must show the human the actual spec wording and wait for them to explicitly approve *that wording* before writing a single line of implementation. If you are unsure whether you have approval, you do not have approval — ask again.

  Only after that explicit yes do you implement (`schema.yaml`, crates, tests). Starting implementation before the human has signed off on the spec is a process violation, even if the feature itself is fine.
- **Implementation refinements flow back to the spec.** If you discover during implementation that the spec is wrong, incomplete, or ambiguous, update `site/spec.md` to match what you actually built.
- **Touch one, check the other.** Whenever you change the spec, double-check the implementation still matches; whenever you change the implementation, update the spec. A change to either is incomplete until both agree.

## Commands

```bash
# Build
cargo build --workspace
cargo build --workspace --all-targets   # includes tests, examples, benches

# Test
cargo test --workspace
cargo test -p data-dict                 # single crate
cargo test -p data-dict lint            # tests matching "lint" in data-dict crate

# Validate a file
cargo run -p data-dict-cli -- validate-schema site/examples/otters.yaml
```

To review/accept insta snapshots: `cargo insta review`.

## Crate architecture

Rust workspace with three crates:

- `crates/data-dict/` — core library: YAML parsing, schema validation, lowering to typed model, and semantic linting. All logic lives here.
- `crates/data-dict-cli/` — thin CLI wrapper (`validate-schema`, plus `parquet types` / `parquet validate`). Keep it thin.
- `crates/data-dict-parquet/` — reads Parquet file schemas and maps column types to data-dict types.

### Schema validation pipeline

```
YAML file
  → quarto_yaml: parse to AST with source spans
  → structural validation against schema.yaml (embedded via include_str!)
  → lower.rs: AST → typed model (DataDict, Table, Column, Relationship, ...)
  → lint.rs: semantic rules DD001–DD008
  → Result<(), Vec<Diagnostic>>
```

### Lint rules (DD001–DD009)

DD001–DD008 are errors (they fail validation); DD009 is a warning (reported but does not fail validation).

| Rule | Description |
|------|-------------|
| DD001 | `foreign_key` column has no matching relationship with `primary_key` |
| DD002 | Relationship references non-existent table |
| DD003 | Relationship references non-existent column |
| DD004 | `join` expression fails to parse or references wrong number of tables |
| DD005 | Column in `conflicts` doesn't appear on both sides of the join |
| DD006 | Cardinality inconsistent with column constraints |
| DD007 | Column missing required representation key (`values`, `range`, or `examples`) |
| DD008 | Column has `units` but its type is not `number(quantity)` |
| DD009 | Document omits the recommended `$learn_more` key (warning) |

Test fixtures for these rules are in `crates/data-dict/tests/fixtures/{valid,invalid,lint}/`. Each fixture has a `# expected: ...` header documenting the intended outcome.

Diagnostic hints always start with a capital letter.

If a schema change causes `site/examples/` to fail, don't fix them. Instead report them to me so I can fix upstream.


## Data format

- Keys in `data-dict.yaml` use snake_case (e.g. `primary_key`, `foreign_key`, `$learn_more`).

## Prose

- Use sentence case for headings.

## Code

- Use nanoparquet for reading/writing parquet files (R code).
