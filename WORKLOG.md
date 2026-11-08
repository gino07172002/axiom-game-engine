# WORKLOG

## 2026-11-09 (late night) — Claude (Opus 5.5) — paths and path following (plan-engine-components item 3), not committed

**Done (uncommitted, for review):** `path.spline` (2..256 local points, `closed`, `linear` | centripetal `catmull_rom` sampled 8 pieces/segment) and `path.follow` (path by entity ID, speed, `start` | `start_fraction`, `end` loop / ping_pong / stop + `on_arrive` behavior event with `follower` / `path` text fields, yaw `orient` (+Z forward) with `turn_rate` / `yaw_offset`, lateral `offset` with 1 m corner blending, `paused`, `enabled`) in `axiom-core/src/path.rs`; compile checks `invalid_path` / `path_conflict` (new `Registry::movers`: core.motion, fsm.agent, rts.unit, action.actor, fps.controller/sentinel) / `path_budget` (256 paths, 65,536 polyline nodes, 2048 followers). Runtime `axiom-runtime/src/path_follow.rs` (`path.move`, phase 12): flat node table with arc lengths built once, piece search from the previous piece, 0 allocs per steady step, 64 events/step budget halts explicitly, spawned paths/followers halt explicitly, checkpoints refuse explicitly, hot reload keeps progress when path + start/end/paused are unchanged. Actions `path.pause|resume|set_speed|restart|inspect|summary`; `observe` metric `path`. Ops `follow_path`, `edit_path_point` (+ preset `path`); ACL `create path`, `follow SEL PATH [speed= end= offset=]`, `path ID add|insert|move|remove`; editor commands `object.path`, `object.follow_path`, `path.add_point`, `path.remove_point`, "+ Object ▸ Path", Inspector ▸ Path points, gizmo (polyline, crosses, arrow, PATH/FOLLOW, start lines), draggable point handles (horizontal plane, snap), badges PATH / FOLLOW, parity tests. `perf_bench --generated-paths PxF`. Docs `docs/paths.zh-TW.md`. Tests 351 → 369.

**Measured (release):** 64 paths × 1024 followers step p50 0.201 ms (0 allocs), 64 × 2048 0.425 ms, 256 × 2048 0.412 ms; `path.move` build 0.6–1.7 ms. axiom-town A/B (5 interleaved rounds): step 0.0224 → 0.0215 ms, allocs 71/84 unchanged, observation 0.0425 → 0.0414 ms.

**Next:** item 4 (navigation agents). Possible follow-ups: Play-preview path overlay, vertical drag of point handles, cheaper yaw (≈0.2 µs/follower now, mostly atan2/sin/cos), checkpoint section for follower progress.

**Gotchas added:** `axiom editor --screenshot X.png` also writes `X.json`; do not redirect stdout to that same name (os error 32). Followers take the path's Y as their pivot, so a cube follower on a ground path sinks by half its height: lift the path (or offset the mesh part). `axiom import FILE` takes a world JSON file, not a package dir; a fresh `--db` with `--pack DIR` initializes from the package.
## 2026-11-09 (night) — Claude (Opus 5.5) — trigger zones (plan-engine-components item 2), not committed

**Done (uncommitted, for review):** `trigger.zone` component (box / sphere, world transform incl. parents, scale ignored; filter `{tags, components}` with selector AND semantics; `on_enter` / `on_exit` behavior.program events with optional text payload `trigger` / `entity`; `once`, `enabled`, `exit_margin` hysteresis) in `axiom-core/src/trigger.rs`, compile-time checks (`invalid_trigger`, `trigger_budget`: 256 zones; needs `trigger.detect`). Runtime `axiom-runtime/src/trigger.rs` (phase 25): hashed XZ grid (4 m cells, counting sort into reused arrays, fallback to the full list above 64 cells), sorted inside-vectors, 64 events/step and 262,144 candidate tests/step budgets that halt explicitly, spawned zones halt explicitly, checkpoints refuse explicitly, hot reload keeps occupancy. `observe` metric `trigger` + `trigger.inspect` / `trigger.summary`. Presets `trigger.box|sphere` (ACL `create trigger box|sphere`, editor `object.trigger_*`, "+ Object"), `TRIG` badge, wireframe gizmo, Play-preview Behavior overlay draws zones (pink when occupied). New ACL verb `remove SEL COMPONENT` (`ops::remove_component`, also used by Inspector ▸ Remove component). axiom-town bench / hair-clip FSM+near spots replaced (same at_bench/at_clip changes on the same ticks in a scripted walk). `perf_bench --generated-triggers ZxM`. Docs `docs/triggers.zh-TW.md`. Tests 331 → 351.

**Next:** item 3 (path following). Possible follow-ups: checkpoint support for occupancy (needs a per-system checkpoint section), stay events, bounds-based (not pivot) overlap.

**Gotchas added:** first brute-force version cost 0.93 ms/step at 256×1024 (release) → grid (0.076 ms). In editor Play preview the scene view is replaced by the game view, so play-time feedback must go into the app's Behavior overlay (app.rs), not the viewport gizmo. Parity tests compare JSON numbers literally: ACL `[2,1,0.5]` ≠ form-written `[2.0,1.0,0.5]`. The old town FSM spots were pushed by `nav.blocker` on step 1 ((6,0,4)→(6,0,3.85), (-2.3,…)→(-2.5,…)); zones are authored at the pushed positions.

## 2026-11-09 (later) — Claude (Opus 5.5) — perf baseline + editor stall fixes, not committed

**Done (uncommitted):** benchmark harness (`examples/perf_bench.rs` with counting allocator, `scripts/perf-bench.ps1`, `perf-summary.ps1`, `perf-live-edit.ps1`), baseline + results in `docs/perf-baseline.zh-TW.md`. Fixes: editor scenarios are background jobs (`jobs.rs`: cancel, progress, watchdog, timed join; Scenarios tab + status bar + `observe editor.jobs`); revision activation builds the next runtime on a worker and adopts it on the main thread (`WorldRuntime::adopt`, `RuntimeSystem: Send`), store decoded-revision cache shared with the bridge, editor `Arc<World>`, change-list diff and old runtime/world freed off-thread; animation asset / procedural geometry caches; RTS templates compiled in one pass; single production spawn without state clone; event delivery without consumers skips the state clone; static mesh vertices shared and not re-uploaded; shader pipelines cached by WGSL content with a 512-pipeline budget (`observe shader_gpu`); per-frame slow-frame profiler (`observe frame`).

**Next:** editor-local commits still block the main thread (SQLite fsync ~110 ms + double compile + JSON round trip; 4096 world ~330 ms) — needs async editor writes or a durability decision by the owner. Swapchain acquire (`get_current_texture`) blocks 200–475 ms per frame on this MX150/Optimus machine in every build (see `observe frame.last_slow`); try present mode / frame latency / backend.

**Gotchas added:** release `-j4` incremental rebuild of axiom-runtime ≈ 4–6 min (the first release build of the example after a profile change rebuilt all deps, 22 min). `observe` flattens metrics (use `$o.reload`, not `$o.metrics.reload`). `fps` in observe is smoothed with a 0.1 s cap and hides 250 ms frames; use `frame.frames`/`slow_frames`. No Python here; probes are temporary Rust examples (delete after use).

## 2026-11-09 — Claude (Opus 5.5) — lighting system (plan-engine-components item 1), not committed

**Done (uncommitted, for review):** `render.light` component (directional / point / spot / approximate area; strict validation; budgets 4 directional + 32 local) and `render.environment` resource (ambient color | gradient trilight, optional exp² fog) in `axiom-core/src/light.rs`; GPU packing `axiom-runtime/src/lighting.rs` + shared WGSL prelude `lighting.wgsl` (Globals 1184 → 2992 B, stride 3072). Defaults are pixel-identical (before/after diff on axiom-town, fps, shader-lab, action, effects-tour: ≤2 px differ by 1/255). Presets `light.*`, ACL `create light KIND`, editor commands `object.light_*`, "+ Object" entries, gizmos, `LIGHT` badge, parity tests. `content/light-lab` + `Play Light Lab.cmd`. Docs `docs/lighting.zh-TW.md`.

**Next:** spot shadows (measured: one 2048² shadow pass ≈ +0.9 ms/frame on rts-bench, debug build, vsync off), configurable sky gradient, shadow matrix following the camera (texel-snapped).

**Gotchas added:** Smoke screenshots take the window size from the primary monitor (1805×950 vs 960×600 changed mid-session); capture BEFORE/AFTER back to back (`git stash` tracked changes, build, capture, pop). Smoke runs are vsync-capped (AutoVsync), so frame-time A/B needs a temporary `AutoNoVsync` build. PowerShell variables are case-insensitive (`$long` == `$Long`). There is no Python on this machine; image diffs use C# via `Add-Type`.
## 2026-11-08 (night 3) — Claude (Opus 5.5) — Effects Atlas camera, editor plan, editor M1

**Done:** Effects Atlas mouse camera + navigation (`d86eb1e`; smoke PASS after the machine recovered). Editor plan `docs/plan-editor-human-tuning.md` (Godot/Unity parity, M1–M10) with spikes S1–S4 recorded (egui_dock 0.16 / egui-snarl 0.7.1 work with egui 0.31; Game view/camera preview via viewport composition, no offscreen pass; typed schema conforms on all 41 instances, ~3.5x bytes so projected on demand). **M1** editor shell: `editor/commands.rs` registry (32 commands) drives menu bar, Ctrl+Shift+P palette, shortcuts (overridable), F1 shortcut table, preferences (`*.editor-prefs.json`), status bar, `editor_view {command}` / `view --command`, `discover editor_commands`, preview Pause/Step. 257 tests, editor smoke ok.

**Next:** M2 (hierarchy context menu/rename/visibility/lock/reorder, fly camera, ortho views, box select, component gizmos, **egui_dock panels** moved here), then M3 Inspector 2.0 (typed schema, live apply). See plan file.

**Gotchas added:** egui 0.31 has no `Button::selectable`; use `Button::new(..).selected(..)`. Labels are selectable by default; node-graph drags need `selectable_labels = false`. Shortcut matching must treat `ctrl` and `command` as the same key (tests send CTRL without COMMAND). Glyphs like ▾ ❚❚ ⤒ are missing from the bundled fonts; prefer plain text or "…". Windows PowerShell 5.1 piping text into a native exe's stdin fails JSON parsing; pass a request file instead (effects smoke fixed). A bridge capture taken in the same request as a UI-changing command shows the previous frame; capture in a second request.
## 2026-11-08 (night 2) — Claude (Opus 5.5) — RTS Frontier playable + polish

**Done (pushed `8ba9d24`):** `content/rts` is now "Frontier" (generator `axiom-rts/src/frontier.rs`, export with `export_rts_pack frontier`): `rts.session.opponent` AI director, tower `attack`, `rts.attack.engage` (attack-move), depot-based victory, `approach()` cell spreading, shots/death presentation, health/production bars, minimap (`minimap: true`), `rts.camera`, `team_colors`. Core `ui.document.styles` + `chrome:"compact"`. Old 64-unit fixture = `content/rts-bench` (byte-identical to old pack); m2/animation/shader smoke and `build-m4-content.ps1` point there. `Play RTS.cmd` uses `data/frontier.sqlite`. 250 tests; m2(+M1.5/M1/M0), combat, m4 smoke PASS; release stress PASS.

**Next:** user feedback on Frontier feel/difficulty (opponent knobs live in `rts.session.opponent`; scripted win at 4:08 in `tests/rts_frontier.rs`, `FRONTIER_TRACE=1` prints a 30 s timeline). Ideas: fog of war, upgrades, more production buildings, mascot units.

**Gotchas added:** Smoke scripts were written for PowerShell 7; under 5.1 add `$null=$p.Handle` after `Start-Process -PassThru` and `Get-Content -Encoding utf8` before `ConvertFrom-Json` (fixed in m2/combat/m4; `build-m4-content.ps1` still needs pwsh because of `-AsHashtable`). `cargo test` stops at the first failing test binary; use `--no-fail-fast` to see all failures. `serde_json::to_value` of f32 differs from the written file (shortest f32 text); compare via a text round-trip. GPU/debug frame timings on this laptop swing 2–5x with other load; do not compare single runs. `gh` is not installed in PowerShell; query CI via the public API by `head_sha`.
## 2026-11-08 (late night) — Claude (Opus 5.5) — Axiom Town, HUD regions, collision

**Done (pushed):** Axiom Town social demo (`content/axiom-town`, `Play Axiom Town.cmd`, `runtime.bindings`, FSM `wander/face/clip`, `ui.nameplate`). `9429b54`: `ui.document.regions` (anchored HUD), core `nav.blocker` (module `axiom_core::blocker`) shared by Action obstacles + FSM agents, agent separation, wander resampling. 242 tests, M1 smoke PASS.

**Next:** RTS demo is not fun: the enemy faction has no AI at all (only idle soldiers auto-attack within 4 m), no health bars, hit/death feedback, or fog; units are primitive boxes. User asked to look at it — proposal pending (enemy AI director + feedback visuals + map/content tuning).

**Gotchas added:** Do not name a core module `nav`: `axiom_rts` also exports `nav`, and tests glob-import both crates (E0659 ambiguity). `cargo test` can fail with "failed to remove axiom.exe" (os error 5) if a smoke/play process is still exiting; rerun. After `env.ps1`, `$env:TEMP` points to `.tools\tmp`, so read logs after sourcing it.
## 2026-11-08 (night) — Claude (Opus 5.5) — round 2: CI, data-only combat, benchmark, editor polish

**Done (pushed):** `.gitattributes` + Windows CI (green from `b567f5b`). FSM `attack` + state `emit` → `content/skirmish` (data-only victory/defeat). `benchmarks/ai-tasks.json` + `docs/benchmark-ai-tasks.md` (ACL 9.5 KB vs selective JSON 18.8 KB vs legacy 136.6 KB, replay, bytes not tokens). Editor `ab42cc4`: search `tag:/has:/prefab:`, component picker, Global/Local gizmo (Rotate default changed to Global), skinned-mesh picking/focus, `view --open`. 234 tests.

**Next candidates:** agent-driven benchmark (real exploration/retries), death visuals + ranged attacks, mascot as a controllable character, cross-platform build check.

**Gotchas added:** In PowerShell, `R` is an alias of `Invoke-History`; never name helper functions `R`. Commit messages containing double quotes break native arg passing in PS 5.1, so use `git commit -F file`, and write the file with `[IO.File]::WriteAllText` (no BOM); `Set-Content -Encoding utf8` adds a BOM that ends up in the subject (happened in `ab42cc4`). Poll CI by `head_sha`, not "latest run".

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
