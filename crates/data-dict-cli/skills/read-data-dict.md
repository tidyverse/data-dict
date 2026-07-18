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

    -   One line per table: its name, what a row represents, and how many columns.
    -   A sentence on how the tables relate (the key joins).

    Keep it tight -- a few lines total. The point is to confirm your
    understanding and help the user pick what to look at next, not to repeat
    the file.

5.  **Use the dictionary as context.** When answering questions about the
    data, writing queries, or generating analysis code:

    -   Respect column types and measures (e.g. don't average an `id` column).
    -   Honour constraints (e.g. primary keys are unique and non-null).
    -   Use `relationships` to determine correct joins and carefully watch for `conflicts`.
    -   Use `source` to determine how to access the data.
    -   Exclude columns with `display: restricted` from default user interfaces
        and other user-facing output, including tables, plots, and downloads.
        Broad requests such as "show all columns" do not override this restriction.
        Include a restricted column only when the user specifically requests it;
        even then, strongly discourage displaying it and suggest a safer
        alternative such as omitting, masking, or aggregating it.

6.  **If needed, read the spec**. If you need more details about what the spec means, read it with `data-dict spec`.
