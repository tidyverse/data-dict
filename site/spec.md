# data-dict.yaml

This document describes version **0.1.0** of the `data-dict.yaml` specification.

A data dictionary has two kinds of top-level keys. `$`-prefixed metadata keys that describe the dictionary itself and the data keys that describe the data. The `$` prefix marks a key as meta, distinguishes it from content, and keeps these keys grouped at the top of the file. 

The metadata keys are:

* `$version` (required): the version of the `data-dict.yaml` spec the document conforms to. Currently `0.1.0`. While the spec is pre-1.0, breaking changes are expected, but once the spec stabilises at 1.0, breaking changes will always increment at least the minor version.
* `$learn_more` (optional, but recommended): a URL where readers can learn about the `data-dict.yaml` format, so that people and tools meeting the file for the first time can find out what it is. Use <http://data-dict.tidyverse.org/>. Omitting it is valid, but a validator will emit a warning rather than an error (see [Validation](validation.md)).

The content keys all hold the actual information about the data:

* [`tables`](#tables) is where the bulk of most data-dict.yaml files will be. It describes the tables and their columns.
* [`relationships`](#relationships) describes the relationships between tables. It gives the details you need to safely create joins.
* [`glossary`](#glossary) provides a place to define important domain-specific terms. This is a good place to write down those special words that your company loves to use.
* [`version`](#version) records the version of the data the dictionary describes — a version number, a date, or an opaque hash.

## Tables

`tables` is a named list that describes each table in the dataset. Each key is the table's name, which must be non-empty and unique. Each table represents a rectangle of data with observations in the rows and variables in the columns. Each table has the following properties:

* `description`: a human-readable description of the table. May contain markdown, and is usually a few sentences or a paragraph. A good description answers two questions:
    * **What's the grain?** What does a row represent? (e.g. "each row is a food item", "each row is one patient visit").
    * **What's the population?** What's been included or filtered out to produce this dataset? (e.g. "only completed orders from 2020 onwards", "excludes test accounts").
* `details`: additional information about the table. This is the place for "here be dragons": assumptions baked into the data, known weak spots, surprising calculations, and known problems. Also covers how the data was collected or constructed. Can be any length.
* `source`: ways to access the data. Optional at the spec level, so you can draft a dictionary before its data exists, but required to validate against data (see [Validation](validation.md)).
* `columns` (required): an ordered list of column metadata.

For example:

```yaml
tables:
  food:
    description: >
      Each row is a food item in the USDA FoodData Central database.
      Includes both branded and foundation foods.
    source:
      parquet: inst/parquet/food.parquet
    columns:
      - name: fdc_id
        type: number(id)
        constraints: [primary_key]
        description: Unique identifier for the food item.
        examples: [167512, 174231, 325871, 534109, 715322]
      - name: description
        type: string
        constraints: [required]
        description: Full text description of the food.
        examples: [Hummus, Egg rolls, Cheese spread, Grapes, Pickle relish]
      - name: food_category_id
        type: number(id)
        constraints: [foreign_key]
        description: Links to the food_category table.
        examples: [9, 11, 14, 18, 25]
      - name: data_type
        type: enum
        values: [foundation, branded]
        description: Whether the food is a foundation or branded food.
```

### Source

`source` describes how to access the table's data. It's a map whose keys describe the access method and whose values give the location. Currently the only supported key is `parquet`:

```yaml
source:
  parquet: inst/parquet/food.parquet
```

* `parquet`: path to a Parquet file (may include globs).

Parquet is the only source `data-dict` can currently validate against, so it's the only one the spec defines. We expect to add more access methods in the future — most importantly `SQL` (a schema-qualified table name such as `foodbank.food`, or a full `SELECT` query), and likely others such as R, Python, and Posit Connect pins.

`source` is optional while you're only validating the spec, letting you sketch a table before its data exists. But the metadata and data levels validate the dictionary against real data, so they require every table they check to declare a `source`.

### Columns

Each entry in the `columns` list is a column descriptor. Columns are matched to the underlying data by `name`, so the order in which you list them does not need to match the column order in the data.

Each descriptor has the following properties:

* `name` (required): column name. Used to match the descriptor to a column in the underlying data. Must be non-empty and unique within a table.
* `type`: the column's data type (see [Types](#types)). Should match (approximately) the underlying data type. Optional — see below.
* `constraints`: a list of column-level constraints (see [Column constraints](#column-constraints)).
* `description`: a human-readable description of the column. Can use markdown.
* `details`: additional information about the column, e.g. how it was computed or edge cases to watch out for. Can be any length.
* `units`: the unit of measurement, for `number(quantity)` columns only (see [Measures](#measures)).

A column also carries one of `values`, `range`, or `examples`, which represents the data it contains. Which one is determined by its `type` (see [Types](#types)).

A column may also be listed with only its `name` and no `type`. This acknowledges the column without describing it and you should use it for columns that you don't care about but don't want flagged as undocumented. Such a column makes no claims about its contents, so it's never check, but it must still exist in the data.

#### Description & details

The `description` and `details` are free text fields that humans and agents can use to jot down important notes. The `description` should be short, typically a few sentences or at most a paragraph and will be displayed in user interfaces. The `details` can be any length, and is a good place to carefully record all the details of the table.

#### Types

Types capture data types at a level that makes sense for analysis, which is typically coarser than the logical types of the underlying data.

The supported types are:

* `number`: numeric values (integers or floating-point). Can be qualified with a measure in parentheses: `number(id)`, `number(ordinal)`, or `number(quantity)`. See [Measures](#measures).
* `string`: UTF-8 text strings.
* `boolean`: true/false values.
* `date`: calendar dates, written as ISO 8601 strings (`YYYY-MM-DD`, e.g. `2024-01-31`).
* `datetime`: date-times with timezone, written as ISO 8601 strings (e.g. `2024-01-31T09:30:00Z`).
* `enum`: a column with repeated values from a known set. The allowed values are listed in the `values` property.

Every type *except* `boolean` has some way of representing the data it contains: an exhaustive set of values, a range, or a handful of examples. Each such column carries exactly one of the following three properties, and which one is determined by the column's `type`:

* `values`: the allowed values for an `enum` column. Can be a list (`[M, F, U]`) when values are self-explanatory, or a map (`{M: Male, F: Female, U: Unknown}`) when values need labels. The values themselves must be scalars (string, number, or boolean); in the map form the labels must be strings. (`boolean` columns implicitly have `values: [true, false]`, no need to explicitly include it.)
* `range`: a two-element list `[min, max]` giving the inclusive minimum and maximum *observed* in the column. Like `examples`, it describes the data rather than constraining it — a value outside the range will generate a warning, not a validation error. Used for the ordered numeric and temporal types: `number(ordinal)`, `number(quantity)`, `date`, and `datetime`. Both elements must match the column's type, and the minimum must not exceed the maximum.
* `examples`: a list of ~5 representative values from the column. Used for all other types: `string`, `number`, and `number(id)`. Each example must match the column's type. A handful of concrete examples helps LLMs understand the column far better than a description alone. For instance, knowing that an id column holds `[1, 2, 3, 4, 5]` versus `[10000, 1235452, 234234]`. A good baseline is to select 5 evenly spaced values along the sorted unique values, and then add any particularly surprising values as you encounter them.

#### Measures

The `number` type can be qualified with a measure in parentheses that classifies what operations are meaningful:

| Type | Can compare | Can average | Can sum | Examples |
|------------|-------------|-------------|---------|----------|
| `number(id)` | No | No | No | primary keys, foreign keys, codes |
| `number(ordinal)` | Yes | No | No | ranks, years, sequence numbers |
| `number(quantity)` | Yes | Yes | Yes | weights, counts, amounts |

A `number(quantity)` column can also declare its `units`: a free-text string naming the unit of measurement, such as `kg`, `USD`, or `seconds`. Units are only meaningful for quantities — they're how you tell apart two columns that share a `range` but measure different things — so `units` is an error on any other type.

```yaml
- name: mass
  type: number(quantity)
  units: g
  range: [0, 5000]
```

#### Column constraints

The `constraints` property is a list of constraint names. The supported constraints are:

* `primary_key`: the set of columns with the `primary_key` constraint uniquely identifies each row. Implies `required` and `unique`.
* `foreign_key`: the column references a primary key in another table (or in the current table, if a self-join). The specific relationship is defined in [`relationships`](#relationships).
* `required`: the column does not contain null/missing values.
* `unique`: the column's values are distinct (no duplicates).

## Relationships

`relationships` is a list of join descriptors. Each entry describes how two tables are related.

* `cardinality` (required): either `one-to-one`, `one-to-many`, or `many-to-one`. Describes the relationship from the left table to the right table in the join expression.
* `join` (required): a join expression of the form `table1.column = table2.column`, or `table1.date >= table2.start AND table1.date <= table2.end`.
* `description`: human-readable description of the relationship. Only needed if it's not clear from the context.
* `conflicts`: a list of column names that appear in both tables with different meanings. These fields would cause ambiguity in a join and may need to be renamed or dropped.

For example:

```yaml
relationships:
    cardinality: many-to-one
    join: food.food_category_id = food_category.id
    conflicts: [description]
```

## Glossary

`glossary` is a map from term to definition. Each entry provides a plain-language definition of a domain-specific term used in the table or column descriptions, or is likely to be used by a domain expert working with this data.

```yaml
glossary:
  foundation food: >
    A food whose nutrient and food component values are derived
    primarily by chemical analysis.
```

## Version

`version` records the version of the data this dictionary describes, so people and tools can tell two snapshots of the data apart and know which one a given dictionary goes with. (This is distinct from `$version`, which records the version of the *spec* the document conforms to.)

`version` is optional. It's a map with exactly one of three keys, which names both the kind of version and its value:

* `number`: a hand-curated version number, such as `1.2.0`.
* `date`: a release date in ISO 8601 form (`YYYY-MM-DD`), such as `2024-01-31`, for data refreshed on a schedule.
* `hash`: an opaque identifier, such as `a1b2c3d`, derived from the data itself.

`data-dict` checks that exactly one key is present and that a `date` is a valid ISO 8601 date, but otherwise treats the version as opaque.

```yaml
version:
  date: 2024-01-31
```
