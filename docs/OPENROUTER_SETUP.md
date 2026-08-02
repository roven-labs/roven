# OpenRouter setup on Windows

For normal local use, store the key once in Windows Credential Manager through
PMEMC:

```powershell
pmemc auth set
pmemc auth status
```

`auth set` hides the key while typing, asks for confirmation, and stores it in
the operating-system credential store. `auth status` never prints the key.
Remove it with:

```powershell
pmemc auth remove
```

`pmemc init` initializes local storage and uses `openrouter/free` by default;
it never asks for a credential. Bare `pmemc` asks for a hidden key only after
it has validated a clean Git repository and prepared CodeGraph.

PMEMC reads `OPENROUTER_API_KEY` first, then Windows Credential Manager. It
does not read `.env` files, and the key must not be committed to the repository,
passed as a command-line argument, or stored in SQLite.

## CI or non-interactive environment key

Use the CI platform's encrypted secret store and inject `OPENROUTER_API_KEY`
at runtime. For a temporary local fallback, use Windows Settings > System >
About > Advanced system settings > Environment Variables and add these as User
variables:

```text
OPENROUTER_API_KEY             <the OpenRouter key>
PMEMC_OPENROUTER_TIMEOUT_SECS  120
PMEMC_OPENROUTER_MAX_ATTEMPTS  3
```

`PMEMC_OPENROUTER_MODEL` is optional. Without it, PMEMC sends
`openrouter/free`; OpenRouter can route each request to a different available
free model, and PMEMC records the returned model identifier in its local audit
metadata.

Close and reopen PowerShell after saving the variables. Do not print them,
commit them, or share screenshots containing them.

To verify only that the fallback key exists, without printing it:

```powershell
if ([string]::IsNullOrWhiteSpace($env:OPENROUTER_API_KEY)) {
    throw "OPENROUTER_API_KEY is not available in this PowerShell session"
}
```

Then run an approved inspection:

```powershell
cargo run -- inspect project-1
```

The response is untrusted: PMEMC validates it and stores proposals as pending
review. It does not create verified facts until the operator reviews and
finalizes them.

## Rotation

Revoke an exposed or expired key in OpenRouter, create a replacement, run
`pmemc auth set`, and confirm with `pmemc auth status`. Never place the
replacement in `.env`, source code, SQLite, Git configuration, or command-line
arguments.
