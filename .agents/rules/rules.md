# Agent Rules & Guidelines

As an AI agent operating in this repository, you are bound by the following strict documentation protocol.

## 1. Documentation Integrity (The "Doc-Sync" Rule)
Any code changes made to this project—whether refactoring, adding new features, or fixing bugs—must be accompanied by an evaluation of the project's core documentation.

Whenever you modify code, you MUST ask yourself:
* **Does this change alter how the user interacts with the script?** If so, immediately update `README.md`.
* **Does this change alter the core technical architecture, dependencies, or required skills?** If so, immediately update `.agents/skills/audio-description-frame-perfect-sync/SKILL.md`.
* **Did you add, remove, or significantly modify the purpose of a script?** If so, immediately update its corresponding index file (`scripts/python/README.md` or `scripts/powershell/README.md`).
* **Does this change introduce new workflows or require new guidelines for future agents?** If so, immediately update `AGENTS.md`.

**CRITICAL INSTRUCTION:** Do not wait for the user to ask you to update the documentation. If a code change warrants a documentation update, you must include the documentation update in the same set of modifications. Treat documentation as first-class code.

## 2. Professionalism & Quality
* Write clean, maintainable, self-documenting code.
* Always validate changes against cross-platform requirements (macOS/Windows).
* Do not leave broken states or half-implemented features.
* Prioritize robust error handling and logging over print statements.

## 3. Workspace Hygiene (The "Scratch" Rule)
* **Never pollute the repository root** with intermediate test files, generated plots, temporary MKVs, or other leftover script outputs.
* All temporary artifacts, scratch scripts, or experimental outputs must be generated inside the `scratch/` directory (which is ignored by Git). If a tool you are using (like `describealign`) auto-generates directories, you must run it from within the `scratch/` directory or immediately clean up its generated junk when finished.
