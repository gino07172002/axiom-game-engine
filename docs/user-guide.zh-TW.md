<p align="center">
  <img src="../assets/branding/axiom-logo-white.png" alt="Axiom" width="360">
</p>

# Axiom 使用指南(詳細版)

Rust 原生、AI-first、不使用 Bevy 的遊戲引擎。M1.5 提供穩定 ID 的局部操作、精確差異、增量觀察、安全重試與成本 benchmark；M2 加入 RTS 框選、群組命令、平面導航、採集與生產。相同執行檔可載入 FPS、ADV、RTS、空世界與雙相機內容包，交易式 authoring 與固定步進 runtime 持續分離。

骨架動畫已接上同一執行檔。雙擊 `Play Animation.cmd` 可看骨架蒙皮、動畫混合、IK 與 retargeting。匯入支援 glTF／GLB、PMX 以及 Unity Humanoid 匯出的 `.axrig`。Rigify 透過 Blender 烘焙後的變形骨架匯入。操作、座標設定和驗證方式見[骨架動畫指南](skeletal-animation.zh-TW.md)。

雙擊 `Play Shader Lab.cmd` 可查看 Lit、Toon、自訂 WGSL 掃描線、遮罩與透明材質。方塊與匯入網格共用 `render.material`，WGSL 與材質參數可透過現有交易熱更新。詳見[Shader 系統指南](shaders.zh-TW.md)。

雙擊 `Play Effects Atlas.cmd` 開啟 36 展台的材質與 VFX 展示館，對照 Flax 官方 Materials／Particles Features Tour。包含 GPU compute 粒子、骨骼拖尾、場景熱扭曲、投影貼花與體積霧；每項實作與差異見[Effects Atlas 清單](effects-atlas.zh-TW.md)。

雙擊 `Edit Axiom.cmd` 可開啟原生場景編輯器。場景樹、3D 視口、Inspector 和 AI 協定面板共用同一個 World IR 與修訂紀錄。詳見[場景編輯器指南](scene-editor.zh-TW.md)。

雙擊 `Play Love No War.cmd` 可玩夜場雙人舞原型。邀舞後，用畫面上的節拍和 J／K／L 完成八拍。舞步、判定窗口及兩位舞者的動畫由內容包定義。詳見[雙人舞內容包](love-no-war.zh-TW.md)。

雙擊 `Play Action.cmd` 可玩 M3 第三人稱動作原型 Ember Court。WASD 移動、滑鼠或 Q／E 環繞、F 鎖定、左鍵三段連擊、Space 翻滾。技能的取消窗、體力、傷害、無敵和音效事件使用固定步進資料。詳見 [M3 Action 指南](m3-action.zh-TW.md)。

雙擊 `Play Alliance.cmd` 可玩 M4 聯合作戰關卡 Beacon Alliance。RTS 小隊採礦與生產、Action 英雄戰鬥及 ADV 任務位於同一個世界，G 切換操控。內容包使用精確相依版本、資產摘要與跨包事件／資源契約。詳見 [M4 組合指南](m4-composition.zh-TW.md)。

雙擊 `Play Alliance Combat.cmd` 使用獨立資料庫體驗共用碰撞與傷害：英雄和小隊可集火同一守衛，英雄可鎖定 RTS 敵人，Action 敵人也會攻擊附近士兵。詳見 [共用戰鬥指南](shared-combat.zh-TW.md)。

## 啟動

Windows 工作區可直接雙擊 `Play Axiom.cmd`，或：

```powershell
cd D:\rust_learn
.\target\debug\axiom.exe play
```

從原始碼建置先執行 `. .\scripts\env.ps1`，再執行 `cargo build --workspace`。全新 Windows checkout 使用 `scripts/setup.ps1` 安裝工作區內的 Rust／GNU 工具鏈。編譯快取、工具及 temp 都留在 D 槽；建置限制為 2 個並行工作，開發版不產生除錯符號。

```powershell
.\target\debug\axiom.exe editor
.\target\debug\axiom.exe --db data/rts-edit.sqlite --pack content/rts editor
```

## 五個內容包

```powershell
# 原有 FPS；v1 資料庫會自動追加 v2 遷移版本
.\target\debug\axiom.exe play

# 沒有玩家、生命、槍械、相機或物理的世界
.\target\debug\axiom.exe --db data/empty.sqlite --pack content/empty play

# 同一世界的透視與正交視圖，包含通用運動系統
.\target\debug\axiom.exe --db data/views.sqlite --pack content/two-cameras play --start

# ADV：最後的訊號；也可雙擊 Play ADV.cmd
.\target\debug\axiom.exe --db data/adv.sqlite --pack content/adv play --start

# RTS：邊境之戰（對抗電腦）；也可雙擊 Play RTS.cmd
.\target\debug\axiom.exe --db data/frontier.sqlite --pack content/rts play --start
```

`--pack` 是內容包目錄，包含 `manifest.json` 與 World IR v2 的 `world.json`。在 play 中明確指定會以一筆交易載入該內容包；再次指定會重新載入包內的初始定義。後續保留資料庫修改時，使用相同 `--db` 並省略 `--pack`。

未指定包時，新的資料庫優先讀取 `content/fps`；從其他工作目錄啟動、找不到包檔時使用內嵌的相同 FPS 範例。既有資料庫保留自己的世界。非 Windows 平台需自行安裝 Rust／C 工具，將 rust-toolchain channel 改成 stable；尚未跨平台實測。

FPS 操作：WASD 移動、滑鼠視角、左鍵射擊、R 裝填、Space 跳躍、Shift 衝刺、Tab 控制台、Esc 暫停。消滅哨兵完成原型；掩體阻擋射擊，頭部命中有額外傷害。一般世界使用上方 RUN／PAUSE／RESTART／CONSOLE，無須 FPS 控制器。

ADV 操作：點擊選項推進；在工坊按 E 或點選調查箱子以取得鑰匙，再返回開門。拒絕協助、放棄搜尋、成功求救分別通往三個結局。介面上可切換 `zh-TW`／`en`、快速存讀檔；Esc 暫停，RESTART 回到開場。中文使用 Windows 已安裝的微軟正黑體，不隨專案散布系統字型。

RTS 操作（`content/rts`，邊境之戰）：左鍵拖曳框選藍方單位，Shift 加選；右鍵地面移動、水晶採集、敵人或建築攻擊。WASD 平移鏡頭、Q／E 縮放；左下小地圖左鍵跳轉鏡頭、右鍵移動選取單位。下方指令列可選取工人／部隊、採集、「進攻敵方主堡」（攻擊移動：沿途迎擊防守者再回頭打主堡）與生產工人 50、衛兵 75（近戰、高血量）、弓手 90（射程 6 m）。地圖分北、中、南三條路線，各方有主堡、兩座防禦塔與兩處礦區，中路兩側另有可爭奪的大礦。敵方由資料驅動的電腦指揮（`rts.session.opponent`）：採礦、生產、主堡附近遇敵會回防，約 2:30 起發動逐波增強的攻勢。摧毀敵方主堡獲勝，主堡被毀則失敗。單位與建築上方顯示血條與生產進度，攻擊有投射物與斬擊特效，倒下的單位會淡出、被毀建築留下廢墟。HUD 文字、版面（`regions`、`styles`、`chrome:"compact"`）與按鈕均來自 `content/rts/world.json`，由 `export_rts_pack frontier` 產生。`content/rts-bench` 是固定的 64 單位回歸／壓測場景（無電腦指揮）。此階段尚無 RTS 私有模擬狀態的存讀檔。

## 共用玩法、敘事與存檔

`behavior.program` 定義 bool／i64／f64／text 變數、具型別事件、條件及動作；`narrative.graph` 組合進入動作、條件選項、節點跳轉與結局。條件支援 always／eq／ge／all／not；動作支援 set／add／emit／delay／go_to／scene／end。參照先編譯成索引，執行時不解析 JSON 路徑。每批命令或固定步進最多 512 指令、128 個待處理事件，敘事進入深度最多 16；超額失敗會保留前一份玩法狀態。

`ui.document` 定義多語系 catalog、文字／變數／本地化變數／對話綁定、條件按鈕、選項、圖片及進度條。UI、ActionMap、scenario 和 loopback 命令使用相同玩法介面。包內圖片為相對路徑，載入包時檢查範圍並解析；PNG 在 runtime 準備階段解碼，缺少資產會導致啟用失敗並保留舊世界。

```powershell
.\target\debug\axiom.exe schema gameplay
.\target\debug\axiom.exe --db data/adv.sqlite narrative-check --budget 4096
.\target\debug\axiom.exe --db data/adv.sqlite scenario examples/adv-rescue.scenario.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 command examples/adv-help.command.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 save quick
.\target\debug\axiom.exe --connect 127.0.0.1:4242 load quick
.\target\debug\axiom.exe --db data/adv.sqlite play --start --load-slot quick
```

`narrative-check` 在明確預算內探索選項、UI 按鈕與延遲事件，回傳各結局的路徑；不包含未知的外部事件來源，預算不足時 `complete:false`，不宣稱完成證明。`observe.gameplay` 提供變數、可見選項、節點、結局、場景、UI 綁定值及最近 128 筆 trace。

存檔位於資料庫同層的 `saves/`，使用世界 ID 命名空間及受限 slot 名稱；最多 8 MiB。包含獨立 `save_version`、來源 authoring revision、內容定義、模擬時鐘、穩定 entity／node IDs、型別欄位、旗標、待處理事件及語系。讀檔不執行節點進入動作、不新增 SQLite revision；寫入先 flush 暫存檔並保留上一份 `.bak`，若主檔遺失可讀取備份。毀損或不相容存檔不會取代目前狀態。

Checkpoint 採嚴格內容相容檢查；v0.4 亦接受已知的 v0.3.0／v0.3.1 checkpoint v1。編輯後可 undo 到相同定義，再讀舊進度，revision 編號可以不同。尚未提供任意內容版本之間的存檔遷移。原生系統必須明確支援 checkpoint；目前支援通用欄位、motion 和 shared gameplay，FPS 的 Rapier／戰鬥與 RTS 私有狀態會明確拒絕存檔。

## AI 修改流程

最精簡的入口是 ACL(Axiom Command Language)。一行一個敘述，輸出是精簡文字，底層仍編譯成同一套交易式協定(CAS、diff、undo、冪等重試都沿用):

```powershell
.\target\debug\axiom.exe discover acl                      # 文法
.\target\debug\axiom.exe acl -e "spawn enemy.basic count=10 id=enemy/{i} at=0,0,0 step=2,0,0 tag=enemy"
.\target\debug\axiom.exe acl -e "set enemy/* core.health/value 40 where core.transform/position/0>=10"
.\target\debug\axiom.exe acl -e "select entities[tag=enemy] where core.health/value<50 fields core.health/value limit 5"
.\target\debug\axiom.exe acl -e "inspect enemy/3"
.\target\debug\axiom.exe acl --preview script.acl          # 多行編輯 = 一個 revision
.\target\debug\axiom.exe --connect 127.0.0.1:4242 acl -e "history 5"
```

`select` 回傳 `enemy/5 core.health/value=40` 這類行，結尾 `matched=N rev=R [next=CURSOR]`。編輯回傳 `ok rev=R changes=N` 及最多 12 筆前後值。JSON 協定也可用 `{"op":"acl","text":"..."}`,`select` 另支援 `id_glob`、`where`、`format:"lines"`,編輯另有 `set_where`。

### 通用行為與診斷

`fsm.machines` 資源定義狀態機(動作:stop／move_toward／move_away;條件:near／far／health_below／in_state_ticks／always),實體掛 `fsm.agent` 即生效，世界需啟用 `fsm` 能力與 `fsm.step` 系統。範例見 `content/behavior-lab`。「敵人為什麼不追？」用一個呼叫回答，回傳目前狀態、上次轉移，以及每條外出條件的實測值與判定:

```powershell
.\target\debug\axiom.exe --db data/lab.sqlite --pack content/behavior-lab call fsm.inspect '{"id":"enemy/0"}'
#  … "state":"idle","transitions":[{"to":"chase","when":"near(player)<=15","measured":30.02,"pass":false}]
.\target\debug\axiom.exe --db data/lab.sqlite call fsm.summary --ticks 2400      # 無頭模擬 2400 tick 後
.\target\debug\axiom.exe --connect 127.0.0.1:4242 call fps.inspect '{"id":"enemy/0"}'   # 即時 FPS 哨兵
```

FSM 狀態目前不進 checkpoint,存檔會明確拒絕。

其他 agent 也可使用 `discover` 按需探索、`select` 取得局部欄位及 `set_field` 依 ID 修改；完整 `query world` 保留給匯出與除錯。

```powershell
.\target\debug\axiom.exe discover local
.\target\debug\axiom.exe --db data/adv.sqlite select examples/m15-select-choice.json
.\target\debug\axiom.exe --db data/adv.sqlite preview examples/m15-edit-choice.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /rts/summary --field /rts/selected
.\target\debug\axiom.exe schema rts
.\target\debug\axiom.exe schema actions
```

範例檔的 `expected_rev` 必須依查詢結果調整。`select` 支援 entities、resources、prefabs、scenes、nodes、choices、rules、widgets、variables，預設 16 筆／8192 bytes；投影與分頁游標有明確上限。`set_field`／`put_object`／`remove_object` 使用穩定 target ID，choice 另指定 node，無須先取得整個敘事圖或陣列位置。Prefab 局部覆寫保留其他欄位的繼承。

Preview／apply 回傳包含資源的 ID 差異與欄位路徑；`diff REV_A REV_B` 可分頁取完整差異。`discover TOPIC --if-hash HASH` 避免重讀相同 schema。`observe-delta --cursor CURSOR` 回傳指定投影的變更；游標失效、重啟或恢復 checkpoint 時明確 reset，trace 缺口亦會標示。

`--request-id ID` 可冪等重試 apply／undo，receipt 與 revision 原子提交 SQLite。即時 command／save／load 另需 `--runtime-session`（從 observe 取得），成功結果只在該 host session 去重，舊 session 不會在新程序重新執行。詳見 [M1.5 合約與 benchmark](m15-implementation.md)。

```powershell
.\target\debug\axiom.exe schema
.\target\debug\axiom.exe schema components
.\target\debug\axiom.exe schema systems
.\target\debug\axiom.exe schema fps
.\target\debug\axiom.exe query
.\target\debug\axiom.exe preview examples/more-enemies.json
.\target\debug\axiom.exe apply examples/more-enemies.json
.\target\debug\axiom.exe verify --ticks 1200
.\target\debug\axiom.exe history
.\target\debug\axiom.exe undo 1
```

Schema 依已安裝能力註冊表產生；可查 world、components、resources、systems、edits，以及單一組件名稱。Query scopes 支援 summary、world、entities、components、resources、systems、behavior。預設回覆摘要，實體查詢可用 `--limit` 限制回傳。

Preview 執行相同的編輯、型別檢查、世界編譯與約束驗證，回傳差異而不增加 revision。Apply 的整批修改通過後才 commit；錯誤回覆包含 diagnostics。`expected_rev` 提供 CAS，讓多個 agent 能偵測衝突。相同內容不新增版本。

通用 v2 編輯：

```json
{"op":"apply","expected_rev":1,"edits":[
  {"kind":"put_entity","entity":{"id":"prop","components":{
    "core.transform":{"position":[1,0,0]},
    "core.health":{"value":10,"max":10}
  }}}
]}
```

Entity 可沒有空間或生命組件。原生組件包含 Transform、Health、Motion、RenderMesh、Camera、Collider；世界也能宣告具型別的自訂 component／resource schema。Prefab 組件可局部覆寫，parent 關係會檢查循環，Scene 可以停用成員。

完整可操作的非 FPS 範例：

```powershell
.\target\debug\axiom.exe --db data/sandbox.sqlite --pack content/empty query
.\target\debug\axiom.exe --db data/sandbox.sqlite preview examples/v2-add-box.json
.\target\debug\axiom.exe --db data/sandbox.sqlite apply examples/v2-add-box.json
.\target\debug\axiom.exe --db data/sandbox.sqlite play --start
```

原有 `enemies.count`、`weapon.damage` 等路徑與障礙物／行為指令，由 FPS 套件的相容 adapter 處理。原本的 `examples/more-enemies.json`、`power-rifle.json`、`passive-ai.json` 可繼續使用。通用引擎不定義這些玩法欄位。

## 即時更新、觀察與版本

遊戲每 250 ms 檢查資料庫。在模擬安全點準備新的 compiled world、系統與呈現資料，遷移既有狀態後才切換；啟用失敗保留上一個世界。

```powershell
.\target\debug\axiom.exe --connect 127.0.0.1:4242 apply examples/power-rifle.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe
```

Observe 分開回報 `rev`（已啟用）、`requested_rev` 與 `activation`。FPS HUD 也使用同一個啟用版本。authoring commit 不代表 runtime 已啟用；`activation_failed` 帶有診斷。

原生 Transform 在定義不變時保留執行位置；自訂欄位在 schema／定義不變時保留值，entity references 依穩定邏輯 ID 重新連結。FPS 模組依邏輯 ID 保留傷害、裝填、角色與擊殺狀態；變更地形或出生點會安全重置角色位置。世界 undo 恢復定義，重新開始會重建遊玩狀態。

Adapter 僅綁定 loopback TCP，JSONL 協定不依賴 MCP 或特定 LLM。預設連接埠 4242，`play --port 0` 可停用；多個遊戲使用不同 `--db` 與 `--port`。Stdio 批次使用 `axiom serve`，每行一個 request／response。

## 舊資料與存取

`import` 接受 v1 FPS 或 v2 世界，`export` 輸出 v2。開啟舊資料庫時，舊 revision 保持原文，另追加 `migrate:1->2`。對舊 revision 執行 undo，會轉換後追加 v2 恢復版本；不回退 SQLite 實體 schema。

原始 v1 範例保留在 `assets/worlds/proving-ground.json`，用於遷移與回歸。可編輯的 v2 FPS 內容位於 `content/fps/world.json`；模型方塊、材質、競技場標記都存於內容資料。`content/fps/models.json` 是基準配方的模型來源；已載入世界的外觀以 world.json／資料庫中的 `fps.visuals` 為準。

## 驗證

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo build --workspace --examples
.\scripts\m0-smoke-test.ps1
.\scripts\m1-smoke-test.ps1
.\scripts\m15-smoke-test.ps1
cargo build --release -p axiom -j1
.\scripts\m2-smoke-test.ps1
.\target\debug\axiom.exe --db data/scenario.sqlite scenario examples/fps-regression.scenario.json
```

Scenario 透過通用 ActionMap 及 typed commands 注入輸入，支援中途斷言、存讀檔、選項可見性與觀察值比對；不建立視窗。M1 smoke 預設包含 M0／FPS 回歸，再驗證三個 ADV 結局、中英文 UI、圖片、跨程序 checkpoint、preview／資料編輯／undo、即時命令及狀態遷移，並比對執行檔 SHA256。報告位於 `artifacts/m1-validation.json`，截圖位於 `artifacts/m1-*.png`。

M2 smoke 預設包含 M1.5 → M1 → M0，不必逐一重跑上述腳本。RTS 功能場景（`content/rts-bench`）執行 7200 ticks：64 初始單位、32 友方全數群組抵達、採集交付、生產兩單位至 66。另驗證原生 64／128／256 單位畫面、即時命令與生產冪等重試。`artifacts/m2-validation.json` 保存完整證據，`artifacts/m2-*.png` 為截圖。

`scripts/m2-stress-test.ps1` 使用 release 執行檔，每個數量各 3 個新程序，先暖機 240 ticks、量測 2400 ticks；雙方全體接受週期群組命令，停用戰鬥維持人口，檢查資源／路徑／無重疊不變量。報告分開列出 step 分位數、命令、驗證與全程序記憶體，位於 `artifacts/m2-stress.json`。這是 headless CPU 模擬壓測；原生 GPU 畫面驗證另行記錄。

```powershell
.\target\debug\axiom.exe benchmark benchmarks/m15.json --output artifacts/m15-benchmark.json
.\target\release\axiom.exe rts-stress --units 256 --ticks 2400 --output artifacts/rts-256.json
```

AI benchmark 記錄 legacy／local 計畫的 request／response bytes、工具次數、失敗及驗收成功率；可接外部 JSONL agent adapter。未提供真實模型 usage 時 tokens 為 null，bytes 不代表 tokens，尚未宣稱真實 token 節省。

GPU smoke 使用隱藏原生視窗，主動推進指定畫面幀數。`frame_ms_cpu` 包含提交、可能的呈現等待，以及擷取當幀的 GPU readback／PNG 寫入；它不代表純 CPU 或 GPU 執行時間。尚未宣稱 token、FPS 或 RAM 優勢。

## 結構與目前範圍

| 模組 | 職責 |
|---|---|
| axiom-core | World IR v2、型別／能力／系統註冊、世界編譯、交易、preview、版本與套件載入 |
| axiom-fps | FPS schema、v1 adapter、相容指令與內容配方 |
| axiom-rts | RTS schema、平面格網 A*、有界命令／路徑服務、經濟與生產、內容配方 |
| axiom-runtime/world.rs | 固定步進、註冊系統、typed storage、generation IDs、狀態遷移、啟用狀態 |
| axiom-runtime/presentation.rs／render.rs | 不可變呈現快照、獨立 GPU renderer、多相機 |
| axiom-runtime/fps | FPS 原生控制／碰撞／戰鬥、模型呈現與 HUD |
| axiom-runtime/rts.rs | RTS 能力註冊、平面射線輸入、typed storage 同步、動態實體及呈現抽取 |
| axiom-core/gameplay.rs | 共用玩法／敘事／UI IR、型別與引用編譯、schema |
| axiom-runtime/gameplay.rs／narrative.rs | 有界事件與敘事執行、trace、可達性探索 |
| axiom-runtime/bound_ui.rs／checkpoint.rs | 綁定 UI、圖片準備及獨立存讀檔 |
| content/* | 由相同執行檔載入的世界資料與能力宣告 |

目前提供 FPS、ADV、RTS 與 Action 的資料／玩法閉環。M3 Action 使用 `axiom-action` 的 Ability IR 和純模擬，原生相機、輸入、骨架與音效整合由 runtime 註冊。新增物件、變數、事件、敘事分支、UI、單位參數、生產配方與技能時序可走資料；新增原生演算法仍需 Rust 建置。共用 `RuntimeCommand::Call` 依註冊能力派送，UI／scenario／loopback 共用相同命令，核心不依遊戲類型分支。FPS 與 Action HUD 由註冊模組提供原生 UI。

RTS 使用四方向格網 A*、有界排隊／快取、格子預留與阻塞重排；預設每 4 ticks 處理導航決策，移動維持 120 Hz。這是小型策略原型，非完整 navmesh 或大型戰略 AI。格網熱編輯需重新開始；`max_units` 限制包含死亡單位的 resident slots，尚未回收屍體。完整能力與限制見 [M2 實作說明](m2-implementation.md)。

Collider 在核心是具型別資料；現有 FPS 模組執行角色碰撞與射線，並保留原型的固定膠囊尺寸、軸對齊掩體與單一主要 FPS 視圖限制。Action 使用有界地面的平面圓形角色與矩形阻擋。骨架資產可匯入 glTF／GLB；音效目前提供資料定義的合成 PCM cues。原生場景編輯器已可用。通用剛體、任意音訊檔匯入、任意腳本、多人網路與跨平台 bit-exact 重播仍屬後續工作。

更多設計：[architecture.md](architecture.md)、[M2 實作說明](m2-implementation.md)、[M1.5 實作說明](m15-implementation.md)、[M1 實作說明](m1-implementation.md)、[M0 實作說明](m0-implementation.md)、[後續路線圖](ai-first-general-engine-roadmap.zh-TW.md)。
