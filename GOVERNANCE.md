# Governance

Codasaurus is maintained by Lohit Kolluri. This document describes how
decisions are made while the project has a single maintainer, and what changes
when that stops being true.

## Roles

- **Maintainer** — merge rights, release rights, security response, final say
  on scope. Currently: Lohit Kolluri ([@lohitkolluri](https://github.com/lohitkolluri)).
- **Contributor** — anyone with a merged contribution. No ongoing obligations.

See [MAINTAINERS.md](MAINTAINERS.md) for the current list and areas.

## How decisions are made

- **Bug fixes, docs, tests** — pull request, CI green, maintainer review, merge.
- **Features and behaviour changes** — open an issue first so the design and
  scope are agreed before code is written.
- **Breaking changes** — issue + a `BREAKING CHANGE:` commit footer, released
  as a minor bump while the project is pre-1.0.
- **Security reports** — handled privately per [SECURITY.md](SECURITY.md).

The maintainer has final say when consensus is not reached. Decisions that
affect users are recorded in the issue or pull request that implements them.

## Becoming a maintainer

Sustained, high-quality contributions and review participation are the bar.
A new maintainer is added by the existing maintainer, announced in a pull
request that updates [MAINTAINERS.md](MAINTAINERS.md).

## Changes to this document

Open a pull request. Governance changes follow the same review path as code.
