---
title: "Design"
---

## Inspirations

`data-dict.yaml` draws inspiration from three lineages of work: data dictionaries, semantic models and data contracts.

## Data dictionaries

Data dictionaries come from the sciences, particularly the social sciences, where documenting datasets for reuse and secondary analysis has a long history. Survey research gave us standards like DDI, and fields with large shared datasets, like neuroimaging's BIDS, developed their own conventions. More recently, the Frictionless and Data Package standards brought the same ideas to machine-readable tabular data.

* [Data management in large-scale education research](https://datamgmtinedresearch.com/document#document-dataset)
* [Frictionless data](https://datapackage.org/standard/table-schema)
* [Data Package Standard](https://datapackage.org)
* [Brain Imaging Data Structure](https://bids.neuroimaging.io)
* [Data Documentation Initiative](https://ddialliance.org)

## Semantic models

Semantic models come from the data warehousing and business intelligence community. They describe data in business terms — dimensions, metrics, and the joins between tables — so that analysts and BI tools can write queries that return consistent answers, without needing to know the underlying schema.

* [Hex's semantic modelling](https://learn.hex.tech/docs/connect-to-data/semantic-models/semantic-authoring/modeling-specification)
* [Snowflake's semantic views](https://docs.snowflake.com/en/user-guide/views-semantic/overview)

It's worth noting that while semantic models influenced the design of `data-dict.yaml`, it is not a **[semantic model](semantic-models.md)**. It doesn't model dimensions or metrics, because that distinction reflects intended use, not the data itself.

Additionally, while terminology is still evolving, the "semantic" in semantic models is typically interpreted narrowly, focussing on structural semantics — what's needed for queries to return consistent values — rather than what the data actually _means_.

## Data contracts

Data contracts come from the data engineering community, where pipelines move data between teams. A contract is an agreement between producers and consumers about the shape and quality of a dataset, enforced automatically so that breaking changes are caught in CI rather than in downstream dashboards.

* [Soda's contract language](https://docs.soda.io/reference/contract-language-reference)
* [dbt tests](https://docs.getdbt.com/docs/build/data-tests?version=1.12)
