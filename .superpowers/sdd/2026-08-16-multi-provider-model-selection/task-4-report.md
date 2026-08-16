# Task 4 Report

## Scope completed

Completed Task 4 for the multi-provider model-selection plan with the minimum code needed to satisfy the required verification steps:

- grouped `agent::run` inputs to remove the `too_many_arguments` Clippy error
- collapsed the requested nested `if` in `src/ui/transcript.rs`
- updated the installer to install `pmemc.exe` under `C:\Users\visha\AppData\Local\Programs\PMEMC`
- ran the required verification chain including install and version check

## Files changed

- `src/agent.rs`
  - Added `AgentRun<'a>` to group runtime dependencies passed into `agent::run`.
  - Updated `run` to accept `AgentRun<'_>` plus `messages` and `emit`.
  - Preserved existing behavior for tool dispatch, cancellation, runtime logging, and context usage.
  - Added focused regression coverage for grouped runtime dependencies and context usage propagation.
- `src/ui/terminal.rs`
  - Updated the `agent::run` call site to build and pass `agent::AgentRun`.
  - Collapsed the nested `if` in model-selection event handling to satisfy Clippy.
- `src/ui/transcript.rs`
  - Collapsed the nested `if` at the requested `project.path` rendering site.
- `scripts/install.ps1`
  - Changed the default install root to `%LOCALAPPDATA%\Programs\PMEMC`.
  - Changed the installed binary name to `pmemc.exe` while still copying from Cargo’s unchanged `target\release\roven.exe`.
  - Updated installer/uninstaller/status text to reference PMEMC.
- `src/ui/startup.rs`
  - Moved `banner_lines` above the test module to clear `clippy::items-after-test-module`.
  - No behavior change; this was required to make `cargo clippy --all-targets -- -D warnings` pass.

## TDD record

### Failing test before the `AgentRun` refactor

Command:

```powershell
cargo test agent_run_groups_runtime_dependencies_without_changing_context_usage -- --nocapture
```

Observed failure:

```text
error[E0432]: unresolved import `super::AgentRun`
error[E0061]: this function takes 8 arguments but 3 arguments were supplied
```

That was the expected pre-implementation failure proving the grouped run input did not exist yet.

### First post-refactor test failure

The first implementation compiled, but the new test incorrectly depended on OpenRouter host detection and failed with:

```text
left: [Finished]
right: [ContextUsage(50), Finished]
```

I replaced that with a tiny in-test `ContextUsageProvider` so the regression stayed focused on `AgentRun` behavior rather than endpoint classification.

## Verification

Commands run:

```powershell
cargo test agent_run_groups_runtime_dependencies_without_changing_context_usage -- --nocapture
cargo fmt
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
& 'C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe' --version
```

Key results:

- `cargo test agent_run_groups_runtime_dependencies_without_changing_context_usage -- --nocapture`
  - passed: `1 passed; 0 failed`
- `cargo fmt --check`
  - passed with no output
- `cargo clippy --all-targets -- -D warnings`
  - passed
- `cargo test`
  - library tests: `142 passed; 0 failed`
  - CLI tests: `3 passed; 0 failed`
  - doctests: `0 failed`
- `cargo build --release`
  - passed
- `.\scripts\install.ps1`
  - installed `C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe`
  - installer version check printed `roven 0.1.0`
- installed binary version check
  - `C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe --version`
  - output: `roven 0.1.0`

## Concerns

- The installer now produces the requested `pmemc.exe`, but the binary still reports `roven 0.1.0` because the crate/package name was intentionally not renamed.
- Live OpenRouter and Ollama round trips were not claimed or tested here; the verification remained local/offline as required.

## Final fix wave

### Scope completed

Addressed the whole-branch release findings without expanding scope:

- added the existing untracked `src/context.rs` unchanged so `src/lib.rs` builds from a clean checkout
- aligned shipped CLI help, version output, docs, and auth/setup guidance on the `pmemc` command name
- kept the application data root at `%LOCALAPPDATA%\Roven\data`
- fixed the installer `LOCALAPPDATA` error wording from Roven to PMEMC

### Files changed

- `src/context.rs`
  - Added the existing minimal `percent` helper and its focused regression test unchanged.
- `src/cli.rs`
  - Set `bin_name = "pmemc"` so the shipped help/version surface identifies the installed command.
- `src/commands/auth.rs`
  - Updated user-facing setup guidance from `roven auth set` to `pmemc auth set`.
- `src/ui/startup.rs`
  - Updated the startup next-step hint to `pmemc auth set`.
- `src/ui/terminal.rs`
  - Updated runtime error/setup guidance to the `pmemc` command name.
- `src/ui/view.rs`
  - Updated view-level assertions that checked the setup hint string.
- `tests/cli.rs`
  - Updated command-surface coverage to assert PMEMC/`pmemc` help text and `pmemc` version output.
- `README.md`
  - Updated shipped command examples to `pmemc`.
- `docs/SETUP.md`
  - Updated installed command examples to `pmemc` and install root to `%LOCALAPPDATA%\Programs\PMEMC`.
  - Preserved `%LOCALAPPDATA%\Roven\data` as the application data root.
- `docs/PROVIDERS.md`
  - Updated provider workflow examples to `pmemc`.
  - Preserved `%LOCALAPPDATA%\Roven\data` as the application data root.
- `PRODUCT.md`
  - Updated user-facing workflow examples to `pmemc`.
- `scripts/install.ps1`
  - Updated the `LOCALAPPDATA` error text to reference PMEMC.

### TDD record

Commands run before the final production edits:

```powershell
cargo test help_describes_the_current_command_surface -- --nocapture
```

Observed failure before `bin_name = "pmemc"`:

```text
assertion failed: stdout.contains("pmemc")
```

After fixing the help surface, the full-suite verification exposed the remaining user-facing version regression:

```text
test version_identifies_the_roven_binary ... FAILED
assertion failed: stdout.starts_with("roven ")
```

That failure was expected once the shipped command surface moved to `pmemc`, so I updated the focused CLI regression accordingly.

### Verification

Commands run:

```powershell
cargo test help_describes_the_current_command_surface -- --nocapture
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
& 'C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe' --version
& 'C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe' --help
```

Key results:

- `cargo test help_describes_the_current_command_surface -- --nocapture`
  - passed: `1 passed; 0 failed`
- `cargo fmt --check`
  - passed with no output
- `cargo check`
  - passed
- `cargo clippy --all-targets -- -D warnings`
  - passed
- `cargo test`
  - library tests: `142 passed; 0 failed`
  - CLI tests: `3 passed; 0 failed`
  - doctests: `0 failed`
- `cargo build --release`
  - passed
- `.\scripts\install.ps1`
  - passed
  - printed `pmemc 0.1.0`
  - installed `C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe`
- installed binary verification
  - `C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe --version`
  - output: `pmemc 0.1.0`
  - `C:\Users\visha\AppData\Local\Programs\PMEMC\pmemc.exe --help`
  - output includes `PMEMC — Project Memory Assistant` and `Usage: pmemc [COMMAND]`

### Concerns

- The executable/help/docs now align on `pmemc`, but the crate/package/internal library name remains `roven` by request, so source-level identifiers still use that name.
- The application data root remains `%LOCALAPPDATA%\Roven\data` intentionally, because no storage-root migration was in scope.
