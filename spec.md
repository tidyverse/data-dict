# data-dict.yaml

This document describes version **0.1.0** of the `data-dict.yaml` specification.

A data dictionary has one required top-level key, `version`, plus three optional keys that hold the actual content:

* `version` (required): the version of the `data-dict.yaml` spec this document conforms to. Currently `0.1.0`.
* [`tables`](#tables) is where the bulk of most data-dict.yaml files will be. It describes the tables and their columns.
* [`relationships`](#relationships) describes the relationships between tables. It gives the details you need to safely create joins.
* [`glossary`](#glossary) provides a place to define important domain-specific terms. This is a good place to write down those special words that your company loves to use.

While the spec is pre-1.0, breaking changes between versions should be expected. Once the spec stabilises at 1.0, the major version will only change on breaking changes.

## Tables

`tables` is a named list that describes each table in the dataset. Each table represents a rectangle of data with observations in the rows and variables in the columns. Each table has the following properties:

* `description` (required): a human-readable description of the table. May contain markdown, and is usually a few sentences or a paragraph.
* `details`: additional information about the table, e.g. how it was collected, constructed, or any important caveats for its use. Can be any length.
* `source` (required): ways to access the data.
* `columns` (required): an ordered list of column metadata.
* `constraints`: a list of table-level assertions.

For example:

```yaml
tables:
  food:
    description: >
      Each row is a food item in the USDA FoodData Central database.
      Includes both branded and foundation foods.
    source:
      parquet: inst/parquet/food.parquet
      R: foodbank::food
      SQL: foodbank.food
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

`source` is a map whose keys name the access method and whose values give the location. For example:

```yaml
source:
  parquet: inst/parquet/food.parquet
  R: foodbank::food
  SQL: foodbank.food
```

The currently supported keys are:

* `parquet`: path to a Parquet file (may include globs).
* `SQL`: a (possibly schema-qualified) table name (e.g. `food` or `foodbank.food`) or a full `SELECT` query.
* `R` and `Python`: R or Python code that returns the data (e.g. `foodbank::food`, or `read.csv("food.csv", comment.char = "#")`).
* `pin`: the name of a Posit Connect pin.

This variety of source types reflects the variety of ways which you might retrieve a dataset. It's good practice to upstream as much of this processing as possible so that over time you exclusively use `parquet` or `SQL` with a table.

### Columns

Each entry in the `columns` list is a column descriptor with the following properties:

* `name` (required): column name. Must match the column name in the underlying data.
* `type`: the column's data type. Must match (approximately) the underlying data type (see [Types](#types)).
* `constraints`: a list of column-level constraints (see [Column constraints](#column-constraints)).
* `description` (required): a human-readable description of the column. Can use markdown.
* `details`: additional information about the column, e.g. how it was computed or edge cases to watch out for. Can be any length.

A column also carries one of `values`, `range`, or `examples`, which represents the data it contains. Which one is determined by its `type` (see [Types](#types)).

#### Description & details

The `description` and `details` are free text fields that humans and agents can use to jot down important notes.

The `description` is required, and typically a few sentences or at most a paragraph. It's a good place to document the most important information about the column. It will be displayed in user interfaces.

The `details` are optional, can be any length, and is a good place to carefully record all the details of the table.

#### Types

Types capture data types at a level that makes sense for analysis, which is typically coarser than the logical types of the underlying data.

The supported types are:

* `number`: numeric values (integers or floating-point). Can be qualified with a measure in parentheses: `number(id)`, `number(ordinal)`, or `number(quantity)`. See [Measures](#measures).
* `string`: UTF-8 text strings.
* `boolean`: true/false values.
* `date`: calendar dates.
* `datetime`: date-times with timezone.
* `enum`: a column with repeated values from a known set. The allowed values are listed in the `values` property.

Every type has some way of representing the data it contains: an exhaustive set of values, a range, or a handful of examples. Each column therefore carries exactly one of the following three properties, and which one is determined by the column's `type`:

* `values`: the allowed values for an `enum` column. Can be a list (`[M, F, U]`) when values are self-explanatory, or a map (`{M: Male, F: Female, U: Unknown}`) when values need labels. (`boolean` columns implicitly have `values: [true, false]`, no need to explicitly include it.)
* `range`: a two-element list `[min, max]` giving the inclusive range. Used for the ordered numeric and temporal types: `number(ordinal)`, `number(quantity)`, `date`, and `datetime`.
* `examples`: a list of ~5 representative values from the column. Used for all other types: `string`, `number`, and `number(id)`. A handful of concrete examples helps LLMs understand the column far better than a description alone. For instance, knowing that an id column holds `[1, 2, 3, 4, 5]` versus `[10000, 1235452, 234234]`. A good baseline is to select 5 evenly spaced values along the sorted unique values, and then add any particularly surprising values as you encounter them.

#### Measures

The `number` type can be qualified with a measure in parentheses that classifies what operations are meaningful:

| Type | Can compare | Can average | Can sum | Examples |
|------------|-------------|-------------|---------|----------|
| `number(id)` | No | No | No | primary keys, foreign keys, codes |
| `number(ordinal)` | Yes | No | No | ranks, years, sequence numbers |
| `number(quantity)` | Yes | Yes | Yes | weights, counts, amounts |

#### Column constraints

The `constraints` property is a list of constraints. Each entry is either a **structural constraint** (a bareword naming a structural or relational fact about the column) or an **assertion** (a map carrying an expression that must hold for the data).

The structural constraints are:

* `primary_key`: the set of columns with the `primary_key` constraint uniquely identifies each row. Implies `required` and `unique`.
* `foreign_key`: the column references a primary key in another table. The specific relationship is defined in [`relationships`](#relationships).
* `required`: the column does not contain null/missing values.
* `unique`: the column's values are distinct (no duplicates).

An assertion is a map with an `assert` key holding a boolean expression that must be true for every row, plus an optional `description`:

```yaml
columns:
  - name: postcode
    type: string
    constraints:
      - required
      - assert: LENGTH(postcode) <= 10
```

Bare column names in the expression refer to columns of the same table, so a column assertion may relate its column to any sibling. See [Assertions](#assertions) for the expression grammar.

Note that `values` and `range` (see [Types](#types)) already express membership and bounds constraints — `values` restricts an `enum` to its listed set, and `range` bounds an ordered column — so you don't need an assertion to repeat them.

### Table constraints

A table's `constraints` property is a list of assertions, using exactly the same form as a [column assertion](#column-constraints): a map with an `assert` key and an optional `description`. The only difference is scope — a table constraint isn't tied to a single column, so it's the natural home for rules that span columns:

```yaml
tables:
  survey:
    constraints:
      - assert: end_date >= start_date
        description: A contract can't end before it starts.
      - assert: NOT(q3) OR (q4 IS NOT NULL AND q5 IS NOT NULL)
        description: If q3 is true, q4 and q5 must be answered.
```

Table constraints can only carry assertions; the structural barewords (`primary_key`, `unique`, …) live on columns.

### Assertions

An `assert` expression is a single-table, row-level boolean expression written in a small SQL-like sublanguage. It is evaluated against every row, and the constraint holds when the expression is true for all of them. Bare names refer to columns of the table.

Assertions are deliberately **per-row and single-table**: an expression sees only the columns of one row at a time. There are no aggregates and no subqueries — cross-table rules belong in [`relationships`](#relationships), and the per-row restriction keeps assertions deterministic and cheap to check.

The supported grammar is:

* **Comparison:** `=`, `!=` / `<>`, `<`, `<=`, `>`, `>=`
* **Logic:** `AND`, `OR`, `NOT`, parentheses for grouping
* **Null tests:** `IS NULL`, `IS NOT NULL`
* **Membership:** `x BETWEEN lo AND hi`, `x IN (...)`, `x NOT IN (...)`
* **Pattern matching:** `LIKE` / `NOT LIKE` (with `%` and `_` wildcards) and `SIMILAR TO` (regular expressions)
* **Conditional:** `CASE WHEN ... THEN ... ELSE ... END`
* **String functions:** `LENGTH`, `LOWER`, `UPPER`, `TRIM`, `STARTS_WITH`, `ENDS_WITH`
* **Numeric functions & arithmetic:** `ABS`, `ROUND`, `FLOOR`, `CEIL`, `MOD`, and `+ - * /`
* **Date/time:** `NOW()`, `interval(<n>, <unit>)`, and arithmetic between dates and intervals
* **Column selection:** `COLUMNS(...)`, to apply one predicate to many columns at once (see below)

Assertions state what must be **true**, so conditional rules are written as implications, e.g. `NOT(q3) OR q4 IS NOT NULL` or `NOT(q3 AND q4 IS NULL)`.

#### Selecting multiple columns

To apply the same predicate to a group of columns without repeating it, an assertion may use a `COLUMNS(...)` expression — a simple subset of [DuckDB's `COLUMNS`](https://duckdb.org/docs/current/sql/expressions/star). The supported forms select columns by:

* `COLUMNS(*)`: all columns in the table.
* `COLUMNS('<regex>')`: columns whose name matches the regular expression.
* `COLUMNS([a, b, c])`: an explicit list of column names.

The enclosing expression is evaluated once per selected column, and the assertion holds only when it is true for **every** selected column (the results are combined with `AND`). So:

```yaml
constraints:
  # Every q4–q8 answer is present whenever q3 is true.
  - assert: NOT(q3) OR COLUMNS('q[4-8]') IS NOT NULL
    description: q4–q8 must be answered when q3 is true.
  # No column anywhere in the table is null.
  - assert: COLUMNS(*) IS NOT NULL
```

The lambda form (`COLUMNS(c -> ...)`) and the star modifiers (`EXCLUDE`, `REPLACE`, `RENAME`) are **not** supported.

## Relationships

`relationships` is a list of join descriptors. Each entry describes how two tables are related.

* `description` (required): human-readable description of the relationship.
* `cardinality` (required): either `one-to-one`, `one-to-many`, or `many-to-one`. Describes the relationship from the left table to the right table in the join expression.
* `join` (required): a join expression of the form `table1.column = table2.column`, or `table1.date >= table2.start AND table1.date <= table2.end`.
* `conflicts`: a list of column names that appear in both tables with different meanings. These fields would cause ambiguity in a join and may need to be renamed or dropped.

For example:

```yaml
relationships:
  - description: Each food belongs to one food category; each category contains many foods.
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
