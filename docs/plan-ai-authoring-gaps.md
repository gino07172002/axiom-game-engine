# 計畫:AI-first 缺口補齊與編輯器強化

> 依 rigorous-plan 產出。每個 milestone 用 rigorous-change 執行;偏離計畫須在文末「修訂補記」留痕,不改寫原文。
> 來源:2026-11-08 對照原始設計文件的審查(設計缺口 + 編輯器缺口 + 編輯器 bug)。

## Phase 0 — 需求與成功樣貌

### 需求
讓 AI 用少量、精簡、可驗證的指令完成大部分開發;人類用編輯器審核、微調、指揮 AI。補齊:
- **協定**:精簡指令語言(ACL)、萬用字元/條件選擇器、精簡行式輸出、依選擇器批次修改與模板生成。
- **行為**:通用實體行為狀態機(資料定義、編譯成 SoA)、逐實體行為觀測(目前狀態、上一次轉移、各條件值)。
- **驗證**:scenario 實體欄位/行為狀態斷言、全域 seeded RNG(入 checkpoint)、跨程序確定性重播比對。
- **效能量測**:載入時間、RAM、VRAM 量測進 observe;依量測決定是否做二進位編譯快取。
- **擴充**:不改 `axiom-runtime/src/lib.rs` 就能註冊新系統。
- **Token**:可接真實模型 usage 的 benchmark adapter。
- **編輯器**:4+ 個已知 bug;schema 表單、多選、歷史時間軸、診斷面板、snapping、拖放改父層、網格 picking;AI 提案審核佇列、選取→精簡上下文、bytes 成本、CAS 衝突對照、scenario 面板、執行期行為疊層、bridge 控制(select/focus/screenshot);prefab 本體編輯、材質參數控制項、敘事/行為圖唯讀檢視。

### 最終驗收 demo 腳本
```text
# 1. 精簡指令,一次生成與批次修改(無 Rust 重編)
axiom --db %TMP%/demo.sqlite --pack content/empty acl -e "spawn prefab=enemy.basic count=10 id=enemy/{i} at=grid(0,0,2)"
axiom --db ... acl -e "set entities[tag=enemy].core.health/value 150"
axiom --db ... acl -e "select entities[tag=enemy] where core.health/value<50 fields=id,core.health/value limit 5"
  → 行式輸出:   enemy/3 core.health/value=20
                matched=2 rev=4
# 2. 行為與「為什麼不追」
axiom --db ... acl -e "inspect enemy/3 behavior"
  → state=idle last=- conds: dist_to(player)<15:false(22.4)
# 3. Headless 驗證
axiom --db ... scenario examples/behavior-chase.scenario.json  → PASS enemy/3.behavior.state==chase ...
# 4. 量測
axiom --db ... verify --measure → load_ms=… compile_ms=… rss_mb=… (vram 僅 play 時回報)
# 5. 編輯器:AI 提案 → 人審核
axiom --connect 127.0.0.1:4252 acl --propose -e "set entities[tag=enemy].core.health/value 80"
  → 編輯器 Proposals 面板出現帶前後值的差異;按 Approve → 新 revision;時間軸可點選並 undo
```

### 約束
- 不加 Bevy;core(交易式 authoring)與 runtime(固定步進)分離;FPS 原生玩法留在 `axiom-runtime/src/fps`。
- 既有 JSON 協定、examples、smoke 腳本必須持續可用(ACL 是前端,不是替換)。
- Windows/D 槽建置環境(`scripts/env.ps1`);每次引擎改動跑 fmt/test/clippy。
- 不宣稱 token/GPU/RAM 改善,除非有量測。bytes ≠ tokens。
- 使用隔離 QA 資料庫,不碰使用者 `data/*.sqlite`。

### Non-goals
- 編輯器內嵌 LLM client(指令框產生「任務」與上下文給外部 agent,不直接呼叫模型)。
- 把 FPS/RTS/Action 既有私有行為遷移到通用行為層(只加 inspect;遷移另案)。
- 完整視覺化節點編輯器(圖只做唯讀檢視)。
- navmesh、多人、跨平台 bit-exact。
- 重寫 ECS 或儲存格式。

### 未知與假設(保守解讀)
- 「全部改」= 上列全部;排序依價值與風險,每個 milestone 獨立可回退。
- 真實 token 量測需要使用者提供 API key;沒有 key 時只交付 adapter 並標 BLOCKED。

## Phase 1 — 勘查與風險

已勘查介面:`axiom-core/src/protocol.rs`(Request/CoreEdit,Extension 編輯經 registry handler)、`local.rs`(Target/Selection/select,ids/tags/components 篩選、bytes 預算、cursor)、`ir.rs:822 CompiledWorld`(未 derive serde)、`axiom-runtime/src/lib.rs:36-52`(註冊寫死)、`world.rs:229 RuntimeSystem`(含 combat/egui hook)、`scenario.rs:47 Assertion`、`axiom-fps/src/legacy.rs:60-113`(FPS 狀態機)、editor `ui.rs/viewport.rs/mod.rs/resource.rs`。egui 0.31、rapier 0.23。

| 風險 | 不成立的後果 | 處置 |
|---|---|---|
| R1 載入是否真的慢到值得做二進位快取 | 做了沒效益的格式 | **Spike S1**:量 fps/rts/effects-tour 的 JSON 解析 + compile 時間;< 20 ms 則不做快取,只交付量測 |
| R2 選擇器展開與 prefab 繼承欄位互動 | set 到繼承欄位時產生錯誤覆寫 | 展開用既有 `Target::edit`(已處理 materialize_parents);測試涵蓋 prefab 實例 |
| R3 ACL 語法歧義(值含空白、JSON 值) | AI 重試率上升 | 值一律 JSON 字面量或裸詞;語法表進 `discover acl`;parser 錯誤附欄位位置 |
| R4 Codex 同時改檔 | 合併衝突 | 等 Codex 停工後才動;先把其成果單獨 commit |
| R5 RuntimeSystem trait 拆分波及所有系統 | 大規模回歸 | 不拆 trait;改為 Plugin 註冊 + 外部注入,hook 保留預設實作 |
| R6 Rapier enhanced-determinism 影響效能/行為 | FPS 回歸 | 僅以 feature 開關加跨程序重播測試;預設不改 |
| R7 真實 token 需外部 key | 無法完成量測 | adapter 交付 + BLOCKED 申報 |

## Phase 2 — 決策(ADR)

**決策 1:ACL 實作位置**
- A:ACL 編譯成既有 `Request`/`CoreEdit`(前端)。B:新的獨立執行路徑。
- 決定 A。理由:交易、receipt、diff、CAS、undo 全部沿用;唯一新增是 parser + 選擇器。後果:ACL 能力上限受 Request 限制,需要時加 CoreEdit 變體。

**決策 2:選擇器修改如何入庫**
- A:新增 `CoreEdit::SetWhere{selector, field, value}`,apply 時對 pre-edit world 決定性展開。B:CLI 端先 select 再展開成 N 筆 set_field。
- 決定 A。理由:B 受 256 筆上限且有 TOCTOU;A 在同一交易內展開,diff 仍逐實體記錄。上限 4096 個匹配,超過回診斷。

**決策 3:where 條件語意**
- 作用在 compiled(含 prefab 繼承)值;運算子 `== != < <= > >=`、`and`;JSON pointer 欄位。不支援 or/函式(之後再加)。

**決策 4:精簡輸出**
- `Selection.format: "json"|"lines"`,預設 json(相容);ACL 預設 lines。行格式:`<id> <field>=<json-compact>...`,結尾 `matched=N rev=R [next=CURSOR]`。

**決策 5:通用行為層**
- A:新 resource `behavior.fsm`(狀態、轉移、條件:dist_to(tag) / health_below / timer / var;動作:move_toward(tag,speed) / stop / emit(event))+ runtime 系統 `axiom-runtime/src/behavior.rs`,SoA 狀態、上一次轉移、條件值快取供 inspect。B:擴充 FPS legacy 狀態機。
- 決定 A。理由:FPS legacy 綁 FPS 欄位;通用層讓 empty/ADV 類世界可用。FPS 另加 inspect,不遷移。

**決策 6:擴充機制**
- `pub trait Plugin { fn register(&self, r:&mut RuntimeRegistry) }`;`installed_registry()` 改為由內建 plugin 清單組成;`app::run` 接受外部 registry。示範:`examples/` 中的外部 crate 註冊系統。

**決策 7:Seeded RNG**
- `core.rng` resource `{seed}` → `RuntimeState.rng`(SplitMix64,無新依賴),checkpoint 序列化其狀態。

**決策 8:AI 提案審核**
- A:SQLite 新表 `proposals(id, created_at, source, request, preview, status)`;新 Request `Propose`/`ListProposals`/`ResolveProposal`。B:只存在編輯器記憶體。
- 決定 A。理由:跨程序、可審計、AI 可查狀態;approve 走正常 Apply(帶 expected_rev)。

**決策 9:二進位快取** —— 依 Spike S1 結果決定(可能不做)。

## Phase 3 — Milestones

| M | 名稱 | 驗收(操作 → 看到) | 依賴 |
|---|---|---|---|
| M0 | 收編 Codex 成果 | Codex 停工 → 單獨 commit 其修改;`cargo test --workspace` 基線結果記錄 | — |
| M1 | 編輯器 bug 修正 | resource 載入失敗時 Apply 停用;拖曳中外部寫入會保留可提交狀態或明確丟棄;Ctrl+R 不切工具;同幀多寫入不再衝突;scene ID 重複顯示持續訊息;篩選樹保留縮排;F 依包圍盒對焦 — 各有單元測試或程式邏輯測試 | M0 |
| M2 | 選擇器 + where + lines 輸出(walking skeleton) | `select` JSON 加 `id_glob`、`where`、`format:lines`;`CoreEdit::SetWhere`;測試 100 實體一次改 HP、where 過濾、prefab 繼承 | M0 |
| M3 | ACL 前端 | `axiom acl -e/-f/stdin`、loopback `{"op":"acl"}`;支援 select/set/spawn/delete/undo/inspect/history;`discover acl` 語法表;錯誤含位置 | M2 |
| M4 | 通用行為層 + inspect | `behavior.fsm` 資源;demo 世界敵人在玩家進入 15 m 後 chase;`inspect` 回傳 state/last/conds;FPS inspect 單一敵人 | M3 |
| M5 | 驗證擴充 | Assertion `Field{entity,pointer,op,value}`、`Behavior{entity,state}`;seeded RNG 進 checkpoint;同一 scenario 兩程序結果 hash 相同 | M4 |
| M6 | 量測 + (條件式)快取 | `verify --measure`/observe 回報 load/compile ms、RSS;play 回報 VRAM 估計(配置量,標明非驅動實測);依 S1 決定快取 | M0 |
| M7 | Plugin 註冊 | 外部 example crate 註冊系統,不改 lib.rs;既有測試全綠 | M0 |
| M8 | 編輯器:schema 表單 + 歷史/診斷面板 | 元件欄位依 RecordSchema 產生數值/布林/文字/引用控制項;History 面板列修訂 reason,點選顯示前後值,一鍵 undo;Verify 診斷可點跳物件 | M1 |
| M9 | 編輯器:多選、剪貼、快捷鍵、拖放、snapping、picking、格線 | Ctrl/Shift 多選批次刪除/移動;Ctrl+C/V/D、Delete;樹拖放改父層;snap 0.25/15°;local/world 切換;非方塊網格 picking;常駐格線 | M8 |
| M10 | 編輯器:AI 協作 | proposals 表與 Request;Proposals 面板(前後值差異、Approve/Reject);選取→ACL 上下文複製;每筆請求 bytes;CAS 衝突對照;bridge `editor.select/focus/screenshot` | M3, M8 |
| M11 | 編輯器:scenario 面板 + 執行期疊層 | 編輯器內跑 scenario 顯示 PASS/FAIL;Play-in-editor 實體上方顯示行為狀態 | M4, M5, M10 |
| M12 | 編輯器:prefab 本體、材質控制項、圖檢視 | 編輯 prefab 定義;`render.material` 滑桿/取色;敘事/行為圖唯讀列表式圖 | M8 |
| M13 | Token benchmark adapter | Anthropic API adapter(讀 `ANTHROPIC_API_KEY`);有 key 則量 legacy vs ACL;無 key 標 BLOCKED | M3 |

M2 是 walking skeleton:JSON 選擇器 → SetWhere 交易 → diff → lines 輸出,穿過 core 全部層。

## Phase 4 — Pre-mortem

1. **整合炸在 SetWhere × prefab 繼承 × diff**:對策 — M2 測試必含 prefab 實例與 diff 斷言。
2. **ACL 語法讓 AI 反而更常出錯**:對策 — 語法極小、`discover acl` 附範例、parser 錯誤附建議;舊 JSON 路徑不移除。
3. **編輯器改動無法自動驗證(GUI)**:對策 — 邏輯抽到可單元測試的函式;每個編輯器 milestone 跑 `editor --smoke-frames --screenshot` 並目視截圖。
4. **範圍爆炸(圖編輯器、LLM 內嵌)**:對策 — 已寫進 non-goals。
5. **Codex 再度並行修改**:對策 — 每個 milestone 開始前 `git status` 檢查非本計畫的修改,有就停下回報。
6. **M4 行為層最樂觀**:對策 — 條件/動作先只做 4+3 種,其餘列後續。

## Phase 5 — 執行協定

1. 每個 milestone:rigorous-change;驗收跑 `cargo fmt --all`、`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`,加 milestone 專屬演示,證據寫入下方紀錄。
2. 每個 milestone 結束:commit 一次(可回退點)、回顧計畫、寫 `WORKLOG.md`。
3. 偏離計畫:先在「修訂補記」記錄再繼續。

## 執行紀錄

(每個 milestone 完成後追加:日期、驗收證據、偏離)

## 修訂補記

(原計畫 → 變成什麼 → 為什麼)

- 2026-11-08:M13「Token benchmark adapter(Anthropic API)」→ **刪除**。理由:使用者確認引擎的預設使用方式是 AI agent(Claude Code、Codex 等)直接操作 CLI/loopback,不是引擎呼叫外部模型 API。成本指標維持既有的 request/response bytes 與工具次數;不新增 API key 依賴。Non-goals 追加「引擎內呼叫外部模型 API」。
