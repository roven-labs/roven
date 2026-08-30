Register the current trusted project and build its V2 project snapshot for later tailored resume generation.

Inspect the repository first. Ask the user for the resume-ready `project_name` and the context/contribution facts that source code cannot reveal. Then call `prepare_project` once with `path`, `project_name`, `project_facts`, `user_context_facts`, and `user_contribution_facts`. If the project is already registered or registration is blocked, report the result and stop.

Inspect the repository using the available read-only project tools. Read enough relevant source code, configuration, documentation, and tests to understand the important parts of the project. Do not attempt to document the repository file by file.

The saved project facts should preserve high-value project evidence, not general codebase documentation.

Capture information that establishes:

* **Project purpose** — the problem being solved and the intended use of the system.
* **Core capabilities** — the most meaningful things the implemented system can do.
* **Technical architecture** — the important components and how they work together.
* **Engineering highlights** — significant implementation challenges, technical mechanisms, architectural decisions, algorithms, workflows, integrations, or other non-trivial engineering.
* **Technology usage** — important languages, frameworks, platforms, databases, models, APIs, protocols, or infrastructure together with how they are meaningfully used.
* **Quality and reliability evidence** — meaningful testing, security, performance, reliability, validation, deployment, or operational engineering.
* **Measurable evidence** — concrete numbers, limits, scale, performance results, test results, or other measurements only when supported by repository evidence.

Prefer facts that can later provide the building blocks of a strong resume bullet: what was built, how it was implemented, what makes it technically significant, and any verified result or scale.

Omit repository details that do not contribute to those categories. Do not preserve file-by-file descriptions, routine boilerplate, ordinary configuration, exhaustive endpoint or component lists, dependency inventories, development trivia, or repeated implementation details merely because they exist in the repository.

Describe technologies in the context of what they accomplish rather than listing them without meaning.

Distill low-level implementation details into the engineering capability they demonstrate while preserving technically important specifics.

Do not invent impact, metrics, scale, technical decisions, or capabilities. Base every saved fact on successfully inspected repository evidence.

Do not infer that the user personally implemented a feature merely because it exists in the repository. Record project evidence only; personal contribution can be established separately.

Keep the final evidence profile compact and information-dense. If a category has no meaningful evidence, omit it rather than filling it with weak information.

After the evidence profile and questionnaire are complete, pass them to `prepare_project` as the V2 `project_facts`, `user_context_facts`, and `user_contribution_facts` arrays.

Report success only when the snapshot has actually been saved. Do not modify project files or perform unrelated actions.
