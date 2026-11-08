# 觸發區(`trigger.zone`)

觸發區是一個方塊或球體,跟著實體的世界位置與旋轉(含 parent)。符合篩選條件的實體「樞紐點」(世界位置)進入或離開時,發出 `behavior.program` 事件,所以 `behavior.program` 的規則與 `runtime.bindings` 的原生呼叫都會照常執行,不需要改 Rust。
執行期系統是 `trigger.detect`(capability `trigger`)。範例:`content/axiom-town` 的長椅與髮夾觸發點。

## 元件

```json
{"shape":"sphere","radius":1.6,"exit_margin":0.6,"filter":{"tags":["player"]},
 "on_enter":"near_bench","on_exit":"left_bench","once":false,"enabled":true}
```

| 欄位 | 說明 |
|---|---|
| `shape` | `box` / `sphere`(必填) |
| `half_extents` | 只限 `box`(且必填):本地 X / Y / Z 的半長,公尺 0.01..1000 |
| `radius` | 只限 `sphere`(且必填):公尺 0.01..1000 |
| `filter` | `{tags:[…], components:[…]}`,各最多 8 個。實體要**同時**帶有每個 tag 與每個元件才算(和選取器 `entities[tag=…,has=…]` 相同的 AND 語意)。空的 filter = 任何有 `core.transform` 的實體。觸發區自己永遠不算 |
| `on_enter` | 進入時發出的 `behavior.program` 事件 ID,`""` = 不發 |
| `on_exit` | 離開(或被停用)時發出的事件 ID,`""` = 不發 |
| `once` | 預設 false。true = 整個執行期最多發一次進入、一次離開事件;進出狀態仍持續追蹤 |
| `enabled` | 預設 true。停用的觸發區(或實體所在場景未啟用)**凍結**:不判定、不發事件,保留原本的區內名單 |
| `exit_margin` | 公尺 0..100,預設 0。遲滯:已在區內的實體要離開「形狀外擴 margin」才算離開,避免在邊界上反覆進出 |

驗證是嚴格的:未知欄位、形狀不符的欄位(例如 sphere 帶 `half_extents`)、超出範圍的數值都會讓交易失敗並回傳原因(可恢復的請求錯誤)。

事件的 payload 只能宣告文字欄位 `trigger`(觸發區實體 ID)和 / 或 `entity`(進出的實體 ID),或不宣告欄位。例:

```json
"events":{"near_bench":{"fields":{}},"gate_in":{"fields":{"trigger":"text","entity":"text"}}}
```

世界層級的檢查(在 `World::compile`,錯誤碼 `invalid_trigger` / `trigger_budget`):有 `trigger.zone` 就必須排入 `trigger.detect` 系統;事件必須在 `behavior.program` 宣告;payload 規則如上。

## 判定規則

- 「在區內」= 實體的世界樞紐點落在形狀內(球:距離 ≤ 半徑;方塊:轉到觸發區的本地座標後,三軸都 ≤ 半長)。**不使用**實體的包圍盒或碰撞形狀。
- 形狀只吃世界位置與旋轉,**不吃縮放**(和 `render.light` 的面光源大小一樣),所以尺寸一律是公尺。
- 只有啟用中的實體會被判定;實體被停用(場景關閉)視同離開,會發 `on_exit`。
- 每個固定步(120 Hz)的順序:觸發區依編寫順序;同一個觸發區先處理離開、再處理進入;同一類依實體編寫順序。所有事件先收集,判定完才依序發出。
- 系統的 phase 是 25:在 fsm(8)、motion(10)、rts(15)、action / fps(20)之後,animation(30)之前。
- 執行期 spawn 出來的實體(例如 RTS 生產)若符合 filter,會加入候選。spawn 出來的實體若**帶有** `trigger.zone`,系統會停止並回報原因(不支援執行期新增觸發區)。
- 熱重載(新 revision):依觸發區 ID 保留區內名單與 once 計數;區內實體依 ID 重新對應,被刪除的實體直接移除(不發離開事件)。形狀改小時,下一步就會發出離開事件。
- **存檔(checkpoint)不支援**:區內名單是模擬狀態,存檔格式沒有欄位,`checkpoint` 會明確失敗(`system trigger.detect does not implement checkpoints`),不會默默丟掉。

## 預算(超過就明確失敗)

| 預算 | 值 | 超過時 |
|---|---|---|
| 每個世界的觸發區 | 256 | 交易失敗(`trigger_budget`) |
| 每步的候選檢查數(所有觸發區的 filter 候選數總和) | 262,144(= 256 × 1024) | 執行期建立失敗;執行中因 spawn 超過時系統停止,`observe` 的 `trigger.halted` 寫明原因 |
| 每步發出的事件 | 64 | 這一步一個都不發,系統停止,`trigger.halted` 寫明原因 |
| 每個 filter 的 tag / 元件 | 各 8 | 交易失敗 |

`behavior.program` 處理事件時的失敗(例如規則預算)不會停止系統;第一個錯誤記在 `trigger.fault`。

## 複雜度與量測

每一步:
1. 算出每個啟用中候選實體的世界樞紐點(有 parent 時沿 parent 鏈相乘,不配置記憶體),用計數排序放進雜湊過的 XZ 網格(格寬 4 m,桶數 = 候選數 × 2 取 2 的冪,64..8192,陣列在建立 / spawn 時配置,每步重用)。
2. 每個啟用的觸發區只檢查它外接球(球半徑,或方塊半長的長度,加上 `exit_margin`)涵蓋的格子裡、屬於它 filter 的候選(filter 成員用 bitset 判斷),結果排序去重(雜湊碰撞可能重複)。外接範圍超過 64 格的大觸發區改為檢查整個 filter 候選清單。
3. 和上一步的區內名單(排序過的 slot 向量)比對,新名單寫在重複使用的暫存向量後交換。

複雜度:O(候選數 + 桶數 + Σ 觸發區 × 附近候選數);最壞情況(全部擠在同一格或全是大觸發區)退化成 Σ 觸發區的 filter 候選數,由 262,144 的預算封頂。穩定狀態的步長**不配置記憶體**(量測:每步配置 p50 0)。

量測(**release**,`perf_bench --generated-triggers ZxM`:Z 個半徑 1 m 的球每 4 m 一個、M 個 `player` 以 0.5 m/s 漂移、每 2 m 一個;每個設定一個新程序,1200 步;3 回合交錯,步長 p50 中位數):

| 設定 | 暴力法(第一版) | 網格(現在) |
|---|---|---|
| 256 × 1024(預算上限) | 0.932 ms(p95 1.0–1.9) | **0.076 ms**(p95 0.10–0.13) |
| 64 × 256 | 0.070 ms | 0.020 ms |
| 0 × 1024(只有 motion,對照組) | 0.009 ms | 0.008 ms |

暴力法在上限時每步約 0.93 ms(120 Hz 下約佔一個核心的 11%),所以改用網格;網格版與暴力法逐步比對結果相同(測試 `grid_query_matches_a_brute_force_reference`,40 個大小 0.3–30 m 的球 / 旋轉方塊 × 300 個移動實體)。最壞情況(所有觸發區都大到走全清單)約等於上表暴力法的 0.93 ms。

axiom-town(release,`perf_bench --pack`,改動前用舊內容 + 舊程式,改動後用新內容 + 新程式,5 回合交錯 ABAB…,中位數):步長 p50 0.0219 → 0.0220 ms、每步配置 p50 / max 71 / 84 → 71 / 84(不變)、`observation()` 0.039 → 0.042 ms(配置 330 → 339,多了 `trigger` 這一段)、render_frame 1.60 → 1.61 ms、編輯→生效 p50 17.9 → 18.1 ms、工作集約 18 MiB(不變)。差異都在噪音內。

## 觀測(不需要 GPU)

`observe` 的 metrics 有 `trigger`:

```json
"trigger":{"zones":2,"occupied":1,"inside":{"spot/bench-east":["hero"]},
  "recent":[[377,"enter","spot/bench-east","hero"],[429,"exit","spot/bench-east","hero"]],
  "emitted":2,"pairs":2,"halted":null,"fault":null}
```

- `inside`:有實體在內的觸發區(最多列 32 個,每個最多 8 個 ID,多的寫成 `"+N"`)。
- `recent`:最近 32 筆進出,`[tick, enter|exit, 觸發區, 實體]`,舊到新。被 `once` 擋下或沒設事件的進出也會列出(這是進出紀錄,不是事件紀錄;事件數看 `emitted`)。
- 也可以用動作 `trigger.inspect {"id":"觸發區 ID"}`(區內名單、候選數、進出計數)與 `trigger.summary`。情境斷言可用 `{"kind":"call","action":"trigger.inspect","args":{"id":…},"pointer":"/inside/0","op":"eq","value":"hero"}`。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object 選單 / 指令面板:`object.trigger_box`、`object.trigger_sphere` | `create trigger box\|sphere [id=] [at=x,y,z] [parent=] [scene=]`(也可寫 `create trigger.box`) |
| Hierarchy「+ Object」▸ Box trigger / Sphere trigger | 同上 |
| Inspector 的元件表單(只寫改動的欄位) | `set ID trigger.zone/FIELD VALUE` |
| Inspector ▸ Remove component | `remove SEL COMPONENT` |

預設值:box 半長 1 m、sphere 半徑 1.5 m,位置 (0,0,0),filter `{"tags":["player"]}`,沒有事件。世界缺少時會一起加上 `trigger` capability 和 `trigger.detect` 系統(不加 `render`)。預設 ID 是 `trigger.box`、`trigger.box/2`…

Gizmo(場景工具列 Gizmos 開關;隱藏的物件不畫):box = 依實體軸向的線框;sphere = 三個軸向圓;有 `exit_margin` 時多一圈暗色的外框(中間高度)。顏色:青綠 = 一般,暗綠 = 停用,白 = 選取。Hierarchy 標籤 `TRIG`。

Play 預覽(編輯器的 ▶ Play)的「Behavior overlay」(預設開啟,和 FSM 狀態標籤同一個開關)會把每個正在判定的觸發區畫成線框投影到遊戲畫面上:青綠 = 空的,**粉紅加粗 = 有實體在區內**。編輯模式的執行期不模擬,所以場景視口的 gizmo 不顯示區內狀態。

## Axiom Town 的取代

原本長椅與髮夾是掛 `fsm.agent` 的隱形物件,用兩個狀態 + `near 1.6` / `far 2.2` 轉換發出 `near_*` / `left_*` 事件。現在改成 `trigger.zone`(球,半徑 1.6,`exit_margin` 0.6 → 2.2 公尺才算離開),行為等價:

- 測試 `axiom_town_trigger_zones_match_the_fsm_near_workaround` 用同一組輸入讓主角走近長椅、離開、走近髮夾、離開,比較舊版(FSM)與新版每一步的 `at_bench` / `at_clip`:**同樣的值、同樣的 tick**。
- 舊版的隱形 FSM 物件會被 `nav.blocker` 推開(第一步就從 (6,0,4) 移到 (6,0,3.85),髮夾從 (-2.3,0,-1.9) 移到 (-2.5,0,-1.9),髮夾模型也跟著移)。新版直接把觸發區編寫在推開後的位置,所以觸發中心與髮夾的顯示位置都和舊版一樣。
- **差異(刻意)**:舊版的隱形物件也是「會互相推開的身體」,主角或 NPC 靠近到 0.55 m 內會把觸發點推走(觸發中心跟著跑),NPC 閒逛也會繞開它們。新版觸發區不是身體,不會被推、也不擋 NPC。上面的測試路徑刻意不靠近到 0.55 m 內。
