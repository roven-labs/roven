# PMEMC Version 1 Hardening Design

## Goal

Harden the existing Version 1 implementation so that untrusted model output cannot mislabel in-progress work, approved baselines correctly detect later changes, evidence provenance remains accurate, unsafe repository paths cannot escape the repository root, and review conflicts remain operator-controlled.

## Scope

This is maintenance inside the existing Version 1 contract. It does not add future resume, portfolio, GUI, MCP, cloud, graph-database, background-monitoring, or additional-provider features. It does not redesign the entire crate; it introduces focused domain validation and safety helpers where the current implementation has contract violations.

## Design

1. Keep the provider response schema, but validate each proposal against the staged evidence before persistence. A proposal backed by any staged, unstaged, or untracked file may not be stored as `committed`. The operator may still confirm a statement during review, but the stored fact must retain an explicit `in_progress` or `user_confirmed` state.
2. Use one canonical set of confidence and lifecycle values across provider validation, SQLite persistence, and finalization. The existing provider values (`exact`, `inferred`, `user_confirmed`) become the accepted fact-evidence confidence values; storage will no longer reject valid provider values.
3. Compare the current Git snapshot with the approved baseline. Existing working-tree state is baseline state, not automatically a new change. New, removed, renamed, or content-changed paths are detected relative to the saved baseline fingerprints and status.
4. Store commit provenance only for committed evidence. In-progress evidence receives a working-tree marker and its content fingerprint; it is never attributed to the HEAD commit.
5. Make conflict detection conservative. For the same project and fact kind, materially different statements sharing relevant evidence must create a pending conflict instead of silently creating another verified fact. The operator remains the final authority.
6. Route repository file reads through a containment check. Every candidate path is canonicalized and must remain below the canonical repository root before it is classified, parsed, or placed in an evidence bundle. Blocked credential names remain un-overridable.
7. Make staged provider attempts recoverable after interruption. A `staged_pending_provider` attempt can be resumed or safely marked as failed without creating a competing inspection attempt.

## Data flow

```text
Git snapshot + approved baseline
        -> baseline comparator
        -> selected paths
        -> safe repository reader
        -> redacted evidence bundle
        -> provider schema validation
        -> evidence/lifecycle/provenance validation
        -> pending review
        -> operator decision
        -> transactional fact + evidence + baseline finalization
```

## Failure behavior

- Validation failure stores no trusted provider proposals.
- Provider failure preserves the previous baseline and verified facts.
- File containment failure skips the unsafe path and records it as unsupported rather than reading outside the repository.
- Finalization failure rolls back facts, decisions, evidence, and baseline together.
- Interrupted provider work remains retryable and does not permit a second concurrent inspection attempt.

## Testing strategy

Each defect receives a public-behavior regression test before implementation:

- in-progress evidence cannot become committed;
- `inferred` and `user_confirmed` proposals can finalize;
- baseline state is not reported as a later change;
- changed fingerprints are reported;
- uncommitted evidence has no commit provenance;
- materially contradictory proposals require conflict resolution;
- symlinked paths outside the repository are excluded;
- staged provider attempts are recoverable.

## Non-goals

No new production dependency, asynchronous runtime, external service, CLI command, or future-version module is required.
