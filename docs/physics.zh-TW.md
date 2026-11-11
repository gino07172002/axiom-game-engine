# 3D 剛體物理(`physics.body` / `physics.collider` / `physics.step`)

箱子從高處落下、堆疊、碰撞反彈,球滾下斜坡,角色推開擋路的箱子。執行期系統是 `physics.step`(capability `physics`),以已經在依賴樹裡的 rapier3d 0.23 做固定步進模擬(選型見 `docs/plan-engine-next.zh-TW.md` 執行紀錄 ▸ P 的 ADR)。不需要改 Rust:內容、ACL、編輯器都能建立與修改。

示範內容包:`content/physics-lab`(只用引擎內建方塊與程序球體),雙擊 `Play Physics Lab.cmd`(使用獨立的 `data/physics-lab.sqlite`)。

## 三種角色

排了 `physics.step` 的世界裡,每個 `physics.collider` 依所在實體決定角色(建立執行期時判定,依實體順序加入,這是確定性的前提):

| 角色 | 條件 | 行為 |
|---|---|---|
| 靜態(static) | 沒有 `physics.body`,自己與祖先都沒有被其他系統移動 | 固定在編寫位置(含 parent、縮放)。所在場景未啟用時不參與碰撞 |
| 跟隨(follower) | 沒有 `physics.body`,但實體或祖先被其他系統移動(`core.motion`、`path.follow`、`nav.agent`、`fsm.agent`、`rts.unit`、`action.actor`、`fps.*`…) | 每步把運動學剛體驅動到實體目前的世界姿勢,**推開**動態剛體(單向:角色控制器仍用自己的碰撞,不會被箱子擋住) |
| 動態 / 運動學 | 有 `physics.body` | 動態:重力、接觸、摩擦、彈性、睡眠;運動學:依自己的 `velocity` / `angular_velocity` 移動,推開動態剛體,本身不被推動 |

所以「角色推箱子」不需要任何控制器程式碼:角色實體掛一個膠囊碰撞體即可(示範包的 porter 是 `path.follow` 角色;測試也涵蓋 `core.motion`)。FPS 的玩家與哨兵本來就有膠囊碰撞體,排了 `physics.step` 的 FPS 世界裡會推開動態箱子;FPS 自己的競技場碰撞忽略帶 `physics.body` 的實體(會動的東西不能當固定牆);FPS 競技場的地板與外牆只存在於 FPS 模擬內,`physics.step` 看不到,在 FPS 世界放剛體需要另外編寫靜態碰撞體當地板(它同時是 FPS 的掩體方塊,必須符合 FPS 掩體的規則:軸對齊、不低於地面、邊長 ≤ 22 m)。Action 控制器在動畫 / 音訊之後才執行,它的角色以上一步的姿勢推箱子(差一步,1/120 秒)。

## 元件

`physics.collider`(既有元件,向後相容;舊內容不需修改,序列化結果不變):

```json
{"shape":{"shape":"box","half_extents":[0.5,0.5,0.5]},"friction":0.8,"restitution":0.2}
{"shape":{"shape":"capsule","half_height":0.5,"radius":0.35}}
{"shape":{"shape":"sphere","radius":1}}
```

| 欄位 | 說明 |
|---|---|
| `shape` | `box`(`half_extents`)、`capsule`(沿本地 Y,`half_height` 是兩端球心距離的一半)、**`sphere`(新增)**。尺寸 0.001..10000 m。實體縮放套用方式與導航 / 相機阻擋相同:箱子各軸乘縮放,膠囊與球的半徑乘最大縮放 |
| `friction` | 選填,0..10,預設 0.5(兩個碰撞體取平均) |
| `restitution` | 選填,0..1,預設 0(兩個碰撞體取較大者:彈力球在預設地板上照樣彈) |

`physics.body`(新元件,需要同一實體上的 `physics.collider` 與 `core.transform`、根實體、`physics.step` 系統):

```json
{"kind":"dynamic","mass":1,"velocity":[0,0,0],"angular_velocity":[0,0,0],
 "linear_damping":0,"angular_damping":0.05,"gravity_scale":1,"can_sleep":true,"ccd":false}
```

| 欄位 | 說明 |
|---|---|
| `kind` | `dynamic`(預設)或 `kinematic`。靜態場景就是沒有剛體的碰撞體 |
| `mass` | 公斤,0.001..100000,預設 1(慣量由碰撞體形狀計算) |
| `velocity` / `angular_velocity` | 動態:初速度;運動學:固定速度(m/s、rad/s,每分量 ±1000) |
| `linear_damping` / `angular_damping` | 0..100,預設 0 / 0.05 |
| `gravity_scale` | -10..10,預設 1 |
| `can_sleep` | 預設 true:低於閾值約 2 秒後睡眠;接觸與物理動作會喚醒 |
| `ccd` | 預設 false:小而快的物體用連續碰撞偵測(較貴) |

`physics.settings`(選填資源):`{"gravity":[0,-9.81,0],"solver_iterations":4}`,`solver_iterations` 1..16(每個 1/120 秒步進的接觸求解迭代;越多堆疊越穩、越慢)。

編寫錯誤(`World::compile`,交易失敗並回傳原因):

- `invalid_physics`:剛體沒有碰撞體、是子實體、世界沒排 `physics.step`;碰撞體掛在剛體的子孫上(不支援複合剛體)。
- `physics_conflict`:同一實體只能有一個移動者。`physics.body` 也登記在註冊表的 `movers`(`physics.body → physics.step`),所以在剛體上加 `path.follow` / `nav.agent` 也會分別得到 `path_conflict` / `nav_conflict`;導航格子與相機阻擋不把剛體當靜態障礙物。
- `physics_budget`:剛體加上被其他系統移動的碰撞體(跟隨者)合計超過 1024 個。

## 執行規則

- phase 22:在移動者(fsm 8、motion 10、path 12、nav 13、tween 14、rts 15、fps 20)之後,`trigger.detect`(25)之前(觸發區看得到剛體的新位置)。
- 每步:同步場景啟用狀態 → 把跟隨者驅動到實體目前姿勢(瞬移見「限制」)→ rapier 以 `FIXED_DT`(1/120 秒)步進一次 → 把醒著的剛體姿勢寫回 `core.transform`(縮放不變)。睡著的剛體不寫。
- 離原點任一軸超過 10 km 的剛體停用,列在 `out_of_bounds`,不會讓 `core.transform` 超出合法範圍。
- 執行期 spawn 出帶 `physics.body` 的實體:系統停止並在 `physics.halted` 寫明原因;只有碰撞體的 spawn 不加入模擬,計數在 `ignored_spawns`。
- 確定性:同一個執行檔、同樣的輸入,結果逐位元相同(scenario 的 `state_hash` 一致;跨程序、debug 與 release 量測見 ADR)。rapier 未開 `enhanced-determinism`,不保證跨 CPU 架構 / 作業系統一致。
- 熱重載(新 revision):實體 ID 相同且編寫的 transform 沒變的剛體保留模擬中的姿勢、速度與睡眠狀態;改了 transform 的剛體從新的編寫位置靜止開始;改了 `velocity` / `angular_velocity` 的剛體改用新的速度(運動學剛體的行為就是它的速度)。拿掉 `physics.body` 的實體變成靜態碰撞體,停在模擬到的位置。接觸快取重建(堆疊在重載瞬間可能有極小的調整)。
- **存檔(checkpoint)支援**:世界啟用 `save` capability 時,checkpoint 除了 transform 之外,在 `systems["physics.step"]` 存每個剛體的速度、角速度、睡眠狀態與睡眠計時;讀檔會還原。缺少這份狀態的存檔(例如手改過)明確拒絕,不會默默讓剛體從靜止重來。讀檔後求解器的接觸快取重建,所以「存檔後繼續」與「沒存檔一路跑下去」不保證逐位元相同,但同一份存檔讀兩次一定相同。

## 執行期指令(AI、UI、行為程式共用)

| 動作 | 參數 | 效果 |
|---|---|---|
| `physics.impulse` | `{"id":剛體,"impulse":[x,y,z]}` | 在質心施加衝量(N·s),喚醒剛體 |
| `physics.set_velocity` | `{"id":剛體,"velocity":[..],"angular_velocity":[..]}`(至少一個) | 設定速度,喚醒剛體 |
| `physics.inspect` | `{"id":有碰撞體的實體}` | kind、position、velocity、angular_velocity、speed、mass、sleeping、out_of_bounds |
| `physics.summary` | `{}` | 同 observe 的 `physics` 段 |

只有動態剛體接受衝量與速度;靜態碰撞體、跟隨者、運動學剛體回傳說明原因的錯誤。編輯器 Play 時選取物件,Inspector 的即時區會顯示 `physics.inspect` 的結果。

`observe` 的 `physics` 段:`dynamic`、`kinematic`、`followers`、`static`、`awake`、`sleeping`、`contacts`(有接觸點的碰撞對)、`steps`、`awake_ids`(醒著的剛體,最多列 16 個)、`out_of_bounds`(停用的剛體 ID)、`ignored_spawns`、`halted`。不含任何時間量測,所以會進入 `state_hash`。

## 編寫

ACL:

```
physics crate body                              # 預設動態剛體;沒有碰撞體就依網格擬合一個箱子
physics entities[tag=prop] body kind=kinematic
physics ball body mass=2 shape=sphere           # shape= 重新依網格擬合碰撞體
set crate physics.collider/restitution 0.6
set crate physics.body/mass 4
resource create physics.settings
call physics impulse {"id":"crate","impulse":[0,6,0]}
```

擬合:碰撞體以實體原點為中心,所以每個半軸取網格部件在該軸最遠的點(偏心的網格會得到較大的對稱箱子);沒有網格就是 1 m 方塊。

編輯器:Object ▸ Add Rigid Body(選項對話框:Kind / Mass / Collider;下方顯示等價的 ACL 行),與 ACL 走同一個 `ops::add_rigid_body`(parity 測試 `rigid_bodies_match_acl`)。Add Component 也能加 `physics.body` / `physics.collider`(Inspector 表單由型別化 schema 產生)。場景視口的元件 gizmo 畫出模擬用的形狀(含縮放):靜態綠色、動態藍色、運動學與跟隨者琥珀色,Play 時睡著的剛體變暗,選取的物件白色。

## 效能

release 量測(`perf_bench --generated-physics N --ticks 600`,全部醒著、4 個一疊,3 輪 p50 的中位數,機器同時有其他編譯):64 / 256 / 1024 剛體每步 0.28 / 0.74 / 2.65 ms。睡著的剛體幾乎不花時間。剛體與碰撞體在建立執行期時一次配置;本模組的步進迴圈不配置記憶體,但 rapier 內部每步仍有 15 / 21 / 27 次配置(約 3 / 12 / 49 KB)。

## 限制

- 沒有關節、複合剛體、網格碰撞體、觸發事件 / 碰撞事件、載具、布料。
- 角色控制器是單向推動者:動態剛體不會擋住角色(角色仍走自己的碰撞)。
- 跟隨者一步內移動超過 0.5 m(60 m/s)或轉動超過 0.2 rad(約 1400°/s)視為瞬移:直接放到新姿勢,不帶推動速度(否則 ping_pong 路徑在端點瞬間轉身 180° 會把貼著的箱子以上百 m/s 甩出去)。與它重疊的動態剛體由接觸修正慢慢推出。
- 只在本機 Windows / x86-64 驗證確定性。
