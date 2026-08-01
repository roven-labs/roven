# PMEMC Onboarding and Free-Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `pmemc init` a safe first-run onboarding wizard and default provider configuration to OpenRouter's free router while preserving explicit advanced overrides and actual model provenance.

**Architecture:** Keep first-run interaction in the CLI command layer, credential storage in `credentials.rs`, model defaults and provider response metadata in `provider.rs`, and durable invocation persistence in the existing application/storage path. Do not add a settings database, GUI, paid fallback, or new provider interface.

**Tech Stack:** Stable Rust, clap, rusqlite, serde, keyring, rpassword, ureq, temporary repository/database fixtures.

## Global Constraints

- Target native Windows 11 and PowerShell.
- Provider credentials never enter the database, logs, exports, or repository.
- `init` must remain idempotent and must not call a model provider.
- Model output remains untrusted and schema-validated.
- No new production dependency is needed for this change.
- Run `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` before completion.
- Do not commit changes unless explicitly authorized.

---

### Task 1: Default OpenRouter model

**Files:**
- Modify: `src/provider.rs`
- Test: `tests/provider.rs`
- Modify: `docs/OPENROUTER_SETUP.md`
- Modify: `docs/V1_SPEC.md`

**Interfaces:**
- Produce `pub const DEFAULT_MODEL_ID: &str = "openrouter/free"`.
- `OpenRouterConfig::from_environment_with` treats a missing or blank
  `PMEMC_OPENROUTER_MODEL` as `DEFAULT_MODEL_ID`.
- Existing non-secret timeout and retry overrides remain unchanged.

- [ ] **Step 1: Write the failing tests**

Add tests proving a missing model resolves to `openrouter/free`, a blank model
also resolves to `openrouter/free`, and an explicit model remains unchanged:

```rust
#[test]
fn missing_model_uses_the_free_router_default() {
    let config = OpenRouterConfig::from_environment_with(|_| None)
        .expect("default model should be valid");
    assert_eq!(config.model_id(), "openrouter/free");
}

#[test]
fn explicit_model_override_remains_supported() {
    let config = OpenRouterConfig::from_environment_with(|name| {
        (name == "PMEMC_OPENROUTER_MODEL").then(|| "provider/model".into())
    })
    .expect("explicit model should be valid");
    assert_eq!(config.model_id(), "provider/model");
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run `cargo test --test provider missing_model_uses_the_free_router_default
explicit_model_override_remains_supported`.

Expected result: the tests fail because the current implementation requires
`PMEMC_OPENROUTER_MODEL` and does not expose `model_id()`.

- [ ] **Step 3: Implement the minimal default**

Add the constant and accessor, then change model resolution to:

```rust
let model_id = value("PMEMC_OPENROUTER_MODEL")
    .filter(|model| !model.trim().is_empty())
    .unwrap_or_else(|| DEFAULT_MODEL_ID.to_owned());
```

Update provider error/help text and setup documentation so the environment
variable is described as an optional override.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the same focused provider tests. Expected result: both pass with no new
warnings.

---

### Task 2: Persist the actual routed model

**Files:**
- Modify: `src/provider.rs`
- Modify: `src/application.rs`
- Test: `tests/provider.rs`

**Interfaces:**
- `OpenRouterProvider<T>` tracks the last non-empty model identifier returned by
  OpenRouter using `RefCell<Option<String>>`.
- `ModelProvider::metadata()` returns the configured model before a request and
  the actual routed model after a successful OpenRouter response.
- `OpenRouterTransport::complete` continues returning the raw response body;
  the existing OpenRouter envelope parser extracts its top-level `model`.

- [ ] **Step 1: Write the failing routed-model test**

Add a provider test whose scripted OpenRouter envelope contains a top-level
`model` different from `openrouter/free`, then assert after `propose`:

```rust
assert_eq!(provider.metadata().model_id, "provider/actual-free-model");
```

- [ ] **Step 2: Run the focused test and verify RED**

Run `cargo test --test provider provider_metadata_records_the_actual_routed_model`.

Expected result: it fails because metadata still reports the configured model.

- [ ] **Step 3: Implement response-envelope provenance**

Change the private OpenRouter envelope parser to return the validated
`ProviderResponse` plus an optional top-level model ID. Store that ID in the
provider after a successful response. Keep the configured model as the
fallback for fake providers, failures, and envelopes without a valid model.

Update `application::submit_staged_bundle` to obtain metadata again after a
successful `propose`, while retaining the pre-request metadata for failure
records.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run the focused routed-model test and the existing provider/application tests.
Expected result: actual model metadata is persisted without changing response
schema validation or failure behavior.

---

### Task 3: First-run `init` onboarding

**Files:**
- Modify: `src/commands/mod.rs`
- Modify: `src/commands/auth.rs`
- Modify: `src/credentials.rs`
- Test: `tests/initialization.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- `init` checks whether the database existed before initialization.
- First-run interactive behavior is gated by `std::io::IsTerminal`; piped and
  CI input never blocks.
- Existing credential-store operations are reused; no second secret-storage
  implementation is introduced.

- [ ] **Step 1: Write the failing non-interactive init test**

Spawn `pmemc init` with a temporary `LOCALAPPDATA` and non-terminal stdin. The
test must assert success, the initialized data path, `openrouter/free`, and a
non-blocking instruction containing `pmemc auth set`.

- [ ] **Step 2: Run the focused initialization test and verify RED**

Run `cargo test --test initialization init_reports_free_model_setup_guidance`.

Expected result: it fails because current `init` prints only the data path.

- [ ] **Step 3: Implement the first-run flow**

In the `Init` handler:

1. Resolve paths and record `database_path().exists()` before initialization.
2. Initialize storage exactly as before.
3. Print the effective default/override model without exposing secrets.
4. If this is not first run, stop.
5. If stdin is not a terminal, print setup guidance and stop successfully.
6. If a stored credential or non-empty environment fallback exists, report it
   without printing its value and stop.
7. Otherwise ask `Configure OpenRouter now? [Y/n]`.
8. On approval, reuse hidden input, confirmation, and Credential Manager store.
9. On decline or setup failure, preserve the initialized database and print an
   actionable next step.

The wizard must not instantiate `OpenRouterProvider` or make an HTTP request.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the initialization and CLI tests. Confirm repeated `init` does not prompt
and non-interactive init exits without waiting for input.

---

### Task 4: Documentation and full verification

**Files:**
- Modify: `README.md`
- Modify: `docs/OPENROUTER_SETUP.md`
- Modify: `docs/IMPLEMENTATION_PLAN_V1.md`
- Modify: `docs/V1_SPEC.md`

- [ ] **Step 1: Document the new user flow**

Document `pmemc init` as the first-run entry point, `openrouter/free` as the
default, `PMEMC_OPENROUTER_MODEL` as an optional override, and the fact that
free-model limits/availability can vary. State that paid fallback is never
automatic.

- [ ] **Step 2: Run the complete verification suite**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Expected result: all commands exit successfully; tests cover default model,
actual routed model, first-run guidance, credential failure preservation, and
the existing 50-test suite remains green or increases only with these focused
cases.
