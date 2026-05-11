# data-dict.yaml

`data-dict.yaml` is a data dictionary specification for describing collections of related tables: their contents, constraints, connections, and domain vocabulary. It's designed to be co-written by humans and AI agents.

The repo contains:

- `spec.md`: the full specification
- `examples/`: example data dictionaries downloaded from other repos (see `download-examples.R`). Do not edit these directly — they are overwritten on each sync. When spec changes affect example documents, note that examples will need to be re-synced from their source repos.
- `README.md`: project overview and motivation

## Prose

- Use sentence case for headings.

## Code

- Use nanoparquet for reading/writing parquet files
