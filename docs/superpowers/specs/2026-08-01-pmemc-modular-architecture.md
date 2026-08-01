# PMEMC Modular Architecture Refactor

## Goal

Reduce the orchestration hotspot in `src/lib.rs` without changing the Version 1 command surface, persistence model, trust boundaries, or public test entry points.

## Current problem

`src/lib.rs` currently owns CLI dispatch, inspection/provider submission, review interaction, status rendering, baseline comparison, and project commands. These responsibilities have different reasons to change and force unrelated behavior through one compilation unit.

## Target structure

```text
src/
├── lib.rs                         # public facade and module wiring
├── application.rs                 # provider submission use cases
├── baseline.rs                    # baseline comparison and fingerprint policy
└── commands/
    ├── mod.rs                     # command dispatch and shared parsing
    ├── inspect.rs                 # inspection approval and staging
    ├── review.rs                  # operator review workflow
    ├── status.rs                  # status reporting
    └── project.rs                 # project/history commands
```

`storage.rs` remains the concrete SQLite adapter in this pass. Introducing repository ports or splitting migrations/queries is deliberately deferred because Version 1 has one storage implementation and the requested change is file responsibility, not a persistence redesign.

## Dependency rules

- `lib.rs` depends on the application and command modules only for public wiring.
- `commands/*` may use `application`, `baseline`, `git`, `inspection`, `provider`, and `storage`; they must not duplicate storage policy.
- `application.rs` owns provider invocation and submission transitions; it does not parse CLI input.
- `baseline.rs` owns status snapshots, fingerprints, and comparison; it does not print or mutate SQLite.
- Domain, Git, inspection, provider, and storage modules retain their existing boundaries.
- Existing `pmemc::submit_approved_bundle` remains available through a re-export.

## Non-goals

No new dependency, database migration, CLI command, async runtime, provider, port interface, or future Version 1 feature is introduced.

## Verification

Run the existing public test suite plus formatting and Clippy. The refactor is successful only if behavior and public paths remain unchanged.
