# PMEMC

PMEMC is a native Windows terminal application with a local UI preview and
local credential management.

Running bare `pmemc` opens a full-screen UI preview. It lets you compose local
turns and displays a fixed preview reply; it does not read a credential, make
an external request, or contact a model provider.

Credential management remains available through:

```powershell
pmemc auth set
pmemc auth status
pmemc auth remove
```

`auth set` stores a secret in Windows Credential Manager without echoing it.
`auth status` reports only whether a credential is configured. `auth remove`
deletes that stored credential. PMEMC does not print the credential, write it
to a repository, or keep it in a PMEMC database.

The UI preview has no provider transport, streaming, persistent session,
agent tools, repository access, source discovery, model selection, or external
request capability. Those capabilities require their own approved contract.

## Install

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new PowerShell session and run `pmemc` from any directory.
