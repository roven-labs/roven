# Credential setup on Windows

This file documents PMEMC's local credential lifecycle. It does not configure
the UI preview to use OpenRouter or any other provider.

```powershell
pmemc auth set
pmemc auth status
pmemc auth remove
```

`auth set` hides credential input and stores it in Windows Credential Manager.
`auth status` reports configuration status without printing the credential.
`auth remove` deletes the stored credential.

Do not put secrets in repository files, command-line arguments, application
databases, logs, test fixtures, or terminal output.

The current UI preview does not read this credential and has no configured
model, external transport, agent tool, or repository workflow. Accordingly,
this document contains no model or network setup instructions.
