# Validation

## Two levels of validation

Validation happens at two levels:

* Validating the **dictionary** checks that the file is well-formed and internally consistent — that types are valid, foreign keys have matching relationships, joins parse, and so on. These checks have an unambiguous right answer, so most are errors. This is performed by `data-dict validate-schema`.

* Validating the dictionary against the **data** (or equivalently validating the data against the dictionary) checks that the data and dictionary are consistent. If there's an inconsistency, we can't tell which needs to change. If you're creating the dictionary as you learn about the data, then you might need to change the dictionary. If you're using the dictionary to validate a dataset, there might be an upstream issue that you need to resolve. This is performed by `data-dict parquet validate`.

Validating the dictionary is cheap, because it does not need to look at the data. This means it can be done continually while you edit the `data-dict.yaml`. Validating the data can be expensive, depending on the data source.

Validating the data always implies validating the dictionary first.

## Errors vs warnings

A validator reports two severities of problem: **errors** and **warnings**. The distinction is about urgency, not importance.

* An **error** means the dictionary is invalid or there's a critical mismatch between the data and dictionary. Errors will cause a production pipeline to fail, and you must fix them immediately.

* A **warning** means the dictionary is usable but the data and dictionary may have drifted apart. Warnings will not cause a production pipeline to fail, but if you're actively working on the project you should make sure to fix them.
