---
title: "data-dict.yaml"
---

`data-dict.yaml` is a data dictionary specification that describes a collection of related tables: their contents, constraints, connections, and the specialised vocabulary you need to understand them. It is designed to be a living document that tracks your understanding of a dataset as it evolves.

`data-dict.yaml` is designed to be lightweight. It doesn't attempt to precisely describe every possible type of metadata in a machine-readable way. Instead it focuses on precisely recording the most important components, leaving the remainder to plain text fields that require a human or agent to interpret. This means that `data-dict.yaml` doesn't itself do **data cleaning**, but it is a useful complement to tools that do.

You can read the details of the spec in [the specification](spec.md), or dive in by looking at a few [examples](examples/index.qmd):

* [dabstep](examples/dabstep.qmd)
* [elevators](examples/elevators.qmd)
* [foodbank](examples/foodbank.qmd)
* [loan-application](examples/loan-application.qmd)
* [otters](examples/otters.qmd)

## Why `data-dict.yaml`?

There have been many previous attempts to encode data dictionaries in structured text. What makes `data-dict.yaml` different? Why revisit this problem now?

* The costs of creating a data dictionary are lower than ever before because AI agents can automate much of the boilerplate, including porting documentation from existing unstructured formats (e.g. `.doc`, `.html`, `.pdf`).
* The benefits of creating a data dictionary are higher, because AI agents need the context that currently exists only in your head. As a very pleasant side-effect, this also helps your human colleagues, particularly those who are newer to your organisation.
* LLMs change what it means for something to be machine-readable. While we explicitly encode the most important structures, we can leave the more unusual quirks to free-form text.
* Unlike previous data dictionaries, we assume data is stored in parquet files or database tables. This means that many parsing details are out of scope, radically simplifying the spec.
* The cost of describing the data semantics in multiple places (i.e. `data-dict.yaml` and data transformation code) is lower because an AI agent can easily keep both in sync.

## When should you use `data-dict.yaml`?

`data-dict.yaml` is designed to support a wide range of scenarios. You might use it:

* Before you have any data, as a way to be concrete about your goals and expectations. In the future, we plan to support [data simulation](https://github.com/tidyverse/data-dict/issues/20) so that you can generate a dummy dataset that's compatible with your data dictionary.
* When you first encounter a new dataset, as a way of recording what you learn, as you learn it. This can be particularly useful when you're working with public datasets, as an LLM might have more knowledge of the data than you.
* Retrospectively, when you've already spent a bunch of time with a dataset. Creating a `data-dict.yaml` gets the knowledge out of your head and makes it accessible to your human and AI collaborators. And you can get started quickly by asking an LLM to extract what it knows based on the code you've written so far.
* When you're working with regularly updated data produced by someone else. Maybe you get updates from your collaborators via email, or maybe your data engineering team has a bad habit of not telling you when they update variable definitions. `data-dict.yaml`'s ability to validate data against the spec ensures that you're never surprised when the data changes.

## Inspirations

Here are a few of the resources that guided the design of `data-dict.yaml`:

* [Data management in large-scale education research](https://datamgmtinedresearch.com/document#document-dataset)
* [Frictionless data](https://datapackage.org/standard/table-schema)
* [Hex's semantic modelling](https://learn.hex.tech/docs/connect-to-data/semantic-models/semantic-authoring/modeling-specification)
* [Snowflake's semantic views](https://docs.snowflake.com/en/user-guide/views-semantic/overview)
* [Soda's contract language](https://docs.soda.io/reference/contract-language-reference)
* [dbt tests](https://docs.getdbt.com/docs/build/data-tests?version=1.12)

It's worth noting that while semantic models influenced the design of `data-dict.yaml`, it is not a **[semantic model](semantic-models.md)**. This means it doesn't think about dimensions or metrics, because that distinction reflects intended use, not the data itself. It's primarily designed to support data scientists, not data analysts.

Additionally, while terminology is still evolving, the "semantic" in semantic models is typically interpreted narrowly, focussing on structural semantics (what's needed for queries to return consistent values) not what the data actually _means_.

## Missing features

`data-dict.yaml` currently ships a validator (see the [CLI](https://github.com/tidyverse/data-dict#readme)) that checks a dictionary against the spec and against the underlying data. We plan to add more tooling in the future:

* **User facing documentation**: There's currently no way to turn your `.yaml` file into attractive HTML documentation of your data. If you've put the time into maintaining an accurate data dictionary, we want to make it easy to turn it into a beautiful website that you can share with your colleagues.

* **Large tables**: A standalone `data-dict.yaml` is not designed for hundreds of tables or hundreds of columns. We also plan to provide tools that allow you to aggregate multiple dictionaries and index larger data catalogs.

* **Export**: Export your data dictionary to other formats like csv, excel, and googlesheets.
