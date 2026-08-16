# Provider setup

Roven lets you choose the provider, model, endpoint, and API key. The endpoint
must be the complete HTTPS chat URL accepted by that provider. Roven does not
append a chat path to the endpoint you enter. For context usage, it may call
the provider's documented metadata route separately.

## Ollama Cloud

Use Ollama's native API endpoint when you want Ollama's real context usage
metadata and native tool-calling format:

```text
https://ollama.com/api/chat
```

Create the profile:

```powershell
pmemc auth set
```

Enter values similar to these when prompted:

```text
Profile name: Ollama Cloud
Provider HTTPS endpoint: https://ollama.com/api/chat
Model ID: minimax-m3:cloud
API key: <your Ollama API key>
```

Select it as the default:

```powershell
pmemc auth use
```

Choose `Ollama Cloud` by its number, then start Roven:

```powershell
pmemc
```

Do not use this endpoint for native Ollama usage:

```text
https://ollama.com/v1/chat/completions
```

That is Ollama's OpenAI-compatible route. It can work for requests, but it is
not the native route used for Ollama context-window metadata in Roven.

## OpenRouter

Create a profile with OpenRouter's complete chat-completions endpoint:

```powershell
pmemc auth set
```

Use values similar to these:

```text
Profile name: OpenRouter
Provider HTTPS endpoint: https://openrouter.ai/api/v1/chat/completions
Model ID: <OpenRouter model id>
API key: <your OpenRouter API key>
```

Select and start it:

```powershell
pmemc auth use
pmemc
```

## Check or change the active provider

```powershell
pmemc auth list
pmemc auth status
pmemc auth use
```

`auth list` shows the endpoint and model for every profile. Confirm that the
selected Ollama profile says exactly `https://ollama.com/api/chat`.

The executable update does not rewrite existing profiles. If an older Ollama
profile still points to `/v1/chat/completions`, create a new native profile and
select it with `pmemc auth use`.

## Context usage

The footer displays only a percentage, for example:

```text
minimax-m3:cloud · 2% context used
```

For native Ollama, Roven reads the model context window from Ollama's model
metadata and the used prompt tokens from the final chat response. For
OpenRouter, it reads the model context length from OpenRouter's model metadata
and prompt usage from the response usage object. These are real
provider-reported values, not character-count estimates. The percentage can
change after each model response, including responses that call tools.

If the footer says `context unavailable`:

1. Run `pmemc auth status`.
2. Run `pmemc auth list` and check the selected endpoint.
3. For Ollama Cloud, recreate or select a profile using exactly
   `https://ollama.com/api/chat`.
4. Confirm that the API key and model ID are valid.

## API-key storage and diagnostics

API keys are stored in the operating-system credential store. They are not
written to `provider-profiles.json`, prompts, or normal application logs.

On Windows, provider metadata and diagnostics are stored under:

```text
%LOCALAPPDATA%\Roven\data
```

When an Ollama stream fails, Roven preserves the raw Ollama stream in:

```text
%LOCALAPPDATA%\Roven\data\ollama-stream-failures.log
```

That diagnostic file can contain model output and tool-call arguments. Treat it
as local application data and do not share it if the response contains private
project information.
