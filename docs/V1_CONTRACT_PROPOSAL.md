# Approved V1 Contract Amendment — Capability-Free Reset

## Status

Approved by the operator. This amendment supersedes conflicting V1 statements
about conversational sessions, projects, databases, CodeGraph, models,
external transport, and agent tools.

## Contract

PMEMC retains only the local `pmemc auth` credential lifecycle. It accepts,
checks for, and removes a secret through Windows Credential Manager without
printing or otherwise persisting the secret in PMEMC application data.

Bare `pmemc` has no session behavior. PMEMC exposes no model-facing tools and
does not inspect, register, index, read, or write repositories. It has no
project or memory database and makes no external request.

All former repository, source-analysis, agent, model, and external-service
workflows are removed rather than retained as fallback behavior. Any later
capability must be proposed and approved as a separate contract before code or
documentation introduces it.
