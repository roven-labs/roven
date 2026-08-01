# PMEMC Inspection Observability Design

**Date:** 2026-08-01  
**Scope:** Version 1 inspection command  
**Status:** Implemented

## Goal

Make `pmemc inspect <project>` explain its existing work as a short, structured terminal workflow so the operator can see where an inspection is, what data crossed each boundary, and why a failure occurred.

## Constraints

- Keep the existing V1 command surface; do not add a daemon, log file, GUI, TUI, telemetry service, or provider tool loop.
- Never print API keys, source excerpts, full prompts, raw provider responses, or suspected secrets.
- Preserve the current approval gate, read-only repository policy, retryable attempts, and transactional storage behavior.
- Use stable Rust and existing dependencies only.
- Keep progress output useful in PowerShell and deterministic enough for tests and redirected output.

## Recommended architecture

The inspection command remains the workflow coordinator. A small terminal reporter becomes its output adapter:

```text
commands::inspect::run
        |
        +--> InspectionReporter --------------------> terminal
        |       (phase, detail, success/warn/failure)
        |
        +--> Git adapter ----------------------------> repository metadata/status
        +--> Inspection builder ---------------------> bounded evidence bundle
        +--> Storage adapter -------------------------> staged attempt / pending review
        +--> ModelProvider ---------------------------> untrusted provider response
                |
                +--> safe validation diagnostics ----> InspectionReporter
```

The reporter is presentation-only. It does not own workflow state, database writes, credentials, or provider calls. The command emits events at actual boundaries rather than simulating internal progress.

## User-visible inspection flow

The normal output is always-on for interactive PowerShell sessions:

```text
[1/7] Repository check                 ✓ master @ 7ec4d0c
[2/7] Evidence preparation             ✓ 12 files, 48 KB, 0 redactions
[3/7] Local staging                    ✓ attempt 3
[4/7] OpenRouter request               ✓ provider=openrouter
      waiting for model response...
[5/7] Response validation              ✓ 11 proposals, 3 questions
[6/7] SQLite storage                   ✓ pending review
[7/7] Inspection complete              ✓ run: pmemc review Siftara
```

The retry path labels the difference explicitly:

```text
[2/7] Evidence preparation             ✓ reused attempt 3 bundle
      current repository changes       1
      approved bundle                  12 files, 48 KB
```

The seven labels are presentation stages, not new domain states:

1. Repository check: project identity, branch, commit, and changed-path count.
2. Evidence preparation: initial or incremental selection, file count, bounded byte count, and redaction count.
3. Local staging: attempt identifier and whether the bundle was newly staged or reused.
4. OpenRouter request: provider and configured model; then a waiting line while the synchronous request runs.
5. Response validation: safe validation result and proposal/question counts.
6. SQLite storage: pending-review persistence result.
7. Completion: next command or safe failure guidance.

If a failure occurs, the reporter marks the current stage and explains the safe category without exposing untrusted content:

```text
[5/7] Response validation              ✗ invalid response
      reason                           proposal evidence path was not in the approved bundle
      state                             no proposals stored; attempt 3 can be retried
```

## Color and terminal policy

Use a tiny local style helper rather than a new dependency:

- Green: completed or success.
- Cyan: active stage and neutral metadata.
- Yellow: retry, warning, reused evidence, or operator action.
- Red: failure.
- Reset after every styled span.

Color is enabled only when standard output is a terminal and `NO_COLOR` is not set. Redirected output and tests receive plain text. The same semantic labels remain present without ANSI escapes. This keeps PowerShell readable and avoids contaminating scripts or snapshots.

The reporter writes progress to standard output. Existing command errors continue to go through the application boundary and standard error. A provider key or repository content must never be passed to the reporter.

## Provider diagnostics

`ProviderError::InvalidResponse` currently collapses several validation failures into one message. Replace that single opaque case with bounded, non-secret diagnostic variants or a bounded reason value covering:

- response body was not valid JSON;
- missing `choices[0].message.content`;
- model content was not valid PMEMC JSON;
- unsupported schema version or unknown fields;
- invalid fact kind, blank statement, blank question, duplicate evidence path, or evidence path outside the bundle;
- committed proposal cited non-committed evidence.

The error display includes the reason, while durable storage continues to persist only the existing safe failure category (`invalid_response`). Do not persist raw response text. The provider boundary remains responsible for validation; the reporter only displays the returned safe reason.

## Data-flow invariants

```text
repository files
   -> approved bounded bundle
   -> provider request
   -> untrusted response
   -> schema + evidence validation
   -> pending proposals/questions
   -> human review
   -> verified facts and baseline
```

Progress output must not imply a later stage completed before its operation returns successfully. In particular:

- “OpenRouter request” does not mean proposals are trusted.
- “Response validation” does not mean facts are verified.
- “SQLite storage” means pending review only; final facts still require `pmemc review`.
- Any failure before finalization leaves verified memory and the previous baseline unchanged.

## Minimal implementation units

- Create `src/output.rs`: terminal detection, ANSI style helpers, and inspection reporter methods. No workflow logic.
- Modify `src/lib.rs`: expose the private output module to command modules.
- Modify `src/commands/inspect.rs`: emit reporter events at the existing Git, evidence, staging, provider, validation, and persistence boundaries.
- Modify `src/provider.rs`: retain the current safe failure category while returning bounded validation reasons for operator display.
- Modify `tests/inspection.rs`: assert stage order, retry/reused-bundle messaging, and no-color behavior in captured output.
- Modify `tests/provider.rs`: assert each validation diagnostic is bounded and contains no response body or secret.
- Update `README.md`: document the inspection progress display and the fact that logs are terminal-only and sanitized.

No database migration is required. No new production dependency is required.

## Testing strategy

Test-first cases:

1. A captured non-terminal inspection uses the seven-stage labels and contains no ANSI escape sequences.
2. A retry reports the retained attempt and reused bundle rather than pretending to rebuild evidence.
3. A successful provider response reports proposal and question counts before the pending-review completion message.
4. Invalid JSON, missing provider content, and invalid evidence references produce distinct safe diagnostics.
5. Existing failure behavior still records a retryable provider attempt and leaves verified facts and baselines unchanged.

Run the required project checks after implementation:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

## Deliberate non-goals

- No persistent debug log file.
- No raw request/response logging.
- No progress bar or spinner that hides the current phase.
- No asynchronous provider rewrite.
- No new `--verbose` flag until the always-on structured output proves insufficient.
- No changes to provider selection, evidence limits, or review authority.
