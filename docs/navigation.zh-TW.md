# 導航代理(`nav.agent` / `nav.grid`)

`nav.agent` 讓實體在 XZ 平面上走到一個點或另一個實體,自動繞過 `nav.blocker` 與靜態碰撞體(建築、牆、長椅),能穿過比身體直徑寬的門口進房間。用途:街上的行人與 NPC、玩家點地面指揮 NPC、行為程式把 NPC 送進房間。
執行期系統是 `nav.steer`(capability `nav`)。不綁 RTS:RTS 單位仍用自己的格子(`axiom_rts::nav`),兩者共用同一個 A* 引擎(見下方「設計決定」)。不需要改 Rust:內容、ACL、編輯器都能建立與修改。

## 代理元件 `nav.agent`

```json
{"speed":1.4,"acceleration":8,"turn_rate":720,"radius":0.3,"arrive_distance":0.2,
 "target":{"entity":"shop/door"},"follow_target":false,"avoidance":true,
 "on_arrive":"npc_arrived","on_fail":"npc_lost","enabled":true}
```

| 欄位 | 說明 |
|---|---|
| `speed` | 最高速度,公尺 / 秒,0..50,預設 1.4 |
| `acceleration` | 加速與煞車,公尺 / 秒²,0.1..200,預設 8。接近終點時依 `v = sqrt(2·a·剩餘距離)` 減速,停在終點而不是衝過頭 |
| `turn_rate` | 轉身速度,度 / 秒,0..3600,預設 720;0 = 立刻面向行走方向。**模型正面是 +Z** |
| `radius` | 身體半徑,公尺,0.05..2,預設 0.3:和障礙物、其他代理保持的距離。比直徑寬的門口一定找得到路 |
| `arrive_distance` | 距離目標多近算到達,0.01..50,預設 0.2 |
| `target` | 選填:`{"point":[x,y,z]}`(世界座標,Y 忽略)或 `{"entity":"ID"}`(該實體目前的位置)。有寫的話執行期一開始就出發 |
| `follow_target` | 只用於實體目標:到達後目標又走開(超過 `arrive_distance + 0.5` m)就再跟上去。點目標寫 true 會被拒絕 |
| `avoidance` | 預設 true:和其他導航代理保持距離(簡單分離,見「限制」) |
| `on_arrive` | `behavior.program` 事件(`""` = 不發),到達時發一次 |
| `on_fail` | 走不到時發的事件,`reason` 為 `unreachable`(沒有路)、`outside_grid`(起點或目標在格子外)、`goal_blocked` / `start_blocked`(1.5 m 內找不到可走的格子)、`path_too_long`(超過 128 個轉折點)、`stuck`(重新規劃 3 次仍沒進展) |
| `enabled` | 預設 true。false = 完全不動、不寫 transform |

事件 payload 只能宣告文字欄位 `agent`(代理 ID)、`target`(目標實體 ID,點目標為 `"x,z"`)、`reason`(`arrived` 或失敗原因),或不宣告欄位:

```json
"events":{"npc_arrived":{"fields":{"agent":"text","target":"text"}},"npc_lost":{"fields":{"agent":"text","reason":"text"}}}
```

驗證是嚴格的:未知欄位、超出範圍的值都會讓交易失敗並回傳原因(可恢復的請求錯誤)。世界層級的檢查(`World::compile`,錯誤碼):
- `invalid_nav`:有 `nav.agent` 卻沒排 `nav.steer`;目標實體不存在、是自己或沒有 `core.transform`;事件沒在 `behavior.program` 宣告、payload 欄位不合規則,或沒排 `behavior.execute`;`nav.grid` 欄位錯誤;`pointer_agent` 不是代理。
- `nav_conflict`:**同一個實體只能有一個移動者**。有 `nav.agent` 的實體不能同時有 `core.motion`、`fsm.agent`、`rts.unit`、`action.actor`、`fps.controller`、`fps.sentinel`、`path.follow`。`nav.agent` 也登記在註冊表的 `movers`(`nav.agent → nav.steer`),所以 `path.follow` 加在代理上會得到 `path_conflict`。
- `nav_budget`:見下方預算。

## 導航格子 `nav.grid`(資源,可省略)

```json
"nav.grid":{"cell_size":0.25,"bounds":{"min":[-40,-40],"max":[40,40]},"floor_y":0,
            "colliders":true,"expansions_per_step":4096,"pointer_agent":"hero"}
```

| 欄位 | 說明 |
|---|---|
| `cell_size` | 格子邊長,0.05..4 m,預設 0.25 |
| `bounds` | 格子涵蓋的世界 XZ 範圍。省略時自動取「障礙物 + 所有有位置的實體 + 點目標」外加 4 m;執行期用 `nav.go` 走到範圍外會得到 `outside_grid`,大世界建議明寫 |
| `floor_y` | 地面高度。靜態碰撞體的世界包圍盒和 `floor_y+0.3 .. floor_y+2.0` 有重疊才算障礙物(地板、路緣、招牌、屋頂不算) |
| `colliders` | 預設 true:沒有被任何系統移動的實體(自己與所有祖先都沒有移動者元件、不是代理、不是路徑跟隨者;代理底下的身體碰撞體跟著代理走,不算障礙物)的 `physics.collider`(box / capsule,取世界包圍盒的 XZ)也是障礙物。`nav.blocker` 一律是障礙物(和 FSM、action 一樣用**編寫位置**,不吃旋轉) |
| `expansions_per_step` | 64..262,144,預設 4,096(release 約 0.7 ms):每個固定步(120 Hz)最多花在尋路上的工作量(A* 展開節點數 + 視線檢查走過的格子數 + 找最近可走格子時檢查的格子數)。用完的請求排隊到下一步 |
| `pointer_agent` | 選填:主鍵點擊地面時,這個代理走到點到的位置(`floor_y` 平面) |

### 建立方式(不在每一步做)

格子在**建立執行期時**建好(revision 生效時在背景 worker 建,不在主執行緒),每一格存「格子中心到最近障礙物的距離」(clearance,毫米,u16),所以同一張格子服務所有半徑的代理。結果以「格子設定 + 每個障礙物矩形」的精確內容為鍵快取在 process 內(最多 8 份,未使用的先淘汰):改代理速度、目標、其他物件時不重建;移動障礙物、改 `bounds` / `cell_size` 才重建。執行期間障礙物不會變(會動的 `nav.blocker` 仍以編寫位置計算,和 FSM 一致)。

可走的條件:clearance ≥ 半徑 − 半格。放寬半格是為了保證「比直徑寬的門口一定有一格可走」(格子中心不一定剛好在門的正中間);代理本身由精確的推出(`push_out`)保持在障礙物外。代價:寬度介於「直徑 − 一格」與「直徑」之間的縫隙可能被規劃進去但實際擠不過,代理會卡住 → 重新規劃 3 次 → `on_fail`(`stuck`)。要嚴格就把 `cell_size` 調小。

## 執行規則

- 系統 phase 13:在 fsm(8)、motion(10)、`path.move`(12)之後(跟著路徑走的目標位置已更新),rts(15)、action / fps(20)、`trigger.detect`(25)之前(觸發區看得到代理的新位置)。
- **尋路**:請求進 FIFO 佇列。每一步先試起點到目標的直線視線(看得到就不用 A*),否則用 8 方向 A*(不切障礙物的角,octile 啟發),在 `expansions_per_step` 用完時暫停、下一步從原地繼續(分段不影響結果)。找到的格子路徑先取轉折點,再用視線檢查拉直成最多 128 個路點(拉直本身也算工作量;每條路至少保留 1,024 格的拉直額度,所以單步最多超出預算 1,024 + 一次直線檢查)。
- 目標或起點在障礙物裡:改用 1.5 m 內最近的可走格子;到達就以那個點為準。找格子時檢查的格子數也算進每步工作量。
- 到達、失敗、`nav.stop` 都會取消該代理還在排隊或搜尋中的重新規劃,所以 `on_arrive` 不會重複、到達後不會再冒出 `on_fail`。
- **移動**:朝下一個路點加速(上限 `acceleration`),轉角前 `radius × 0.75`(0.1..0.6 m)內換下一個路點,最後一段依剩餘距離煞車,到 `arrive_distance` 內算到達(發 `on_arrive`)。實體目標的最後一段直接朝實體目前位置走;實體離規劃時的位置超過 max(1 m, 4 格)就重新排隊規劃(舊路先繼續走)。
- **避讓**:代理放進每步重建的桶格子(計數排序,不配置記憶體),只和相鄰桶的代理比:重疊時兩邊各退一半;迎面相撞時兩邊再往自己的右手邊各讓一點,所以能錯身而過。之後對附近的障礙物矩形做精確推出。
- **卡住**:想走卻連續 60 步(0.5 s)實際前進不到 25% 就重新規劃;3 次後 `on_fail`(`stuck`)。
- 只轉 yaw 面向速度方向(速度 > 0.05 m/s 時)。代理有 parent 時,寫入的是相對 parent 的本地位置與旋轉。Y 不變(平面導航,單一樓層)。
- 停用的代理、或所在場景未啟用的實體,不動也不寫入。編輯模式不模擬。
- 熱重載(新 revision):依代理 ID 保留狀態(目標、速度、朝向、到達 / 失敗),條件是編寫的 `target` 與 `radius` 沒變;格子也沒變時連目前的路一起保留(不重新搜尋),格子變了就以同一個目標重新規劃。條件不成立時從編寫的狀態重新開始。
- **存檔(checkpoint)不支援**:排了 `nav.steer` 的世界 `checkpoint` 會明確失敗(`system nav.steer does not implement checkpoints`),不會默默丟掉。
- 執行期 spawn 出帶有 `nav.agent` 的實體時,系統停止並在 `observe` 的 `nav.halted` 寫明原因。

## 執行期指令(AI、UI、行為程式共用)

| 動作 | 參數 | 效果 |
|---|---|---|
| `nav.go` | `{"id":代理,"point":[x,y,z]}` 或 `{"id":代理,"entity":ID}` | 設目標、排隊規劃;結果之後由 `on_arrive` / `on_fail` 通知 |
| `nav.stop` | `{"id":代理}` | 清掉目標,煞車停下 |
| `nav.inspect` | `{"id":代理}` | state、reason、target、position、speed、剩餘路點(最多 32 個)、queued、searching、repaths |
| `nav.summary` | `{}` | 同 observe 的 `nav` 段 |

錯誤是可恢復的:不存在的代理或目標回 `not_found`,參數格式錯誤、停用的代理、走向自己都會說明原因。

行為程式把 NPC 送進房間(`runtime.bindings`):

```json
"runtime.bindings":{"events":[
  {"event":"shop_opens","calls":[{"kind":"call","capability":"nav","action":"go","args":{"id":"npc/baker","entity":"shop/counter"}}]}]}
```

觸發區也能接:`trigger.zone` 的 `on_enter` 綁 `nav.go`,玩家走近時 NPC 走過來。

## 預算(超過就明確失敗或延後)

| 預算 | 值 | 超過時 |
|---|---|---|
| 每個世界的代理 | 1,024 | 交易失敗(`nav_budget`) |
| 格子 | 262,144 格(每邊 ≤ 1,024) | 交易失敗(`nav_budget`,提示設定 `bounds` 或加大 `cell_size`) |
| 障礙物 | 1,024(`nav.blocker` 本身另有 512 上限) | 交易失敗(`nav_budget`) |
| 每條路的路點 | 128 | 該代理 `on_fail`(`path_too_long`) |
| 每步尋路工作量 | `expansions_per_step`(預設 4,096) | 剩下的請求延到下一步;`observe` 的 `budget.deferred_steps` 計數 |
| 每步的事件 | 64 | 這一步一個都不發,系統停止,`nav.halted` 寫明原因 |

## 記憶體與量測

- 格子:每格 2 bytes(256 × 256 = 128 KiB)。搜尋器:每格 12 bytes(cost、parent、世代戳記;同一份重複使用,開始新搜尋不清記憶體)+ 可重用的 open heap。每個代理預先保留 128 個路點(1 KiB)。
- 穩定狀態的步長不配置記憶體(open heap、路點、桶陣列都重複使用;事件發出時才配置)。

量測(**release**,`perf_bench --generated-nav N`:64 × 64 m 城鎮,8 × 8 棟 4 × 4 m 建築、4 m 街道,格子 256 × 256(0.25 m);N 個代理以 1.4 m/s 追 N 個以 2 m/s 繞兩條街道迴圈的標記,`follow_target`,永遠追不上,所以每走 1 m 就重新規劃,佇列一直是滿的;1200 步,warm-up 120 步):

| 設定(3 個新程序,依序列出) | 步長 p50 | p95 | p99 | max | 每步配置 p50 / max | 單步最多工作量 | 延後步數 / 搜尋次數 |
|---|---|---|---|---|---|---|---|
| 256 代理,預算 4,096(預設) | 0.643 / 0.698 / 0.642 ms | 0.802 / 1.028 / 0.817 | 0.886 / 1.225 / 0.900 | **0.967 / 1.412 / 1.105** | 0 / 1 | 4,648 | 1,291 / 4,806 |
| 256 代理,預算 16,384 | 0.483 / 0.473 / 0.472 ms | 2.339 / 2.310 / 2.309 | 2.639 / 2.626 / 2.631 | 3.454 / 2.854 / 3.457 | 0 / 1 | 17,022 | 86 / 5,579 |

- 預算決定每步的上限:預設 4,096 時最慢的一步約 1–1.4 ms(其中尋路約 0.7 ms,其餘是 256 個代理的轉向、避讓、推出與寫 transform),16,384 時約 3–3.5 ms。預設選 4,096:請求多等幾步(延後 1,291 步,約 10 s 的模擬時間內),但不會出現數毫秒的尖峰。
- 每步配置 p50 = 0;max = 1 次(16 KiB,1,200 步中一次:open heap 第一次長大後就保留;`WorldRuntime::step` 本身也有這 1 次,見 `paths.zh-TW.md`)。
- 格子建立(256 × 256、64 棟建築):1.1–2.0 ms;`nav.steer` 系統建立 1.1–2.0 ms(背景建立執行期時發生)。工作集約 18 MiB。
- 編輯器預覽路徑(冷快取、主執行緒,`agent/0` 走到另一頭的標記):0.39–0.58 ms(release;debug 會慢數倍,上限 65,536 節點)。
- 對 RTS 與城鎮沒有影響(release,改動前後的程式,`perf_bench --pack`,5 回合交錯 ABBA,中位數 [p50 範圍]):

| 內容包 | 步長 p50 前 → 後 | p95 前 → 後 | 每步配置 p50 / max 前 → 後 | 編輯→生效 p50 |
|---|---|---|---|---|
| rts(Frontier) | 0.0099 [0.0098..0.0129] → 0.0101 [0.0096..0.0122] ms | 0.0199 → 0.0181 | 36 / 678 → 36 / 672 | 46.2 → 46.8 ms |
| rts-bench | 0.0195 [0.0194..0.0205] → 0.0205 [0.0199..0.0207] ms | 0.0294 → 0.0273 | 77 / 78 → 77 / 78 | 20.3 → 20.3 ms |
| axiom-town | 0.0178 [0.0176..0.0185] → 0.0182 [0.0178..0.0192] ms | 0.0192 → 0.0203 | 71 / 84 → 71 / 84 | 16.8 → 17.4 ms |

  rts-bench 的 p50 慢了約 1 µs(5%),三次 A/B(程式逐步調整中)都慢 0.9–1 µs,只有第二次的範圍完全不重疊;原因沒有確認(候選:`CompiledEntity` 多了一個欄位、RTS 搜尋改走共用引擎)。RTS 搜尋現在每條執行緒重用緩衝,Frontier 的單步最大配置因此少了 6 次。

## 觀測(不需要 GPU)

`observe` 的 metrics 有 `nav`:

```json
"nav":{"grid":{"size":[256,256],"cell":0.25,"origin":[-32,-32],"obstacles":64,"blockers":64,
              "blocked_cells":16384,"build_ms":3.1,"cached":false},
  "agents":3,"idle":0,"pending":1,"moving":1,"arrived":1,"failed":0,
  "at":{"bob":["moving",1.25,3.5],"ann":["arrived",4.0,2.0],"cat":["pending",0.0,0.0]},"more":0,
  "failures":{},"queue":0,"searching":"cat",
  "budget":{"per_step":4096,"last":4096,"max":4512,"deferred_steps":42,"searches":9},
  "emitted":1,"halted":null,"fault":null}
```

- `at`:前 32 個代理的 `[狀態, x, z]`,其餘以 `more` 計數;`failures` 最多 16 個 `{ID: 原因}`。
- `budget.last` / `max`:上一步 / 歷來最多的尋路工作量;`deferred_steps`:有請求被延後的步數。
- 情境斷言:`{"kind":"call","action":"nav.inspect","args":{"id":"bob"},"pointer":"/state","op":"eq","value":"arrived"}`。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object 選單 / 指令面板 `object.obstacle`、Hierarchy「+ Object」▸ Obstacle | `create obstacle [id=] [at=] [parent=] [scene=]`:1 m 方塊 + `nav.blocker {half:[0.5,0.5]}`(牆:再 `set ID nav.blocker/half [4,0.1]` 並改網格大小) |
| `object.nav_agent`(Object ▸ Add Nav Agent):選取的物件都加上預設 `nav.agent`,世界缺少時補 `nav` capability 與 `nav.steer` 系統 | `nav SEL agent [speed=] [radius=]` |
| `nav.target_object`(Nav Target: Selected Object):選取中的代理走向「選取中唯一不是代理的那個物件」 | `nav AGENTS target ENTITY` |
| `nav.target_point`(Nav Target: Click Ground…):下一次在場景視口點地面(通過主選取代理高度的水平面),選取中的代理目標設為該點;Esc 取消 | `nav AGENTS target x,y,z` |
| `nav.clear_target` | `nav AGENTS target none` |
| Inspector 的元件表單 | `set ID nav.agent/speed 2` 等 |

Gizmo(場景工具列 Gizmos;隱藏的物件不畫):代理腳下的半徑圓、`NAV` 標籤、到編寫目標的線與十字;**選取的代理**另外畫出它從現在位置走到目標的預覽路徑(綠色;同步但有上限的搜尋 65,536 節點,依輸入快取,不會每幀重算)。`view.nav_grid`(View ▸ Toggle Navigation Grid Overlay,存在偏好檔 `nav_grid`)在選取物件(沒選時為相機注視點)周圍 81 × 81 格內,用紅色 × 標出選取代理(沒選代理時以 0.3 m 計)不能進入的格子,最多 2,400 個。Hierarchy 標籤 `NAV`。Play 預覽的 Behavior overlay 會畫每個代理剩下的路線(綠線)。

## 設計決定(ADR)

- **共用 A* 引擎**:`axiom_core::grid_search::GridSearch`(純函式模組,可分段、世代戳記、重用 heap;4 方向單位成本 Manhattan,或 8 方向 10/14 octile)。RTS 的 `Grid::search` 改成呼叫它的 4 方向模式(搜尋緩衝每條執行緒重用一份,每次查詢只配置結果本身):open list 以 (f, g, 格子索引) 全序排序、預算檢查在 pop 之後(超出時把剛 pop 的放回去,下一段先拿到),所以結果與原本逐位相同。`crates/axiom-rts/tests/nav_equivalence.rs` 保留舊實作逐字複本,在 3,200 組隨機格子 / 起終點 / 預算上比對 path、expanded、exhausted 全部相同(把啟發式改成 2 倍時會失敗,證明測試有效)。
- 沒有直接沿用 RTS 的 `Grid`:它限 128 × 128、每次搜尋配置記憶體、只有 4 方向、不能分段,也不支援多種半徑;改它會改變 RTS 行為。
- 不另寫第二個尋路器:代理的格子(clearance 光柵、直線視線、拉直)放在 `axiom_core::navigation`,搜尋本身共用。
- 8 方向模式的同 f 值節點優先展開 g 較大(較深)的,開闊地展開數少很多(256 代理基準:步長 p50 0.96 → 0.59 ms,release);4 方向(RTS)維持原本的 g 較小優先。
- 核心模組不能叫 `nav`(會和 `axiom_rts::nav` 在 glob import 時衝突),所以叫 `navigation` 與 `grid_search`;執行期模組叫 `nav_agent`。

## 限制

- 平面、單一樓層;沒有斜坡 / 樓梯 / 多層。障礙物是 XZ 矩形(`nav.blocker` 不吃旋轉;碰撞體取世界包圍盒,旋轉的箱子會變大)。
- 障礙物在執行期間不會變:開門、移動中的車不會改變格子(門口之類請用觸發區 + `nav.stop` / `nav.go` 控制)。
- 比一格還薄的牆(例如 0.05 m 厚、格子 0.25 m)可能沒有任何格子中心落在牆內;一般半徑的代理仍會被 clearance 擋住,但半徑很小(< 半格)的代理可能被規劃穿牆,實際被推出擋住後以 `stuck` 失敗。牆至少做到一格厚,或把 `cell_size` 調小。
- 避讓只是簡單的成對分離 + 迎面右讓,不是 RVO / ORCA:很擠的窄走廊可能互相卡住而觸發 `stuck`;只對其他導航代理避讓(不避 FSM 代理、玩家)。
- 自動 `bounds` 只看編寫內容;執行期走到範圍外會 `outside_grid`。
- 不支援 checkpoint、執行期 spawn 的代理。
