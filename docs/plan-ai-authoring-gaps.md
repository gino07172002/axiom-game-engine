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

- **M0(2026-11-08)**:Codex 展示台修改單獨提交 `92c29a1`;基線 `cargo test --workspace` exit 0。另加入使用者提供的 Logo(視窗圖示、編輯器工具列、README/文件)`0981847`,編輯器截圖目視確認。
- **M1(2026-11-08)**:修正 —
  1. 資源投影載入失敗時 `loaded=false`,Apply/Preview/Copy 與文字框停用,`request()` 拒絕(測試 `failed_root_load_cannot_replace_the_whole_resource`)。
  2. 拖曳中外部寫入:取消拖曳、以新修訂重建草稿、顯示原因(測試 `external_write_during_drag_cancels_…`)。
  3. Q/W/E/R/F 僅在無修飾鍵時生效;新增 Ctrl+Y 重做(測試 `modified_letter_keys_…`)。
  4. 同一幀多個 Apply 合併成單一原子批次 `editor::coalesce`(測試 `same_frame_applies_merge_into_one_batch`)。
  5. Scene ID 重複:持續顯示訊息並停用 Create。
  6. F 對焦改用網格部件包圍盒(測試 `focus_uses_mesh_part_bounds_not_the_pivot`)。
  7. `scripts/editor-smoke-test.ps1`:PowerShell 5.1 `ExitCode` 為 `$null` 的誤判(先取 `$process.Handle`)。
  證據:clippy exit 0;workspace 184 passed / 0 failed;editor smoke `ok:true`(`artifacts/editor-20261108-134912-117`),截圖目視確認。
  未處理:篩選時樹攤平(屬設計,移到 M9 一併處理);`Copy scene query` 的 `"scene":null` 經查 `Option` 可接受 null,非 bug。
- **M2(2026-11-08)**:`Selection` 新增 `id_glob`、`where`(eq/ne/lt/le/gt/ge,AND,作用於含 prefab 繼承的 compiled 值)、`format:"lines"`;抽出 `local::matches()`;新增 `CoreEdit::SetWhere`(交易內決定性展開、預設上限 4096、零匹配/超量回結構化診斷);CLI 對 lines 結果直接印文字;`discover local` schema 已更新。證據:`tests/selectors.rs` 5 項;clippy exit 0;workspace 189 passed;FPS 內容實測 `set_where enemy*` → 8 筆變更、1 個 revision、回應 1136 bytes。
  偏離:`Request::Select.query` 與 `SetWhere.select` 改為 `Box<Selection>`(clippy large_enum_variant;JSON 線格式不變)。
- **M3(2026-11-08)**:`axiom-core/src/acl.rs` + `Request::Acl` + `axiom acl [-e|FILE|stdin] [--preview] [--expected-rev]` + `discover acl`。敘述:select/inspect/history/diff/undo(單獨)與 set/spawn/delete(可多行、同一 revision、以編譯時 rev 做 CAS)。`;` 只在引號/括號外分隔。lines/inspect/diff 顯示時把浮點數四捨五入到 6 位(`local::display_value`,不影響儲存)。
  證據:`tests/acl.rs` 4 項 + 單元 3 項 + `display_value` 1 項;clippy exit 0;workspace 197 passed。FPS 實測:`select … limit 4` 回應 264 B(舊 `query entities --limit 9` 3617 B);`set enemy* … where …>=100` → 8 筆、回應 538 B;經 `--connect` 對執行中的編輯器 `acl set` → `ok rev=2`,編輯器 1.58 s 內 authoring_rev=2。
  已知:PowerShell 5.1 傳給原生程式時會吃掉內嵌雙引號,需寫 `\"`;bash 不受影響。
- **M4(2026-11-08)**:`axiom-runtime/src/fsm.rs`(能力 `fsm`、資源 `fsm.machines`、元件 `fsm.agent`、系統 `fsm.step` phase 8、動作 `fsm.inspect`/`fsm.summary`、observe metrics `fsm`、熱重載保留狀態;checkpoint 明確拒絕)。FPS 新增 `fps.inspect`(哨兵狀態、距離、冷卻、各轉移判定與 note)。CLI `axiom call CAP.ACTION [JSON] [--ticks N]`(有 `--connect` 走即時、否則無頭模擬)。新內容包 `content/behavior-lab`。
  證據:`tests/fsm.rs` 6 項(含確定性、熱重載、驗證、checkpoint 拒絕、fps.inspect);clippy exit 0;workspace 203 passed;CLI 實測 tick 0 `near(player)<=15 measured=30.02 pass=false`,tick 1200 `chase`、`last idle→chase @904`;GPU 截圖目視確認(tick 1400:attack 2 / chase 3 / idle 1)。
  偏離:ACL 是 authoring(Store)層,無法呼叫 runtime;runtime 診斷改由 `axiom call` 提供,ACL `inspect` 仍為 authoring 記錄。
- **M5(2026-11-08)**:`RuntimeState.rng`(SplitMix64,`core.rng {seed}` 資源,預設固定 seed,熱重載在 seed 未變時延續,checkpoint 新增可選 `rng` 欄位、舊存檔相容);FSM `chance{per_tick}` 條件消耗該串流(inspect 不消耗);scenario 新斷言 `field`(即時 position/health/enabled/components)與 `call`(僅 `*.inspect`/`*.summary`);scenario 結果新增 `state_hash`(tick、rng、transforms、health、非計時 metrics)。範例 `examples/behavior-chase.scenario.json`。
  證據:`tests/determinism.rs` 4 項;clippy exit 0;workspace 207 passed;兩個獨立程序跑同一 scenario,hash 皆為 `3d81c13dc3f25543`,5/5 斷言通過;`m1-smoke-test.ps1` PASS(含 checkpoint 存讀)。
  未做:Rapier `enhanced-determinism`(R6:僅同機器同程式,跨平台 bit-exact 列為 non-goal);FPS 的 Rapier 狀態仍不支援 checkpoint(既有行為)。
- **M6(2026-11-08)**:`axiom-runtime/src/measure.rs`(`load_profile`:read+decode/compile/runtime build 的 min/median/max 與程序工作集;`gpu_memory`:wgpu allocator report);`axiom verify --measure N`;play/editor smoke 輸出新增 `gpu_memory`、`process_memory`。
  **Spike S1 結論(release,中位數,9 次)**:fps 105 KB → read+decode 1.60 ms / compile 5.78 ms / runtime build 7.77 ms;rts 1.22 / 2.98 / 13.49;effects-tour 2.58 / 19.58 / 92.83;behavior-lab 0.10 / 0.12 / 0.83。二進位格式只能取代 read+decode(≤ 2.6 ms,遠低於 20 ms 門檻)→ **不做二進位場景快取**(決策 9)。effects-tour 的成本在原生系統建置,屬另一個效能題目。
  VRAM:wgpu 24 在 Vulkan 後端不提供 allocator report,實測回 `available:false`,如實回報而非估算。工作集:無頭 14.5 MB,FPS 開視窗遊玩 378 MB(含驅動與視窗)。
  證據:`tests/measure.rs`;clippy exit 0;workspace 208 passed。
- **M7(2026-11-08)**:`world::Plugin` trait、`RuntimeRegistry::with/try_with`(拒絕悄悄取代既有 system factory 或 action schema);CLI 拆成 library(`axiom::main_with(registry)`),`main.rs` 只剩一行。外部範例 `crates/axiom-cli/examples/custom_plugin`(`spin` 能力、`spin.rate` 元件、`spin.step` 系統,只用公開 API)+ `examples/spin-pack`。
  證據:`axiom-cli/tests/plugin.rs` 4 項(系統生效 60 tick=45°、無外掛的 registry 拒絕內容包、外掛 schema 套用於 ACL 編輯、取代既有系統被拒);clippy exit 0;workspace 212 passed;範例執行檔 GPU smoke `systems=spin.step`、ACL 查詢正常;原版 `axiom` 對 spin-pack 回 `missing_capability`。
  偏離:未拆分 `RuntimeSystem` trait 的類型專屬 hook(R5:hook 皆有預設實作,外掛不需實作;拆分屬重構,另案)。
- **M8(2026-11-08)**:Inspector 元件改為 Form/JSON 雙模式。Form 依值的 JSON 形狀產生控制項(數值 DragValue 保留整數/浮點、bool、字串、≤4 數字陣列、巢狀物件可摺疊),提示取自 registry 描述與世界 RecordSchema;Apply 只寫出改變的葉節點(`set_field`),保留 prefab 繼承;未知元件 ID 立即警示。新增 History 面板(revision 列表、點選看前→後差異、Restore 追加還原 revision)與 Problems 面板(結構化診斷 code/path/message、點路徑選取並對焦實體、Verify 按鈕)。
  證據:editor 單元測試 14 項(新增表單只寫葉節點、診斷路徑解析、History/Problems 以真實 Store 資料渲染);clippy exit 0;workspace 217 passed;editor smoke `ok:true`;History 面板 GPU 截圖目視確認(暫時性截圖修改已還原)。
- **M9(2026-11-08)**:多選(Ctrl 切換、Shift 加入;樹與視口皆可;次要選取以藍色外框顯示)、Delete 批次刪除、Ctrl+D 批次複製(ID 不衝突)、Ctrl+C/Ctrl+V(egui-winit 送 Copy/Paste 事件;複製 ID 清單到系統剪貼簿,貼上可接受外部 ID 清單)、Esc 清除選取、場景樹拖放改父層(含「拖到這裡成為根物件」區)、篩選結果顯示祖先麵包屑、Snap(0.25 m / 15° / 0.1,拖曳時按 Ctrl 反轉)與 Grid 常駐開關。
  證據:editor 單元測試 16 項(新增多選/批次/剪貼簿/Esc、snap 量化);clippy exit 0;workspace 219 passed;editor smoke `ok:true`;工具列與樹截圖目視確認(`⤒` 字元缺字已改純文字)。
  未做(M9):local/world 座標切換(Move 仍為世界軸、Rotate/Scale 為本地軸);匯入骨架網格(glTF)的 picking——目前只命中 `render.mesh` 部件與無網格實體的樞紐標記;拖放改父層未做自動化 GUI 測試(egui dnd 需真實指標流程),僅手動路徑經由 `Reparent` 既有測試覆蓋。

## 修訂補記

(原計畫 → 變成什麼 → 為什麼)

- 2026-11-08:M13「Token benchmark adapter(Anthropic API)」→ **刪除**。理由:使用者確認引擎的預設使用方式是 AI agent(Claude Code、Codex 等)直接操作 CLI/loopback,不是引擎呼叫外部模型 API。成本指標維持既有的 request/response bytes 與工具次數;不新增 API key 依賴。Non-goals 追加「引擎內呼叫外部模型 API」。
- **M10a(2026-11-08)**:AI 提案審核。SQLite `proposals` 表;`Request::Propose`(提交時即 preview 驗證、不寫 revision、佇列上限 256)、`Proposals`、`ResolveProposal{approve, rebase}`(核准在同一交易內以 base_rev 做 CAS,`rebase` 改用目前 revision 重新驗證;拒絕永不寫入);ACL `--propose`;CLI `axiom proposals`、`axiom resolve ID --approve|--reject [--rebase]`。編輯器 Proposals 分頁(數量徽章、清單標示過期 base、前→後差異可點選實體、Approve / Rebase & approve / Reject),每 0.25 s 以 `proposal_signature()` 輕量輪詢;observe 新增 `pending_proposals`、`selection`。
  證據:`tests/proposals.rs` 3 項 + 編輯器面板測試;clippy exit 0;workspace 223 passed。即時實測:對執行中的編輯器,直接寫 DB 與經 bridge 各提交一筆 → 編輯器 `pending_proposals=2`;CLI 核准 #1 → rev 2、編輯器同步、剩 1 筆;#2 因 base 過期回 `revision_conflict` 並保持 pending。
- **M10b(2026-11-08)**:`Request::EditorView{select, focus, dock, capture}`(經 bridge 走指令通道;有未儲存草稿時拒絕;不存在的 ID/分頁名回結構化錯誤;截圖只寫到 `artifacts/captures/<name>.png`,名稱限 `[A-Za-z0-9-_.]`)與 CLI `axiom --connect … view`;編輯器 AI 分頁「Copy selection context」輸出與 ACL inspect 同格式的精簡上下文(含繼承後的有效元件值),並顯示上一筆請求/回應位元組(標明 bytes ≠ tokens);`discover editor` 新增 view/review 說明。CAS 衝突對照由 Proposals 面板的 stale base 提示 + Rebase & approve 提供。
  證據:remote_view/context_text 單元測試;clippy exit 0;workspace 224 passed;editor smoke `ok:true`。即時實測:`view --select stage/crate --select stage/beacon --focus --dock proposals --capture review` → 選取 2 個、PNG 寫出並目視確認(多選藍框、Proposals(1) 面板含 Approve/Rebase/Reject 與差異);`--capture ..\evil` 與 `--select ghost` 被拒。
  除錯紀錄:啟動前已存在的提案原本要等第一次 0.25 s 輪詢才載入 → 改為 signature 未知時立即載入。另確認 `observe` 由 bridge 執行緒快照回應,成功不代表已繪製第一幀(測試腳本需以檔案穩定判斷截圖完成)。
- **M11(2026-11-08)**:Play-in-editor 行為疊層(`FsmSystem::labels` + `RenderView::project` 支援相機 viewport 與正交/透視;只顯示狀態名以免擁擠;可勾選關閉)。編輯器 Scenarios 分頁(列出 `examples/*.scenario.json`,Run 以目前 revision 無頭執行,顯示 PASS/FAIL、每條斷言實際值、ticks 與 state_hash)。`editor_view` 新增 `play: true|false`(AI 可遠端開始/停止預覽),CLI `view --play`。
  證據:`tests/fsm.rs` 新增疊層投影測試(6 個 agent 皆在畫面內、相機後方不投影);Scenarios 分頁以真實 scenario 成功/失敗結果渲染測試;clippy exit 0;workspace 226 passed。即時實測:對執行中的編輯器 `view --play true` → tick 934 時 `{"chase":1,"idle":5}`,截圖目視確認標籤位於對應方塊上方且與狀態一致。
- **M12(2026-11-08)**:Assets 分頁新增 prefab「edit」(Prefab 檢視器以 M8 表單編輯 prefab 預設值,只寫改變的葉節點;顯示繼承實例數)與 MATERIALS 清單(Material 檢視器:依 shader 宣告的 min/max 產生參數滑桿、tint 取色器、alpha_cutoff;寫入 `render.shaders` 的對應子樹)。表單對名稱含 color/tint/accent、值為 3–4 個 0..1 數字的欄位改用取色器(涵蓋 `render.mesh` 部件顏色)。新增 Graphs 分頁(唯讀):fsm 狀態機(初始狀態、動作、轉移條件)、narrative.graph(節點、說話者、選項→下一節點、結局)、behavior.program 規則列表。
  證據:編輯器單元測試 21 項(新增材質/prefab 草稿經 Store 寫入驗證、Graphs 以 behavior-lab 與 adv 內容渲染、取色器判定);clippy exit 0;workspace 228 passed;editor smoke `ok:true`;Graphs 分頁 GPU 截圖目視確認。
  未做:可視化節點圖編輯(non-goal);材質檢視器僅單元測試與編譯驗證,未截圖(需手動開啟)。

## 第二輪(2026-11-08 晚)

- **M14 `.gitattributes` + CI**:LF 正規化(.cmd/.bat/.ps1 保持 CRLF、二進位標註);GitHub Actions(windows-latest + MSYS2 MinGW gcc,fmt/clippy/test)。第一次因工具鏈安裝步驟失敗,改為明確安裝 `stable-x86_64-pc-windows-gnu`(結果見執行紀錄)。
- **M15 資料驅動戰鬥**:FSM 新增 `attack{tag,range,damage,interval_ticks}` 動作與狀態 `emit`(進入狀態時發送 `behavior.program` 事件,若事件宣告文字欄位 `agent` 則帶入實體 ID;建立時驗證事件存在且需 `behavior.execute`);目標搜尋略過 health ≤ 0 的實體;summary 新增 hits/emitted/fault,inspect 新增 cooldown。新內容包 `content/skirmish`(1 英雄 vs 6 敵,擊殺計數規則 → `victory`,英雄倒下 → `defeat`)與 `examples/skirmish-victory.scenario.json`。
  證據:`tests/fsm.rs` 新增 3 項(勝利結局經事件與規則達成、只改 grunt 傷害數值即導致 `defeat`、未宣告事件與非法 interval 在執行前被拒);clippy exit 0;workspace 231 passed;CLI scenario 3/3 通過;GPU 截圖目視確認。
  未做:死亡視覺(屍體仍以原外觀留在場上)、投射物/遠程攻擊、群體 AI 協調。
