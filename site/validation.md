# Validation

## Three levels of validation

Validation happens at three levels, each a strict superset of the one before it:

* Validating the **spec** checks that the dictionary file itself conforms to the data-dict spec — that it is well-formed and internally consistent, with valid types, foreign keys that have matching relationships, joins that parse, and so on. These checks have an unambiguous right answer, so most are errors. This level looks only at the `data-dict.yaml` file, never at the data. This is performed by `data-dict validate-spec`.

* Validating the **metadata** checks that the data's column names and types match the dictionary. It reads only the data's metadata (for example, a Parquet file's footer), not its values, so it stays cheap. This is performed by `data-dict validate-meta`.

* Validating the **data** checks that the data's values match the dictionary — that required columns have no nulls, and so on. This is the only level that reads the data itself, so it can be expensive, depending on the data source. This is performed by `data-dict validate-data`.

The last two levels compare the dictionary against the data (or equivalently, the data against the dictionary). When they disagree, we can't tell which side needs to change. If you're creating the dictionary as you learn about the data, then you might need to change the dictionary. If you're using the dictionary to validate a dataset, there might be an upstream issue that you need to resolve.

Each level implies the ones before it: validating the metadata validates the spec first, and validating the data validates both the spec and the metadata first. Validating the spec and metadata are cheap, so they can be run continually while you edit the `data-dict.yaml`; validating the data adds a full scan and get more expensive as the size of the data increases.

Each check has a code prefixed by its level: spec checks are `S01`, `S02`, …; metadata checks `M01`, …; data checks `D01`, …. Severity is independent of level — any level can raise errors or warnings.

## Errors vs warnings

A validator reports two severities of problem: **errors** and **warnings**. The distinction is about urgency, not importance.

* An **error** means the dictionary is invalid or there's a critical mismatch between the data and dictionary. Errors will cause a production pipeline to fail, and you must fix them immediately.

* A **warning** means the dictionary is usable but the data and dictionary may have drifted apart. Warnings will not cause a production pipeline to fail, but if you're actively working on the project you should make sure to fix them.

## Spec-validation checks

When validating the spec, each problem with the dictionary is one of:

* **Unresolved foreign key** (S01, error): a `foreign_key` column has no `relationships` entry pointing it at a `primary_key` column.
* **Unknown table** (S02, error): a relationship references a table that is not defined in `tables`.
* **Unknown column** (S03, error): a relationship references a column that does not exist on its table.
* **Invalid join** (S04, error): a `join` expression fails to parse, or references neither one (self-join) nor two tables.
* **Unresolved conflict column** (S05, error): a name in `conflicts` is not a column on both sides of the join.
* **Inconsistent cardinality** (S06, error): the declared cardinality is inconsistent with the constraints on the joined columns (e.g. `one-to-many` whose "one" side is not `primary_key` or `unique`).
* **Wrong representation key** (S07, error): a column's data representation key is absent or wrong for its type (`enum` → `values`; `number(ordinal)`, `number(quantity)`, `date`, `datetime` → `range`; otherwise → `examples`, except `boolean`, which needs none).
* **Units without quantity** (S08, error): a column has `units` but its type is not `number(quantity)`.
* **Missing `$learn_more`** (S09, warning): the document omits the recommended `$learn_more` key.
* **Duplicate column name** (S10, error): two column descriptors within the same table share a `name`.
* **Empty name** (S11, error): a table name or a column `name` is empty.

## Metadata-validation checks

When validating the data's metadata against the dictionary, each column mismatch is one of:

* **Type mismatch** (M01, error): a column's declared type is incompatible with the data.
* **Missing column** (M02, error): a column the dictionary describes is absent from the data. This applies even to columns listed by name only — listing a column that doesn't exist is an error.
* **Undocumented column** (M03, warning): a column present in the data that the dictionary does not describe. This is a warning, not an error: if a production pipeline adds a column, validation should not fail, but you should document it (or at least list it by name) next time you touch the dictionary.

## Data-validation checks

When validating the data's values against the dictionary, each column mismatch is one of:

* **Nulls in a required column** (D01, error): a `required` or `primary_key` column contains nulls.
