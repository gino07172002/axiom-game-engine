# 互動元件(`interact.target`)

互動元件讓「互動者」(預設是帶 `player` tag 的實體)走到物件旁邊按一個鍵就能使用它:靠近角色按 E 交談、走到門前按 E 開門。使用時發出 `behavior.program` 事件,所以 `behavior.program` 的規則與 `runtime.bindings` 的原生呼叫都會照常執行,不需要改 Rust。提示文字走共用的 `ui.document` 綁定(`{"kind":"prompt"}`),不是另外畫的覆蓋層。

執行期系統是 `interact.focus`(capability `interact`)。範例:`content/axiom-town` 的露米與琳(取代原本「FSM near + `nearby` 變數 + `interact` 輸入事件」的做法,見最後一節)。

## 元件

```json
"interact.target":{"range":2.4,"exit_margin":0.8,"prompt":"talk_lumi","event":"talk_lumi"}
```

| 欄位 | 說明 |
|---|---|
| `range` | 公尺 0.1..100,預設 2。互動者與目標的**世界樞紐點**(含 parent)之間的 3D 距離 ≤ range 才可互動 |
| `exit_margin` | 公尺 0..100,預設 0。遲滯:已經被聚焦的目標要超過 range + exit_margin 才放開(和 `trigger.zone` 的 `exit_margin` 同義),避免提示在邊界閃爍 |
| `view_angle` | 度 1..360,預設 360(不要求朝向)。目標必須落在互動者朝向 ±view_angle/2 之內(XZ 平面)。互動者的朝向 = 世界 +Z(引擎的模型慣例,和 `fsm.agent`、`path.follow` 一樣) |
| `prompt` | 在地化 key,或直接寫文字(UI catalog 找不到這個 key 時原樣顯示),≤ 256 bytes;`""` = 預設提示 `Interact`(也可以在 catalog 加 `Interact` 這個 key 來翻譯) |
| `action` | `input.bindings` 的**按鈕名稱**,預設 `interact`。runtime 不寫死任何按鍵:按鍵由綁定決定(town 是 `"interact":"KeyE"`)。`""` = 沒有按鍵,只能用 `interact.use` |
| `event` | 使用時發出的 `behavior.program` 事件,`""` = 不發。payload 只能宣告文字欄位 `interactor`(互動者 ID)和 / 或 `target`(目標 ID),或不宣告欄位 |
| `cooldown` | 秒 0..3600,預設 0。使用後這段時間內不能被聚焦、也不能使用(依固定步換算,120 Hz,四捨五入) |
| `once` | 預設 false。true = 整個執行期只能被使用一次(所有互動者合計),用過之後不再被聚焦 |
| `enabled` | 預設 true。停用的目標(或實體所在場景未啟用)不會被聚焦 |
| `priority` | 整數 -1000..1000,預設 0。數字大的優先 |
| `interactor` | `{tags:[…], components:[…]}`,各最多 8 個,預設 `{"tags":["player"]}`。和 `trigger.zone` 的 filter、選取器 `entities[tag=…,has=…]` 相同的 AND 語意。寫 `{}` = 任何有 `core.transform` 的實體。目標自己永遠不算;`core.health` 歸零的實體不能互動 |

驗證是嚴格的:未知欄位、超出範圍的數值都會讓交易失敗並回傳原因(可恢復的請求錯誤)。世界層級的檢查(`World::compile`,錯誤碼 `invalid_interact` / `interact_budget`):有 `interact.target` 就必須排入 `interact.focus` 系統;`action` 必須是 `input.bindings` 裡的按鈕;有事件時必須有 `behavior.execute`,事件必須在 `behavior.program` 宣告,payload 規則如上。

## 聚焦與使用

- **聚焦**:每個固定步,替每個互動者在它可用的目標中挑一個:先比 `priority`(大者勝),再比距離(近者勝),再比朝向(越正面越好),全部相同時取編寫順序在前的。可用 = 啟用、場景啟用、沒有 once 用掉、不在 cooldown、在 range 內(已聚焦的用 range + exit_margin)、符合 view_angle、互動者符合 `interactor` filter。互動者本身停用或死亡就沒有焦點。
- **本機互動者(local interactor)**:鍵盤只驅動「所有目標 filter 聯集中,編寫順序第一個啟用且活著的實體」(單人遊戲就是玩家)。提示 UI 也顯示它的焦點。其他互動者(例如 AI 夥伴)用 `interact.use` 指定。
- **按鍵**:目標 `action` 按鈕的上升緣(按住不會重複觸發),使用的是**上一步算出來的焦點**,也就是玩家當時看到的提示;按鈕必須是那個焦點目標的 `action`(其他按鈕不作用)。按下時會再確認目標仍然啟用、場景仍啟用、沒有被 once 用掉、不在 cooldown(同一步稍早的規則可能改變這些);距離與朝向則以提示顯示時為準。`interact.use` 不指定 target 時也一樣。
- **時機**:系統 phase 7,在 `behavior.execute`(5)之後、`fsm.step`(8)與所有移動系統之前。順序:先處理按鍵,再用上一幀顯示的位置重新計算焦點,最後更新提示。這正好是 FSM `near` / `far` 條件看到的時機,所以 town 的移植能逐 tick 相同。
- **使用** = 使用次數 +1、設定 cooldown、記錄、發出事件(和 `trigger.zone`、FSM 狀態事件走同一條 gameplay 指令路徑);`runtime.bindings` 在同一步結束時執行(由 `interact.use` 動作觸發的,在下一步的步界執行)。

## 提示 UI(`ui.document`)

在 `ui.document.widgets` 放一個文字 widget,綁定 `prompt`:

```json
{"kind":"text","id":"prompt","binding":{"kind":"prompt"}}
```

- 有焦點時顯示 `[E] 和露米聊聊`:`E` 來自目標 `action` 對應的 `input.bindings` 按鍵(`KeyE` → `E`、`Digit1` → `1`,其餘原樣);文字依目前語系查 catalog,查不到就原樣顯示。文字裡寫 `{key}` 時改成在該處代入按鍵(例:`"按 {key} 交談"`),不再加前綴。沒有 `action` 的目標只顯示文字。
- 沒有焦點時是空字串;放在 `regions` 裡的空文字不畫(和其他 HUD widget 一樣)。切換語系(`locale` 指令)立即生效。
- **預設提示**:世界有 `behavior.execute` 但沒有任何 widget 綁定 `prompt`(或根本沒有 `ui.document`)時,共用 UI 會在畫面中央偏下畫一個預設提示框,內容相同。
- `observe` 的 `gameplay.bindings.prompt` 會顯示目前的提示文字(可用 scenario `equals` 斷言)。

## 動作與觀測(不需要 GPU)

| 動作 | 參數 | 結果 |
|---|---|---|
| `interact.use` | `interactor`(選填,預設本機互動者)、`target`(選填,預設該互動者目前的焦點) | `{interactor, target, event, uses}`。指定的目標必須**現在**可用;不可用時回 `not_interactable` 並寫明原因(`out of range (8.00 m > 2 m)`、`outside the target's view_angle`、`cooling down (60 ticks left)`、`once and already used`、`disabled`、`inactive scene`、filter 不符);不存在回 `not_found` |
| `interact.inspect` | `id`:目標 ID 或互動者 ID | 目標:狀態、使用次數、cooldown 剩餘 tick、被誰聚焦、候選數;互動者:焦點,以及每個目標(最多 32 個)是否可用、距離與原因 |
| `interact.summary` | — | 同 `observe` 的 `interact` |

`observe` 的 metrics 有 `interact`:

```json
"interact":{"targets":2,"interactors":1,"local":"hero","focus":{"hero":"npc/lumi"},
  "recent":[[377,"hero","npc/lumi","key"],[912,"hero","npc/rin","call"]],
  "used":2,"pairs":2,"halted":null,"fault":null}
```

`focus` 最多列 32 個互動者;`recent` 最近 32 次使用 `[tick, 互動者, 目標, key|call]`,舊到新。事件處理失敗(例如規則預算)不會停止系統,第一個錯誤記在 `fault`。Play 預覽的 Behavior overlay 會從互動者畫一條黃線到它聚焦的目標。

情境斷言範例:`{"kind":"call","action":"interact.inspect","args":{"id":"hero"},"pointer":"/focus","op":"eq","value":"npc/lumi"}`;情境指令 `{"tick":30,"operation":{"op":"command","command":{"kind":"call","capability":"interact","action":"use","args":{}}}}`。

## 狀態、熱重載、存檔

- 熱重載(新 revision):依目標 ID 保留使用次數與 cooldown;依互動者 ID 保留焦點;依按鈕名稱保留「按住中」狀態(按住的鍵不會因為重載再觸發一次);接著用新內容重新計算一次焦點(例如目標被停用、range 改小就立刻放開),再發布提示。
- 執行期 spawn 出來的實體若符合 filter,會成為互動者候選。spawn 出來的實體若**帶有** `interact.target`,系統停止(焦點清空、提示消失)並在 `observe` 的 `interact.halted` 寫明原因(不支援執行期新增目標)。
- **存檔(checkpoint)不支援**:使用次數、cooldown、焦點是模擬狀態,存檔格式沒有欄位,`checkpoint` 會明確失敗(`system interact.focus does not implement checkpoints`),不會默默丟掉。
- 輸入情境(input contexts)把系統的輸入切走時,所有按鈕視為按住,切回來時要先放開再按才算。

## 預算(超過就明確失敗)

| 預算 | 值 | 超過時 |
|---|---|---|
| 每個世界的目標 | 256 | 交易失敗(`interact_budget`) |
| 每步的互動者 × 目標檢查數(每個目標的 filter 候選數總和) | 16,384(= 256 × 64) | 執行期建立失敗;執行中因 spawn 超過時系統停止,`interact.halted` 寫明原因 |
| 每個 filter 的 tag / 元件 | 各 8 | 交易失敗 |
| 每步發出的事件 | 按鍵最多 1 次(只有本機互動者) | — |

## 複雜度與量測

每一步:算出每個可用互動者的世界樞紐點與朝向(O(互動者)),每個目標的樞紐點(O(目標)),然後**每個目標只掃它 filter 的候選**:O(Σ 目標 × filter 候選),由 16,384 的預算封頂。沒有用觸發區的空間網格:目標最多 256 個、檢查只是一次減法、長度與比較,上限時的成本見下表(release 實測),所以暴力法足夠。暫存陣列(姿勢、最佳候選、目標樞紐點)在建立 / spawn 時配置、每步重用,提示只在本機焦點改變時更新(`Arc<str>` 共用字串,不複製)。

量測(**release**,`perf_bench --generated-interact TxP`:T 個 range 2 m 的目標每 3 m 一個、每三個有一個 view_angle 120°,P 個 `player` 以 0.6 m/s 漂移並自轉;每個設定一個新程序,1200 步;3 回合,步長含 `motion.integrate`):

| 設定 | 步長 p50 | p95 | 每步配置 p50 / max |
|---|---|---|---|
| 256 × 64(預算上限,16,384 對) | 0.090–0.099 ms | 0.100–0.131 ms | 0 / 1 |
| 64 × 16 | 0.007 ms | 0.007–0.011 ms | 0 / 1 |
| 0 × 64(只有 motion,對照組) | 0.004 ms | 0.004 ms | 0 / 1 |

上限時互動檢查約 0.09 ms / 步(120 Hz 下約一個核心的 1.1%),每對約 5.6 ns,所以沒有做空間網格。試過「先用平方距離粗篩再開根號」:上限時反而變慢(0.105 ms,3 回合),已撤回。系統建立(256 目標)約 0.12–0.13 ms。

axiom-town(release,`perf_bench --pack`,改動前用舊內容 + 舊程式,改動後用新內容 + 新程式,5 回合交錯 ABAB…,中位數):步長 p50 0.0178 → 0.0179 ms、p95 0.0203 → 0.0209 ms、每步配置 p50 / max 71 / 84 → 69 / 82(少了 `interact` 輸入事件)、`observation()` 0.0344 → 0.0342 ms(配置 339 → 350,多了 `interact` 這一段)、render_frame 1.33 → 1.32 ms、編輯→生效 p50 16.7 → 17.2 ms。審查修正後的最終程式再跑 5 回合:步長 p50 0.0179 → 0.0178 ms、p95 0.0193 → 0.0192 ms、配置 71 / 84 → 69 / 82、`observation()` 0.0341 → 0.0342 ms。差異都在噪音內。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object 選單 / 指令面板:`object.interactable`(Make Interactable),作用在選取的物件(選取順序) | `interact SEL [event=E] [prompt=TEXT] [range=M] [action=BUTTON]`(`prompt="兩個字以上用引號"`) |
| Inspector 的元件表單(只寫改動的欄位) | `set ID interact.target/FIELD VALUE` |
| Inspector ▸ Remove component | `remove SEL interact.target` |

兩邊共用 `ops::make_interactable`,一個 revision 內完成:加上 `interact.target`(已有的會被取代)、世界缺少時補 `interact` capability 與 `interact.focus` 系統;事件(預設 `interacted`)沒宣告時在 `behavior.program` 宣告(文字欄位 `interactor`、`target`;沒有 program 就建立,並補 `behavior` capability 與 `behavior.execute`);按鈕(預設 `interact`)沒綁定時綁到 `KeyE`,但 `KeyE` 已經被別的動作用掉時拒絕(`missing_binding`),請先自己綁一個鍵。

Gizmo(場景工具列 Gizmos 開關):樞紐點周圍的 range 圓(黃色;停用暗黃;選取白)、有 `exit_margin` 時外面再一圈暗色的「放開」圓,圓邊標 `USE`。Hierarchy 標籤 `USE`。view_angle 屬於互動者的朝向,不畫在目標上。

## Axiom Town 的取代

原本:露米 / 琳的 FSM 在 `near 2.4 / 2.6` 時進入 greet / serve 狀態並發出 `near_*`,規則把變數 `nearby` 設成角色名;`far 3.2 / 3.4` 時進入 bye 並發出 `left_*`,規則把 `nearby` 設回 `none` 並回到 hub;E 鍵經由 `behavior.program.inputs` 變成 `interact` 事件,兩條規則依 `nearby` 跳到對話節點。

現在:露米 `{"range":2.4,"exit_margin":0.8,"prompt":"talk_lumi","event":"talk_lumi"}`、琳 `{"range":2.6,"exit_margin":0.8,"prompt":"talk_rin","event":"talk_rin"}`,規則 `talk-lumi` / `talk-rin` 直接跳到節點;`nearby` 變數、`interact` 事件與輸入對應、`lumi-near` / `rin-near` 規則都移除。FSM 仍負責打招呼、轉身面向玩家、動畫,`left_*` 仍讓對話回到 hub。畫面上方多了提示(`[E] 和露米聊聊`)。

- 測試 `axiom_town_interactables_match_the_fsm_nearby_workaround`:同一組輸入讓主角走向露米、離開、走向琳、離開,整段路(1,920 tick)**每隔一個 tick 按一次 E**(960 次按鍵),比較新舊兩版:每個 tick 的主角 / 露米 / 琳位置相同、每一次進入對話節點的 tick 與節點相同(露米 228 次、琳 152 次,含按鍵落在範圍邊界前後的 tick)、最後變數相同。把系統 phase 改成 26(移動之後)時,這個測試會因為差一個 tick 而失敗,證明它對時機敏感。
- **差異(刻意)**:舊版琳離開後進入 bye 狀態 170 tick(鞠躬動畫),這段時間內玩家走回 2.6 m 內也不會進入 serve,所以按 E 沒有反應;新版只看距離與遲滯,走回範圍內就能再交談。露米的 bye 只有 1 tick、接著 idle 1 tick,玩家不可能在 2 tick 內從 3.2 m 外走進 2.4 m,所以沒有差異。上面的測試路徑在琳的 bye 期間不回頭。
- 朝向提醒:town 主角的 `action.actor.model_yaw` 是 0,動作系統的「前方」是實體的 -Z;如果要對主角使用 `view_angle`,請把 `model_yaw` 設成 π(和 `content/action` 一樣),讓實體 +Z 就是前方。town 目前沒有用 `view_angle`。
