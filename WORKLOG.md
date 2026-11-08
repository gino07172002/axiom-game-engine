# WORKLOG

## 2026-11-08 — Claude (Opus 5.5) — AI authoring gaps, milestones M0–M3

**Plan:** `docs/plan-ai-authoring-gaps.md` (milestones M0–M12; M13 dropped — the engine is driven by AI agents through the CLI, not through an external model API). Each milestone has an execution-log entry with evidence.

**Done (all pushed to https://github.com/gino07172002/axiom-game-engine `main`):**
- M0: Codex's showcase work committed separately (`92c29a1`). Logo added: `assets/branding/`, `crates/axiom-runtime/src/branding.rs` (window icon + editor toolbar), README/docs headers.
- M1 `b304196`: editor bugs (resource overwrite after failed load, stale drag, modifier shortcuts + Ctrl+Y, same-frame CAS coalescing, scene-id message, F-focus bounds) + editor smoke script ExitCode fix.
- M2 `255e471`: `Selection.id_glob/where/format=lines`, `local::matches()`, `CoreEdit::SetWhere`.
- M3 `aaed4f0`: ACL (`axiom-core/src/acl.rs`, `Request::Acl`, `axiom acl`, `discover acl`), display-only float rounding.

**Next:** M4 generic behavior layer (`behavior.fsm` resource + `axiom-runtime/src/behavior.rs`) and per-entity `inspect` (incl. FPS sentinel state). Then M5 scenario assertions + seeded RNG, M6 load/RSS measurement (spike S1 decides binary cache), M7 plugin registration, M8–M12 editor features.

**Environment / gotchas:**
- Always `. .\scripts\env.ps1` before cargo. Gate = `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (197 passed at M3).
- `ensure!(cond, Diagnostic::error(..))` loses the structured diagnostic code (anyhow wraps it as text). Use `if !cond { return Err(Diagnostic::error(..)) }`.
- Adding fields to `Selection` trips clippy `large_enum_variant`; it is boxed in `Request::Select` and `CoreEdit::SetWhere`.
- egui tests: `i.modifiers` comes from `RawInput.modifiers`; Ctrl shortcuts need `Modifiers::CTRL | Modifiers::COMMAND`.
- Windows PowerShell 5.1: `Start-Process -PassThru` needs `$null = $p.Handle` or ExitCode is `$null`; embedded `"` in native args must be written `\"`.
- Codex may work in parallel: check `git status` for foreign edits before each milestone. Never commit third-party assets (PMX/FBX/Unity exports live under ignored `data/`).
- QA databases go under the scratchpad / `artifacts`, never the user's `data/*.sqlite`.
