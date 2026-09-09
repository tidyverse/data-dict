---
title: "Quickstart"
---

In this quickstart you'll create a data dictionary for a real dataset — frog jumping records from the Calaveras County Jumping Frog Jubilee — by asking an AI agent to do the heavy lifting, then checking and rendering the result yourself. It takes about ten minutes.

## Setup

1. [Install the `data-dict` CLI](install.md).
2. Clone the data repo: `git clone https://github.com/hadley/frog-jumping`. It contains the data (`frogs.parquet`) and existing documentation (`data-collection.md`).

## Ask an agent to draft the dictionary

The CLI ships with a skill that teaches AI agents how to create a data dictionary, so all you need to get started is to point your agent to the data and any existing documentation:

> Use the data-dict cli to document the frogs dataset. You can find existing documentation in data-collection.md

Behind the scenes, the agent will:

1. Read the creation skill with `data-dict skill-create`.
2. Run `data-dict draft frogs.parquet` to profile the data and generate a starting `data-dict.yaml`, with inferred types, observed ranges, and a `todo` note for everything only a human can decide.
3. Work through the todos, mining `data-collection.md` for descriptions, units, and glossary terms — and asking you about anything it can't determine.

## No agent? No problem!

Call `data-dict draft frogs.parquet` yourself then work through the file, filling in the `todo` items that data-dict created for you.

## Check the dictionary against the data

However the dictionary was created, verify that the data agrees with it:

```sh
data-dict validate-data data-dict.yaml
```

This checks column names and types, ranges, and any constraints that you've declared (such as uniqueness or primary key-foreign key relationships).

## Render it as a website

```sh
data-dict render-spec data-dict.yaml
```

This produces a self-contained HTML page — tables, columns, relationships, and glossary — that you can share with anyone. See the [rendered otters dictionary](examples/rendered/otters.html) for what the output looks like.

## Next steps

* Read the [specification](spec.md) for everything a dictionary can express.
* Learn what the validators check in [validation](validate.md).
* Browse more [examples](examples/index.qmd).
