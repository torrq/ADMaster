# Agent Rules & Guidelines

As an AI agent operating in this repository, you are bound by the following strict documentation protocol.

## 1. Documentation Integrity (The "Doc-Sync" Rule)
Any code changes made to this project -- whether refactoring, adding new features, or fixing bugs -- must be accompanied by an evaluation of the project's core documentation.

Whenever you modify code, you MUST ask yourself:
* **Does this change alter how the user interacts with the GUI or CLI?** If so, immediately update `README.md`.
* **Does this change alter the core technical architecture, DSP invariants, or workflow skills?** If so, immediately update `.agents/skills/audio-description-frame-perfect-sync/SKILL.md` and `.agents/skills/adsync-alignment/SKILL.md`.
* **Does this change introduce new workflows, build/release rules, or guidelines for future agents?** If so, immediately update `AGENTS.md`.

**CRITICAL INSTRUCTION:** Do not wait for the user to ask you to update the documentation. If a code change warrants a documentation update, you must include the documentation update in the same set of modifications. Treat documentation as first-class code.

## 2. Professionalism & Quality
* Write clean, maintainable, self-documenting Rust code with zero compiler warnings.
* Always validate changes against cross-platform requirements (Windows, Linux, macOS) and keep GUI dependencies gated behind `#[cfg(feature = "gui")]`.
* Keep `Cargo.lock` synced with `Cargo.toml` so `--locked` CI release builds always pass.
* Do not leave broken states or half-implemented features.
* Never reference commercial brands or third-party proprietary products other than the open-source projects used or adapted by ADMaster.

## 3. Workspace Hygiene (The "Scratch" Rule)
* **Never pollute the repository root** with intermediate test files, generated plots, temporary MKVs, or other leftover outputs.
* All temporary artifacts, scratch scripts, or experimental outputs must be generated inside the `scratch/` directory (which is ignored by Git).

## 4. Pure ASCII Protocol (No Unicode Characters)
* **All Rust source files, Python asset scripts, YAML workflows, JSON/TOML configuration files, markdown documentation, comments, and console outputs MUST be 100% pure ASCII.**
* Never use Unicode checkmarks (use `[OK]`), Unicode cross marks (use `[X]`), em-dashes (use `--`), en-dashes (use `-`), Unicode bullets (use `*`), Unicode arrows (use `->` or `<-`), curly quotes (use standard `'` or `"`), mathematical symbols (use `+/-`, `*`, `pi`, `^2`), or box-drawing characters.
