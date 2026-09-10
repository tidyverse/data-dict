---
title: "Who, why and when"
---

<!-- A "who" section is planned; for now this page covers why and when. -->

## Why use data-dict?

A data dictionary is one place to record everything you know about your data, making it accessible to everyone who works with it. data-dict helps you do this with two coupled components, a specification and a command-line interface (CLI).

The specification is:

* Open and supported by Posit, a public benefit corporation with a mission to create free and open-source software for data science, scientific research, and technical communication. We don't yet have a formal governance model, but in the meantime, you're very welcome to propose changes and additions in the [issues](https://github.com/tidyverse/data-dict/issues).

* Built by data people, for data people. We understand the challenges data folks face, and we've designed the spec to address them directly. This means including pieces that other dictionaries omit: support for metadata like data version and column units, descriptions of the relationships between datasets, and a glossary for domain- and team-specific terminology.

* A plain text YAML 1.2 document. Compared to other formats like Excel and PDF, YAML is easily diffable so that you can see how it changes over time. Compared to formats like JSON or XML, it's easily editable by humans, not just machines.

* Polyglot by design. data-dict is built for teams that work across R, Python, and SQL: the specification is language-neutral, and the CLI is a single binary that fits into any pipeline, regardless of which language produced or consumes the data.

* Assumes that data lives in parquet and database tables. Parquet is an open format designed to store data compactly while maximising performance on modern hardware. This keeps the scope of the specification tight (no need to describe the many wrinkles of CSV) and encourages best practices for data storage.

The CLI, `data-dict`, is the other half of the story. It enforces the standard and provides a growing set of useful tools.

* It is open source and free, and does not require any hosted services.

* It's self-contained, so that all you need to [install](install.md) is a single binary. This makes it straightforward to create, check, and maintain a data dictionary on both your local machine and in CI.

* It validates the data contract, ensuring the data and dictionary stay consistent. Validation is actively expanding, and currently covers variable names, types, ranges, and constraints such as uniqueness.

* It renders your dictionary as a beautiful, self-contained website, so anyone on your team can browse tables, columns, relationships, and the glossary without reading raw YAML. See the [otters dictionary](examples/rendered/otters.html) for a live example.

## When should you use data-dict?

data-dict is designed to support a wide range of scenarios. You might use it:

* Before you have any data, as a way to be concrete about your goals and expectations. In the future, we plan to support [data simulation](https://github.com/tidyverse/data-dict/issues/20) so that you can generate a dummy dataset compatible with your data dictionary.
* When you first encounter a new dataset, as a way of recording what you learn as you learn it. This can be particularly useful with public datasets, where an LLM may know more about the data than you do.
* Retrospectively, when you've already spent considerable time with a dataset. Creating a `data-dict.yaml` gets the knowledge out of your head and makes it accessible to your human and AI collaborators. You can get started quickly by asking an LLM to extract what it knows based on the code you've written so far.
* When working with regularly updated data produced by someone else. Maybe you get updates from collaborators via email, or your data engineering team has a habit of not announcing changes to variable definitions. data-dict's ability to validate data against the spec ensures you're never surprised when the data changes.
