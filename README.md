# `data-dict.yaml`

`data-dict.yaml` is a lightweight, YAML-based data dictionary specification. It
describes a collection of related tables — their columns, types, constraints,
relationships, and the domain vocabulary you need to understand them — in a
single file that humans and AI agents can co-author and keep in sync with your
data.

**Full documentation, including the detailed specification, lives at
[data-dict.tidyverse.org](https://data-dict.tidyverse.org).**

This repo contains two things:

* **The specification** — the prose definition of the format, in
  [`site/spec.md`](site/spec.md) (rendered at
  [data-dict.tidyverse.org](https://data-dict.tidyverse.org)).
* **The CLI** — a Rust command-line tool that validates a `data-dict.yaml`
  file against the spec and against the underlying data.

See [`examples/`](examples/) for complete data dictionaries, or the
[overview](https://data-dict.tidyverse.org) for the motivation behind the
project.

## The CLI

The `data-dict` CLI validates dictionaries. It can:

* Check that a file is structurally valid and internally consistent
  (`validate-schema`).
* Compare a dictionary against a real Parquet file to confirm the data matches
  what the dictionary claims (`parquet validate`).
* Print the column types of a Parquet file (`parquet types`).

### Install

Build and install from source with [Cargo](https://rustup.rs):

```bash
cargo install --git https://github.com/tidyverse/data-dict data-dict-cli
```

Or clone the repo and build locally:

```bash
git clone https://github.com/tidyverse/data-dict.git
cd data-dict
cargo build --workspace --release
# binary is at target/release/data-dict
```

### Usage

```bash
# Validate a dictionary against the spec
data-dict validate-schema examples/otters.yaml

# Validate a dictionary against a parquet file
data-dict parquet validate dict.yaml data/food.parquet --table food

# Inspect a parquet file's column types
data-dict parquet types data/food.parquet
```

## Development

This is a Rust workspace with three crates:

* `crates/data-dict/` — core library: YAML parsing, schema validation, lowering
  to a typed model, and semantic linting.
* `crates/data-dict-cli/` — thin CLI wrapper.
* `crates/data-dict-parquet/` — reads Parquet schemas and maps column types to
  data-dict types.

```bash
cargo build --workspace
cargo test --workspace
```

The website is a [Quarto](https://quarto.org) project in [`site/`](site/), published automatically to [data-dict.tidyverse.org](https://data-dict.tidyverse.org) on every push to `main`.
