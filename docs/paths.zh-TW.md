# 路徑與路徑跟隨(`path.spline` / `path.follow`)

`path.spline` 是掛在實體上的一串點(實體的本地座標),`path.follow` 讓另一個實體沿著某條路徑以固定速度移動,可循環、來回或停在終點,並可朝向切線。用途:街道上的車輛、人行道上的行人、巡邏路線、遊覽車。
執行期系統是 `path.move`(capability `path`)。不需要改 Rust:內容、ACL、編輯器都能建立與修改。

## 路徑元件 `path.spline`

```json
{"points":[[-4,0,-3],[4,0,-3],[4,0,3],[-4,0,3]],"closed":true,"interpolation":"linear"}
```

| 欄位 | 說明 |
|---|---|
| `points` | 2..256 個 `[x,y,z]`,**路徑實體的本地座標**,依行進順序。相鄰兩點至少相距 0.01 m |
| `closed` | 預設 false。true = 最後一點接回第一點(不要重複第一點;需要 3 點以上) |
| `interpolation` | `linear`(預設,直線段)或 `catmull_rom`(centripetal Catmull-Rom,通過每個點,每段取樣 8 小段) |

- 路徑跟著實體的世界位置與旋轉(含 parent)。**不吃縮放**(和 `trigger.zone` 一樣),所以沿路徑的距離一律是公尺。
- 路徑可以不被任何人跟隨(只是一條資料);路徑本身也可以被其他系統移動,跟隨者下一步就跟著走。

## 跟隨元件 `path.follow`

```json
{"path":"road/main","speed":8.0,"start_fraction":0.25,"end":"loop","orient":true,"turn_rate":90,
 "yaw_offset":0,"offset":1.5,"on_arrive":"","paused":false,"enabled":true}
```

| 欄位 | 說明 |
|---|---|
| `path` | 路徑實體的 ID(必須有 `path.spline`;不能是自己,也不能是自己的子孫) |
| `speed` | 公尺 / 秒,0..100,預設 1.5 |
| `start` | 起點距離(公尺,預設 0)。超過路徑長度時:loop 取餘數、ping_pong 折返、stop 夾在終點 |
| `start_fraction` | 0..1,以路徑長度的比例指定起點(和 `start` 擇一) |
| `end` | `loop`(預設;開放路徑到終點跳回起點,封閉路徑一直繞)/ `ping_pong`(兩端折返)/ `stop`(停在終點,發 `on_arrive`) |
| `orient` | 預設 true:只轉 yaw,面向前進方向。**模型的正面是 +Z**(和 FSM、RTS 一樣) |
| `turn_rate` | 度 / 秒,0..3600;0 = 立刻面向切線,其他值 = 平滑轉向(停在終點後仍會轉完) |
| `yaw_offset` | 度,-360..360,加在朝向上(正面不是 +Z 的模型) |
| `offset` | 公尺,-50..50:**在路徑前進方向的右側**(負值 = 左側)平移,平面是路徑實體的本地 XZ。讓多個行人共用一條人行道路徑而不重疊。是路徑幾何的側邊,ping_pong 回程時仍在同一側 |
| `on_arrive` | `behavior.program` 事件 ID(`""` = 不發)。只有 `end: stop` 能用(其他模式永遠不會「到達」,寫了會被拒絕)。到達終點那一步發一次;`path.restart` 後再到達會再發 |
| `paused` | 預設 false。true = 一開始就暫停(放在起點),用 `path.resume` 繼續 |
| `enabled` | 預設 true。false = 完全不動、不寫 transform(也不能被暫停 / 恢復) |

事件 payload 只能宣告文字欄位 `follower`(跟隨者 ID)和 / 或 `path`(路徑 ID),或不宣告欄位:

```json
"events":{"bus_arrived":{"fields":{"follower":"text","path":"text"}}}
```

驗證是嚴格的:未知欄位、超出範圍的值、`start` 與 `start_fraction` 同時給、非 stop 模式寫 `on_arrive`,都會讓交易失敗並回傳原因(可恢復的請求錯誤)。

世界層級的檢查(`World::compile`,錯誤碼):
- `invalid_path`:有 `path.follow` 卻沒排 `path.move` 系統;`path` 指向不存在或沒有 `path.spline` 的實體;路徑是跟隨者自己或其子孫(跟隨者會移動自己的路徑);`on_arrive` 事件沒在 `behavior.program` 宣告、payload 欄位不合規則,或沒排 `behavior.execute`。
- `path_conflict`:**同一個實體只能有一個移動者**。有 `path.follow` 的實體不能同時有 `core.motion`、`fsm.agent`、`rts.unit`、`action.actor`、`fps.controller`、`fps.sentinel`(這些元件的系統也會寫 transform,兩者會互相覆蓋)。錯誤訊息會指出元件與它的系統。要組合時,把跟隨者放在 parent 或 child 實體上(例如車身是跟隨者,子物件做別的事)。清單由各套件在註冊表的 `movers` 登記,新的移動系統也應該登記。
- `path_budget`:見下方預算。

## 執行規則

- 系統 phase 12:在 fsm(8)、motion(10)之後(移動中的路徑 parent 已更新),rts(15)、action / fps(20)、`trigger.detect`(25)之前(觸發區看得到跟隨者的新位置)。
- 每個固定步(120 Hz):跟隨者依「parent 在 child 之前」的順序前進 `speed × 1/120` 公尺,然後寫入 transform(位置;`orient` 時連同 yaw)。跟隨者有 parent 時,寫入的是相對 parent 的本地位置與旋轉。
- 路徑的世界框架每步重算一次(如果路徑的祖先也是跟隨者,會落後一步)。
- 暫停 / 到達終點的跟隨者每步仍會放在原距離上(路徑移動時跟著動),只是不前進。停用的跟隨者、或所在場景未啟用的實體,不動也不寫入。
- 編輯模式(編輯器場景視口)不模擬:跟隨者停在編寫的位置,gizmo 會畫一條暗線指向它在路徑上的起點;Play 時第一步就放到起點。
- **轉角**:直線路徑的轉角兩側各 1 m(最多該段長度的一半)內,方向在前後兩段之間平滑過渡,所以側向 `offset` 與朝向都是連續的(有 offset 的跟隨者會切內角 / 繞外角,不會瞬移)。沒有 offset 的跟隨者位置仍精確走在折線上。
- **Catmull-Rom**:建立執行期時取樣成折線(每段 8 小段)並建好累計距離表(arc-length),之後每步只查表;沿取樣折線的速度是精確的定值。
- 熱重載(新 revision):依跟隨者 ID 保留進度(距離、方向、暫停、已到達、朝向、`path.set_speed` 設的速度),條件是跟隨同一個路徑 ID、該路徑的 `path.spline` 沒變,而且跟隨者的 `start` / `start_fraction` / `end` / `paused` 沒變;其他欄位(例如 `speed`、`offset`)改了照樣保留進度並立刻生效。條件不成立時從起點重新開始。
- **存檔(checkpoint)不支援**:進度是模擬狀態,存檔格式沒有欄位,排了 `path.move` 的世界 `checkpoint` 會明確失敗(`system path.move does not implement checkpoints`),不會默默丟掉。
- 執行期 spawn 出帶有 `path.spline` 或 `path.follow` 的實體時,系統停止並在 `observe` 的 `path.halted` 寫明原因(不支援執行期新增路徑 / 跟隨者)。

## 執行期指令(AI、UI、行為程式共用)

都是註冊過的 runtime action,走 `command` / `axiom call` / `runtime.bindings`:

| 動作 | 參數 | 效果 |
|---|---|---|
| `path.pause` | `{"id":跟隨者}` | 停在目前距離 |
| `path.resume` | `{"id":跟隨者}` | 繼續 |
| `path.set_speed` | `{"id":跟隨者,"speed":0..100}` | 改速度(熱重載時隨進度保留) |
| `path.restart` | `{"id":跟隨者}` | 回到起點、清除已到達(暫停狀態不變) |
| `path.inspect` | `{"id":跟隨者或路徑}` | 跟隨者:distance、length、progress、forward、speed、paused、arrived;路徑:length、closed、points、nodes、followers |
| `path.summary` | `{}` | 同 observe 的 `path` 段 |

錯誤是可恢復的:不存在的 ID 回 `not_found`、停用的跟隨者、速度超出範圍都會說明原因。

行為程式讓車在路口停下:`behavior.program` 規則發出事件(例如號誌變紅時 `red_light`),`runtime.bindings` 把事件綁到指令:

```json
"runtime.bindings":{"events":[
  {"event":"red_light","calls":[{"kind":"call","capability":"path","action":"pause","args":{"id":"car/1"}}]},
  {"event":"green_light","calls":[{"kind":"call","capability":"path","action":"resume","args":{"id":"car/1"}}]}]}
```

觸發區也能接:`trigger.zone` 的 `on_enter` 事件綁到 `path.pause`,車開進路口區域就停。

## 預算(超過就明確失敗)

| 預算 | 值 | 超過時 |
|---|---|---|
| 每個世界的路徑 | 256 | 交易失敗(`path_budget`) |
| 每條路徑的點 | 256 | 交易失敗(元件驗證) |
| 所有路徑的折線節點總數(linear:點數,封閉 +1;catmull_rom:段數 × 8 + 1) | 65,536 | 交易失敗(`path_budget`);執行期表格約 1.8 MiB(每節點 28 bytes) |
| 每個世界的跟隨者 | 2,048 | 交易失敗(`path_budget`) |
| 每步的 `on_arrive` 事件 | 64 | 這一步一個都不發,系統停止,`path.halted` 寫明原因 |

`behavior.program` 處理事件時的失敗不會停止系統;第一個錯誤記在 `path.fault`。

## 複雜度、記憶體與量測

建立執行期時:每條路徑展開成折線(本地點、累計距離、該小段的 XZ 方向、轉角混合寬度,每節點 28 bytes),跟隨者依 transform 順序排好;成本與「節點數 + 跟隨者數」成正比。每一步:
1. 每條路徑算一次世界框架(沿 parent 鏈相乘,不配置記憶體)。
2. 每個跟隨者:前進、從上一步的小段往後找(通常 0–1 步;跳躍時改用二分搜尋)、內插、寫 transform。O(跟隨者數 + 路徑數)。
3. 穩定狀態的步長**不配置記憶體**(事件陣列預先配好 64 格)。

量測(**release**,`perf_bench --generated-paths PxF`:P 條封閉路徑,一半是 20×10 m 直線矩形、一半是 16 點 Catmull-Rom 圓;F 個跟隨者平均分配,速度 1.5–3 m/s,offset ±1 m,一半有 turn_rate;每個設定一個新程序,1200 步,3 回合,取中位數):

| 設定 | 步長 p50 | 步長 p95 | 每步配置(p50) | `path.move` 建立 | 整個執行期建立 |
|---|---|---|---|---|---|
| 64 路徑 × 1024 跟隨者 | **0.201 ms** | 0.252 ms | 0 | 0.61 ms | 8.0 ms |
| 64 × 2048(跟隨者上限) | 0.425 ms | 0.479 ms | 0 | 1.01 ms | 13.3 ms |
| 256 × 2048(路徑上限,一半是 Catmull-Rom,共 17,152 節點) | 0.412 ms | 0.511 ms | 0 | 1.74 ms | 16.4 ms |
| 64 × 0(沒有跟隨者) | 0.0001 ms | 0.0001 ms | 0 | 0.35 ms | 1.7 ms |

步長約每個跟隨者 0.2 µs(前進 + 查表 + atan2 / sin / cos 算 yaw + 寫 transform),和跟隨者數成正比、和路徑節點數無關;1024 個跟隨者在 120 Hz 下約佔一個核心的 2.4%。建立成本和節點數 + 跟隨者數成正比(背景建立執行期時發生,不在主執行緒)。每步配置 max 為 1(沒有跟隨者時也一樣,來自 `WorldRuntime::step` 本身,不是這個系統)。

axiom-town(沒有路徑,檢查不退步;release,`perf_bench --pack content/axiom-town`,改動前的程式 vs 改動後的程式,5 回合交錯 ABAB…,中位數):步長 p50 0.0224 → 0.0215 ms、每步配置 p50 / max 71 / 84 → 71 / 84(不變)、`observation()` 0.0425 → 0.0414 ms(配置 339 → 339)、render_frame 1.643 → 1.633 ms、編輯→生效 p50 18.1 → 17.7 ms、執行期建立 24.2 → 23.6 ms、工作集約 18.4 MiB(不變)。差異都在噪音內。

## 觀測(不需要 GPU)

`observe` 的 metrics 有 `path`:

```json
"path":{"paths":2,"followers":3,"moving":2,"paused":["car/2"],"arrived":["bus"],
  "at":{"bus":[48.0,1.0],"car/1":[12.34,0.412],"car/2":[3.0,0.1]},"more":0,
  "emitted":1,"halted":null,"fault":null}
```

- `at`:前 32 個跟隨者(依執行順序)的 `[距離 m(0.01), 進度 0..1(0.001)]`,其餘以 `more` 計數。
- `paused` / `arrived`:最多各列 16 個 ID。`moving` = 啟用、未暫停、未到達、速度 > 0 的數量。
- 情境斷言:`{"kind":"call","action":"path.inspect","args":{"id":"car/1"},"pointer":"/arrived","op":"eq","value":true}`。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object 選單 / 指令面板 `object.path`、Hierarchy「+ Object」▸ Path | `create path [id=] [at=x,y,z] [parent=] [scene=]`:8×6 m 封閉矩形,世界缺少時補 `path` capability 與 `path.move` 系統 |
| `object.follow_path`(Object ▸ Follow Path):選取的物件跟隨「選取中的那一條路徑」(Ctrl+點一起選),沒選路徑時用世界裡唯一的路徑 | `follow SEL PATH [speed=] [end=loop\|ping_pong\|stop] [offset=]`。選多個時起點平均分散(`start_fraction` = k/n,依選取順序) |
| `path.add_point`(Object ▸ Add Path Point、Inspector ▸ Path points ▸ Add point):加在選取的點之後(拆開到下一點的那段;開放路徑的終點則延伸),沒選點時加在最後 | `path ID add [x,y,z]` / `path ID insert I [x,y,z]`(沒給座標:add = 延伸最後一段 / 封閉路徑拆開收尾段;insert I = 拆開到第 I 點的那段) |
| `path.remove_point`(Object ▸ Remove Path Point、Inspector 的 Remove point):移除選取的點,沒選時移除最後一點 | `path ID remove I` |
| 場景視口拖曳點把手(在通過該點的水平面上移動,吸附開啟時 X / Z 依移動吸附間距取整,Ctrl 反轉) | `path ID move I x,y,z`(本地座標) |
| Inspector 的元件表單(只寫改動的欄位,`points` 每個座標都能直接改) | `set ID path.follow/speed 4` 等 |
| Inspector ▸ Remove component | `remove SEL path.follow` |

所有改點的功能都寫整個 `points` 陣列(一筆 `set_field`),結果必須仍是合法路徑(否則回 `invalid_path`,不寫 revision)。

Gizmo(場景工具列 Gizmos 開關;隱藏的物件不畫,鎖定的路徑不顯示把手):橘色折線(Catmull-Rom 畫取樣後的曲線;超過 512 小段時跳著畫)、每個控制點一個十字、第一段中間一個箭頭表示方向、第一點標 `PATH`;選取的路徑畫成白色,並在控制點畫方形把手(選取中的點較大、黃色)。點把手 = 選這個點(不改物件選取);拖曳把手 = 移動這個點,放開時經 `ops::edit_path_point` 寫入。跟隨者標 `FOLLOW`,並畫一條暗線到它在路徑上的起點。Hierarchy 標籤 `PATH`、`FOLLOW`。

已知缺口:點把手只能在水平面上拖(要改高度用 Inspector 或 ACL);從水平方向看(Front / Right 等正交視圖)拖不動。Play 預覽沒有額外的路徑 overlay(跟隨者本身在畫面上移動)。
