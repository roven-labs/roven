# PMEMC First-Run Onboarding and Free-Model Design

## Goal

Make the first PMEMC run usable without environment-variable setup while using
OpenRouter's free model router by default.

## User experience

`pmemc init` remains idempotent and does not call OpenRouter. On the first
interactive run it initializes local storage, reports `openrouter/free` as the
default model, and offers hidden OpenRouter credential setup. Declining or
failing credential setup does not undo database initialization. Non-interactive
runs never wait for input and print the normal next-step guidance instead.

After setup, the normal flow is:

```text
pmemc init
pmemc project add .
pmemc inspect project-1
pmemc review
```

`PMEMC_OPENROUTER_MODEL` remains an optional advanced override. The existing
`pmemc auth set|status|remove` commands remain available for explicit
credential management.

## Architecture

The CLI command layer coordinates first-run interaction. The credential module
continues to own hidden input and Windows Credential Manager access. The
provider configuration owns the default model and environment override. The
application and storage layers continue to own provider invocation and durable
review state; `init` never crosses into provider invocation.

The provider records the actual model returned by OpenRouter when using
`openrouter/free`. If a test or provider response has no actual model, the
configured model remains the metadata fallback.

## Failure handling

- Existing initialized storage is never removed because credential setup fails.
- Empty keys and mismatched confirmation remain rejected without persistence.
- Non-interactive initialization prints instructions and exits successfully.
- Missing credentials during inspection remain a retryable configuration
  failure.
- Free-router rate limits and unavailable models remain provider failures; PMEMC
  never silently selects a paid model.
- Provider invocation metadata contains only provider and model identifiers,
  never credentials.

## Verification

Tests cover default and explicit model resolution, first-run/non-interactive
initialization behavior, credential setup failure preservation, actual routed
model metadata, and existing provider/storage regression paths. Formatting,
Clippy, and all Rust tests remain required.

## Deliberate non-goals

This change does not add a GUI, model picker, paid fallback, background watcher,
configuration database, or project-selection wizard.
