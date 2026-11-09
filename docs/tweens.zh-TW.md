# 屬性補間 / 動畫軌(`tween.clips`)

`tween.clips` 讓物件依關鍵影格改變自己(或子孫)的型別化屬性:門打開又關上、紅綠燈換色、面板淡入用的數值。一個元件可以有多個具名片段(clip,例如 `open`、`close`),每個片段有多條軌(track),每條軌是一串 `{t, v, ease}` 關鍵影格。播放由執行期動作 `tween.play` / `stop` / `seek` 控制,所以 `runtime.bindings` 與 `behavior.program` 規則(經由事件)都能播放,不需要改 Rust。`once` 片段播完可以發出 `on_finish` 事件。

執行期系統是 `tween.animate`(capability `tween`,phase 14)。

## 為什麼不是把骨架動畫泛化

既有的關鍵影格只有骨架動畫(`axiom-animation` 的 `clip::Track`):值固定是 `Vec4`、目標是骨頭、內插只有 step / linear / glTF cubic spline,而且每次取樣會配置一整個 `Pose`。它服務 glTF/PMX 匯入的資料格式,改它會牽動匯入與重定向。補間需要的是另一組東西:逐段 easing、1 或 3 個數值、寫到任意實體的型別化欄位、每步零配置。共用的部分(關鍵影格搜尋 + 內插)寫成一個小函式 `axiom_core::tween::sample`,VFX 與 effects-tour 的站點動畫是程序化公式,沒有可泛化的關鍵影格資料。

## 元件

```json
"tween.clips":{"clips":{
  "swing":{"on_finish":"door_moved","tracks":[
    {"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90,"ease":"ease_out"}]}]},
  "blink":{"mode":"loop","autoplay":true,"duration":2,"tracks":[
    {"target":"signal/lamp","property":"render.light/color","keys":[{"t":0,"v":[1,0,0]},{"t":1,"v":[0,1,0],"ease":"step"}]}]}
}}
```

| 欄位 | 說明 |
|---|---|
| `clips` | 名稱 → 片段,每個元件 1..16 個。同一時間只播一個片段 |
| `enabled` | 預設 true。停用的元件不前進,動作會被拒絕 |
| 片段 `tracks` | 1..16 條軌 |
| 片段 `duration` | 秒 0.01..3600;省略 = 所有軌最後一個 key 的時間。key 不能超過 duration |
| 片段 `mode` | `once`(預設,播到尾端或倒播到 0 就停,發 `on_finish`)/ `loop` / `ping_pong` |
| 片段 `speed` | 0.01..100,預設 1(播放速率) |
| 片段 `autoplay` | 預設 false。執行期開始時從 0 正向播放(第一步才寫入值,所以編輯模式看到的是編寫值);每個元件最多一個 |
| 片段 `on_finish` | `once` 片段到達終點時發出的 `behavior.program` 事件(`""` = 不發)。payload 只能宣告文字欄位 `entity`(元件所在實體)和 / 或 `clip` |
| 軌 `target` | 實體 ID:自己或子孫(`""` = 自己)。變數軌沒有 target |
| 軌 `property` | 見下表 |
| 軌 `keys` | 1..256 個 `{t, v, ease?}`,`t` 秒、嚴格遞增;`v` 是數字或 `[x,y,z]` |

### 可動畫的屬性(型別化,建立執行期時解析成欄位)

| `property` | 值 | 寫入 |
|---|---|---|
| `core.transform/position` | `[x,y,z]` 本地座標 | 目標的 `core.transform` |
| `core.transform/rotation` | `[x,y,z]` 歐拉角(度,XYZ 順序,和編輯器 Inspector 一樣) | 整個本地旋轉 |
| `core.transform/yaw` | 度,繞 +Y | 整個本地旋轉(門用這個) |
| `core.transform/scale` | `[x,y,z]` 0.0001..10000 | 本地縮放 |
| `render.light/color` | rgb 0..1 | 目標 `render.light` 的顏色 |
| `render.light/intensity` | 0..100 | 目標燈光強度 |
| `render.mesh/color` | rgb 0..1 | 目標 `render.mesh` **每個** part 的顏色(只想動一個部件就把它放在子實體) |
| `render.mesh/emission` | 0..16 | 每個 part 的自發光 |
| `behavior.program/variables/NAME` | 數字 | `behavior.program` 的 number 變數(int 變數會四捨五入) |

值都是**絕對值**,逐個數字內插;第一個 key 之前停在第一個值、最後一個之後停在最後一個值。`rotation` / `yaw` 的角度是數字內插,所以 0 → 350 會轉 350 度(要短路徑就寫 0 → -10)。未知屬性、維度不對、超出範圍的值都是編寫錯誤。

**材質(`render.material` / `render.shaders` 的參數)不支援**:那是全世界共用的資源,不是逐實體的屬性;要逐物件變色請用 `render.mesh/color` / `emission`。

**UI**:`ui.document` 沒有不透明度或位移屬性(那是規劃中的 M7 UI 動效)。現在能做的是動一個 number 變數:`progress` widget、文字的 `variable` 綁定、widget 的 `when` 條件都會即時反映它(例如變數從 0 補到 1 讓進度條滑入、到 0.5 以上才顯示面板)。

### Easing

`ease` 寫在 key 上,決定「抵達這個 key 的那一段」的曲線(第一個 key 的 ease 沒有作用):

| ease | 權重 w(u),u = 段內 0..1 |
|---|---|
| `linear`(預設) | u |
| `step` | 0,直到抵達 key 的那一刻才變成 1 |
| `ease_in` | u² |
| `ease_out` | u(2−u) |
| `ease_in_out` | u < 0.5 ? 2u² : 1 − (2−2u)²/2 |

所有曲線都單調、介於 0..1,所以取樣值永遠在兩個 key 之間:key 合法,寫出去的值就合法。

## 編寫錯誤(`World::compile`,可恢復的請求錯誤)

錯誤碼:`invalid_tween`(欄位、目標、系統、事件)、`tween_conflict`、`tween_budget`;元件本身的格式錯誤是 `invalid_component`。

- 要排入 `tween.animate`;`on_finish` 與變數軌還要 `behavior.execute`;事件要在 `behavior.program` 宣告,payload 規則如上;變數要宣告成數字。
- `target` 必須存在,而且是元件所在實體或它的子孫;目標要有被動到的元件(`core.transform` / `render.light` / `render.mesh`)。
- **移動者衝突**:transform 軌的目標若同時被另一個系統移動(`Registry::movers`:`core.motion`、`nav.agent`、`fsm.agent`、`rts.unit`、`action.actor`、`fps.*`,以及 `path.follow`)→ `tween_conflict`。只動燈光或網格顏色則沒問題。要讓移動中的物件擺動,把補間放在它的子實體上。
- **靜態障礙**:transform 軌的目標或它的祖先如果被補間,而子樹裡有 `nav.blocker` 或 `physics.collider` → `tween_conflict`(見下一節)。
- 同一個屬性只能由一個元件動(兩個元件寫同一個實體的 rotation、同一個變數 → `tween_conflict`);同一個片段裡也不能有兩條軌寫同一個東西(`rotation` 和 `yaw` 算同一個)。不同片段可以動同一個屬性(一次只播一個)。

## 門與導航(nav.blocker)

**移動中的 blocker 不支援,也沒有開關。** 導航格子(`nav.grid`)在建立執行期時從靜態障礙算一次,依內容雜湊快取在程序內;`nav.blocker` 也在建立時被 FSM 代理與動作控制器各自複製一份。執行期改 blocker 需要三個系統同時增量更新(格子的 clearance、代理的障礙桶、動作控制器的碰撞矩形),這一版沒有做,所以直接把「被補間的子樹裡有靜態障礙」定為編寫錯誤,不讓畫面和碰撞默默不一致。

街道 demo 的做法:
1. 門扇(有 mesh、被 `yaw` 軌轉動)**不要**加 `nav.blocker`。門框兩側的牆各自是 blocker,門洞留空,導航代理永遠可以通過門洞。
2. 「門關著不能過」用遊戲邏輯表達:門口放一個 `trigger.zone`,規則依 `open` 變數決定要不要把玩家擋回來或顯示提示;或者讓 NPC 的 `nav.go` 只在 `door_moved` 事件後才下達。
3. 需要真的會阻擋的門,等導航的動態障礙(增量更新格子)完成;那是另一個里程碑。

## 播放與動作(不需要 GPU)

| 動作 | 參數 | 結果 |
|---|---|---|
| `tween.play` | `id`、`clip`、`reverse`(選填)、`restart`(選填) | 換片段或 `restart`:跳到起點(倒播則是終點)的姿態並播放;同一個片段:從目前時間往指定方向繼續(門開到一半按關,會從那裡轉回去,不會跳)。`once` 片段已經在那一端時保持停止 |
| `tween.stop` | `id` | 停在目前時間,值保留 |
| `tween.seek` | `id`、`time`(0..片段長度)、`clip`(選填,預設目前片段) | 立刻跳到該時間並寫入值;播放狀態不變 |
| `tween.inspect` | `id` | `clip`、`time`、`length`、`state`(`idle` / `playing` / `reversing` / `stopped`)、`enabled`、每個片段的長度、模式、速度、autoplay、on_finish、軌 `[target, property, key 數]` |
| `tween.summary` | — | 同 `observe` 的 `tween` |

- 每個固定步(120 Hz)播放中的元件時間前進 `speed × dt`:`once` 夾在 0..長度並停止、`loop` 取餘數、`ping_pong` 在兩端反彈並換方向;然後依目前時間取樣該片段每一條軌並寫入。場景未啟用或 `enabled:false` 的元件不前進。
- 時機:phase 14,在 `fsm.step`(8)、`motion`(10)、`path.move`(12)、`nav.steer`(13)之後,`rts`(15)、動作 / FPS(20)、`trigger.detect`(25)之前;所以觸發區看得到這一步被補間的姿態。
- `on_finish`:到終點的那一步排進佇列;每步最多發 64 個,多的下一步再發(不會停系統)。事件走和 `path.follow` 的 `on_arrive` 一樣的 gameplay 指令路徑,規則同步執行,`runtime.bindings` 在步尾執行。發送失敗不會停止動畫,第一個錯誤記在 `fault`。
- 只要「事件發生就播放」:ACL `play_on=E`(`tween door clip open … play_on=use_door`)會在 `runtime.bindings` 的事件 E 加一個 `tween.play {id, clip}` 呼叫(資源不存在就建立、事件沒宣告就以無欄位宣告);移除片段時一併移除這些呼叫。這是 op 層的便利寫法,元件本身沒有這個欄位。
- 從互動開門再關門:`interact.target {event:"use_door"}` → 規則依 `open` 變數發 `open_door` / `close_door` → `runtime.bindings` 呼叫 `tween.play {id:"door",clip:"swing"}` / `{…,"reverse":true}`。注意同一個事件的兩條規則是依序執行的,所以「切換」要用「發另一個事件、在那個事件的規則裡才改變數」的寫法(見 `crates/axiom-runtime/tests/tweens.rs` 的 `world()`)。

`observe` 的 metrics 有 `tween`:

```json
"tween":{"components":2,"playing":1,"at":{"door":["swing",0.4583,"playing"],"signal":["blink",1.2,"playing"]},
  "more":0,"emitted":3,"queued":0,"halted":null,"fault":null}
```

`at` 只列有目前片段的元件,最多 32 個(`more` 是其餘數量)。情境(scenario)可以用 `{"kind":"field","entity":"door","field":"/rotation/1","op":"ge","value":0.707}` 斷言旋轉(field 紀錄現在也有 `rotation` 與 `scale`),或 `{"kind":"call","action":"tween.inspect","args":{"id":"door"},"pointer":"/state","op":"eq","value":"stopped"}`。

## 熱重載、執行期 spawn、存檔

- 熱重載(新 revision):依元件所在實體 ID 保留播放(目前片段、時間、方向、是否播放),條件是那個片段**定義完全相同**(軌、key、選項);保留後立刻用新執行期重新寫入值(新執行期的燈光 / 網格從編寫值開始)。片段改了就回到 idle:燈光、網格回到編寫值,transform 則依引擎既有規則(編寫值沒變的 transform 保留執行期姿態)停在原地,直到再次播放。
- 執行期 spawn 出來的實體若帶有 `tween.clips`,系統停止(`tween.halted` 寫明原因),之後的動作會被拒絕;spawn 一般實體沒有影響。
- **存檔(checkpoint)不支援**:播放狀態是模擬狀態,存檔格式沒有欄位,`checkpoint` 會明確失敗(`system tween.animate does not implement checkpoints`)。

## 預算(超過就明確失敗)

| 預算 | 值 | 超過時 |
|---|---|---|
| 每個世界的 `tween.clips` 元件 | 2,048 | 交易失敗(`tween_budget`) |
| 每個元件的片段 / 每個片段的軌 / 每條軌的 key | 16 / 16 / 256 | 交易失敗(`invalid_component`) |
| 每個世界的軌 | 16,384 | `tween_budget` |
| 每個世界的 key | 131,072 | `tween_budget`(剛好 131,072 可以,測試釘住邊界) |
| 每步發出的 `on_finish` | 64 | 其餘排隊到下一步 |

## 資料佈局與量測

建立執行期時,每條軌解析成一個型別化寫入(實體 slot + 欄位,或變數索引),key 複製進全世界共用的三個平坦陣列(時間、值(每個 key 1 或 3 個 float)、ease)。取樣用每條軌一個「上一段」提示:正常播放 O(1) 找到段落,只有跳躍(seek、loop 回繞)才用二分搜尋。每步不配置記憶體;`observe` / `inspect` 組 JSON 時才配置。

量測(**release**,`perf_bench --generated-tween NxT`:N 個方塊各一個 autoplay 片段、T 條軌(yaw、position、scale、mesh 顏色,依序取前 T 條),每軌 8 個 key 分布在 2 秒、easing 混用,loop / ping_pong 交替、速度 0.5..2,所以每一步每個元件都取樣;每個設定一個新程序,1,200 步,3 回合交錯):

| 設定 | 步長 p50 | p95 | 每步配置 p50 / max | 系統建立 |
|---|---|---|---|---|
| 1024 × 4 軌 | 0.130–0.134 ms | 0.148–0.174 ms | 0 / 1 | 4.3–4.7 ms |
| 1024 × 1 軌 | 0.084–0.087 ms | 0.096–0.105 ms | 0 / 1 | 1.2 ms |
| 256 × 4 軌 | 0.032–0.033 ms | 0.035–0.038 ms | 0 / 1 | 1.2–1.4 ms |

步長含 `WorldRuntime::step` 本身的固定成本。1024 × 4(4,096 軌)約 0.13 ms / 步,120 Hz 下約一個核心的 1.6%;每個元件約 80 ns(前進時間、寫 transform),每多一條軌約 15 ns。每步配置的 max 1 是 1,200 步中偶發的一次,其他基準(互動、路徑)也有,不是每步發生。

axiom-town(沒有補間,只看有沒有拖慢既有路徑;release,`perf_bench --pack content/axiom-town`,改動前 = `32a5d77` 的 worktree 建置,5 回合交錯 ABAB…,中位數):步長 p50 0.0180 → 0.0177 ms、p95 0.0188 → 0.0213 ms(單次範圍 0.0184–0.0223 vs 0.0185–0.0231,重疊,屬噪音)、每步配置 p50 / max 69 / 82 → 69 / 82、`observation()` 0.0349 → 0.0346 ms(配置 350 → 350)、render_frame 1.33 → 1.31 ms、編輯→生效 p50 152.5 → 150.2 ms。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object ▸ Add Tween Clip(`object.tween_clip`,作用在選取):新片段 `clip`、`clip2`…,第一條軌是 `core.transform/position`,0 秒與 1 秒各一個 key,都記錄目前的值(所以片段一開始不改變任何東西) | `tween SEL clip NAME [property=P] [target=ID] [duration=S\|auto] [mode=..] [speed=X] [autoplay=on\|off] [on_finish=E] [play_on=E]` |
| Inspector ▸ Tween timeline ▸ Record key(`tween.record_key`):把選取軌的**編寫值**存成播放頭時間的 key(同一時間已有 key 就改它的值) | `tween ID key CLIP TRACK add T`(不寫值 = 記錄目前值;也可寫 `90` / `x,y,z` 與 `ease=E`) |
| 在時間軸上拖曳 key(放開時寫入,吸附 0.01 秒) | `tween ID key CLIP TRACK move I T` |
| Delete key(`tween.remove_key`,作用在選取的 key) | `tween ID key CLIP TRACK remove I` |
| 模式下拉、Autoplay 勾選 | `tween ID clip NAME set mode=..\|autoplay=..`(還有 `duration=`、`speed=`、`on_finish=`;改成 loop / ping_pong 時編輯器會一併清掉 on_finish) |
| Add track ▸ 屬性 | `tween ID track CLIP add P [target=ID]` |
| Remove clip(最後一個片段會移除整個元件) | `tween ID clip NAME remove` |
| 元件表單 / JSON | `set ID tween.clips/clips/NAME/... V` |

兩邊共用 `axiom_core::ops` 的 `add_tween_clip`、`set_tween_clip`、`remove_tween_clip`、`add_tween_track`、`remove_tween_track`、`edit_tween_key`;一個 revision 內補 `tween` capability 與 `tween.animate` 系統(`on_finish` / 變數軌另補 `behavior.execute`,並在 `behavior.program` 宣告事件,文字欄位 `entity` / `clip`)。從 prefab 繼承的 `tween.clips` 不能用這些操作改(覆寫無法刪掉繼承的片段或 key),會回 `inherited`,請改 prefab。

時間軸(Inspector ▸ Tween timeline,有 `tween.clips` 的物件放在 Transform 之前,因為筆電螢幕上 Inspector 很短):每個片段一條,每條軌一列,key 是菱形標記,紅線是播放頭。點片段名稱切換;點空白處或拖曳 = 移動播放頭並在場景視口**預覽**該時間的姿態 / 顏色(只影響這一幀的繪製,不寫 revision、不改模擬,按 Stop preview 結束);點 key 選取、拖曳 key 改時間。Play 預覽時播放頭跟著執行期走,編輯按鈕停用。Hierarchy 標籤 `TWEEN`。

對等測試:`tween_clips_keys_and_options_match_acl`(兩個物件同時加片段、第二個片段的名稱、空世界補 capability / 系統、移動物件後 Record key、拖曳 key、Delete key、模式、Autoplay、Add track、Remove clip、無法使用的情況)、`tween_scrub_previews_without_writing_or_moving_the_simulation`、`timeline::tests::click_scrubs_and_dragging_a_key_requests_a_move`(egui 事件實際點擊與拖曳);`crates/axiom-core/tests/ops.rs` 的 `tween_verbs_match_the_ops_and_record_keys`(每個動詞 = op,19 種結構化錯誤)。
