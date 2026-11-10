<img src="../assets/branding/axiom-icon-256.png" alt="Axiom" width="96" align="right">

# Axiom 原生場景編輯器

雙擊 `Edit Axiom.cmd`，或在專案根目錄執行以下指令。首次啟動會在 `data/editor.sqlite` 建立 Assembly Yard 範例。後續開啟同一個資料庫即可繼續編輯。

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe editor
```

以不同資料庫編輯既有內容包，例如 `.\target\debug\axiom.exe --db data/adv-edit.sqlite --pack content/adv editor`。每次指定 `--pack` 都會載入包內定義。續編時保留 `--db`，省略 `--pack`。

## 編輯視窗

| 區域 | 操作 |
| --- | --- |
| Hierarchy | 依場景或名稱搜尋，展開父子樹。新增空物件、方塊及相機，建立、啟用、停用或移除場景。篩選場景後新增物件，會自動加入該場景。 |
| Scene | 左鍵選取，右鍵拖曳環繞，中鍵拖曳平移，滾輪縮放，F 對焦。Q／W／E／R 切換選取、移動、旋轉、縮放。移動使用世界軸，旋轉和縮放使用物件本地軸。 |
| Inspector | 可改顯示名稱、位置、角度、縮放、父物件、場景成員及元件。元件表單由型別化 schema 產生(`discover typed:ID`):有範圍與單位的拖曳數值、列舉下拉、取色器、四元數以歐拉角編輯、物件參照(搜尋清單或把 Hierarchy 的列拖上來)、片段 / 事件 / 狀態機下拉、可摺疊的巢狀物件、陣列逐項 ▲ ▼ + x 與 + Add item(超過 32 項分頁),欄位說明在滑鼠提示,上方可篩選欄位;不認得的形狀以 JSON 欄位編輯。元件標頭:Enabled、Reset、Copy / Paste、Remove、相機的 Make Main Camera、tween / 音源的 ▶ Play。多選時值不同的欄位顯示「—」。JSON 分頁編輯物件自身的覆寫。 |
| Game | 和 Scene 同一格。顯示 runtime 的啟用相機(分割畫面照各相機的 viewport),上方可選長寬比 Free / 16:9 / 4:3 / 1:1(在分頁內加黑邊,存在偏好檔)、相機下拉(只改這個分頁看哪台相機,不寫入)與 **Set as Main Camera**(把下拉選的相機設為主相機,一筆修訂,等於 ACL `camera main ID`)。分頁沒顯示時不渲染。 |
| Changes / Assets / AI | 檢視預覽、提交與外部修訂的局部差異，實例化 prefab，複製查詢或最近的交易 JSON。 |
| UI Editor | Window ▸ UI Editor(與 Scene 同一格的分頁)。以遊戲的 HUD 程式碼在參考解析度畫出 `ui.document`(縮放進分頁);把 widget 拖到九宮格另一格(放開時一筆修訂);點選後在右側面板改 Region、Style、按鈕 Command、Fields(型別化表單)與 Effects(新增 / 編輯 / 移除 / ▶ Preview);工具列切換參考解析度與預覽語系(缺字警告),並預覽 `ui.nameplate`。詳見 [ui-effects.zh-TW.md](ui-effects.zh-TW.md)。 |
| Graphs | **State machine**:上方下拉選 `fsm.machines` 的機器(或 New machine…),畫布上每個狀態一個節點(左側綠條 = 初始狀態,副標 = 動作 / 進入事件 / 片段,橘框加「!」= 檢查問題,滑鼠停留看說明),箭頭 = 轉移,標籤是條件,`*` 轉移從「Any state」出發。拖曳節點移動(放開時寫一筆修訂,位置存在 `editor.layout`,runtime 不讀);從節點右緣的圓點拖到另一個節點新增轉移;雙擊空白處新增狀態;拖空白處或中鍵平移;右鍵有 Set as initial state / Rename / Delete、Raise priority、Add state here。右側面板編輯選取的狀態(改名、Make initial、Action 下拉 + 型別化欄位、Emit on enter、Clip / loop)或轉移(From / To、When 下拉 + 欄位、Up / Down 優先順序),一次互動一筆修訂、Esc 取消。**Rules**:`behavior.program` 規則表,+ Rule、Up / Down、x,選取的規則可改 event、when(巢狀條件與 operand 各有 kind 下拉)、actions(+ Action)。**Overview**:舊的唯讀總覽(含 narrative)。Play 時目前狀態的節點變綠並標 `live N`。 |
| Changes / AI | 檢視預覽、提交與外部修訂的局部差異,複製查詢或最近的交易 JSON。 |
| Assets | 資產瀏覽器:左側類型樹(模型 glb、圖片、材質、shader、prefab、FSM 狀態機、音效片段、UI 文件、輸入表,各附數量)與來源資料夾,上方搜尋、Grid / List 切換、New…、Thumbnails。格子 / 列表都是虛擬清單(只排版看得到的列)。點選看詳細資料(檔案、參照數),**Open** 在 Inspector 開 prefab / 材質 / 所屬資源,**Place in Scene** 或把格子**拖到 Scene**(落在指標下的地面)建立物件(模型 = 骨架模型物件、prefab = 實例、材質 = 套用材質的方塊),拖到 Inspector 的同類參照欄位(模型 / 材質 / 狀態機 / 音效片段)直接設定。**Find Usages** 列出每個參照(點列跳過去),**Rename…** 改 ID 並在同一筆修訂修正所有參照。右鍵選單有同樣的動作與 Copy reference。 |

每個區域都是可停靠的分頁(Hierarchy、Scene、Inspector、World 與 Changes / Proposals / History / Problems / Scenarios / Graphs / Assets / AI)。拖曳分頁標題可移到其他格或放在格子上方出現的上 / 下 / 左 / 右按鈕分割;拖到外面變成浮動視窗(Scene、Game 除外,它們由 renderer 直接畫在分頁位置)。分頁的 x 或中鍵關閉,Window 選單重新開啟(回到原本的位置附近)。**Window ▸ Layouts** 有內建的 Default(舊版的固定配置)與 Tall Inspector(Inspector 佔滿右側全高,適合 1080p 筆電),**Save Layout As…** 存成自己的名稱、**Reset Layout** 回到 Default。版面存在 `*.editor-prefs.json`(只影響這台機器的畫面,不進修訂);存檔內容有問題時改用 Default,狀態列會說明。Scene 分頁關閉或被同一格的其他分頁蓋住時不渲染 3D 視圖。

Assets 的縮圖:模型(靜止姿勢、材質顏色)與材質(以材質 tint 著色的球,**不是**材質的自訂 shader)由背景 job 在編輯器的 GPU 上離屏渲染後讀回,圖片在 CPU 解碼縮放;結果以內容雜湊存成 `<db>.thumbs/<hash>.png`(不進修訂,刪掉只會重新產生)。看得到的格子缺縮圖時自動排一個小 job(一次最多 24 個),**Thumbnails** 產生全部;進度顯示在工具列與狀態列,可 Cancel。貼圖最多同時保留 64 張(128×128,共 4 MiB),最久沒畫的先釋放,再出現時從磁碟快取載回;每幀最多上傳 4 張。沒有 GPU 時模型與材質縮圖標為失敗(橘點),不會卡住。AI:`assets [KIND]`、`usages KIND:ID`、`asset place|create|rename …`(`discover assets`),`editor_view {command:"asset.find_usages", arg:"model:hero"}`。

World 視窗的資源列會在 Inspector 載入資源草稿。可指定 JSON pointer 僅載入所需欄位，例如 `action.abilities` 的 `/abilities/slash2`。Preview、Apply、Reload 和 Copy JSON 沿用交易及修訂衝突規則。每次載入限制為 64 KiB，較大的資源可用更窄的 JSON pointer 檢查。

新增的物件有穩定 ID。名稱可更換，指向 ID 的引用會繼續有效。複製會複製單一物件與它的父層、場景成員關係。刪除物件時，子物件保留本地變換並成為根物件。交易會驗證仍指向已刪物件的型別化引用。重新掛父層時可保留世界姿態，若所需的本地變換含剪切，會回報診斷。

三軸拖曳在放開時提交一筆修訂，Esc 取消拖曳。Inspector 預設即時套用(Live):拖曳數值、取色、下拉、勾選或在文字欄按 Enter / 離開,一次互動提交一筆修訂(拖曳中不寫),燈光與網格的欄位拖曳時直接在場景預覽;Esc 在提交前取消。關掉 Live(Inspector 上方或 Preferences)則回到草稿 + Preview / Apply / Reload。Object 選單與命令面板的建立指令(Make Interactable、Add Tween Clip、Add Audio Source、Follow Path、Add Nav Agent、Camera Follow Selected、Audio source ▸ New clip…)先開選項對話框,下方顯示等價的 ACL;快捷鍵與 bridge 直接用預設值。Tween 時間軸的 Play on 把片段綁到事件。Undo／Redo 還原本視窗的提交，並追加新修訂。偵測到其他寫入者時，視窗會清除本地 Undo／Redo 堆疊，已提交的紀錄仍可透過 `history` 及 `undo REV` 存取。

F5 從已提交修訂啟動固定步進的 Play。編輯器的選單、Hierarchy、Inspector 與其他分頁都留著,遊戲畫面與遊戲的 `ui.document` HUD 畫在 Game 分頁裡(Play 時自動切到 Game)。在遊戲畫面上點一下,鍵盤與(遊戲要求時)鎖定的滑鼠才交給遊戲,分頁邊框變成青色;Esc、點編輯器其他地方或切走 Game 分頁就收回。滑鼠移到遊戲畫面上時,遊戲收到滑鼠事件(例如點地面走路)。Play 期間編輯器唯讀:會改內容的指令變灰並說明「stop Play first」,Inspector 上方顯示 **Live (read-only)**:runtime 的位置 / 角度 / 縮放與各系統 `*.inspect` 的結果(每秒約 4 次),下面的編輯值不能改。AI 仍可用協定直接寫修訂(Stop 後才看得到)。Shift+F5 停止;結束時捨棄模擬狀態並載入最新可啟用的修訂。預覽期間固定使用啟動時的版本。支援的預覽存檔會寫入資料庫專用的 `*.preview-saves`。工具列的 **Maximized**(Play ▸ Play Maximized)保留舊的整個視窗預覽(隱藏編輯器,F8 停止,Esc 暫停),適合需要 Esc 鍵的遊戲。

選取有 `render.camera` 的物件時,Inspector 最上方出現 **Camera Preview**(那台相機現在看到的畫面,不論是否為主相機);勾選框或 View ▸ Toggle Inspector Camera Preview 開關。它只在 Inspector 停靠且看得到時多畫一個 view;浮動視窗裡的 Inspector 不顯示預覽。

編輯相機顯示啟用場景及全域物件。停用場景的物件仍可從場景樹檢查。視口可點選方塊組合網格，其他空間物件可透過原點標記或場景樹選取。

## 選單、命令面板與快捷鍵

視窗頂端是選單列(File / Edit / View / Object / Assets / World / Play / Window / Help),最底下是狀態列(訊息、選取數、問題數、待審提案數、REV、bridge 位址、FPS)。所有選單項目、快捷鍵、命令面板(**Ctrl+Shift+P**,輸入關鍵字後按 Enter)與 AI 的遠端指令都來自同一份指令註冊表,行為完全相同。無法執行的指令會呈灰色,滑鼠停留時說明原因(例如「nothing selected」)。

常用快捷鍵:Q / W / E / R 切換工具、F 對焦、X 切換 Global / Local、Ctrl+Z / Ctrl+Y(或 Ctrl+Shift+Z)復原 / 重做、Ctrl+D 複製物件、Delete 刪除、Esc 取消選取、Ctrl+Shift+N 新增空物件、F5 Play、Ctrl+1…8 開啟 / 聚焦 Changes…AI 分頁、Ctrl+9 開關 World、Ctrl+, 偏好設定、F1 快捷鍵表。

**偏好設定**(Edit ▸ Preferences…,或場景工具列 Snap 旁的「…」)可調整 Snap 的移動 / 旋轉 / 縮放間距。偏好存在資料庫旁的 `*.editor-prefs.json`(例如 `data/editor.editor-prefs.json`),不會進入世界修訂。可在該檔的 `"shortcuts"` 改綁快捷鍵,例如 `{"edit.duplicate": "Ctrl+J"}`;空字串代表取消綁定。

Play 時工具列右側有 **■ Stop**(Shift+F5)、**Pause / Resume**(Ctrl+Alt+P)、**Step**(Ctrl+Alt+.,暫停並只前進一個 1/120 s 固定步)、時間倍率 0.25× / 0.5× / 1× / 2×(Play ▸ Time Scale;只改每秒跑幾步,每一步仍是同樣的 1/120 s,結果可重現)與 tick 數。遊戲有鍵盤時,編輯器只保留這些 Play 快捷鍵。Maximized 預覽右上角仍是 Stop(F8)、Pause / Resume(Esc)與 Step。

Game 分頁與 Play 的指令也在註冊表裡:`editor_view {"dock":"game"}`、`{"play":true}`、`{"command":"play.pause"}`、`{"command":"play.step"}`、`{"command":"play.time_scale","arg":"0.5"}`、`{"command":"game.aspect","arg":"4:3"}`、`{"command":"game.camera","arg":"cam2"}`、`{"command":"game.main_camera","arg":"cam2"}`;`observe` 的 `editor.play`(state / time_scale / tick / manual_steps)與 `editor.game`(visible / aspect / camera / rect / views / dropped_views / input / camera_preview)回報狀態(`discover editor_game`)。
Play 預覽右上角有 Stop(F8)、Pause / Resume(Esc)與 Step(暫停時前進一個固定步)。Behavior overlay 開著且 Graphs 有開啟的狀態機時,左下角顯示該機器的唯讀圖,每個 agent 目前所在的狀態變綠並標 `live N`。

**行為(Inspector ▸ Behavior)**:FSM Agent…(Machine 下拉、Radius)、Patrol…、Guard…、NPC Dialogue…,都先開選項對話框並顯示等價 ACL(`fsm SEL agent M`、`behavior SEL patrol|guard|dialogue …`);`fsm.agent` 元件標頭的 **Edit Graph** 在 Graphs 開啟它的機器。Problems 分頁的 STATE MACHINES 區段列出狀態機檢查(到不了的狀態、指向不存在狀態的轉移、`behavior.program` 沒宣告的進入事件、事件欄位不合、`fsm.agent` 指向不存在的機器),Open graph 直接跳到該節點。

AI 可以執行同一批指令:`discover editor_commands` 列出所有 ID,再以 `axiom --connect 127.0.0.1:4252 view --command edit.duplicate`(協定:`{"op":"editor_view","command":"edit.duplicate"}`)執行。有未存草稿或指令目前不可用時,會回傳原因而不是靜默略過。
## AI 與使用者共編

視窗預設提供本機 `127.0.0.1:4252` bridge。AI 可以按需取得 schema，查詢有分頁的階層資料，並用現有的 `select` 投影單一物件。以下命令可在編輯器開啟後執行。

```powershell
.\target\debug\axiom.exe --connect 127.0.0.1:4252 discover editor
'{"op":"scene_tree","query":{"scene":"assembly-yard","limit":12,"max_bytes":2048}}' | .\target\debug\axiom.exe --connect 127.0.0.1:4252 rpc
.\target\debug\axiom.exe --connect 127.0.0.1:4252 observe-delta --field /editor --field /rev --field /activation
```

1. 用 `scene_tree` 回傳的 `rev` 作為 `expected_rev`。支援依 `parent` 查直接子物件，或以 `roots_only` 查根物件。游標綁定查詢條件與修訂。
2. 用 `select` 查穩定 ID 與需要的元件欄位。使用 `set_field` 或 `reparent`、`duplicate_entity`、`delete_entity`、`set_scene_membership` 組成小批次。
3. 先送 `preview` 檢查差異，再用 `execute` 包裝 `apply`，以固定的 request ID 安全重試。同一筆交易由視窗與 AI 共用驗證器。
4. 以 `observe_delta` 檢查 `/editor/authoring_rev`、`/rev` 及 `/activation`。有未提交草稿時，外部新修訂會使草稿標記為過期。使用者可先複製草稿 JSON，再 Reload 讀取新修訂。

父層異動會出現在 `/relations/parent/@ID`，場景成員異動會出現在 `/scenes/@SCENE/entities/@ID`。路徑中的斜線會以 `~1` 跳脫。可用 `diff FROM TO` 分頁檢查完整差異。

## 外觀(M-UI 視覺改版)

- **主題**:深色底、粉紅強調色 `#ff4d6d`、圓角卡片;所有顏色集中在 `editor/theme.rs`(另有 `theme::style` 建出編輯器專用的 egui style,遊戲 UI 仍用自己的 style)。
- **圖示**:`assets/editor-icons/` 的 106 個原創 SVG,由 `scripts/build-editor-icons.ps1`(Edge 無頭模式)在建置時點陣化成一張 `atlas.png`(40 px 與 20 px 兩層,白色 + alpha)與 `atlas.json`。編輯器把 PNG 內嵌、啟動時解碼並上傳一次,以頂點色著色;查表是 `Icon` 列舉的索引。改了 SVG 要重跑腳本並提交新的 atlas(`icons::tests` 會檢查名稱與 atlas 一致)。
- **頂列**:Logo + Axiom、選單(File / Edit / View / Object / Assets / World / Play / Window / Help);第二列是圓角膠囊控制項:世界名稱 ▾(World Settings、Verify、Copy Context)、場景 ▾(Hierarchy 的場景篩選,只影響檢視)、`REV n`(Axiom 沒有「未存檔」:每次編輯都是背景寫入器提交的修訂;佇列中有 `writing N`)、Undo / Redo、Run(▾ 選 Game 分頁或 Play Maximized)/ Pause / Step / Stop、Agent API(bridge 連接埠、已處理的請求數、5 秒內有請求為 active;點一下開 AI / Protocol 分頁)、齒輪 = 偏好設定。
- **Hierarchy**:工具列 Add ▾、Duplicate、Delete、Rename、Create Group(`object.group`,Ctrl+G)、Frame,右側「…」= 場景管理與其他選取指令;搜尋框 + 篩選圖示(依場景篩選);每列有類型圖示(滑鼠提示列出全部元件類別)、眼睛 / 鎖(只在滑鼠經過、選取或已設定時顯示)。Prefab 實例以藍色顯示。
- **Scene 視口**:工具圖示(Select / Move / Rotate / Scale)、Snap 開關 + 增量下拉、Global / Local、格線、gizmo、視角下拉,分組成膠囊;3D 視圖內縮並畫上圓角外框;左下 FPS 標籤、右下 Frame 與 Fullscreen(`view.fullscreen`,F11,整個視窗無邊框全螢幕);軸向方塊移到右上。沒有 Rect 工具、Pivot 模式與著色模式下拉(renderer 沒有這些功能)。
- **Inspector**:Entity 卡片(名稱、ID + 複製、Prefab、Tags);Transform、Parent and scenes 為可摺疊卡片;每個元件一張卡片(展開箭頭、強調色的 Enabled 方塊、圖示、名稱;展開的卡片有 Reset、Play / Edit Graph 圖示與「…」選單:Reset、Play、Copy、Paste、Make Main Camera、Remove);一次只展開一張(表單草稿一次只有一個元件)。Components ▸ Advanced 可用 ID 載入任意元件。
- **Assets**:麵包屑(Assets › 類別或資料夾)、Grid / List、縮圖大小滑桿(48–144)、搜尋、類別篩選;格子標籤依寬度截斷,不再壓到鄰格。
- **Graphs**:State machine 工具列多一個 Frame(`graph.frame`):平移畫布讓全部狀態可見。沒有縮放。
- **狀態列**:狀態文字、場景(篩選中的或啟用中的)、物件數、選取數、Draw(世界 draw call + UI 網格數)、Tris(世界三角形 × instance)、GPU alloc(wgpu allocator 報告,每 2 秒更新;Vulkan 沒有報告時顯示 n/a)、Edit / Play Mode、bridge、FPS、版本。Draw / Tris 是 renderer 錄製指令時在 CPU 上計數,不是 GPU 時間。
- **面板外框**:每個停靠區是圓角、1 px 邊框的面板,面板之間留背景色間隙。分頁標題前有圖示;同一格只有一個分頁時顯示為面板標題,有多個時是膠囊形分頁(如 Scene / Game / Graphs)。分頁上不再有 X:每個面板右上角的「…」選單有 Close 與 Reset Layout(`window.close`、`layout.reset`;浮動視窗裡的分頁也有)。
- **預設版面(版面版本 2)**:Hierarchy | Scene、Game、Graphs 在上,Assets、Problems、History 在下 | 全高 Inspector。Changes、Proposals、Scenarios、AI / Protocol 預設關閉,從 Window 選單或 `window.*` / `editor_view {dock}` 開啟(開在下方分頁區);Graphs、UI Editor 開在 Scene 旁。舊版存下的目前版面會在第一次啟動時換成新 Default,原本的排列存成具名版面 **Previous**;舊的預設排列是內建的 **Classic**。Window › Layouts › Reset Layout 隨時回到 Default。
- **密度**:8 pt 間距格線、控制項 24 pt 高、Hierarchy 列高 26 pt、Inspector 欄位為「標籤欄 96 pt + 值」兩欄;收合的物件欄位在名稱後顯示前 36 個字元的摘要。
- QA:`AXIOM_WINDOW_SIZE=WxH`(實體像素)可指定編輯器 / 遊戲視窗大小,用於固定大小截圖。

## 驗證

```powershell
.\scripts\editor-smoke-test.ps1
```

腳本建立獨立 QA 資料庫，透過 bridge 驗證分頁、preview、冪等 apply、增量 observe 及 undo，並在 `artifacts/editor-*` 擷取編輯前後的原生 GPU 截圖。
