---
name: read-data-dict
description: >-
  Read and understand a data-dict.yaml file so you can use it as context when
  querying, analysing, or writing code against the dataset it describes. Use
  whenever you start working with a dataset that has a data dictionary.
---

# Read a data dictionary

Read and understand a `data-dict.yaml` file so you can use it when working with
the dataset it describes.

## Steps

1.  **Find the data dictionary.** Look for a file called `data-dict.yaml` in
    the project root or the same directory as the dataset. If there isn't one,
    tell the user and stop.

2.  **Read the file.** Parse the YAML and familiarise yourself with its three
    top-level sections:

    -   `tables` -- the tables, their columns, types, constraints, and
        descriptions.
    -   `relationships` -- how the tables join together, including cardinality
        and any column-name conflicts.
    -   `glossary` -- domain-specific terms and their definitions.

3.  **Internalise the glossary first.** The glossary defines the vocabulary
    used throughout the rest of the file. Read it before interpreting column
    descriptions so that domain terms are understood in context.

4.  **Briefly summarise the tables.** Once you've read the file, give the user
    a short orientation rather than dumping the whole dictionary back at them:

    -   One line per table: its name, what a row represents, and roughly how
        many columns it has.
    -   A sentence on how the tables relate (the key joins).

    Keep it tight -- a few lines total. The point is to confirm your
    understanding and help the user pick what to look at next, not to repeat
    the file.

5.  **Use the dictionary as context.** When answering questions about the
    data, writing queries, or generating analysis code:

    -   Respect column types and measures (e.g. don't average an `id` column).
    -   Honour constraints (e.g. primary keys are unique and non-null).
    -   Use `relationships` to determine correct joins and carefully watch for
        `conflicts`.
    -   Use `source` to determine how to access the data.
