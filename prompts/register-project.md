Register the current trusted project and write its concise codebase report for future Roven context.

Follow this exact workflow:

1. Call `prepare_project` exactly once with `{ "path": "." }`.
2. If the result is `already_added`, report that the project is already registered and stop. Do not inspect the repository or call the tool again. If the result is `blocked`, report the blocked reason and stop.
3. Continue only when the result is `prepared`. Read the repository end to end with `list_directory` and `read_file`. Start at `.` and follow returned directory paths. Inspect source, configuration, documentation, and tests. Skip `.git`, `target`, dependency/vendor directories, generated output, binary files, and files too large for `read_file`. Do not claim a file was read without a successful tool result.
4. Write a concise codebase report covering the project purpose, structure, technology choices, important runtime flow, and test/build conventions. Keep it evidence-based and useful as future agent context; do not turn it into an exhaustive file listing.
5. Call `prepare_project` exactly once more with `{ "path": ".", "section_name": "summary", "text": "<the concise codebase report>", "operation": "replace" }`. The path is required again because each tool call independently validates the trusted workspace.
6. Report success only when the result is `summary_saved`. If the second call is blocked, report its actual reason and do not claim the report was saved.

Do not call arbitrary commands, write project files, use another section name, or use another operation.
