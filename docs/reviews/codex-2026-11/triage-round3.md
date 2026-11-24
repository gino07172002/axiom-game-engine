# 第三輪分流（triage round 3）

基準：worktree `D:\rust_learn\.claude\worktrees\r1`，branch `integ`，HEAD `b54903b`。這次只做唯讀靜態分析（讀 code、`git diff`、`git show <base>:path`），沒有跑 cargo、smoke 或 GPU。

分類規則（使用者決定）：
- **REGRESSION**：修正前可用或被接受，修正後因我們的 commit 壞掉、被拒或變差。判定時對照修正範圍前後的程式碼。
- **HIGH**：嚴重度為 high、但不是 regression 的項目。
- **BACKLOG**：問題真實存在，但屬於修得不完整（沒有比修前差）、測試缺口，或修前就有的 medium/low 問題。
- **NOT-REAL**：讀 code 後主張不成立。
- 不確定的項目一律列 BACKLOG，並在證據欄加註「待確認」。

各區的修正前基準：
- R1：`4b586d8`（第一輪之前），第二輪範圍 `6428548..dc763b8`
- R3：`dc763b8`
- R5：`b843561`
- R6：`9151b03`
- R7：`3c108cf`

嚴重度來源：A 段各列採用 R<n>.md 原 finding 的嚴重度（R1 的 6.1-* 取自 `R1.gpt-6.1-sol.md`，前輪 N* 取自 `R1.recheck.md`）。C 段各項採用標題上的嚴重度。

## 統計

| 區 | REGRESSION | HIGH | BACKLOG | NOT-REAL |
|---|---|---|---|---|
| R1 | 3 | 0 | 7 | 0 |
| R3 | 2 | 2 | 8 | 0 |
| R5 | 3 | 1 | 7 | 0 |
| R6 | 3 | 0 | 4 | 0 |
| R7 | 1 | 2 | 10 | 0 |
| 合計 | 12 | 5 | 36 | 0 |

R7 的 REGRESSION 是 F3 裡拆出的相容性子項「F3-compat」。F3 本身仍算 HIGH，所以 R7 表格有 13 列，對應 12 個 recheck 項目。

**共通根因：新 validator 讓舊資料庫開不起來。** R5-F2、R6-F8、R6-F12、R7-F3-compat 都屬於這一類。新 validator 掛在 `World::compile` 上，而 `compile` 會在以下三個地方執行：
- `Store::from_connection`（`crates/axiom-core/src/store.rs:108`，開啟時編譯最新修訂）
- `edit_in` 的 `previous.compile`（`store.rs:355-356`）
- `undo_in`（`store.rs:763`）

結果是修前合法存入的修訂現在開不起來，也無法用修復編輯或 undo 救回。建議由一個 agent 先建立共用機制，再讓各區套用：把「新寫入專用」的檢查從 `compile` 拆出，只對 `next` 且只在相關資料有變動時執行。

## R1 authoring core and protocol

| item | 分類 | 嚴重度 | 證據(一行) |
|---|---|---|---|
| 6.2-F3 | REGRESSION | high | `registry.rs:618` `normalize` 與 `typed_component`（:546，第二輪 5fcd714 新增）會先把實體上的陣列覆寫 `["directional"]` 轉成物件，再由 `ir.rs:174` `merge_json` 與 prefab 的 `{kind:point,range:10}` 逐欄合併，保留 `range`，`light.rs:125` 因此 bail，`store.rs:108` `Store::open` 失敗。修前（4b586d8）陣列不是物件，`merge_json`（ir.rs:407）會整筆替換，得到合法的 directional，DB 可以開啟 |
| 6.2-F10 | REGRESSION | medium | `local.rs:369` `compare_numbers` 改為 i128 精確比較（修前 4b586d8 local.rs:350 全部用 f64）；核准時 `store.rs:542` 以新比較器重播已存提案的 `set_where`。修前：`lt 9007199254740993` 的預覽與核准都只命中 …991，結果一致。修後：已審舊提案核准時多命中 …992。只影響超過 2^53 的整數，情境很窄 |
| 6.2-F6 / 6.1-F4 | REGRESSION | high | `protocol.rs:374` `apply_all` 讓整批共用 16 MiB 預算，`ops_merge.rs:505` `apply_edits` 走這條路，所以 `ProposalDiff`（store.rs:625）、`conflicts` 與核准都會回 `budget_exceeded`。修前（4b586d8）完全沒有展開預算，例如 36×481 KB 的已存提案可以 diff 也可以核准；修後連審查資料都讀不到 |
| N1（前輪） | BACKLOG | medium | `local.rs:375-379` 只要有一側是浮點數就走 f64，`9007199254740992.0 eq 9007199254740993` 仍會命中。修前全部用 f64，行為相同；屬修得不完整 |
| N2（前輪） | BACKLOG | medium | `store.rs:625` 已可翻頁，但 `diff.rs:18` 對超過 256 bytes 的值只回長度與指紋。修前根本無法翻到第 65 筆，修後沒有更差 |
| 6.1-F10 | BACKLOG | medium | `axiom-cli/src/lib.rs:572/615` 的 `rpc` 與 `serve` 仍先開 Store 再解析請求，與修前相同；只修好了直接執行 `schema`/`discover` 的路徑 |
| 6.2-F14 / 6.1-F9 | BACKLOG | medium | `discovery.rs:51` animation operand schema 沒有表達 `field:"param"` 與 `param` 的互斥。修前沒有手寫 animation operand schema，沒有變差；屬 schema 缺口 |
| N1 | BACKLOG | medium | 跨世界 `import`（`ReplaceWorld`）後，舊提案仍可依 base_rev 套用到同名物件（store.rs:547/562/795）。修前 import 一樣會無條件取代世界，提案也一樣待審，屬既有問題 |
| N2 | BACKLOG | medium | 待審上限 256（store.rs:453），但列表 `LIMIT ?2` 最多 64 筆（store.rs:476）。4b586d8 store.rs:379/405 是相同的上限與查詢，屬既有問題 |
| N3 | BACKLOG | low | `observe.rs:79-101` 只擋 `/gameplay/trace` 前綴，允許投影整個 `/gameplay`。4b586d8 observe.rs 的同一個 `ensure!` 與任意欄位投影都已存在，屬既有問題 |

## R3 rendering

| item | 分類 | 嚴重度 | 證據(一行) |
|---|---|---|---|
| F7 | REGRESSION | medium | `app.rs:1817-1831` 與 `screenshot.rs:291-311`：修前 `pending_capture` 與 loopback `shots` 各自獨立，下一幀同步擷取，從不拒絕。修後 editor_view capture 佔用 `shots`，造成三個退化：(a) loopback `screenshot` 回「one at a time」被拒；(b) 第 6 個 capture 在 `editor_action` 已套用後才因 `enqueue` 失敗，回錯但編輯留下；(c) 排隊中的擷取要等前一張編碼完才取幀，可能拍到之後的畫面 |
| F1 | REGRESSION | high | `hdr.rs:180-198` 加了 `clamp(1.0,…)`，經 `render.rs:1297` `pixel_rect` 用到 `render.rs:705` 的投影。修前 dc763b8 render.rs:958 使用 `w*width`，次像素寬度合法、不會 panic，且與 `world.rs:1588` 指標射線一致。修後 640px 畫面、`w=0.001` 的視圖寬度從 0.64 變成 1.0，指標射線仍用 0.64。只影響 1px 以下的視圖 |
| F2 | HIGH | high | `shader-smoke-test.ps1:108` `failed_gpu_activation`、`effects-smoke-test.ps1:109/117` `gpu_stage_failure`/`repair` 的名稱與實際驗證內容（提交前拒絕）不符；「GPU pipeline 建置失敗時保留舊修訂」的路徑（app.rs:2139 `prepare_shaders`）已沒有任何測試。行為本身沒有退化 |
| F4 | HIGH | high | 修正有效（`vfx.rs:502/548` 讀預建表），但 `vfx.rs:668` 測試兩邊都呼叫 `evaluate_masked`，且只取 5 個索引，熱路徑改回逐顆重掃也測不出來。屬測試缺口 |
| F8 | BACKLOG | medium | `screenshot.rs:120-133`：`hard_link` 失敗且 `!exists` 時退回 `rename`，留有 TOCTOU 空窗。修前 `File::create` 一律覆寫，修後沒有更差 |
| F11 | BACKLOG | medium | `postfx.wgsl:13-16` 內縮半 texel，分數像素邊界仍會混色。修前色差完全沒有 clamp，修後沒有更差 |
| F10 | BACKLOG | medium | 天空 z=1.0 並用 `LessEqual`（render.rs:1472），仍會蓋掉深度恰為 1.0 的自訂材質。修前 0.9999 同樣會蓋，範圍還更大 |
| F13 | BACKLOG | medium | `vfx.rs:497-505` 的 `vfx.inspect` 對 ribbon 仍用粒子生命週期求值，輸出與修前相同 |
| F12 | BACKLOG | medium | `render.rs:612-624` 已回 `render_post_not_ready`、不寫 PNG；只缺可注入建置失敗的回歸測試 |
| F14 | BACKLOG | low | `render.rs:678-680` 在 acquire 前就更新 `local_lights`；這個指標是 R3 新增的，談不上退化 |
| N1 | BACKLOG | medium | `fxaa.wgsl:29/78`、`hdr.wgsl:28` 都不在 R3 diff 內，屬既有問題 |
| N2 | BACKLOG | medium | `vfx.rs:569-575` `e.clone()`；dc763b8 vfx.rs:519-521 是相同程式碼，屬既有問題 |

## R5 physics, combat, navigation, gameplay

| item | 分類 | 嚴重度 | 證據(一行) |
|---|---|---|---|
| F2 | REGRESSION | high | `fsm.rs:531` `validate_world` 在 `fsm.rs:1066` 註冊為 world validator，`fsm.agent` 的 radius 0.05..5 檢查也是新增的；b843561 只在 `FsmSystem::new` 檢查這些。修前：含未知 `machine`、未宣告 `emit` 或 radius 越界的 DB 可以開啟、也能修復。修後：`store.rs:108` `Store::open` 失敗，`store.rs:356` 的 `previous.compile` 讓修復提交被拒。`editor/graph_tests.rs` `with_runtime` 被迫改寫是直接佐證 |
| F1 | HIGH | high | `combat.rs:80` `inherit` 會帶走 `outbox`，`events.rs:138` `transfer` 只檢查 `pending`，`events.rs:65` 遇到已移除的 export 直接 return，已扣血的命中因此失去事件。修前沒有 outbox（超量時 bus fault 凍結），所以不是 regression |
| F14 | REGRESSION | medium | `tween.rs:741-749` 新增 `queued` 時拒絕 play；`world.rs:716-723` `dispatch_bindings` 失敗只記 `binding_fault`、不重試。修前：積壓期間由綁定觸發的 play 照常播放，on_finish 也照常送出。修後：clip 不播、事件遺失。只在同一步超過 64 個 on_finish 積壓時觸發；`seek` 繞過屬既有問題 |
| F15 | REGRESSION | low | `fsm.rs:340` `schema_kinds` 只列欄位名稱；b843561 `fsm.rs:958` 原本寫 `stop_within?`，修後少了選填標記。serde 仍是 default（fsm.rs:23） |
| F9 | BACKLOG | medium | `nav_agent.rs:1169-1206` 只限制 A* 展開次數；修前整次搜尋都在同一幀，修後沒有更差。「待確認」：目標每幀移動時，長路徑預覽永遠不會完成 |
| F7 | BACKLOG | medium | `nav_agent.rs:707-727` 回溯與轉角掃描仍未計入預算；修前計得更少 |
| F11 | BACKLOG | medium | `camera_rig.rs:763` `None => r.socket = None`；修前目標被刪時走同一分支，同樣會清掉 |
| N1 | BACKLOG | medium | `physics.rs` checkpoint 不保存 `events_halted`；b843561 已是如此 |
| N2 | BACKLOG | medium | `trigger.rs:633-635` 先 `*fired += 1` 再 `emit`；修前相同 |
| N3 | BACKLOG | medium | `nav_agent.rs:781-787` 以實體原始位置判斷是否重排；b843561 :760-766 相同 |
| N4 | BACKLOG | medium | `fsm.rs:210-212` 的 `clip` 只做 `valid_id`；修前相同 |

## R6 audio and input

| item | 分類 | 嚴重度 | 證據(一行) |
|---|---|---|---|
| F8 | REGRESSION | medium | `registry.rs:327` 的 `input.bindings` validator 新增 `b.validate_keys()`（ir.rs:828-836 拒絕同名 button+axis），compile 時在 store.rs:108、355-356、763 執行。修前 9151b03 只檢查長度，同名資料可以寫入，也能開啟。修後 `Store::open` 失敗，`input remove` 修復與 `undo` 也被拒 |
| F12 | REGRESSION | low | 同一條路徑：`ir.rs:850-852` 對每個鍵名呼叫 `check_key_name`，ops_tuning.rs:307 用 197 項的 `KEY_NAMES` 白名單。修前 `PadA`、`W`、`MouseRight` 等鍵名可以寫入；修後含這些鍵名的舊 revision 在 open、edit、undo 時都會失敗。另一半問題（`Escape` 仍被接受，但 Game tab 會攔截）屬修得不完整，歸 BACKLOG |
| F6 | REGRESSION | medium | `spatial_audio.rs:111-160` 新增 `FILE_TEXTS`，以 (path,size,mtime,gain) 為鍵，命中時跳過讀檔與 SHA-256。修前 9151b03 `cached_clip` 每次都呼叫 `file_key`，以內容雜湊判斷。修後同大小、mtime 保留的替換檔會取回舊 PCM。cache miss 時仍在主執行緒同步處理，也還沒修 |
| F4 | BACKLOG | medium | `audio.rs:264` 先送 Shot，`audio.rs:281` 才送 Clips；修前沒有 Shot 機制，短音效整個漏掉，修後沒有更差 |
| N1 | BACKLOG | medium | `editor/mod.rs:2521` 使用當下的 `expected_rev`；9151b03 相同 |
| N2 | BACKLOG | medium | `editor/input_map.rs:172-175` 修飾鍵固定寫成 Left；9151b03 相同 |
| N3 | BACKLOG | low | `editor/mod.rs:1503` gain 固定為 `None`；9151b03 相同 |

## R7 genre packages and game UI

| item | 分類 | 嚴重度 | 證據(一行) |
|---|---|---|---|
| F2 共用導航 | HIGH | high | `axiom-rts/src/simulation.rs:1377-1421` `set_combat` 仍在 `RtsSystem::step`（axiom-runtime/src/rts.rs:589-591）同步重建，成本 O(cells×obstacles)，未量測。修前也在 step 內重建，而且會 O(n) contains 加上 `expect` panic，修後嚴格變好。另外 `Grid::new` 失敗時 `self.combat` 已更新、不再重試，且沒有任何通知 |
| F3 礦物交付溢位 | HIGH | high | `simulation.rs:672-680` 溢位改為 `Idle`+`balance_overflow`（修前 debug panic、release 回繞）；但 `restore` 在 `simulation.rs:1676` 沿用舊 balance，熱載入後交付仍會停擺 |
| F3-compat | REGRESSION | high | `axiom-rts/src/lib.rs:531-543` 新增 `initial_balance + Σminerals ≤ u32::MAX` 檢查，經 `lib.rs:430` 註冊為 `world_validators`，Store::open 與 previous.compile 都會跑。修前 3c108cf 沒有這項檢查，這類世界存入後可以開啟，只有實際交付溢位才出錯。修後整個 DB 開不起來。`content/` 的起始值只有 150/300，實際影響「待確認」 |
| F6 場景旗標 | BACKLOG | medium | `gameplay.rs:651-656` 作者改了旗標時只是不覆寫，留下初始 `on_enter` 的結果；修前一律用舊值覆寫，作者修改從未生效，修後沒有更差 |
| F13 無變數滑桿 | BACKLOG | medium | `gameplay.rs:659-667` 熱載入後 `snap` 只改 `slider_values`；`slider_values`/`bindings` 觀察是本範圍新增的。GUI 顯示前後是否不同「待確認」 |
| F8 RTS 熱載入守恆 | BACKLOG | medium | `simulation.rs:1687-1693` 仍對 goal `request_path`，`:660-663` 只設 `Idle`；3c108cf 相同 |
| F15 Showcase 自動環繞 | BACKLOG | low | `showcase.rs:218-220` 會永久設 `gui_orbit`，但 `showcase.orbit{enabled:false}`（:339-340）仍能關閉，「無法關閉」不成立；首幀多轉約 0.004 rad 是新的小瑕疵 |
| F7 FPS 跨世界移轉 | BACKLOG | medium | `world.rs:1280` `same_world` 在 R2 f18b703 就已存在，所以新測試在舊碼也會通過；legacy `fps/simulation.rs:243-248` 修前修後相同 |
| F9 交付事件毛額 | BACKLOG | medium | `rts.rs:637-640` 實作正確；`tests/rts_frontier.rs:350` 沒有斷言 envelope，屬測試缺口 |
| F12 閒置固定步 | BACKLOG | medium | `gameplay.rs:419-430` 快徑與原行為等價；`tests/m1_gameplay.rs:636` 測不出 clone 被加回來，屬測試缺口 |
| N1 | BACKLOG | medium | `simulation.rs:1601-1626` 只比較 `old.units.len()`；3c108cf 相同 |
| N2 | BACKLOG | medium | `gameplay.rs:221-226` `Op::Call` 會累加，`narrative.rs` 修前修後都不清空；屬既有問題 |
| N3 | BACKLOG | medium | `axiom-fps/src/editing.rs:218-222` `*old = e`；3c108cf 逐字相同 |

---

## 第三輪工作清單

只列 REGRESSION 與 HIGH，共 17 項。標「共通」的項目建議先由同一個 agent 建立共用機制（見下方 C0）。

### 共通 C0：新 validator 讓舊資料庫開不起來（涵蓋 R5-F2、R6-F8、R6-F12、R7-F3-compat）

- 機制：`World::compile` 會跑 component/resource validator 與 `world_validators`，而它被用在：
  - `Store::from_connection`（`crates/axiom-core/src/store.rs:104-108`）
  - `edit_in` 的 `next` 與 `previous`（`store.rs:355-356`）
  - `undo_in`（`store.rs:763`）
  - `verify`（`store.rs:248`）
- 方向：在 `Registry` 加一層「寫入時才檢查」的 validator，例如 `write_validators`，或讓 validator 收到 `Mode::{Read, Write}`：
  - 開庫、`previous.compile`、undo 目標只跑相容讀取檢查。
  - `preview`、`apply`、編輯器提交只對 `next` 跑新規則，而且只在相關資料與 `previous` 不同時跑，讓「移除壞資料」的修復編輯能通過。
- 共用測試骨架：
  1. 用 test-only 原始 SQL 把違反新規則的 world JSON 插入 `revisions`；
  2. 斷言 `Store::open` 成功、`current`/`verify` 可用；
  3. 提交修復編輯成功；
  4. `undo` 回到舊修訂成功；
  5. 乾淨世界上新寫入同樣的壞資料仍被拒。

### R1

**R1-6.2-F3 [REGRESSION, high] 陣列形式的舊覆寫經正規化後改變合併語意，舊 DB 開不起來**

- 檔案：
  - `crates/axiom-core/src/registry.rs:546-570`（`typed_component`/`typed_resource` 設定 `normalize = object_form::<T>`）
  - `registry.rs:602-640`（`decode` → `normalize`）
  - `crates/axiom-core/src/ir.rs:165-180`（prefab 元件 `merge_json`）、`ir.rs:398-410`（`merge_json`：非物件時整筆替換）
- 情境：prefab 有 `render.light {kind:point, range:10}`，實體以 `["directional"]` 覆寫。
  - 修前：陣列整筆取代，得到合法的 directional。
  - 修後：先正規化成 `{kind:directional}`（`range:None` 被省略），合併後保留 `range:10`，`light.rs:125`「directional light has no range」報錯，`Store::open`（store.rs:108）中止。
- 方向：恢復舊合併語意。做法擇一：
  - 實體上的覆寫若原本是陣列（legacy form），正規化時標記為「整筆替換」，或在 merge 前就展開成完整物件，並把省略的 `Option` 欄位以顯式 `null` 寫出，避免繼承 prefab 欄位；
  - 或在 `compile` 合併時依原始值是否為陣列決定替換或合併。
- 另外要處理 `ReplaceWorld`（`protocol.rs:566`）：它會正規化後接受陣列，一般 `SetComponent` 則會拒絕。新寫入應與 `SetComponent` 一致拒絕陣列；只有相容讀取路徑才正規化。
- 測試（`crates/axiom-core/tests/legacy_forms.rs`）：
  - prefab point light（range 10）加上實體陣列 `["directional"]` 的舊 DB 可以開啟、編譯結果為無 range 的 directional、可編輯、可 undo；
  - `ReplaceWorld` 送入陣列元件時被拒。

**R1-6.2-F10 [REGRESSION, medium] 比較器改為精確整數後，已審查的舊提案核准時會選到不同物件**

- 檔案：`crates/axiom-core/src/local.rs:367-381`（`compare_numbers`）；`crates/axiom-core/src/store.rs:537-575`（核准時 `serde_json::from_str(&edits)` 重播，`selectors_grow` 也用新比較器）；`store.rs:439` 提案建立處。
- 修前：f64 比較，預覽與核准一致。修後：升級前存的提案核准時用新比較器重選，可能多命中未審查的物件。
- 方向：建立提案時持久化每個 `set_where` 解析出的目標 ID 集合（存在 `preview` 或新欄位）。核准時若重選結果與存下的集合不同，回 `stale_selector`，要求重新預覽或重新提案。沒有存目標集合的舊提案（升級前建立）一律要求重新預覽後才能核准，並在錯誤訊息中說明。
- 測試（`crates/axiom-core/tests/proposals.rs`）：
  - 手動插入一筆缺少目標集合欄位的舊提案（含 `lt 9007199254740993`），核准被拒且帶指引；
  - 新建提案後若目標集合改變，核准被拒。

**R1-6.2-F6 / 6.1-F4 [REGRESSION, high] 共用 16 MiB 預算讓已存的大提案無法審查或核准**

- 檔案：`crates/axiom-core/src/protocol.rs:366-390`（`apply_all` 共用 `MAX_SET_WHERE_BYTES`）；`crates/axiom-core/src/ops_merge.rs:505`（`apply_edits`）；`store.rs:625`（`ProposalDiff`）、`conflicts`、核准 `store.rs:568`。
- 修前（4b586d8）：沒有展開預算，例如 36 筆、每筆約 481 KB 的已存提案可以 diff、conflicts 與核准。修後：三條路徑都回 `budget_exceeded`，審查者連內容都看不到。
- 方向：
  - 唯讀審查路徑（`proposal_diff`、`conflicts`）對已存提案使用不超過舊單筆上限乘筆數的審查專用預算，或逐筆套用並分頁，讓審查資料仍可讀；
  - 核准時若超出預算，回明確錯誤：「proposal N exceeds the batch budget; split into ≤K edits and propose again」，附上建議的拆批大小，提案保持 pending，不靜默失敗；
  - 新提案仍在 `propose` 時套用共用預算。
- 測試（`crates/axiom-core/tests/selectors.rs` 或 `proposals.rs`）：
  - 直接插入一筆 36×481 KB 的待審提案，`proposal_diff` 第一頁可讀；
  - 核准回帶拆批指引的診斷，狀態仍為 pending。

### R3

**R3-F7 [REGRESSION, medium] editor_view capture 併入 loopback 截圖槽位**

- 檔案：`crates/axiom-runtime/src/app.rs:1781-1831`（驗證區、`editor_action` 迴圈、`self.shots.enqueue(Request{overwrite:true,…})`）；`crates/axiom-runtime/src/screenshot.rs:21`（`QUEUE_LIMIT`）、`:291` `busy()`、`:300-311` `request`/`enqueue`、`:~380` `start_queued`。
- 修前 dc763b8 的行為：`App::pending_capture` 獨立於 loopback，下一幀同步擷取，不拒絕。
- 修後有三個退化：
  1. loopback `screenshot` 在 editor capture 未完成時被拒；
  2. 佇列滿時在 `editor_action` 套用後才失敗，回錯但有副作用；
  3. 取幀要等前一張 PNG 編碼完。
- 方向：
  - editor capture 使用獨立佇列或槽位，loopback `request()` 只看自己的 pending 與 job；
  - 容量與路徑檢查移到 `editor_action` 迴圈之前；
  - 取幀與編碼解耦：下一個可用幀就 grab，Frame 排隊並設記憶體上限。
- 測試（screenshot.rs `order_tests` 或 app.rs tests）：
  - editor capture 排隊時 loopback `request` 仍被接受；
  - 佇列滿時 `editor_view{command,capture}` 回錯，且 command 沒有執行（authoring_rev 與 editor 狀態不變）；
  - 兩次 capture 之間改變狀態，第二張反映請求當下的狀態（用 `Loopback::grabbed` 注入 frame）。

**R3-F1 [REGRESSION, high] 強制至少 1px 的 viewport 改變合法次像素視圖的投影**

- 檔案：`crates/axiom-runtime/src/hdr.rs:180-198`（`viewport()` 的 `clamp(1.0, w-x)`，x clamp 到 `w-1`）；使用處 `render.rs:1297` `pixel_rect`、`:705-707` 投影 aspect、`:960-966` world pass、`:1037` VFX pass、`postfx_gpu.rs:226/291`。指標射線在 `world.rs:1588-1610`。
- 修前 dc763b8 render.rs:958 用 `set_viewport(x*W, y*H, w*W, h*H)`，0.64px 在 wgpu 合法，與射線一致。
- 方向：保留次像素寬高。只把 x、y 夾到 `[0,size)`，extent 取 `min(rect*size, size-x)`，保留 `fit` 的 ulp 下修，不要 1.0 下限；scissor 仍維持 `max(1)`。要確認 HDR resolve 與 volume pass 能接受次像素 viewport。
- 測試（render.rs `camera_rects_stay_inside_odd_and_even_targets` 旁）：
  - `pixel_rect([0,0,0.001,0.001],640,480)` 等於 `[0,0,0.64,0.48]`；
  - 一般 rect 的寬高比等於 `(W*w)/(H*h)`；
  - 邊界 rect 仍滿足 `x+w<=W`。

**R3-F2 [HIGH] Shader 預檢：smoke 報告名稱不符，GPU 啟用失敗路徑沒有覆蓋**

- 檔案：`scripts/shader-smoke-test.ps1:108`（`failed_gpu_activation=$retained`）；`scripts/effects-smoke-test.ps1:109/117`（`gpu_stage_failure`/`repair`）；`crates/axiom-runtime/src/app.rs:2035/2139/2215`（`prepare_shaders`）；`shader_gpu.rs:270`（`shader_pipeline` 診斷）；app.rs:4146-4159 是通用 factory 失敗測試的範例。
- 方向：
  - 報告鍵改名為 `pre_commit_rejection`、`retained_active` 之類，移除 `repair`；
  - 加一個 cfg(test) hook，讓 `prepare_shaders` 回 `shader_pipeline` 錯誤。
- 測試：注入失敗後斷言 `runtime.rev` 不變、`activation=="activation_failed"`、`activation_error.diagnostics[0].code=="shader_pipeline"`。

**R3-F4 [HIGH] VFX 預建遮罩表缺少熱路徑測試**

- 檔案：`crates/axiom-runtime/src/vfx.rs:648-652`（`Running.mask` 建立）、`:502`（inspect）、`:548`（extract）、`:326-335`（`evaluate` 內部也走 `evaluate_masked`）、`:668`（現有測試）。
- 方向：直接測 `extract` 與 `inspect` 用的是 `Running.mask`。做法擇一或並用：
  - 把 `running[id].mask` 換成與 `texture_mask` 不同的已知表，斷言輸出等於用該表求值的結果；
  - 加 cfg(test) 的 `mask_pixels` 呼叫計數，斷言一次 extract（2048 顆）期間為 0 次。
- 另外把位元比對擴大到全部粒子，並改成與測試內獨立實作的舊掃描比較。

### R5

**R5-F2 [REGRESSION, high]（套用 C0）FSM world validator 讓舊 DB 開不起來**

- 檔案：`crates/axiom-runtime/src/fsm.rs:531` `validate_world`（檢查未知 machine、缺 `core.transform`、`emitted_events` :476）；`fsm.rs:1066` 註冊；`fsm.rs:1057-1063` `fsm.agent` radius 0.05..5；`crates/axiom-core/src/ir.rs:368` `r.validate_world`。
- 佐證：`crates/axiom-runtime/src/editor/graph_tests.rs` `Harness::with_runtime` 被迫改寫；`docs/plan-codex-review.zh-TW.md` 的處理紀錄也承認這個影響。
- 方向：依 C0，把這些跨物件檢查改為寫入時才跑；radius 檢查也一樣。順便確認 trigger、nav 是否有相同政策，只為一致性，不擴大本輪範圍。
- 測試：分別插入含 `fsm.agent{machine:"ghost"}`、未宣告 `emit`、radius 越界的舊修訂，各跑一次 C0 骨架。

**R5-F14 [REGRESSION, medium] 綁定觸發的 `tween.play` 在積壓期間被拒且不重試**

- 檔案：`crates/axiom-runtime/src/tween.rs:741-749`（`queued && on_finish` 時 bail）、`:630-633`（完成時 `!player.queued` 判斷）、`:774`（`seek`）；`crates/axiom-runtime/src/world.rs:716-723`（`dispatch_bindings` 只記 `binding_fault`）。
- 方向：不要拒絕 play。把每個 player 的 `queued` 旗標改為計數，或允許同一 player 排多筆完成事件，讓新一輪完成可以排第二筆。這樣也一併處理 `seek` 繞過。
- 測試（`tests/tweens.rs`）：
  - 70 個同步完成的 tween，加上一個在 `door_moved` 時對仍在佇列中的 player 呼叫 `tween.play`（長 clip、有 on_finish）的綁定，斷言 clip 有播放、`binding_fault` 為 null、兩次 on_finish 都送達；
  - seek 版本：積壓期間 seek 到即將完成的 clip，斷言事件不遺失。

**R5-F15 [REGRESSION, low] 公開 schema 遺失 `stop_within?` 選填標記**

- 檔案：`crates/axiom-runtime/src/fsm.rs:255`（`object` closure 沒有 `required`）、`:340` `schema_kinds`；`fsm.rs:23-24` `stop_within` 是 `#[serde(default)]`。
- 方向：`editor_kinds` 的 schema 加 `required`，`schema_kinds` 對非必填欄位加 `?`。
- 測試：`tests/fsm.rs:291` 精確斷言 schema 含 `stop_within?`，或改成整段黃金比對。

**R5-F1 [HIGH] 熱重載帶走 `Combat.outbox`，契約改變後事件被默默丟棄**

- 檔案：`crates/axiom-runtime/src/combat.rs:80`（`inherit` 複製 outbox）、`combat.rs:107`（flush）；`crates/axiom-runtime/src/events.rs:138`（`transfer` 只檢查 `pending`）、`:65`（`publish` 遇到未知 spec 直接 return）；`world.rs:1422-1424`。
- 方向：如果 `old.outbox` 非空，且新舊 `Events` 對 `combat.hit`/`combat.defeated` 的 spec 或 routes 不同，就拒絕重載，訊息與 `Events::transfer` 一致。也可以讓 `transfer` 接受外部積壓旗標。`publish` 遇到未知 spec 不應靜默 return，應記錄 fault。
- 測試（`tests/shared_combat.rs`）：
  - 200 筆命中造成積壓後，熱重載移除 `combat.hit` export，斷言重載被拒、事件全部送達；
  - 對照組：契約不變的重載，事件依序全部送達。

### R6

**R6-F8 + F12 [REGRESSION, medium/low]（套用 C0）`input.bindings` 新規則讓舊 DB 開不起來**

- 檔案：`crates/axiom-core/src/registry.rs:327`（`b.validate_keys()?`）；`crates/axiom-core/src/ir.rs:828-859`（`validate_keys`：同名 button/axis 回 `action_kind`，不在白名單的鍵名回 `invalid_value`）、`ir.rs:302`；`crates/axiom-core/src/ops_tuning.rs:307`（`KEY_NAMES` 白名單）。
- 修前 9151b03 只檢查長度（≤64）。
- 方向：從 resource validator 移除 `validate_keys`，改在寫入路徑檢查。只在 `next` 的 `input.bindings` 與 `previous` 不同時對 `next` 檢查，最好只檢查有變動的 action。不建議用 normalize 改寫：同名 button/axis 沒有不丟資料的改寫方式。
- 測試（`crates/axiom-core/tests/tuning_ops.rs`）：C0 骨架，舊資料含同名 `move` button+axis、`PadA`、`W`、`MouseRight`；先 `input remove move` 成功，再 undo 回舊修訂成功；新 `apply` 寫入同樣資料仍被拒。

**R6-F6 [REGRESSION, medium] 音檔 stamp 快取在內容被替換、但大小與 mtime 不變時回傳舊 PCM**

- 檔案：`crates/axiom-runtime/src/spatial_audio.rs:111-127`（`FileStamp`、`FILE_TEXTS`）、`:132-157`（`cached_clip` 命中時跳過 `file_key`）、`:78`（`file_key`：讀檔加 SHA-256）。
- 修前 9151b03：每次都以內容雜湊判斷。
- 方向，擇一：
  - (a) 照原 F6 建議，把 Play 的 runtime 建置移到背景工作（有界、可取消、回報進度、有 watchdog），在背景照舊讀檔與雜湊，再移除主執行緒上的 stamp 捷徑；
  - (b) stamp 只在同一 world revision 的連續 Play 內使用，revision 變更、熱重載、`resolve_files` 一律走 `file_key`。
- 依 AGENTS.md，效能宣稱要附 A/B 量測，並標明 debug 或 release。
- 測試（`tests/audio_files.rs`）：寫入 WAV A 並建置；用同長度的 WAV B 覆寫，再用 `File::set_modified` 還原 mtime；重建後斷言拿到 B 的 PCM。

### R7

**R7-F3-compat [REGRESSION, high]（套用 C0）RTS 新的餘額上限讓舊 DB 開不起來**

- 檔案：`crates/axiom-rts/src/lib.rs:531-543`（`initial_balance + minerals > u32::MAX` 檢查，本範圍新增）、`lib.rs:430`（`r.world_validators.push(validate_world)`）。
- 方向：依 C0，把餘額上限檢查改為寫入時才跑，或在 Store::open 時降級成 diagnostic。`content/` 的 RTS 世界起始值只有 150/300，實際受影響的 DB「待確認」，可與 F3 一起修。

**R7-F3 [HIGH] 熱載入後餘額可能超過上限，交付停擺**

- 檔案：`crates/axiom-rts/src/simulation.rs:672-680`（`checked_add` 溢位時 `Order::Idle` 加 `last_error="balance_overflow"`，保留不動）、`simulation.rs:1676`（`restore` 中 `self.balance = old.balance`）。
- 情境：舊局 balance 為 `u32::MAX` 且沒有礦；新 revision 起始值減 1、新增一單位礦，作者驗證通過，交付時溢位。
- 方向：在 `restore` 結尾（`check_conservation` 附近）以 u64 檢查每個陣營的「移轉後 balance + 所有 resources.remaining + 所有 units.carried ≤ u32::MAX」，超過就回錯，熱載入被拒並保留舊 revision。
- 測試：純模擬 `Simulation::restore` 與 `WorldRuntime` 熱載入各一個，斷言 reload 被拒且舊狀態保留。現有 `crates/axiom-rts/tests/rts.rs:282` 只測冷啟動。

**R7-F2 [HIGH] RTS 共用導航網格仍在固定步同步重建**

- 檔案：`crates/axiom-rts/src/simulation.rs:1377-1421`（`set_combat`，最壞 16384 格 × 2048 障礙 ≈ 3.4e7 次檢查）；呼叫點 `crates/axiom-runtime/src/rts.rs:589-591`（`RtsSystem::step`）；可參考 `rts.rs:683`、`world.rs:1161` 的準備階段。
- 方向：
  - 共用網格改在 runtime 準備或啟用階段建好，以 `Result` 回報失敗；step 只交換已建好的 grid。
  - 至少改成依障礙 AABB 只掃覆蓋到的格子。
  - `Grid::new` 失敗時不要先設 `self.combat = scene`，應該明確 fault 並允許重試。
  - 遮罩合併與 `last_error` 已解決 panic，不要回退。
  - 依 AGENTS.md 量測重建耗時，並標明 debug 或 release。
- 測試：用一個可透過正式 `WorldRuntime` 啟用的 128×128 世界，共用障礙 ≤2048 筆，驗證首個固定步不 panic、grid 含合併後的阻擋，並記錄耗時。`crates/axiom-rts/tests/rts.rs:246` 只走純模擬，需要補 runtime 版。
