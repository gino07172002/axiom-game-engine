# 待辦:物件元件缺口(對照 Unity / Godot)

> 建立:2026-11-08。和 `docs/plan-editor-human-tuning.md` 一起執行;順序經使用者確認。
> 每一項都依循既有規則:一個元件(含型別化 schema)、編輯器的 gizmo 與表單、無頭 ACL 對應指令,以及一致性測試(見 `docs/editor-parity.zh-TW.md`)。

## 現況(以 `axiom discover` 的實際註冊表為準)

物件元件 20 個,多數綁定特定玩法(`fps.*`、`rts.*`、`action.actor`)。`physics.collider` 只有 box / capsule,而且只供 FPS 使用;音效只有全域的 `audio.library`(2026-11-09 起:`audio.source` / `audio.listener` 已加入);燈光只有寫死的太陽,以及 VFX 產生的點光源(半徑固定 2 m、最多 32 盞)。(2026-11-08 起:`render.light` / `render.environment` 已加入。)

## 執行順序

1. **燈光系統**(使用者指定,參考 Blender / Unity):`render.light` 的 `directional` / `point` / `spot`(內外錐角) / `area`(矩形、圓盤;第一版以代表點近似,並如實標示),以及 `render.environment`(單色,或天空 / 地平線 / 地面三段漸層的環境光)。太陽改由元件提供,未設定時沿用現有預設。陰影:平行光沿用;聚光燈與點光源的陰影先量測成本再決定。**狀態:已完成第一版**,見 [lighting.zh-TW.md](lighting.zh-TW.md)(spot / point / area 陰影未支援,會明確拒絕)。
2. **觸發區**:進入 / 離開事件,接 `behavior.program` 與 `runtime.bindings`(取代城鎮「FSM + near」的權宜做法)。**狀態:已完成第一版**,見 [triggers.zh-TW.md](triggers.zh-TW.md)(`trigger.zone` box / sphere、tag / 元件篩選、once、遲滯;城鎮已改用觸發區;存檔不支援,會明確失敗)。
3. **路徑與路徑跟隨**:可循環、速度、朝向切線(車輛、行人)。**狀態:已完成第一版**,見 [paths.zh-TW.md](paths.zh-TW.md)(`path.spline` linear / Catmull-Rom、`path.follow` loop / ping_pong / stop + `on_arrive`、側向 offset、暫停 / 恢復 / 改速指令、移動者衝突為編寫錯誤;存檔不支援,會明確失敗)。
4. **導航代理**:繞過 `nav.blocker` 與建築的尋路(通用版,不綁 RTS)。**狀態:已完成第一版**,見 [navigation.zh-TW.md](navigation.zh-TW.md)(`nav.agent` 點 / 實體目標、加速煞車、轉身、簡單避讓、`on_arrive` / `on_fail`;`nav.grid` clearance 格子在建立執行期時建好並依內容快取;每步尋路工作量有上限、請求排隊;與 RTS 共用 A* 引擎 `axiom_core::grid_search`,RTS 結果逐位相同;存檔不支援,會明確失敗)。
5. **互動元件**:範圍、提示文字、按鍵,觸發事件(對話、開門)。**狀態:已完成第一版**,見 [interaction.zh-TW.md](interaction.zh-TW.md)(`interact.target` range / 遲滯 / view_angle / priority / cooldown / once、互動者 filter 與觸發區同語意;焦點依 priority → 距離 → 朝向 → 編寫順序;按鍵用 `input.bindings` 的按鈕名稱;提示走 `ui.document` 的 `prompt` 綁定(沒編寫時有預設提示);`interact.use` / `inspect` / `summary`;城鎮的「FSM near + nearby」交談已改用互動元件,逐 tick 等價測試;存檔不支援,會明確失敗)。
6. **屬性補間 / 動畫軌**:開門、燈號、UI 動效(與 M7 共用)。**狀態:已完成第一版**,見 [tweens.zh-TW.md](tweens.zh-TW.md)(`tween.clips` 具名片段 × 軌 × 關鍵影格,easing linear / step / ease_in / ease_out / ease_in_out,once / loop / ping_pong、autoplay、speed、倒播、seek;目標是自己或子孫的 transform(position / rotation / yaw / scale)、`render.light` 顏色與強度、`render.mesh` 顏色與自發光、`behavior.program` 數字變數(UI 進度條 / 文字 / 條件用它;UI 不透明度留給 M7);建立執行期時解析成型別化欄位,key 在平坦陣列、每軌快取段落,每步零配置;`tween.play` / `stop` / `seek` / `inspect` / `summary`、`on_finish` 事件、`play_on` 直接綁定事件;與移動者、靜態障礙衝突是編寫錯誤;**移動中的 `nav.blocker` 不支援**(門扇不要放 blocker,見文件);存檔不支援,會明確失敗)。
7. **3D 聲源與聽者**:位置、衰減、循環。**狀態:已完成第一版**,見 [audio.zh-TW.md](audio.zh-TW.md)(`audio.source` clip / volume / pitch / loop / autoplay / spatial / min-max 距離 / rolloff inverse | linear / priority、`audio.listener`(最多一個 active,沒有時用啟用中的攝影機);片段是 `audio.library.clips` 的程序合成定義(repo 沒有音檔),建立執行期時依內容 hash 合成並快取,有記憶體預算;每步在模擬端算距離衰減與等功率聲像,32 個 voice 依 priority → 距離 → 編寫順序確定性分配,結果在 `observe audio`(無頭可驗證);沿用 WinMM 工作執行緒改為 waveOut 串流混音,音訊執行緒不配置、不等鎖;`audio.play` / `stop` / `inspect` / `summary` 可從 `runtime.bindings` 呼叫;不做都卜勒;存檔不支援,會明確失敗)。
8. **跟隨相機 + 碰撞**:相機不穿牆(進房間時需要)。**狀態:已完成第一版**,見 [camera-follow.zh-TW.md](camera-follow.zh-TW.md)(`camera.follow` 目標 / 距離 min-max / yaw-pitch 環繞(`input.bindings` 軸)/ 注視高度 / 位置與旋轉平滑(半衰期,與步長無關);sphere cast 對靜態阻擋:`physics.collider`、`nav.blocker`、tag `camera_blocker` 的 mesh 部件,共用純模組 `axiom_core::raycast`(雜湊格子、每次 cast 512 個形狀上限);撞牆立即拉近、平滑退回、不近於 min_distance、注視點不會落後穿牆;固定步 phase 44;`camera.follow.set` / `camera.activate`(執行期切換主相機)/ `camera.inspect` / `camera.summary`、`observe camera`;編輯器 Camera Follow Selected / Set as Main Camera 與 ACL `camera SEL follow TARGET` / `camera main ID` 共用 ops;與其他移動者衝突是編寫錯誤;axiom-town 的 action 相機不換(原因見文件);存檔不支援,會明確失敗)。
9. **街道 demo 與缺口報告(最終驗收)**。**狀態:已完成**,見 [acceptance-street.zh-TW.md](acceptance-street.zh-TW.md)(`content/street` + `Play Street.cmd`;世界由 `content/street/build/` 的型別化 seed 請求 + 6 段 ACL 經協定重播建出,`scripts/build-street.ps1` 匯出,重現性測試;`examples/street.scenario.json` 無頭驗證車輛循環、紅燈停車與綠燈恢復、行人、與 NPC 對話、開門進房、房內觸發與相機在房內;真 GPU 編輯器 8 步與 ACL 逐位元相同;23 項缺口,其中編輯器不能編 tags、不能新建資源 / capability 兩項對「人只用編輯器」是阻斷)。

## 次優先(尚未排入)

更多碰撞形狀(球體 / 凸包 / 網格)、碰撞層與遮罩、關節、物理材質、掛在物件上的粒子發射器、動畫狀態機與混合樹、約束(LookAt / Aim / Parent)、計時器、生成器、拖尾與線條渲染、貼花投影器、LOD 與大量實例化、反射探針與光照探針、渲染到貼圖(監視器、鏡子、小地圖)、霧與環境設定。

## 暫不建議

布料、布娃娃、風場、地形與草地、影片播放、網路同步、2D 系列。
