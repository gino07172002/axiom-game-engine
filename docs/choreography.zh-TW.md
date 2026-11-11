# 編排與互動(工作線 W2c)

一個指令同步驅動多個演出者、behavior 規則呼叫 capability、條件讀動畫狀態、Slider widget、滑鼠環繞鏡頭、點擊角色部位。全部是通用機制(不綁特定內容),示範包 `content/duet-lab`(第一方吉祥物 `axiom-short.glb` / `axiom-long.glb`)。

## 指令列表 `batch`(A1)

`RuntimeCommand` 新增 `{"kind":"batch","commands":[…]}`:1..16 個指令(`call` / `emit` / `choose` / `locale`;不能巢狀 `batch`,不能放 `slide`),**同一個 tick 依序執行**。UI 按鈕、loopback、scenario、`runtime.command` 都能送(`runtime.bindings` 仍只接受 `call`)。

```json
{"kind":"button","id":"walk_both","text":"walk","command":{"kind":"batch","commands":[
  {"kind":"call","capability":"animation","action":"play","args":{"entity":"axi","clip":"walk"}},
  {"kind":"call","capability":"animation","action":"play","args":{"entity":"lyn","clip":"walk"}}]}}
```

**先全部驗證再套用**:
1. `call`:動作已註冊、capability 已啟用、參數符合 schema,再加上處理該動作的系統的乾跑(`RuntimeSystem::check_command`,預設沒有)。`animation.*` 實作了乾跑(在玩家的副本上套用,檢查實體、片段、控制器衝突、參數名稱)。
2. `emit` / `choose` / `locale`:依序套用在 gameplay 狀態的**副本**上。
3. 任一項失敗 → 錯誤帶 `/command/commands/I` 路徑,**什麼都沒套用**。

**限制(誠實申報)**:沒有乾跑 hook 的系統只檢查 schema;若驗證通過、套用時才失敗(例如前一個指令改變了狀態),錯誤會說「commands 0..I stay applied」—— 沒有跨系統的交易回滾(native 系統的狀態不可複製)。目前會失敗的情況只有沒有 `check_command` 的系統。

編寫:ACL `ui button WIDGET batch [COMMANDS]`;編輯器 `ui.button WIDGET batch [..]`(同一個 `ops::set_ui_button_command`);UI Editor 的 Command 列顯示「N commands in one tick」,內容在 Fields 編輯。`validate_world` 檢查每個 batch 項目裡的 `call`(`unknown_action` / `missing_capability` / 參數,路徑到 `/commands/I`)。

## 編排資源 `animation.choreography`(A1)

```json
"animation.choreography":{"stages":{
  "greet":{"actors":[{"entity":"axi","clip":"wave","looping":false,"fade_seconds":0.15},
                     {"entity":"lyn","clip":"bow","looping":false}],
           "after":{"entity":"axi","cycles":1,"next":"rest"}},
  "rest":{"actors":[{"entity":"axi","clip":"idle"},{"entity":"lyn","clip":"idle"}]}}}
```

| 欄位 | 說明 |
|---|---|
| `actors` | 1..16 位演出者,同一實體只能出現一次。每位:`clip`(從 0 播放,`fade_seconds` 0..10、`looping` 預設 true、`speed` -8..8 預設 1);沒有 `clip` 時 `speed` = 改播放速率不重置時間(`ramp_seconds` 0..60 線性漸變,同 `animation.speed`);`params` = 控制器參數(數字或 bool,同 `animation.param`)。至少要有其中一項 |
| `after` | `entity` 完成 `cycles`(1..1000,預設 1)個片段循環後接著開始 `next`(單次片段 = 播到結尾)。循環數用與 wave-1 片段結束事件(`on_end`)相同的 `crossings` 計算;控制器實體以目前狀態的主片段計(轉移當步也算離開狀態的最後一步) |

執行期動作 `animation.stage {stage}`:**同一個固定步**套用所有演出者 —— 先在每個玩家的副本上套用並求值,全部成功才一起寫回(真正原子,任一失敗都不改變);成功後成為目前階段(循環數歸零)。`after` 達成時,在**同一步**開始下一個階段;下一階段被拒(不可能在建置驗證後發生,除非執行期改了速度等)時記在 `observe animation.stage.fault`,停止串接。建置時檢查:演出者有 `animation.player`、片段存在、有 `clip` 的演出者不能由控制器驅動、`params` 需要控制器且參數存在、`after.entity` 有玩家、`after.next` 存在(錯誤碼 `invalid_choreography`,路徑到 `/resources/animation.choreography/stages/ID`)。

觀測:`animation.stage = {active, cycles, runs, fault}`(只有有這個資源的世界才出現,舊世界的 `state_hash` 不變)。Checkpoint:存 `active / cycles / runs / fault`(`SavedAnimation.stage`,舊存檔沒有這欄;有編排的世界讀到沒有這欄的存檔會明確拒絕)。熱重載:目前階段名稱仍存在就延續(循環數保留)。

編寫:ACL `stage ID set {JSON} | stage ID remove`(刪除被其他階段 `after.next` 指到的階段 → `in_use`);編輯器 `anim.stage ID set {..} | ID remove`(World 選單;同一個 `ops::edit_stage`);資源不存在時建立。

## 規則呼叫 capability:`GameAction::Call`(A6)

```json
{"kind":"call","capability":"animation","action":"stage","args":{"stage":"greet"}}
```

在 `behavior.program` 規則、敘事 `on_enter`、選項 `actions` 裡都可用。執行:動作在規則跑完時排入 gameplay 的 outbox(每次分派前最多 32 個,超過 → 該 gameplay 交易失敗並回報),由 `WorldRuntime` 在**同一步**(`step` 結束、`command` 之後)走和 UI `call` 相同的路徑分派(`check_call_schema` + 系統 `command`);一個 call 觸發的事件又排入的 call 在下一輪處理,最多 4 輪,剩下的明確丟棄並記錄。

錯誤**可恢復**:失敗的 call 不影響模擬,`observe` 的 `calls = {run, failed, fault}`(只有程式含 `call` 動作時出現)。這些計數存在 gameplay 進度(`Progress.calls`,舊存檔沒有這欄 = 0),所以 checkpoint 讀回後 `state_hash` 一致。`validate_world` 檢查每個 `call` 動作(未註冊動作 `unknown_action`、未啟用 capability `missing_capability`、參數 schema,路徑到 `/resources/behavior.program/rules/@ID/actions/I`)。

編輯器:Graphs ▸ Rules 的動作種類多了 `call`(capability / action / args 表單),寫入走既有的 `set rules[id=ID] actions VALUE`。

## 條件讀動畫狀態:`Operand::Animation`(A7)

```json
{"kind":"not","condition":{"kind":"eq","left":{"kind":"animation","entity":"axi","field":"clip"},"right":{"kind":"literal","value":"wave"}}}
```

| `field` | 型別 | 值 |
|---|---|---|
| `clip` | text | 播放中的片段(控制器:目前狀態的主片段),沒有 = `""` |
| `time` | number | 秒(控制器:正規化時間 × 主片段長度) |
| `normalized` | number | 0..1(控制器:目前狀態的正規化時間) |
| `speed` | number | 播放速率 |
| `playing` | bool | 播放中 |
| `finished` | bool | 單次片段(或控制器單次狀態)已播到結尾(= FSM `clip_finished`) |
| `state` | text | 控制器目前狀態(需要 `animation.controller`) |
| `param` | number | 控制器參數 `param`(bool / trigger 讀成 0 / 1) |

**不是每幀解析 JSON**:編譯時把不同的讀取去重成 `CompiledGameplay::animation_reads`(上限 64),條件裡是 `Read::Animation(i)`;`animation.evaluate` 在建置時把每個讀取解析成(玩家、欄位、參數索引),每步結束(與每個 `animation.*` 指令後)寫進 gameplay 的型別化槽位 `Gameplay::animation`。只有文字值改變時才配置。按鈕 / 文字 / 效果的 `when` 與規則條件都能用,型別照常檢查(`ge` 只接受數字)。需要 `animation.evaluate` 系統;實體沒有玩家、`state` / `param` 沒有控制器、參數不存在 → 建置錯誤。

觀測:`gameplay.animation_reads`(`"axi.clip":"wave"`,只有有讀取時出現)。延遲:規則在本步動畫求值之前跑時讀到的是上一步結尾的值(固定、確定)。

編輯器:Rules / 條件的運算元種類多了 `animation`(entity 選擇器只列有 `animation.player` 的物件、field 下拉、param)。

## Slider widget(C1)

```json
{"kind":"slider","id":"speed","text":"speed","min":0.25,"max":2.0,"step":0.25,"variable":"rate",
 "command":{"kind":"call","capability":"animation","action":"speed","args":{"entity":"axi"}},"arg":"speed"}
```

- `text` 是本地化鍵(同按鈕);`variable`(number / int)和 / 或 `command`(必須是 `call`)+ `arg`(值寫入 `args[arg]`);`value` 是沒有變數時的起始值;`step` 選填;`when` 同其他 widget(隱藏時不能用)。
- 執行期指令 `{"kind":"slide","widget":ID,"value":X}`:夾在 min..max、對齊 step(整數變數取整),**先送 call 再寫變數**,任一被拒都不改變。UI、loopback、scenario 都送同一個指令,所以會被錄進 scenario。
- **節流**:拖曳中只改本地草稿(`UiState.slider_drafts`),放開才送一個 `slide`;鍵盤(方向鍵)/ 點擊軌道每次改變送一個。沒有變數的 slider 顯示最後送出的值(`UiState.slider_values`,只是畫面)。
- `observe gameplay.bindings.ID` = 變數值。效果系統:`on: change` 會跟著變數。
- 編寫:ACL `ui slider add ID TEXT_KEY MIN MAX [variable=V] [step=S] [value=X] [call=CAP.ACTION] [args={..}] [arg=NAME] [when={..}]`(`ops::add_ui_slider`,缺本地化鍵 → `invalid_value`);編輯器 `ui.slider add …`;UI Editor 畫布照常畫出、可拖到區域、Fields 表單編輯所有欄位。

## 環繞鏡頭(C2)

`camera.follow` 新增:
- `input.drag`(預設 false):**主按鈕在遊戲畫面上拖曳**環繞(`drag_yaw` 每拖過整個畫面寬的度數,預設 360,向右拖 yaw 減少;`drag_pitch` 每拖過整個高度,預設 180,向下拖抬高,夾在 min / max_pitch)。走通用螢幕指標事件(`RuntimeSystem::screen_pointer`),每個 move 以拖曳起點為準(絕對),所以符合 scenario 錄製「兩步之間連續 move 只留最後一個」的規則;錄製與重播一致。點在 UI 上不算。
- **滾輪縮放走 input map**:視窗把滾輪刻度變成偽按鍵 `WheelUp` / `WheelDown`,**每個固定步消耗一格**(按住一步),所以 `input.bindings` 的軸 `{"positive":"WheelDown","negative":"WheelUp"}` 接到 `zoom_axis` 即可;每格 = `zoom_speed / 120` 公尺。錄製檔照常記下 ActionMap。
- `target_bone`:注視點跟隨目標骨架某根骨頭的動畫世界位置(再加 `look_height`)。需要目標有 `animation.player`(編寫錯誤 `invalid_camera`);骨頭不存在 → 建置錯誤。透過既有的 socket 機制(`animation_cues.socket_requests`)取得,鍵存在 rig 裡,每步不配置。`camera.follow.set` 新增 `bone`(骨頭名稱或 null);換目標而沒給 `bone` 時回到樞紐點。
- `camera.inspect` 多了 `bone`、`dragging`。
- 編寫:ACL `camera SEL follow TARGET … [drag=on|off] [bone=BONE]`;編輯器 Object ▸ Camera Follow Selected 對話框多了「Mouse drag orbits」「Look at bone」(同一個 `ops::add_camera_follow`);Inspector 表單直接改 `input/drag` 等欄位。

## 點擊角色部位(C3)

元件 `animation.parts`(在有 `animation.player` 的實體上):

```json
"animation.parts":{"auto_radius":0.07,"parts":[{"id":"head","bone":"head","length":0.22,"radius":0.13}],"event":"part_clicked"}
```

- `parts`:`{id, bone, radius 0.005..2 m, to?: 骨頭(膠囊到那個關節) | length?: 沿骨頭 +Y 公尺(兩者皆無 = 球)}`,上限 128。
- `auto_radius`:另外為每根有父骨的骨頭產生一個「父關節 → 該關節」的膠囊,部位 ID = 父骨名稱。
- `event`:點擊時送出的 `behavior.program` 事件,只能有文字欄位 `entity` / `part`(宣告了就填入)。
- 執行期:**主按鈕按下與放開距離 ≤ 畫面 1%** 才算點擊(拖曳環繞不算),用通用指標射線(`WorldRuntime::screen_pointer` 從作用中相機算出)對目前姿勢的所有膠囊做射線-膠囊測試,最近者勝(同距離依實體 / 部位順序,確定)。姿勢只在點擊時讀(每步零成本);每實體 ≤ 128、每世界 ≤ 512 個膠囊。
- 查詢動作 `animation.pick {origin, direction}`:同一個測試,回 `{entity, part, distance}` 或 null,不改變任何東西。
- 觀測:`animation.pick = {targets, capsules, hits, last, fault}`(只有有部位的世界)。Checkpoint 存 `hits / last / fault`。
- 編寫:ACL `parts SEL auto RADIUS [event=E] | add ID BONE radius=R [to=BONE] [length=M] | remove ID | event E|none | clear`;編輯器 `object.parts …`(Object 選單,作用在選取;同一個 `ops::edit_parts`)。

## 示範包 `content/duet-lab`

兩個吉祥物(Axi、Lyn)。UI:Walk together(batch,兩人同一 tick 起步)、Dance together(`animation.stage together`)、Greet(`stage greet`;`when` 讀 `axi.clip`,揮手時自己隱藏;揮完由 `after` 自動接 `rest`)、Fill gauge(+25,滿 100 時規則 `call animation.stage greet` 並歸零)、Axi speed slider(變數 `rate` + `animation.speed`)、右上角顯示點到的實體與部位。鏡頭跟隨 Axi 的 `chest` 骨,滑鼠拖曳環繞、滾輪縮放。`examples/duet-lab.scenario.json` 無頭重播並斷言以上每一項。
