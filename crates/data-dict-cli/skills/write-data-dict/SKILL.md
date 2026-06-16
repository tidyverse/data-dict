---
name: write-data-dict
description: >-
  Create or update a data-dict.yaml file describing a dataset's tables,
  columns, types, relationships, and glossary, following the data-dict spec.
  Use when asked to document a dataset or write/update a data dictionary.
---

# Write a data dictionary

Create or update a `data-dict.yaml` file for a dataset following the
[spec](https://data-dict.tidyverse.org/spec.html). Read the spec before you
start.

The two things that matter most, and where data dictionaries most often go
wrong, are **descriptions** and **types**. Spend your effort there:

-   **Descriptions** are the whole point. A column's `type` and `constraints`
    can be inferred from the data, but its *meaning* cannot. Every description
    must say something the data itself doesn't already tell you.
-   **Types must match the physical data exactly.** A type that contradicts the
    underlying column is worse than no type at all -- it will mislead every
    agent and query that trusts it. Verify, don't guess (see step 6).

## Steps

1.  **Discover the data.** Identify every table (parquet file, database table,
    or data-frame) in scope. For each table, read the schema to get column
    names and physical types. For parquet files, run
    `data-dict parquet types <file>` to see each column's physical type
    alongside the data-dict type it maps to -- start from that mapping rather
    than guessing.

2.  **Create the skeleton.** Start a `data-dict.yaml` with the three
    top-level keys: `tables`, `relationships`, and `glossary`.

3.  **Fill in each table.** For every table:

    a.  Write a `description`: a few sentences explaining what each row
        represents and where the data comes from.

    b.  Add a `source` map with the appropriate access methods (e.g.
        `parquet`, `SQL`, `R`, `Python`). You should only need to provide
        one by default.

    c.  For each column, create an entry with:

        -   `name`: must match the actual column name exactly.
        -   `type`: choose the analytical type that is *consistent with the
            physical type* of the column (`number`, `string`, `boolean`,
            `date`, `datetime`, `enum`, `enum<l1, l2, ...>`). Don't declare
            `number` for a column stored as text, or `date` for a free-text
            field. For numbers, add a measure when possible: `number(id)`,
            `number(ordinal)`, or `number(quantity)`.
        -   `constraints`: list any that apply (`primary_key`, `required`,
            `unique`, `foreign_key`).
        -   `description` (required): a clear explanation of what the column
            contains. This is the most valuable field -- explain units,
            meaning, and anything non-obvious. Don't just restate the column
            name ("the user id" for `user_id` adds nothing).
        -   `examples`: ~5 representative values, chosen by selecting evenly
            spaced values from the sorted unique values. Omit for enums with
            listed levels.

    d.  Add `details` to the table or any column where there are important
        caveats, edge cases, or methodology notes that don't fit in the
        description.

4.  **Define relationships.** For every foreign key, add a relationship entry
    with `description`, `cardinality` (`one-to-many` or `many-to-one`),
    `join`, and any `conflicts` (column names that appear in both tables with
    different meanings).

5.  **Build the glossary.** Add definitions for domain-specific terms used in
    descriptions. If a word would be unfamiliar to a new team member or an AI
    agent, define it. If you don't know what a term refers to, ask the user
    for clarification.

6.  **Verify against the data, and against the schema.** A data dictionary
    that disagrees with the data is actively harmful, so check it:

    -   Run `data-dict validate-schema data-dict.yaml` to confirm it is
        structurally valid and passes the lint rules.
    -   For each parquet table, run
        `data-dict parquet validate data-dict.yaml <file>` to confirm every
        declared type matches the physical data. Fix any mismatch it reports --
        change the `type`, not the data.
    -   Confirm by inspection: do all column names match? Do primary keys
        actually uniquely identify rows? Do foreign key values exist in the
        referenced table? Are `required` columns truly non-null?

## Style

-   Use YAML block scalars (`>` for wrapping, `|` for preserving newlines)
    for multi-line text.
-   Keep descriptions concise but precise. A few sentences is usually right.
-   For `enum` types with a small known set of values, list them inline:
    `enum<M, F, U>`. Use the `description` to explain what each level means
    using a markdown list.
-   Order columns in the same order they appear in the underlying data.
