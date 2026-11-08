# Axiom contributor and agent guide

This is a Rust workspace. Do not add Bevy. Preserve the separation between transactional authoring (`axiom-core`) and fixed-step simulation (`axiom-runtime`). FPS schemas, legacy migration, and content adapters belong in `axiom-fps`; native FPS gameplay belongs in `axiom-runtime/src/fps`, registered through the generic runtime interface.

## Windows environment

Run `. .\scripts\env.ps1` before Cargo. It selects this workspace's toolchain/cache/temp directories on D: and the portable GNU linker in `.tools/mingw64/bin`. Run `scripts/setup.ps1` to bootstrap a fresh checkout. Do not move build caches to C:; space is limited.

## Change the game without changing Rust

1. `target/debug/axiom.exe schema`
2. `target/debug/axiom.exe query` to get current revision.
3. Submit one `apply` JSON request with `expected_rev` and a bounded list of typed edits.
4. `verify`; `observe` through `--connect 127.0.0.1:4242` to confirm live revision.
5. Use `undo REV` to append a restoration revision if needed.

Examples live in `examples/`. World IR v2 content packages live in `content/`; `assets/worlds/` retains the v1 migration fixture. `import` validates a full world; the runtime never reparses asset files each frame. Use `preview` to validate an edit batch without committing it. Do not directly modify SQLite with unrelated tools.

Shared behavior, narrative, and UI authoring types live in `axiom-core/src/gameplay.rs`; execution, bindings, and checkpoints live in the runtime's corresponding modules. UI, scenarios, and loopback commands share typed gameplay commands. Checkpoints are separate from authoring revisions and require explicit native-system support; never silently drop unsupported simulation state.

## Engine changes

Run `cargo fmt --all`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`. For renderer changes, run the GPU smoke examples from `docs/user-guide.zh-TW.md` and inspect the generated screenshots. `scripts/m0-smoke-test.ps1` covers FPS, empty worlds, multiple cameras, content switching, and transactional edits with the same executable. Do not claim GPU timing or token efficiency improvements without measurement. Keep request errors recoverable and protocol output compact.

`scripts/m1-smoke-test.ps1` includes M0 plus ADV endings, localization, image bindings, checkpoints, live commands, and data-only edits. Use isolated QA databases; preserve the user's default world and saves.

RTS schemas, planar navigation, economy, pure simulation, and content recipes belong in `axiom-rts`; native integration belongs in `axiom-runtime/src/rts.rs`. Use registered capability actions and generic pointer rays rather than genre branches in the app. RTS HUD uses shared `ui.document` bindings. Runtime-produced entities are separate from authoring revisions; preserve typed columns and generation handles.

Shared RTS/Action planar collision and damage primitives belong in `axiom-combat`; native orchestration belongs in `axiom-runtime/src/combat.rs`. Opt in with `combat.session` and `combat.resolve`. Controllers publish bodies and hit intents through generic runtime hooks; damage commits once before cross-package event delivery. Preserve owner and generation checks, friendly-fire exclusion, dodge windows, resource conservation, and single death transitions. `scripts/combat-smoke-test.ps1` validates actual hero/squad focus fire, the joint ADV ending, hot reload and AI protocol retries using isolated databases.

Effects Atlas lives in `content/effects-tour`; its inventory pins the official Flax scene revision and records implementation differences. Procedural geometry, seeded VFX, bone sockets and depth/color post effects use generic runtime systems. GUI parameter edits must use the same transactional protocol as AI edits. GPU particle observations are CPU reference values; validate actual compute output with `cargo run -p axiom-runtime --example verify_vfx_gpu`. `scripts/effects-smoke-test.ps1` captures all 36 stations and checks live edits, retries and vertex-pipeline recovery. Never label CPU subdivision, thin-surface transmission, or forward material diagnostics as GPU tessellation, full SSS, or deferred G-buffer sampling.

For AI authoring prefer `discover`, stable-ID `select`/`set_field`, projected `observe_delta`, and `execute` retries. Authoring receipts are durable; runtime receipts require the observed host session. Never equate serialized bytes with tokens.

`scripts/m15-smoke-test.ps1` includes M1 plus local edits, diagnostics, deltas, retries, and benchmark checks. `scripts/m2-smoke-test.ps1` includes all previous milestones plus RTS native rendering/functions and release stress. Build release first with `cargo build --release -p axiom -j1`; `scripts/m2-stress-test.ps1` measures fresh headless processes for 64/128/256 units. Keep CPU measurements distinct from GPU screenshot validation. Unsupported private-system checkpoints must fail explicitly.
