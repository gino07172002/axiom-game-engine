# 跟隨相機與碰撞(`camera.follow`)

`camera.follow` 讓一台 `render.camera` 繞著目標實體(例如玩家)轉,看著目標上方的一點,平滑跟上目標,而且**不穿牆**:目標走進房間時,相機會停在牆前(房間這一側),牆不再擋住時再平滑退回原本的距離。

執行期系統是 `camera.rig`(capability `camera`)。同一個系統也提供執行期的相機動作:`camera.activate`(切換主相機,編輯器計畫 M4 的項目)、`camera.follow.set`、`camera.inspect`、`camera.summary`。

## 元件

```json
"camera.follow":{"target":"hero","distance":6,"pitch":20,"look_height":1.5,
  "input":{"yaw_axis":"orbit","zoom_axis":"zoom"},"collision":{"radius":0.2}}
```

| 欄位 | 說明 |
|---|---|
| `target` | 必填,跟隨的實體 ID(用它的**世界樞紐點**,含 parent)。不能是相機自己或相機的子孫,也不能是另一台跟隨相機(兩台互相跟隨會無限飄走;要跟同一個人就跟它的目標) |
| `distance` | 公尺,預設 5,必須在 `min_distance..max_distance` 內。期望距離(從注視點量起) |
| `min_distance` / `max_distance` | 公尺 0.1..100,預設 1 / 12。相機**永遠不會比 min_distance 更近**,即使牆更近(見「碰撞」);縮放輸入與 `camera.follow.set` 只能在這個範圍內 |
| `yaw` | 度,預設 0。0 = 相機在目標的世界 +Z 側、朝 -Z 看(和 action 控制器的相機同一個慣例);正值從上往下看是逆時針 |
| `pitch` | 度,預設 20,必須在 `min_pitch..max_pitch` 內(-89..89)。正值 = 相機在注視點上方 |
| `min_pitch` / `max_pitch` | 度,預設 -30 / 75 |
| `look_height` | 公尺 -10..10,預設 1.5。注視點(也是環繞中心)= 目標樞紐點往上這麼高 |
| `position_smoothing` | 半衰期秒數 0..5,預設 0.1。注視點追上目標的速度(0 = 剛性)。注視點**不會落後穿過牆**(見下) |
| `rotation_smoothing` | 半衰期秒數 0..5,預設 0.1。yaw、pitch、期望距離逼近目標值(輸入、`camera.follow.set`)的速度 |
| `input` | `{yaw_axis, pitch_axis, zoom_axis}`:`input.bindings` 的**軸名稱**(`""` = 無;不寫死按鍵),`yaw_speed` 度/秒 0..720(預設 120)、`pitch_speed`(預設 60)、`zoom_speed` 公尺/秒 0..100(預設 4)。正向 = yaw 增加 / 抬高 / 拉遠 |
| `collision` | `{enabled 預設 true, radius 公尺 0..2 預設 0.2, recover 半衰期秒數 0..5 預設 0.25}` |
| `enabled` | 預設 true。停用時相機留在原地(不寫 transform) |

需求:同一個實體要有 `render.camera` 與 `core.transform`,而且必須是**根實體**(沒有 parent;相機寫的是世界姿態);世界要排入 `camera.rig`。

## 每一步做什麼

1. 輸入軸移動 yaw / pitch / 距離的目標值(pitch、距離各自夾在範圍內)。
2. 目前的 yaw / pitch / 距離以 `rotation_smoothing` 逼近目標值(yaw 走短的那一邊)。
3. 注視點以 `position_smoothing` 追向「目標樞紐點 + look_height」。為了不讓落後的注視點在轉角**切過牆角**,從真正的樞紐點往落後的注視點做一次 sphere cast,撞到就停在牆前(再退 1 mm,注視點不會剛好落在牆面上 —— 落在面上會被當成「在形狀裡面」而在下一次 cast 被忽略)。
4. 從注視點沿 `(yaw, pitch)` 方向做 sphere cast,長度 = 目前期望距離,得到「允許距離」。
5. 實際距離:允許距離比較近 → **立刻**拉近(同一步,不平滑,所以不會先穿牆再退回);比較遠而且相機之前被擋(正在恢復) → 以 `recover` 半衰期平滑退回;沒被擋時直接跟著期望距離(縮放不延遲)。最後夾在 `min_distance` 以上。
6. 相機位置 = 注視點 + 方向 × 實際距離,朝向注視點(世界 +Y 為上)。

平滑都是指數型、以半衰期表示:`goal + (current - goal) · 2^(-dt/half_life)`。和步長無關:兩步 dt/2 等於一步 dt(`axiom_core::camera::damp`,單元測試釘住),單調、不會過衝。

### 為什麼在固定步更新,不是每個渲染幀

引擎目前**沒有渲染插值**:主迴圈以 120 Hz 固定步推進模擬,每一幀畫的是最新的模擬狀態。目標(玩家、導航代理、路徑跟隨者)只在固定步移動,所以每幀更新相機只會在兩步之間重複看到同一個目標姿態;相機和目標在同一步更新(相機排在所有移動系統之後),兩者之間沒有相對抖動。固定步也讓結果確定、可在無頭模式驗證(scenario、`observe`)。系統 phase 44:在所有移動者與 action / fps 相機控制(action.simulate 在 animation 30 / audio.events 40 之後執行)之後,在 `audio.spatial`(45,沒有聽者時用作用中的相機當聽者)之前。

## 碰撞:哪些東西會擋相機

在建立執行期時(背景 worker 上)收集一次,用**編寫時的姿態**,放進均勻 XZ 格子:

| 來源 | 形狀 |
|---|---|
| `physics.collider` box / capsule | 方塊依實體世界矩陣(含縮放,和導航格子相同);膠囊端點依矩陣、半徑乘最大縮放 |
| `nav.blocker` | XZ 矩形(和行走者相同:實體編寫的位置 + offset,不旋轉)。高度 = 同一個實體自己的 `render.mesh` 部件包圍盒;**沒有 mesh 時視為無限高**(隱形牆) |
| 帶 tag `camera_blocker` 的實體的 `render.mesh` 部件 | 每個部件一個有向方塊(單位方塊 × 部件矩陣) |

不擋相機的:被系統移動的實體(任何 mover 元件、`path.follow`、或在它們底下的子孫)、執行期生成的實體、跟隨目標本身與它的子孫、所在場景未啟用的實體。被 tween 動畫的物件不能當靜態阻擋:`nav.blocker`、`physics.collider` 與 `camera_blocker` mesh 放在被 tween 動畫的階層裡是編寫錯誤 `tween_conflict`。

**Sphere cast 的近似**:半徑 r 的球沿線段移動 = 線段對「每個形狀膨脹 r」做射線測試。膠囊是精確的(半徑 + r);方塊膨脹後保留尖角(沒有圓角),所以在邊與角附近會提早最多 r·(√3−1) 停下 —— 保守,不會比真正接觸晚。

**起點已經貼著牆**(樞紐點離牆不到 r):離開牆的方向不受影響;往牆裡的方向停在牆面(相機可能吃進牆最多 r)。起點在形狀**裡面**時忽略該形狀。

**min_distance 優先**:牆比 min_distance 還近時,相機仍停在 min_distance(可能在牆後)。要貼牆時請把 min_distance 設小一點(房間內建議 ≤ 0.5)。

共用的純函式在 `axiom_core::raycast`(`Shape::cast`、`BlockerGrid`),相機只是第一個使用者。

## 輸入

`input.yaw_axis` / `pitch_axis` / `zoom_axis` 是 `input.bindings.axes` 的名稱,例如:

```json
"input.bindings":{"axes":{"orbit":{"positive":"KeyC","negative":"KeyZ"},"zoom":{"positive":"KeyE","negative":"KeyQ"}}}
```

軸不存在是編寫錯誤(`invalid_camera`)。輸入情境(`input.contexts`)照常路由:沒有路由到 `camera.rig` 時它收到空輸入。滑鼠拖曳環繞還沒做。

## 動作與觀測(不需要 GPU)

| 動作 | 參數 | 結果 |
|---|---|---|
| `camera.follow.set` | `id`(跟隨相機)、選填 `target`、`distance`(min..max)、`yaw`、`pitch`(min_pitch..max_pitch),度 | 移動目標值;相機依平滑設定移過去,換目標時注視點平滑追到新目標。同 `camera.inspect` 的結果 |
| `camera.activate` | `id`(任何 `render.camera`) | 執行期把它設為唯一作用中的相機(其他設為非作用中),回傳 `{active, deactivated, enabled}`。**不改編寫內容**(要寫入請用 ACL `camera main ID` / 編輯器 Set as Main Camera)。拒絕:相機在未啟用的場景;世界有 `input.contexts`(情境擁有各自的相機,請用 `input.activate` 切換) |
| `camera.inspect` | `id` | 跟隨相機:`target`、`desired`(目前期望距離)、`goal`、`allowed`(這一步 cast 允許的)、`actual`、`hit` / `focus_hit`(擋住的實體 ID)、`yaw`、`pitch`、`focus`、`position`、`tests`、`over_budget`;一般相機:`{follow:false, active, enabled, position}` |
| `camera.summary` | — | 同 `observe` 的 `camera` |

`observe` 的 metrics 有 `camera`:`follow`(每台:`id`、`target`、`desired`、`actual`、`hit`、`tests`)、`active`、`activated`、`blockers`、`grid`、`over_budget`、`halted`。錯誤都是可恢復的請求錯誤:不存在 `not_found`、不是相機 `not_a_camera`、超出範圍、型別錯誤。

## 編輯器與無頭(對等)

| 編輯器 | ACL |
|---|---|
| Object ▸ Camera Follow Selected(`object.camera_follow`):選取的相機跟隨選取中唯一一個不是相機的物件 | `camera CAMERAS follow TARGET [distance=] [min=] [max=] [yaw=] [pitch=] [height=] [orbit=AXIS] [zoom=AXIS] [radius=] [collide=on\|off]` |
| Object ▸ Set as Main Camera(`object.main_camera`) | `camera main ID` |
| Inspector ▸ Components ▸ camera.follow 表單 | `set ID camera.follow/FIELD V`(例:`set cam camera.follow/collision/radius 0.4`) |

兩邊都呼叫 `ops::add_camera_follow` / `ops::make_main_camera`;缺少時在同一個 revision 補 `camera` capability 與 `camera.rig` 系統。Hierarchy 標籤 `RIG`。Gizmo:從注視點沿編寫的 yaw / pitch 畫到 max_distance 的線,min_distance(亮)與 max_distance(暗)各一個十字,期望距離一個圓圈,另有相機到注視點的暗線(只在編輯器視圖,隱藏相機時一起隱藏)。

## 編寫錯誤

- `invalid_camera`:沒有 `camera.rig` 系統、沒有 `render.camera`、相機有 parent、目標不存在 / 是相機自己或子孫 / 沒有 `core.transform`、輸入軸沒綁定。欄位超出範圍是元件驗證錯誤。
- `camera_conflict`:相機另有 mover(`core.motion`、`fsm.agent`、`nav.agent`、`action.actor`、`rts.unit`、`fps.*`),或某個資源在頂層 `camera` 欄位指名這台相機(`action.session`、FPS / RTS session、`showcase.tour` —— 那些系統自己放相機)。
- `tween_conflict` / `path_conflict`:tween 的 transform 軌或 `path.follow` 放在跟隨相機上(`camera.follow` 已登記為 mover)。
- `camera_budget`:跟隨相機超過 8 台,或阻擋形狀超過 8192 個(只有在至少一台跟隨相機開著碰撞時才收集與檢查)。

## 預算與效能

- 跟隨相機 ≤ 8(= 作用中相機上限)。阻擋形狀 ≤ 8192。
- 每次 cast 最多測 512 個形狀(相機每步兩次 cast:注視點、相機)。超過時相機停在 min_distance(安全的一側),`observe` 的 `over_budget` 計數,`inspect` 的 `tests` 顯示 512。
- 格子:固定 4 m 的 XZ 格,以雜湊分到 2 的冪次個桶(桶數 ≥ 2 × 插入數),所以遠處的形狀不會讓密集區的格子變粗;跨越超過 64 格的大形狀(地板、邊界牆)放在「每次都測」清單,不複製到每一格。cast 的包圍盒超過 4096 格時改為逐一測試(仍受 512 的上限)。同一次 cast 用標記陣列避免重複測同一個形狀(同桶碰撞只會多測,不會漏測)。
- 每步不配置記憶體(標記陣列在建立時配好)。release 量測(`perf_bench --generated-camera 8x1024`):8 台相機 × 1024 個牆方塊,step p50 約 0.0125 ms、配置 p50 0,建置 0.5–0.9 ms;axiom-town 沒有回歸。細節見 `docs/perf-baseline.zh-TW.md`。

## 熱重載、存檔、生成

- **熱重載**:依相機 ID 保留平滑中的姿態(yaw、pitch、距離、注視點、實際距離),所以不會跳動。`camera.follow` 內容沒變時,目標值(玩家用輸入轉到的角度與縮放、`camera.follow.set` 設的值)與 `camera.follow.set` 換的目標也保留;內容改了則回到編寫的目標值,從目前姿態平滑過去。`camera.activate` 的結果在「所有相機編寫的 active 旗標都沒變」時保留,否則以編寫內容為準。
- **存檔**:不支援。排入 `camera.rig` 的世界做 checkpoint 會明確失敗(`system camera.rig does not implement checkpoints`)。
- **執行期生成**:生成帶 `camera.follow` 的實體會讓系統停止並在 `observe camera.halted` 說明(請改為編寫)。
- 姿態從第一個固定步開始套用;在那之前(含編輯模式)相機停在編寫的位置。

## axiom-town 為什麼沒有換成 `camera.follow`

town 的相機由 `action.simulate` 擁有:`action.session.camera` 是必填欄位,action 控制器每步寫相機 transform,而且它的 yaw(Z / C 的 `orbit` 軸與滑鼠拖曳)同時決定玩家的移動方向(以相機為基準的 WASD)。要換成 `camera.follow` 得改 action 控制器的契約(從相機讀 yaw、session 不再必填相機、重播與 timeline 測試跟著改),超出這個項目的範圍;而且 town 是開放廣場,沒有會擋住相機的室內空間,換了也看不到差別。因此 town 不變;在 action 的相機上加 `camera.follow` 是明確的 `camera_conflict`(有測試)。

## 限制

- 只擋靜態幾何;移動中的物件(門、車)不擋相機。不做遮擋物淡出。
- `nav.blocker` 不旋轉;沒有 mesh 的 blocker 無限高。
- 方塊的 sphere cast 在邊角保守(見上)。
- 沒有滑鼠環繞;沒有肩膀側移(lateral offset)。
- 不支援存檔;執行期生成的跟隨相機不支援。
