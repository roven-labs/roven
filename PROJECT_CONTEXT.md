# Roven

Roven helps students and developers work with a trusted project in a controlled
terminal session.

It validates a trusted Git project and stores registration metadata plus an
evidence-backed project summary. Conversation and tool results are stored
separately in the local session history.

The user can inspect project files through the trusted read-only tools, while
provider keys remain in the operating-system credential store.

The user stays in control: Roven only works inside the trusted project folder and does not remove project data without explicit user action.

`/generate-resume <job description>` generates a Markdown project section using
only the supplied job description and stored project summaries as evidence. It
does not read the workspace or repository, expose provider tools, or invent
achievements, metrics, technologies, or responsibilities. Output is stored at
`%LOCALAPPDATA%\Roven\data\resumes\<uuid>.md`. `/resume` continues to list and
open sessions for the current canonical workspace.
