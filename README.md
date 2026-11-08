<p align="center">
  <img src="assets/branding/axiom-logo-white.png" alt="Axiom" width="320">
</p>

<p align="center">
  <b>An AI-native game engine written in Rust.</b><br>
  AI agents build and change games through a small typed, transactional protocol instead of an editor or engine source code.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-stable-orange.svg">
  <img alt="Status" src="https://img.shields.io/badge/status-prototype%20v0.4-yellow.svg">
  <a href="README.zh-TW.md"><img alt="中文" src="https://img.shields.io/badge/文件-繁體中文-lightgrey.svg"></a>
</p>

---

## What is Axiom?

Most engines are built for people working in an editor. Axiom is built for AI coding agents, with people reviewing and steering the work.

- **The game is a database, not a pile of code.** A world is a typed document made of entities, components, prefabs, scenes and resources. Every change is a transaction that creates a numbered revision, and each revision can be previewed, diffed or undone.
- **Agents read only what they need.** Agents can use selectors (`enemy/*`, `where core.health/value<50`), project only the fields they need and get compact one-line-per-object output. They never have to read a whole scene to change one value.
- **Changes are checked before they are kept.** Schema validation, compare-and-swap revisions, idempotent retries, headless scenarios with assertions, and deterministic state hashes let an agent confirm that a change worked without looking at the screen.
- **Separate authoring and runtime.** Authoring (`axiom-core`) is transactional and stored in SQLite. The runtime (`axiom-runtime`) steps a compiled world at a fixed 120 Hz with native Rust systems. The running game hot-reloads new revisions at safe points.
- **No Bevy.** Axiom has its own small ECS-like world on top of `wgpu`, `winit`, `glam`, `egui` and `rapier3d`.

> **Status:** Axiom is an experimental prototype (v0.4). It has only been tested on Windows 11 (x86_64, GNU toolchain, Vulkan). APIs and file formats may change.

## A 60-second tour

Every command below uses the same database: `axiom --db lab.sqlite …`. The first command creates it from a content pack.

```text
$ axiom --db lab.sqlite --pack content/behavior-lab acl -e "select entities[tag=enemy] fields core.health/value limit 3"
enemy/0 core.health/value=100
enemy/1 core.health/value=100
enemy/2 core.health/value=100
matched=6 rev=1 next=l1.1.…

$ axiom acl -e "set enemy/* core.health/value 40 where core.transform/position/0>=18"
ok rev=2 changes=2
+ /entities/@enemy~14/components/core.health = {"value":40}
+ /entities/@enemy~15/components/core.health = {"value":40}

$ axiom call fsm.inspect '{"id":"enemy/0"}'            # "why isn't this enemy chasing?"
{"ok":true,"result":{"state":"idle","transitions":[…,
  {"to":"chase","when":"near(player)<=15","measured":30.016663,"pass":false}]},"rev":2,"tick":0}

$ axiom scenario examples/behavior-chase.scenario.json  # headless; same state_hash in every process
{"ok":true,"state_hash":"…","assertions":[ 5 passed ], …}

$ axiom acl -e "undo 1"
ok rev=3 restored_from=1
```

(The edit above adds a local override to prefab instances. The prefab default stays untouched.)

## Features

| Area | What you get |
|---|---|
| **Authoring protocol** | Typed JSON requests (`select`, `preview`, `apply`, `undo`, `diff`, `history`, `discover`) and **ACL**, a terse line-based command language that compiles to the same requests. Every request uses compare-and-swap revisions and request-ID retries. |
| **Queries** | ID globs, tag and component filters, `where` predicates that see prefab-inherited values, field projection, cursors and byte budgets. Output can be JSON or compact lines. |
| **Batch edits** | `set_where` updates every matching object in one revision. `spawn` instantiates prefabs N times. Edits are validated against component and resource schemas. |
| **Runtime** | Fixed-step 120 Hz simulation, generation-checked entity handles, typed component columns, and hot reload that keeps state across revisions. |
| **Behavior** | Data-defined finite state machines (`fsm.machines`) for any entity, a shared gameplay/narrative/UI IR, and native FPS, RTS and action-combat modules. |
| **Verification** | Scenarios with input, command and assertion timelines; `field` and `call` assertions; a seeded RNG; state hashes for cross-process comparison; checkpoints and save slots. |
| **Rendering** | `wgpu` forward renderer with authored lights (`render.light`: directional with shadows, point, spot, approximate area) and ambient/fog (`render.environment`), lit, toon and custom WGSL materials, skeletal animation (glTF/GLB, PMX), GPU compute particles, decals, fog and post effects. |
| **Editor** | A native scene editor (hierarchy, viewport gizmos, inspector) that uses the same revisions as AI agents. It picks up external revisions live. |
| **Live bridge** | Loopback JSONL (`--connect 127.0.0.1:4242`) to observe, command, edit and save a running game. MCP is not required. |

## Getting started

### Requirements

- Rust (stable). The repository pins `stable-x86_64-pc-windows-gnu` in `rust-toolchain.toml`.
- A GPU supported by `wgpu` (Vulkan, DirectX 12 or Metal) for the windowed modes. Headless commands do not need a GPU.

### Build on Windows

```powershell
git clone https://github.com/gino07172002/axiom-game-engine.git
cd axiom-game-engine
.\scripts\setup.ps1          # one-time: installs a workspace-local Rust + MinGW toolchain under .tools/
. .\scripts\env.ps1          # every new shell: points cargo at the workspace toolchain
cargo build --workspace
```

### Build on Linux / macOS (untested)

Change `channel` in `rust-toolchain.toml` to `stable`, then run `cargo build --workspace`. You may need your platform's usual `winit`/`wgpu` system packages.

### Run something

```powershell
# Play the bundled FPS sample (WASD, mouse, left click to shoot, Tab for the console)
target\debug\axiom.exe play

# Open the scene editor
target\debug\axiom.exe editor

# Any content pack into its own database
target\debug\axiom.exe --db data\lab.sqlite --pack content\behavior-lab play --start
```

On Windows you can also double-click the `Play *.cmd` and `Edit Axiom.cmd` launchers in the repository root.

## Working with AI agents

An agent only needs the `axiom` executable. A typical loop:

1. **Discover**: `axiom discover acl` (grammar), `axiom discover local` (JSON schema), `axiom schema components`. Pass `--if-hash` to skip re-reading a schema that hasn't changed.
2. **Read** only what is needed: `axiom acl -e "select … where … fields … limit N"`.
3. **Change** in one transaction: `axiom acl --preview script.acl`, then `axiom acl script.acl`. Multi-line edit scripts commit as a single revision.
4. **Verify** without a screen: `axiom scenario file.json`, `axiom call fsm.inspect '{"id":"…"}' --ticks 600`, `axiom verify --measure 5`.
5. **Observe live**: `axiom --connect 127.0.0.1:4242 observe-delta --field /rev`. The running game reloads new revisions on its own.
6. **Roll back**: `axiom undo REV` appends a restoring revision. History is never rewritten.

ACL at a glance (`axiom discover acl` prints the full grammar):

```text
select SEL [where F OP V [and …]] [fields F,F] [limit N]     # read
inspect ID | history [N] | diff A B | undo REV                # read
set SEL F VALUE [where …]                                      # edit
spawn PREFAB count=N id=enemy/{i} at=x,y,z step=dx,dy,dz tag=a
delete SEL
SEL: enemy/7 | enemy/* | entities[tag=enemy,has=core.health] | prefabs | resources[id=x]
F:   core.health/value (component field) | label | /raw/json/pointer
```

The same operations are available as JSON over `axiom serve` (stdio) and the loopback bridge. [`docs/m15-implementation.md`](docs/m15-implementation.md) describes the protocol contract.

## Example content

Every demo is a data-only content pack. All of them run on the same executable.

| Pack | What it shows |
|---|---|
| `content/fps` | First-person shooter: hitscan, sentinels, cover, headshots |
| `content/rts` | Frontier skirmish vs. a data-driven AI opponent: three lanes, towers, three unit types, minimap, health bars and combat effects |
| `content/rts-bench` | Fixed 64-unit RTS regression / stress fixture (box selection, A* navigation, gathering, production; 128/256 via `export_rts_pack`) |
| `content/action` | Third-person melee: combos, dodge windows, stamina, lock-on |
| `content/adv` | Branching narrative with three endings, localization (zh-TW/en), save slots |
| `content/m4` | Composition of RTS, action and ADV packs in one world with cross-pack events |
| `content/behavior-lab` | Generic FSM agents: idle → chase → attack, inspectable per entity |
| `content/effects-tour` | 36 material/VFX stations, including GPU particles, decals, fog and heat distortion |
| `content/shader-lab` | Lit, toon and custom WGSL materials with hot-reloadable parameters |
| `content/animation`, `content/love-no-war` | Skinning, blending, IK and retargeting, plus a rhythm duet prototype |
| `content/empty`, `content/two-cameras` | Minimal worlds without gameplay, and multi-camera views |

All bundled models and images are original and generated by scripts in `scripts/` (Blender and PowerShell). The Effects Atlas reimplements the *topics* of the FlaxEngine feature-tour samples. It contains no Flax assets.

## Architecture

```text
 agent / CLI / editor ──► axiom-core (authoring)            axiom-runtime (simulation)
   ACL, JSON requests       World IR v2, schemas, registry     fixed 120 Hz step, systems
                            transactions + revisions (SQLite)  ─► compiled world, hot reload
                            preview / diff / undo / receipts    wgpu renderer, egui UI, bridge
```

| Crate | Responsibility |
|---|---|
| `axiom-core` | World IR, type and capability registry, compiler, transactions, selectors, ACL, diffs, content packages |
| `axiom-runtime` | Fixed-step world, native systems (FSM, gameplay, FPS, RTS, combat, animation, VFX), renderer, scene editor, live bridge, scenarios, checkpoints |
| `axiom-fps` / `axiom-rts` / `axiom-action` / `axiom-combat` | Genre schemas and pure simulation libraries, each registered through the generic runtime interface |
| `axiom-animation` | Skeleton import (glTF/GLB, PMX, Unity `.axrig`), clips, IK, retargeting |
| `axiom-cli` | The `axiom` executable |

More detail: [`docs/architecture.md`](docs/architecture.md) · [roadmap (zh-TW)](docs/ai-first-general-engine-roadmap.zh-TW.md) · [current plan](docs/plan-ai-authoring-gaps.md)

## Extending with native plugins

Most game changes are data edits. When a feature needs new Rust code, write a `Plugin` in your own crate and ship your own `axiom` executable. The CLI, editor, protocol and scenarios all still work, and no engine crate changes:

```rust
use axiom_runtime::world::{Plugin, RuntimeRegistry};

struct SpinPlugin;
impl Plugin for SpinPlugin {
    fn id(&self) -> &str { "example.spin" }
    fn register(&self, r: &mut RuntimeRegistry) {
        // capabilities, component/resource schemas, actions, systems and factories
    }
}

fn main() {
    axiom::main_with(axiom_runtime::installed_registry().with(SpinPlugin));
}
```

`try_with` refuses plugins that would silently replace an installed system or action. A complete example lives in [`crates/axiom-cli/examples/custom_plugin`](crates/axiom-cli/examples/custom_plugin), with its content in [`examples/spin-pack`](examples/spin-pack):

```powershell
cargo run -p axiom --example custom_plugin -- --db data\spin.sqlite --pack examples\spin-pack play --start
```

## Testing

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.\scripts\m1-smoke-test.ps1        # GPU screenshots, checkpoints, live bridge, ADV endings, FPS regression
```

`benchmarks/ai-tasks.json` compares three ways of doing the same five authoring tasks. Each run reads, previews, applies and verifies, and must pass the same field and scenario checks. In scripted replay, ACL moved 9.5 KB in total, compared with 18.8 KB for selective JSON and 136.6 KB for the legacy whole-world JSON path. These are bytes, not tokens, and the plans were scripted rather than chosen by an agent; see [`docs/benchmark-ai-tasks.md`](docs/benchmark-ai-tasks.md).

Smoke scripts write reports and screenshots to `artifacts/` and use isolated databases. `scripts/m2-stress-test.ps1` measures headless RTS simulation at 64/128/256 units using a release build. CPU measurements are kept separate from GPU screenshot checks, and the project makes no token, FPS or memory claims without a recorded measurement.

## Documentation

- [Detailed user guide (zh-TW)](docs/user-guide.zh-TW.md): controls for every demo, saves, the live bridge and protocol details
- [Scene editor](docs/scene-editor.zh-TW.md) · [Shaders](docs/shaders.zh-TW.md) · [Skeletal animation](docs/skeletal-animation.zh-TW.md) · [Effects Atlas](docs/effects-atlas.zh-TW.md)
- [Action combat](docs/m3-action.zh-TW.md) · [Pack composition](docs/m4-composition.zh-TW.md) · [Shared combat](docs/shared-combat.zh-TW.md) · [RTS](docs/m2-implementation.md)

## Contributing

Issues and pull requests are welcome. Please read [`AGENTS.md`](AGENTS.md) first. It sets the architectural rules (no Bevy, keep authoring and simulation separate, genre code stays in its own crate) and the validation every engine change must pass. It also applies to AI agents that contribute.

Do not commit third-party assets. Local databases and reference assets belong in the ignored `data/` directory.

## License

[MIT](LICENSE) © Axiom Engine contributors. The Axiom logo in `assets/branding/` is part of the project's identity. Please don't use it to imply endorsement.
