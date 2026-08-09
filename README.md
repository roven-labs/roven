# PMEMC

PMEMC is a native Windows CLI at a capability-free V1 reset point.

The only active command group is local credential management:

```powershell
pmemc auth set
pmemc auth status
pmemc auth remove
```

`auth set` stores a secret in Windows Credential Manager without echoing it.
`auth status` reports only whether a credential is configured. `auth remove`
deletes that stored credential. PMEMC does not print the credential, write it
to a repository, or keep it in a PMEMC database.

PMEMC currently has no conversational session, agent tools, repository
registration, project database, source discovery, CodeGraph integration,
model selection, or external request capability. Future capabilities require
their own approved V1 contract before they are implemented.

## Install

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new PowerShell session and run `pmemc` from any directory.
