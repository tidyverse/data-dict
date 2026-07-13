---
title: "data-dict.yaml"
---

`data-dict.yaml` is a data dictionary specification that describes a collection of related tables: their contents, constraints, connections, and the specialised vocabulary you need to understand them. It is designed to be a living document, co-written by humans and agents, that tracks your understanding of a dataset as it evolves.

`data-dict.yaml` is designed to be lightweight. It doesn't attempt to precisely describe every possible type of metadata in a machine-readable way. Instead it focuses on precisely recording the most important components, leaving the remainder to plain text fields that require a human or agent to interpret. This means that `data-dict.yaml` doesn't itself do **data cleaning**, but it is a useful complement to tools that do.

You can read the details of the spec in [the specification](spec.md), or dive in by looking at a few [examples](examples/index.qmd):

* [dabstep](examples/dabstep.qmd)
* [elevators](examples/elevators.qmd)
* [foodbank](examples/foodbank.qmd)
* [loan-application](examples/loan-application.qmd)
* [otters](examples/otters.qmd)

## Why `data-dict.yaml`?

There are several ways a data dictionary can be structured and organized. For example, some open source standards and their files are:

* [Data Package Standard](https://datapackage.org) and its `datapackage.json` file.
* [Brain Imaging Data Structure](https://bids.neuroimaging.io) and its `dataset_description.json` file.
* [Data Documentation Initiative](https://ddialliance.org) and its `codebook.xml` file.

For less structured ways, many online resources suggest writing dictionaries in Excel or similar spreadsheets. For enterprise-level, proprietary dictionaries there are even more options targeting different industries, use-cases, and types of data. The main limitation of these approaches is that they aren't free.

So what does `data-dict.yaml` offer than these other formats don't offer?

* Is open source and free, which means it can be easily integrated into many different systems and workflows without vendor lock-in and expensive subscriptions.
* Uses the human-friendly, not just machine-friendly, YAML format. This makes it easy to read and write, and allows for comments and other human-readable features.
* Designed from the start to be built into a CLI tool, rather than just a set of conventions. This makes it easier to actually create, check, and maintain a data dictionary.
* Related to the above, the CLI has a command to check the metadata against the spec and the underlying data, to ensure that your dictionary is correctly formatted, consistent, and accurate.
* Contains fields for versioning of the data dictionary itself, and consequently its associated data. This provides a way to track what has changed over time and who is using which version of the data dictionary.
* Includes built-in support for documenting constraints, such as required values, a specific set of allowed values, or a range of allowed values. This allows for more precise documentation of the data. It also greatly simplifies the data quality process, as other dictionaries require an external tool or code to run check on the data against some set of assertions.
* Supports for documenting units of measurement, which is often forgotten about in other data dictionaries. This is critical to allowing for better re-use and interoperability.
* Supports precisely documenting relationships between tables, which is often missing in other data dictionaries.
* Includes a glossary field within the standard, to document potentially specialised vocabulary that is used in the data. 
* Is opinionated on using [Parquet](https://parquet.apache.org/) as the data format, which is a widely used and efficient columnar storage format. Parquet is a powerful, open data format, and being opinionated about this encourages data to be stored in it.

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
