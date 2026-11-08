# WORKLOG

## 2026-11-08 (evening) — Claude (Opus 5.5) — editor milestones M8–M12 done; plan complete

**Done (pushed):** M8 `076e339` schema forms, History, Problems. M9 `44dfb38` multi-select, clipboard (egui Copy/Paste events), drag reparent, snap, grid. M10a `b667169` proposal queue (`propose`/`proposals`/`resolve_proposal`, ACL `--propose`, Proposals tab). M10b `c77714d` `editor_view` (select/focus/dock/capture), selection context, request bytes. M11 `feb1849` FSM overlay in Play preview, Scenarios tab, `view --play`. M12 `7a09af2` prefab + material inspectors, color pickers, Graphs tab. Workspace tests: 228 passed.

**In progress (not committed by me):** a background agent is building the mascot (`scripts/blender-mascot.py`, `content/mascot/`, `docs/mascot.zh-TW.md`) from the owner's design sheets in `D:\newExport` with overlay verification. The outfit is deliberately fully covered (sheet lists a child-like 125–135 cm height). Review its overlay report before committing.

**Known gaps (recorded in the plan):** local/world gizmo toggle; picking imported skinned meshes; GUI automation for drag-reparent; material inspector not screenshot-verified; FSM/RNG not in FPS Rapier checkpoints; VRAM unavailable on Vulkan (wgpu 24).

**Gotchas added:** bridge `observe` answers from a snapshot thread, so it can succeed before the first frame; wait on side effects (e.g. capture file size stable). `git add -A` is unsafe while another agent writes files; stage explicit paths.

## 2026-11-08 (later) — Claude (Opus 5.5) — milestones M4–M7 + README

**Done (pushed):** M4 `bc73b91` generic FSM (`fsm.rs`, `content/behavior-lab`, `fsm.inspect`, `fps.inspect`, `axiom call`). M5 `46e5dea` seeded RNG (`core.rng`, checkpoint `rng`), FSM `chance`, scenario `field`/`call` assertions, `state_hash`. README rewritten for open source (`40de979`; old one is `docs/user-guide.zh-TW.md`). M6 `2d4fa4b` `verify --measure`, gpu/process memory; spike S1 → no binary cache. M7 `f9302ca` `Plugin`, `RuntimeRegistry::with/try_with`, CLI is now a lib (`axiom::main_with`), example `crates/axiom-cli/examples/custom_plugin` + `examples/spin-pack`.

**Next:** M8 editor schema-driven component forms + history timeline + diagnostics panel; then M9 (multi-select, clipboard, drag-reparent, snapping, picking, grid, tree-filter depth), M10 (proposals table + review panel, selection→ACL context, bytes, CAS conflict view, bridge editor.select/focus/screenshot), M11 (scenario panel, FSM overlay in play-in-editor), M12 (prefab body, material controls, graph read-only view).

**Gotchas added:** `cargo fmt` reflows freshly written files; re-read before follow-up edits. wgpu 24 on Vulkan returns no allocator report (VRAM honestly `available:false`). Release build: `cargo build --release -p axiom -j1` (~5–10 min); don't edit CLI sources while it runs. Workspace tests: 212 passed at M7.

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
