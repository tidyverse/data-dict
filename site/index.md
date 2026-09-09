---
title: "data-dict"
---

data-dict is two things: a **specification** for data dictionaries (`data-dict.yaml`), and a **validator** (the `data-dict` CLI) that enforces it. The specification describes a collection of related tables: their contents, constraints, connections, and the specialised vocabulary you need to understand them. The validator turns that description into a data contract, checking that your data actually matches what the dictionary claims. Together they form a living document, accessible to both humans and agents, that tracks your understanding of a dataset as it evolves.

The specification is designed to be lightweight. It doesn't attempt to precisely describe every possible type of metadata in a machine-readable way. Instead it focuses on precisely recording the most important components, leaving the remainder to plain text fields that require a human or agent to interpret. This means that data-dict doesn't itself do **data cleaning**, but it is a useful complement to tools that do.

## What a dictionary looks like

A dictionary is a single YAML file, which the CLI renders as a browsable website (abridged from the [otters dictionary](examples/otters.qmd)):

::: {.panel-tabset}

## data-dict.yaml

```yaml
name: alaska-otters
tables:
  - name: otters
    source: { parquet: otters.parquet }
    description: One row per otter.
    columns:
      - name: otter_no
        type: string
        constraints: [primary_key]
      - name: sex
        type: enum
        values: { M: Male, F: Female, U: Unknown }
  - name: measurements
    source: { parquet: measurements.parquet }
    columns:
      - name: otter_no
        type: string
        constraints: [required, foreign_key]
```

## Rendered site

::: {.light-content}
[![](images/otters-light.png)](examples/rendered/otters.html)
:::
::: {.dark-content}
[![](images/otters-dark.png)](examples/rendered/otters.html)
:::

:::

Three commands take you from data to dictionary:

```sh
data-dict draft otters.parquet
data-dict validate-data data-dict.yaml
data-dict render-spec data-dict.yaml
```

See the [quickstart](quickstart.md) for the full walkthrough, including how an AI agent can draft the dictionary for you. Or jump directly to the details of [the specification](spec.md) or look at more [examples](examples/index.qmd).

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

## Why now? (_cough_ AI _cough_)

There have been many previous attempts to encode data dictionaries in structured text. What makes data-dict different, and why revisit this problem now? The answer is AI. We believe AI fundamentally changes both the costs and benefits of a data dictionary:

* The costs of creating a data dictionary are lower than ever, because AI agents can automate much of the boilerplate, including porting documentation from existing unstructured formats (`.doc`, `.html`, `.pdf`). An agent can also surface questions about aspects of the data that are unspecified or ambiguous.
* The benefits are higher, because AI agents need the context that currently exists only in your head. Providing it via a data dictionary helps your AI tools work more accurately.
* The schema can be simpler because LLMs change what it means for something to be machine-readable. You only need to explicitly encode the most important structures, leaving more unusual quirks to free-form text.

## When should you use data-dict?

data-dict is designed to support a wide range of scenarios. You might use it:

* Before you have any data, as a way to be concrete about your goals and expectations. In the future, we plan to support [data simulation](https://github.com/tidyverse/data-dict/issues/20) so that you can generate a dummy dataset compatible with your data dictionary.
* When you first encounter a new dataset, as a way of recording what you learn as you learn it. This can be particularly useful with public datasets, where an LLM may know more about the data than you do.
* Retrospectively, when you've already spent considerable time with a dataset. Creating a `data-dict.yaml` gets the knowledge out of your head and makes it accessible to your human and AI collaborators. You can get started quickly by asking an LLM to extract what it knows based on the code you've written so far.
* When working with regularly updated data produced by someone else. Maybe you get updates from collaborators via email, or your data engineering team has a habit of not announcing changes to variable definitions. data-dict's ability to validate data against the spec ensures you're never surprised when the data changes.

## Get started

Ready to try it? [Install the CLI](install.md) in seconds, browse the [examples](examples/index.qmd) to see what a dictionary looks like, or read the [specification](spec.md) for the full details. Curious about the thinking behind the design? See [design](design.md).
