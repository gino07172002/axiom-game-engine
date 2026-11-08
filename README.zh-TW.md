<p align="center">
  <img src="assets/branding/axiom-logo-white.png" alt="Axiom" width="320">
</p>

<p align="center">
  <b>以 Rust 撰寫、為 AI 而生的遊戲引擎。</b><br>
  AI agent 不需要編輯器，也不需要改引擎原始碼，就能透過一套精簡、具型別、交易式的協定製作和修改遊戲。
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Rust" src="https://img.shields.io/badge/rust-stable-orange.svg">
  <img alt="Status" src="https://img.shields.io/badge/status-prototype%20v0.4-yellow.svg">
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/docs-English-lightgrey.svg"></a>
</p>

---

## Axiom 是什麼？

多數引擎是為了讓人在編輯器裡工作而設計的。Axiom 則以 AI coding agent 為主要開發者，人類負責審核與指揮。

- **遊戲是一個資料庫。** 世界是具型別的文件，由 entity、component、prefab、scene 和 resource 組成。每次修改都是一筆交易，會產生一個編號 revision。每個 revision 都能預覽、比對差異或還原。
- **Agent 只讀需要的部分。** 可以用選擇器(`enemy/*`、`where core.health/value<50`)只取需要的欄位，回傳每個物件一行的精簡文字。不必為了改一個數值而讀取整個場景。
- **修改先驗證才保留。** Schema 驗證、CAS revision、冪等重試、無頭 scenario 斷言和確定性狀態雜湊，讓 agent 不看畫面也能確認修改有效。
- **編輯與執行分離。** 編輯層(`axiom-core`)是存在 SQLite 的交易式資料。執行層(`axiom-runtime`)以 120 Hz 固定步進執行編譯後的世界，由原生 Rust 系統驅動。遊戲執行中會在安全點熱載入新的 revision。
- **不使用 Bevy。** Axiom 在 `wgpu`、`winit`、`glam`、`egui`、`rapier3d` 上實作自己的精簡 ECS 式世界。

> **狀態:** 實驗性原型(v0.4),目前只在 Windows 11(x86_64、GNU 工具鏈、Vulkan)實測過。API 與檔案格式仍可能變動。

## 一分鐘導覽

以下指令都使用同一個資料庫：`axiom --db lab.sqlite …`。第一個指令會從內容包建立它。

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

$ axiom call fsm.inspect '{"id":"enemy/0"}'            # 「這個敵人為什麼不追?」
{"ok":true,"result":{"state":"idle","transitions":[…,
  {"to":"chase","when":"near(player)<=15","measured":30.016663,"pass":false}]},"rev":2,"tick":0}

$ axiom scenario examples/behavior-chase.scenario.json  # 無頭執行；不同程序的 state_hash 相同
{"ok":true,"state_hash":"…","assertions":[ 5 passed ], …}

$ axiom acl -e "undo 1"
ok rev=3 restored_from=1
```

(上面的修改會在 prefab 實例上加入局部覆寫，prefab 本身的預設值不變。)

## 功能

| 領域 | 內容 |
|---|---|
| **編輯協定** | 具型別的 JSON 請求(`select`、`preview`、`apply`、`undo`、`diff`、`history`、`discover`),以及 **ACL**:一種逐行的精簡指令語言，會編譯成同一套請求。所有請求都使用 CAS revision 和 request-ID 重試。 |
| **查詢** | ID 萬用字元、tag 與 component 篩選、能看到 prefab 繼承值的 `where` 條件、欄位投影、cursor 與位元組預算。輸出可選 JSON 或精簡行格式。 |
| **批次修改** | `set_where` 以一個 revision 修改所有符合的物件。`spawn` 可以把 prefab 實例化 N 次。所有修改都會依 component／resource schema 驗證。 |
| **執行層** | 120 Hz 固定步進、帶 generation 檢查的 entity handle、具型別的 component 欄、跨 revision 保留狀態的熱重載。 |
| **行為** | 任何 entity 都可使用資料定義的有限狀態機(`fsm.machines`)、共用的玩法／敘事／UI IR,以及原生 FPS、RTS、動作戰鬥模組。 |
| **驗證** | 含輸入、指令、斷言時間軸的 scenario;`field` 和 `call` 斷言;seeded RNG;跨程序比對用的狀態雜湊;checkpoint 與存檔欄位。 |
| **渲染** | `wgpu` forward renderer,支援編寫的燈光(`render.light`:可投射陰影的平行光、點光源、聚光燈、近似面光源)與環境光／霧(`render.environment`)、Lit／Toon／自訂 WGSL 材質、骨架動畫(glTF/GLB、PMX)、GPU compute 粒子、貼花、霧與後製效果。 |
| **編輯器** | 原生場景編輯器(場景樹、視口 gizmo、Inspector),與 AI agent 共用同一套 revision,並即時同步外部寫入。 |
| **即時連線** | Loopback JSONL(`--connect 127.0.0.1:4242`),可對執行中的遊戲觀察、下指令、修改和存檔。不需要 MCP。 |

## 開始使用

### 需求

- Rust(stable)。repo 的 `rust-toolchain.toml` 固定使用 `stable-x86_64-pc-windows-gnu`。
- 開視窗的模式需要 `wgpu` 支援的 GPU(Vulkan、DirectX 12 或 Metal)。無頭指令不需要 GPU。

### Windows 建置

```powershell
git clone https://github.com/gino07172002/axiom-game-engine.git
cd axiom-game-engine
.\scripts\setup.ps1          # 只需一次：在 .tools/ 內安裝工作區專用的 Rust + MinGW
. .\scripts\env.ps1          # 每開一個新 shell 都要執行：讓 cargo 使用工作區工具鏈
cargo build --workspace
```

### Linux / macOS(未實測)

把 `rust-toolchain.toml` 的 `channel` 改成 `stable`,再執行 `cargo build --workspace`。可能需要先安裝該平台 `winit`／`wgpu` 常用的系統套件。

### 執行

```powershell
target\debug\axiom.exe play                 # 內建 FPS 範例(WASD、滑鼠、左鍵射擊、Tab 開控制台)
target\debug\axiom.exe editor               # 場景編輯器
target\debug\axiom.exe --db data\lab.sqlite --pack content\behavior-lab play --start
```

Windows 上也可以直接雙擊根目錄的 `Play *.cmd` 與 `Edit Axiom.cmd`。

## 與 AI agent 協作

Agent 只需要 `axiom` 執行檔。典型流程:

1. **探索**:`axiom discover acl`(文法)、`axiom discover local`(JSON schema)、`axiom schema components`。加上 `--if-hash` 可以略過沒有變動的 schema。
2. **只讀需要的部分**:`axiom acl -e "select … where … fields … limit N"`。
3. **一筆交易完成修改**:先 `axiom acl --preview script.acl`,再 `axiom acl script.acl`。多行編輯腳本會合成一個 revision。
4. **不看畫面也能驗證**:`axiom scenario file.json`、`axiom call fsm.inspect '{"id":"…"}' --ticks 600`、`axiom verify --measure 5`。
5. **即時觀察**:`axiom --connect 127.0.0.1:4242 observe-delta --field /rev`。執行中的遊戲會自動載入新的 revision。
6. **還原**:`axiom undo REV` 會追加一個還原用的 revision,不會改寫歷史。

ACL 速查(完整文法請執行 `axiom discover acl`):

```text
select SEL [where F OP V [and …]] [fields F,F] [limit N]     # 讀取
inspect ID | history [N] | diff A B | undo REV                # 讀取
set SEL F VALUE [where …]                                      # 修改
spawn PREFAB count=N id=enemy/{i} at=x,y,z step=dx,dy,dz tag=a
delete SEL
SEL: enemy/7 | enemy/* | entities[tag=enemy,has=core.health] | prefabs | resources[id=x]
F:   core.health/value(元件欄位)| label | /raw/json/pointer
```

同樣的操作也能以 JSON 經由 `axiom serve`(stdio)或 loopback bridge 送出。協定規範見 [`docs/m15-implementation.md`](docs/m15-implementation.md)。

## 範例內容

所有 demo 都是純資料的內容包，由同一個執行檔載入。

| 內容包 | 展示內容 |
|---|---|
| `content/fps` | 第一人稱射擊：hitscan、哨兵、掩體、爆頭 |
| `content/rts` | 邊境之戰：對抗資料驅動的電腦對手，三條路線、防禦塔、三種單位、小地圖、血條與戰鬥特效 |
| `content/rts-bench` | 固定的 64 單位 RTS 回歸／壓測場景(框選、A* 導航、採集、生產;128／256 由 `export_rts_pack` 產生) |
| `content/action` | 第三人稱近戰：連段、閃避窗、體力、鎖定 |
| `content/adv` | 三種結局的分支敘事、中英在地化、存檔欄位 |
| `content/m4` | RTS、動作、ADV 三個內容包組合在同一世界，含跨包事件 |
| `content/behavior-lab` | 通用 FSM agent:idle → chase → attack,可逐一檢查 |
| `content/effects-tour` | 36 個材質／VFX 展台，包含 GPU 粒子、貼花、霧和熱扭曲 |
| `content/shader-lab` | Lit、Toon 與自訂 WGSL 材質，參數可熱更新 |
| `content/animation`、`content/love-no-war` | 蒙皮、混合、IK、retargeting,以及節奏雙人舞原型 |
| `content/empty`、`content/two-cameras` | 沒有玩法的最小世界、多相機 |

隨附的模型與圖片都是原創，由 `scripts/` 內的腳本(Blender、PowerShell)產生。Effects Atlas 是依 FlaxEngine 功能導覽範例的*主題*重新實作，不含任何 Flax 素材。

## 架構

| Crate | 職責 |
|---|---|
| `axiom-core` | World IR、型別／能力註冊、編譯器、交易、選擇器、ACL、差異、內容包 |
| `axiom-runtime` | 固定步進世界、原生系統(FSM、玩法、FPS、RTS、戰鬥、動畫、VFX)、renderer、場景編輯器、即時 bridge、scenario、checkpoint |
| `axiom-fps` / `axiom-rts` / `axiom-action` / `axiom-combat` | 各類型的 schema 與純模擬函式庫，都透過通用 runtime 介面註冊 |
| `axiom-animation` | 骨架匯入(glTF/GLB、PMX、Unity `.axrig`)、clip、IK、retargeting |
| `axiom-cli` | `axiom` 執行檔 |

延伸閱讀：[`docs/architecture.md`](docs/architecture.md) · [路線圖](docs/ai-first-general-engine-roadmap.zh-TW.md) · [目前計畫](docs/plan-ai-authoring-gaps.md)

## 用原生外掛擴充

大部分遊戲修改都只需要改資料。需要新的 Rust 程式時，在你自己的 crate 寫一個 `Plugin`,再發行自己的 `axiom` 執行檔。CLI、編輯器、協定和 scenario 都照常可用，引擎的 crate 一行都不用改:

```rust
use axiom_runtime::world::{Plugin, RuntimeRegistry};

struct SpinPlugin;
impl Plugin for SpinPlugin {
    fn id(&self) -> &str { "example.spin" }
    fn register(&self, r: &mut RuntimeRegistry) {
        // 能力、component/resource schema、action、系統與 factory
    }
}

fn main() {
    axiom::main_with(axiom_runtime::installed_registry().with(SpinPlugin));
}
```

外掛若會悄悄取代已安裝的系統或 action,`try_with` 會拒絕載入。完整範例在 [`crates/axiom-cli/examples/custom_plugin`](crates/axiom-cli/examples/custom_plugin),搭配的內容包在 [`examples/spin-pack`](examples/spin-pack):

```powershell
cargo run -p axiom --example custom_plugin -- --db data\spin.sqlite --pack examples\spin-pack play --start
```

## 測試

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.\scripts\m1-smoke-test.ps1        # GPU 截圖、checkpoint、即時 bridge、ADV 結局、FPS 回歸
```

`benchmarks/ai-tasks.json` 用三種方式完成同樣 5 個編輯任務。每次都走讀取、預覽、套用、驗證四步，而且要通過相同的欄位與 scenario 檢查。腳本化重播的結果:ACL 共傳輸 9.5 KB,選擇性 JSON 是 18.8 KB,舊的整包 JSON 是 136.6 KB。這些是位元組，不是 token,而且步驟是預先寫好的，不是 agent 自己決定的，詳見 [`docs/benchmark-ai-tasks.md`](docs/benchmark-ai-tasks.md)。

Smoke 腳本使用隔離的資料庫，報告和截圖輸出到 `artifacts/`。`scripts/m2-stress-test.ps1` 用 release 版量測 64／128／256 單位的無頭 RTS 模擬。CPU 量測和 GPU 截圖驗證分開記錄;沒有實際量測紀錄，就不宣稱 token、FPS 或記憶體效益。

## 文件

- [詳細使用指南](docs/user-guide.zh-TW.md):各 demo 的操作、存檔、即時 bridge 與協定細節
- [場景編輯器](docs/scene-editor.zh-TW.md) · [Shader](docs/shaders.zh-TW.md) · [骨架動畫](docs/skeletal-animation.zh-TW.md) · [Effects Atlas](docs/effects-atlas.zh-TW.md)
- [動作戰鬥](docs/m3-action.zh-TW.md) · [內容包組合](docs/m4-composition.zh-TW.md) · [共用戰鬥](docs/shared-combat.zh-TW.md) · [RTS](docs/m2-implementation.md)

## 參與貢獻

歡迎提出 issue 與 pull request。請先閱讀 [`AGENTS.md`](AGENTS.md),裡面有架構規則(不用 Bevy、編輯層與模擬層分離、各遊戲類型的程式碼留在自己的 crate)以及每次引擎修改都必須通過的驗證。這些規則同樣適用於參與開發的 AI agent。

請勿提交第三方素材。本機資料庫與參考素材請放在已被 ignore 的 `data/` 目錄。

## 授權

[MIT](LICENSE) © Axiom Engine contributors。`assets/branding/` 中的 Axiom Logo 屬於專案識別，請勿用來暗示官方背書。
