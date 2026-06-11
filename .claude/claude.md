# data-dict.yaml

`data-dict.yaml` is a YAML-based data dictionary specification and validator. It describes collections of related tables — their columns, types, constraints, relationships, and domain vocabulary — and is designed to be co-written by humans and AI agents. The main deliverable is a CLI tool that validates data dictionary YAML files against the spec.

The repo contains:

- `spec.md`: the full specification (v0.1.0)
- `examples/`: example data dictionaries downloaded from other repos (see `download-examples.R`). Do not edit these directly — they are overwritten on each sync. When spec changes affect example documents, note that examples will need to be re-synced from their source repos.
- `README.md`: project overview and motivation
- `rust-cli-design.md`: architectural decisions and rationale
- `crates/`: Rust workspace (see crate architecture below)
- `schema.yaml`: JSON Schema for structural validation of data dictionary files

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
cargo run -p data-dict-cli -- validate-schema examples/otters.yaml
```

To review/accept insta snapshots: `cargo insta review`.

## Crate architecture

Rust workspace with three crates:

- `crates/data-dict/` — core library: YAML parsing, schema validation, lowering to typed model, and semantic linting. All logic lives here.
- `crates/data-dict-cli/` — thin CLI wrapper (single command: `validate-schema`). Keep it thin.
- `crates/data-dict-parquet/` — reads Parquet file schemas and maps column types to data-dict types.

### Schema validation pipeline

```
YAML file
  → quarto_yaml: parse to AST with source spans
  → structural validation against schema.yaml (embedded via include_str!)
  → lower.rs: AST → typed model (DataDict, Table, Column, Relationship, ...)
  → lint.rs: semantic rules DD001–DD007
  → Result<(), Vec<Diagnostic>>
```

### Lint rules (DD001–DD007)

| Rule | Description |
|------|-------------|
| DD001 | `foreign_key` column has no matching relationship with `primary_key` |
| DD002 | Relationship references non-existent table |
| DD003 | Relationship references non-existent column |
| DD004 | `join` expression fails to parse or references wrong number of tables |
| DD005 | Column in `conflicts` doesn't appear on both sides of the join |
| DD006 | Cardinality inconsistent with column constraints |
| DD007 | Column missing required representation key (`values`, `range`, or `examples`) |

Test fixtures for these rules are in `crates/data-dict/tests/fixtures/{valid,invalid,lint}/`. Each fixture has a `# expected: ...` header documenting the intended outcome.

## Prose

- Use sentence case for headings.

## Code

- Use nanoparquet for reading/writing parquet files (R code).
