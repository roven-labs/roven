# Setup

Store an OpenRouter API key in the operating-system credential store:

```powershell
pmemc auth set
pmemc auth status
pmemc auth remove
```

`auth set` hides credential input and never prints the key. Bare `pmemc` reads
the credential only after the folder-trust gate is accepted, then uses it for
the fixed `openai/gpt-oss-20b:free` OpenRouter model.

Do not put secrets in repository files, command-line arguments, PMEMC's local
conversation files, logs, test fixtures, or terminal output.
