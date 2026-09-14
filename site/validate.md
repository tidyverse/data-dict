# Validation

As well as describing what the data means, a data dictionary also describes what the data should look like: column types, which columns can contain missing values, and the rules the values must obey. `data-dict` can validate a dataset against these constraints, ensuring that the data is as you expect. This is very important for automated jobs, as it lets you quickly know if your assumptions about the data are outdated and that your code needs to be updated.

Technically there are three levels of validation:

* `data-dict validate-spec` checks the dictionary file itself — that it is well-formed (e.g. uses correct field names) and internally consistent (e.g. relationships refer to variables that exist). It never looks at the data.
* `data-dict validate-meta` checks that the data's column names and types match the dictionary. It reads only the data's metadata (for example, a Parquet file's footer), so is cheap to run.
* `data-dict validate-data` checks that the data's values match the spec. It reads the data itself, so its cost grows with the size of the data.

The first two levels are cheap enough to run continually while you edit a dictionary, and typically crop up while you're creating the dictionary. So in this document, we'll focus on data validation. We'll start by discussing the important difference between errors and warnings, and then go into constraints and assertions. We'll finish off by talking about the expression languages and how you can get a beautiful HTML validation report.

## Errors and warnings

A validation run reports two severities of problem. An **error** means the dictionary is invalid or there's a critical mismatch between the data and the dictionary: errors fail a production pipeline and must be fixed immediately. A **warning** means the dictionary is usable but the data and dictionary may have drifted apart: warnings don't fail a pipeline, but you should resolve them while you're actively working on the project.

For example, a column declared `required` that contains nulls is an error:

```text
error[D01]: A required column must not contain nulls.
  --> dict.yaml:10:23
   |
10 |   - name: t
...
21 |       - name: weight
22 |         type: number(quantity)
23 |         constraints: [required]
   |                       ^^^^^^^^ has 1 null value
```

Whereas a column that appears in the data but not in the dictionary is a warning. It's not going to break your code (since it's an addition not a change or deletion), but it indicates that the data has drifted away from the dictionary and you should update it the next time you work on the project.

```text
warning [M03]: Every column in the data should be described in the dictionary.
  `weight` is in the data (`number`) but not the dictionary
```

## Constraints and assertions

data-dict bundles four "named" constraints that are primarily focused on referential integrity. You can also express a wider range of assertions about the data using SQL-, R-, and Python-like languages, allowing polyglot teams to collaborate on a single file.

### Named constraints

There are four "named" constraints that state structural facts about a column, written as barewords in its `constraints` list:

* `primary_key`: the column (or the set of columns carrying it) uniquely identifies each row. Implies `required` and `unique`.
* `foreign_key`: the column references a primary key in another table, i.e. every value in this column must be the value of a `primary_key` column in another table.
* `required`: the column contains no nulls.
* `unique`: the column's values are distinct.

An `enum`'s [`values`](spec.md#representative-values) act as a constraint too: every value in the column must be one of them.

### Assertions {#assertions}

The named constraints are extremely important, but also quite coarse. There's a wider range of facts you might want to state about the data, so data-dict supplements these constraints with a flexible expression syntax that can apply to an individual [column](spec.md#column-constraints) or to multiple columns within a [table](spec.md#table-constraints):

```yaml
tables:
  - name: contract
    columns:
    - name: postcode
      type: string
      constraints:
        - required
        - assert: LENGTH(postcode) <= 10
    constraints:
      - assert: end_date >= start_date
        description: A contract can't end before it starts.
```

(There's currently no way to express cross-table assertions apart from primary key/foreign key; we may add this in the future.)

Assertions are evaluated row-by-row, using SQL's three-valued logic, so an expression can be `true`, `false`, or `null` (unknown). Following SQL's `CHECK` semantics, a row passes when the expression is `true` or `null`, and only a `false` result violates the constraint. For example, `LENGTH(postcode) <= 10` constrains the length of the values that *are* present but says nothing about missing ones. Pair it with the `required` constraint (or an explicit `IS NOT NULL`) if the value must also be non-null.

## Expression languages {#expression-languages}

Expressions can be written in one of three languages:

* **`sql`** (the default): a SQL-like language.
* **`r`**: an R-like language with support for base and tidyverse function names.
* **`python`**: a Python-like language supporting [Polars](https://pola.rs) expression style.

Note that I say SQL-like, R-like, and Python-like deliberately: there is no SQL/R/Python behind the scenes, just data-dict. This means that you can't use the full features of the language, but we believe we have implemented a subset that's useful for expressing column constraints. Please [reach out](https://github.com/tidyverse/data-dict/issues) if you discover something missing that seems like it would be useful.

There are also some subtle differences in how data-dict evaluates expressions compared to the original languages. These are generally minor differences in floating point behavior and unlikely to affect many data analytic results, but are reported as deviations in the output. For example, R's `round` rounds halves to even, but data-dict rounds them away from zero. 

You can change the default for the whole dictionary with a top-level `language` key, or override it on an individual expression with its own `language`:

```yaml
columns:
  - name: postcode
    type: string
    constraints:
      - assert: end_date >= start_date
      - assert: nchar(postcode) <= 10
        language: r
      - assert: pl.col("postcode").str.len_chars() > 5
```

The following sections describe the supported operations, but our hope is that you don't need to read them in detail: just write what feels natural to you, and data-dict will figure out the rest.

### Arithmetic, comparison, and logic

Arithmetic (`+`, `-`, `*`, `/`) is spelled the same in all three languages. Equality is `=` in SQL but `==` in R and Python; `!=` works everywhere, and SQL also accepts `<>`. `AND`, `OR`, and `NOT` become `&`, `|`, and `!` in R, and `&`, `|`, and `~` in Python — where `&` and `|` bind tighter than comparisons, so each comparison needs parentheses.

```yaml
constraints:
  - assert: qty * unit_price > 100 AND NOT(status = 'cancelled')
  - assert: qty * unit_price > 100 & !(status == "cancelled")
    language: r
  - assert: (pl.col("qty") * pl.col("unit_price") > 100) & ~(pl.col("status") == "cancelled")
    language: python
```

#### `IS NULL`

`x IS NULL` and `x IS NOT NULL` become `is.na(x)` and `!is.na(x)` in R, and `x.is_null()` and `x.is_not_null()` in Python. 

```yaml
constraints:
  - assert: ship_date IS NOT NULL
  - assert: "!is.na(ship_date)"
    language: r
  - assert: pl.col("ship_date").is_not_null()
    language: python
```

(The R spelling is quoted because of its leading `!`, which otherwise makes invalid YAML.)

#### `BETWEEN`

`x BETWEEN lo AND hi` becomes `between(x, lo, hi)` in R and `x.is_between(lo, hi)` in Python.

```yaml
constraints:
  - assert: quantity BETWEEN 1 AND 100
  - assert: between(quantity, 1, 100)
    language: r
  - assert: pl.col("quantity").is_between(1, 100)
    language: python
```

#### `IN`

`x IN (a, b)` becomes `x %in% c(a, b)` in R and `x.is_in([a, b])` in Python.

```yaml
constraints:
  - assert: status IN ('shipped', 'delivered')
  - assert: status %in% c("shipped", "delivered")
    language: r
  - assert: pl.col("status").is_in(["shipped", "delivered"])
    language: python
```

#### `LIKE` and `SIMILAR TO`

In general, both pattern-matching operators become regular-expression matches in R and Python — `str_detect(s, p)` or `grepl(p, s, perl = TRUE)` in R, `s.str.contains(p)` in Python.

Simple `LIKE` patterns are translated to simpler equivalents: an exact match becomes `==`, and a prefix or suffix becomes `startsWith`/`.str.starts_with` or the `ends` equivalents. 

```yaml
constraints:
  - assert: postcode LIKE 'SW%'
  - assert: str_starts(postcode, fixed("SW"))
    language: r
  - assert: pl.col("postcode").str.starts_with("SW")
    language: python
```

```yaml
constraints:
  - assert: postcode SIMILAR TO '[A-Z]{1,2}[0-9]+'
  - assert: str_detect(postcode, "^[A-Z]{1,2}[0-9]+$")
    language: r
  - assert: pl.col("postcode").str.contains("^[A-Z]{1,2}[0-9]+$")
    language: python
```

#### `CASE WHEN`

`CASE WHEN c THEN r ... END` is `case_when(c ~ r, ..., .default = ...)` in R and `pl.when(c).then(r).otherwise(...)` in Python.

```yaml
constraints:
  - assert: CASE WHEN kind = 'weight' THEN value < 1000 ELSE value < 10 END
  - assert: case_when(kind == "weight" ~ value < 1000, .default = value < 10)
    language: r
  - assert: pl.when(pl.col("kind") == "weight").then(pl.col("value") < 1000).otherwise(pl.col("value") < 10)
    language: python
```

#### `COLUMNS()`

All three languages provide an idiom for working with multiple columns: `COLUMNS()` in SQL (from duckdb), `if_all()` for R (from dplyr) and `pl.all_horizontal` in Python (from Polars).

```yaml
constraints:
  - assert: COLUMNS('q[4-8]') IS NOT NULL
  - assert: if_all(matches("q[4-8]"), \(x) !is.na(x))
    language: r
  - assert: pl.all_horizontal(pl.col("^.*(?:q[4-8]).*$").is_not_null())
    language: python
```

However it's written, the predicate is evaluated once per selected column and the results are combined with `AND`: the constraint holds unless the predicate is *false* for some column — true or null for every column passes. 

### Supported functions

Every function has a variant in each of the three languages. 

| SQL | R | Python |
|-----------|---|--------|
| `LENGTH(s)` | `nchar(s)`, `str_length(s)` | `.str.len_chars()` |
| `LOWER(s)` | `tolower(s)`, `str_to_lower(s)` | `.str.to_lowercase()` |
| `UPPER(s)` | `toupper(s)`, `str_to_upper(s)` | `.str.to_uppercase()` |
| `TRIM(s)` | `trimws(s)`, `str_trim(s)` | `.str.strip_chars()` |
| `STARTS_WITH(s, p)` | `startsWith(s, p)`, `str_starts(s, fixed(p))` | `.str.starts_with(p)` |
| `ENDS_WITH(s, p)` | `endsWith(s, p)`, `str_ends(s, fixed(p))` | `.str.ends_with(p)` |
| `ABS(x)` | `abs(x)` | `.abs()` |
| `FLOOR(x)` | `floor(x)` | `.floor()` |
| `CEIL(x)` | `ceiling(x)` | `.ceil()` |
| `ROUND(x, d)` | `round(x, d)` | `.round(d)` |
| `MOD(x, y)` | `x %% y` | `x % y` |
| `IS_FINITE(x)` | `is.finite(x)` | `.is_finite()` |
| `IS_INFINITE(x)` | `is.infinite(x)` | `.is_infinite()` |
| `IS_NAN(x)` | `is.nan(x)` | `.is_nan()` |
| `NOW()` | `Sys.time()` | `pl.lit(datetime.datetime.now())` |
| `interval(n, unit)` | `as.difftime(n, units = "days")` | `pl.duration(days = n)` |
| `MIN(x)`, `MAX(x)` | `min(x, na.rm = TRUE)`, `max(x, na.rm = TRUE)` | `.min()`, `.max()` |
| `SUM(x)` | `sum(x, na.rm = TRUE)` | `.sum()` |
| `AVG(x)` | `mean(x, na.rm = TRUE)` | `.mean()` |
| `COUNT(x)` | `sum(!is.na(x))` | `.count()` |
| `ROW_COUNT()` | `n()`, `.N` (data.table) | `pl.len()` |
| `COUNT_DISTINCT(x)` | `n_distinct(x, na.rm = TRUE)`, `uniqueN(x, na.rm = TRUE)` | `.drop_nulls().n_unique()` |
| `ANY(b)`, `ALL(b)` | `any(b, na.rm = TRUE)`, `all(b, na.rm = TRUE)` | `.any()`, `.all()` |

: {tbl-colwidths="[33,33,33]"}
