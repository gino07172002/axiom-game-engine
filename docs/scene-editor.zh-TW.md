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
| Graphs | **State machine**:上方下拉選 `fsm.machines` 的機器(或 New machine…),畫布上每個狀態一個節點(左側綠條 = 初始狀態,副標 = 動作 / 進入事件 / 片段,橘框加「!」= 檢查問題,滑鼠停留看說明),箭頭 = 轉移,標籤是條件,`*` 轉移從「Any state」出發。拖曳節點移動(放開時寫一筆修訂,位置存在 `editor.layout`,runtime 不讀);從節點右緣的圓點拖到另一個節點新增轉移;雙擊空白處新增狀態;拖空白處或中鍵平移;右鍵有 Set as initial state / Rename / Delete、Raise priority、Add state here。右側面板編輯選取的狀態(改名、Make initial、Action 下拉 + 型別化欄位、Emit on enter、Clip / loop)或轉移(From / To、When 下拉 + 欄位、Up / Down 優先順序),一次互動一筆修訂、Esc 取消。**Rules**:`behavior.program` 規則表,+ Rule、Up / Down、x,選取的規則可改 event、when(巢狀條件與 operand 各有 kind 下拉)、actions(+ Action)。**Overview**:舊的唯讀總覽(含 narrative)。Play 時目前狀態的節點變綠並標 `live N`。**Animator**:`animation.controllers` 的控制器(下拉選或 New controller…),同一個畫布:節點副標是 motion(clip / blend 參數: 片段…),轉移標籤是條件、exit 與淡入秒數。右側先是參數表(種類、預設值、`speed` 來源;Play 中顯示第一個受控實體的即時值,可拖曳或 Fire 送 `animation.param`),再是選取狀態的 Motion 表單或轉移的 From / To、Cross-fade、Exit time、Conditions。Play 時目前狀態變綠標 `live N`,淡入中的狀態底部有進度條。`animation.controller` 元件標頭的 Edit Graph 直接開到 Animator。 |
| Changes / AI | 檢視預覽、提交與外部修訂的局部差異,複製查詢或最近的交易 JSON。 |
| Assets | 資產瀏覽器:左側類型樹(模型 glb、圖片、材質、shader、prefab、FSM 狀態機、音效片段、UI 文件、輸入表,各附數量)與來源資料夾,上方搜尋、Grid / List 切換、New…、Thumbnails。格子 / 列表都是虛擬清單(只排版看得到的列)。點選看詳細資料(檔案、參照數),**Open** 在 Inspector 開 prefab / 材質 / 所屬資源,**Place in Scene** 或把格子**拖到 Scene**(落在指標下的地面)建立物件(模型 = 骨架模型物件、prefab = 實例、材質 = 套用材質的方塊),拖到 Inspector 的同類參照欄位(模型 / 材質 / 狀態機 / 音效片段)直接設定。**Find Usages** 列出每個參照(點列跳過去),**Rename…** 改 ID 並在同一筆修訂修正所有參照。右鍵選單有同樣的動作與 Copy reference。 |

每個區域都是可停靠的分頁(Hierarchy、Scene、Inspector、World 與 Changes / Proposals / History / Problems / Scenarios / Graphs / Assets / AI)。拖曳分頁標題可移到其他格或放在格子上方出現的上 / 下 / 左 / 右按鈕分割;拖到外面變成浮動視窗(Scene、Game 除外,它們由 renderer 直接畫在分頁位置)。分頁的 x 或中鍵關閉,Window 選單重新開啟(回到原本的位置附近)。**Window ▸ Layouts** 有內建的 Default(舊版的固定配置)與 Tall Inspector(Inspector 佔滿右側全高,適合 1080p 筆電),**Save Layout As…** 存成自己的名稱、**Reset Layout** 回到 Default。版面存在 `*.editor-prefs.json`(只影響這台機器的畫面,不進修訂);存檔內容有問題時改用 Default,狀態列會說明。Scene 分頁關閉或被同一格的其他分頁蓋住時不渲染 3D 視圖。

Assets 的縮圖:模型(靜止姿勢、材質顏色)與材質(以材質 tint 著色的球,**不是**材質的自訂 shader)由背景 job 在編輯器的 GPU 上離屏渲染後讀回,圖片在 CPU 解碼縮放;結果以內容雜湊存成 `<db>.thumbs/<hash>.png`(不進修訂,刪掉只會重新產生)。看得到的格子缺縮圖時自動排一個小 job(一次最多 24 個),**Thumbnails** 產生全部;進度顯示在工具列與狀態列,可 Cancel。貼圖最多同時保留 64 張(128×128,共 4 MiB),最久沒畫的先釋放,再出現時從磁碟快取載回;每幀最多上傳 4 張。沒有 GPU 時模型與材質縮圖標為失敗(橘點),不會卡住。AI:`assets [KIND]`、`usages KIND:ID`、`asset place|create|rename …`(`discover assets`),`editor_view {command:"asset.find_usages", arg:"model:hero"}`。

World 視窗的資源列會在 Inspector 載入資源草稿。可指定 JSON pointer 僅載入所需欄位，例如 `action.abilities` 的 `/abilities/slash2`。Preview、Apply、Reload 和 Copy JSON 沿用交易及修訂衝突規則。每次載入限制為 64 KiB，較大的資源可用更窄的 JSON pointer 檢查。

新增的物件有穩定 ID。名稱可更換，指向 ID 的引用會繼續有效。複製會複製單一物件與它的父層、場景成員關係。刪除物件時，子物件保留本地變換並成為根物件。交易會驗證仍指向已刪物件的型別化引用。重新掛父層時可保留世界姿態，若所需的本地變換含剪切，會回報診斷。

三軸拖曳在放開時提交一筆修訂，Esc 取消拖曳。Inspector 預設即時套用(Live):拖曳數值、取色、下拉、勾選或在文字欄按 Enter / 離開,一次互動提交一筆修訂(拖曳中不寫),燈光與網格的欄位拖曳時直接在場景預覽;Esc 在提交前取消。關掉 Live(Inspector 上方或 Preferences)則回到草稿 + Preview / Apply / Reload。Object 選單與命令面板的建立指令(Make Interactable、Add Tween Clip、Add Audio Source、Follow Path、Add Nav Agent、Add Rigid Body、Camera Follow Selected、Audio source ▸ New clip…)先開選項對話框,下方顯示等價的 ACL;快捷鍵與 bridge 直接用預設值。Tween 時間軸的 Play on 把片段綁到事件。場景視口的元件 gizmo 畫出 `physics.collider` 模擬用的形狀(箱子 / 膠囊 / 球,含縮放):靜態綠色、動態剛體藍色、運動學剛體與跟隨者琥珀色,Play 時睡著的剛體變暗;Object ▸ Add Rigid Body 見 `docs/physics.zh-TW.md`。Undo／Redo 還原本視窗的提交，並追加新修訂。偵測到其他寫入者時，視窗會清除本地 Undo／Redo 堆疊，已提交的紀錄仍可透過 `history` 及 `undo REV` 存取。

F5 從已提交修訂啟動固定步進的 Play。編輯器的選單、Hierarchy、Inspector 與其他分頁都留著,遊戲畫面與遊戲的 `ui.document` HUD 畫在 Game 分頁裡(Play 時自動切到 Game)。在遊戲畫面上點一下,鍵盤與(遊戲要求時)鎖定的滑鼠才交給遊戲,分頁邊框變成青色;Esc、點編輯器其他地方或切走 Game 分頁就收回。滑鼠移到遊戲畫面上時,遊戲收到滑鼠事件(例如點地面走路)。Play 期間編輯器唯讀:會改內容的指令變灰並說明「stop Play first」,Inspector 上方顯示 **Live (read-only)**:runtime 的位置 / 角度 / 縮放與各系統 `*.inspect` 的結果(每秒約 4 次),下面的編輯值不能改。AI 仍可用協定直接寫修訂(Stop 後才看得到)。Shift+F5 停止;結束時捨棄模擬狀態並載入最新可啟用的修訂。預覽期間固定使用啟動時的版本。支援的預覽存檔會寫入資料庫專用的 `*.preview-saves`。工具列的 **Maximized**(Play ▸ Play Maximized)保留舊的整個視窗預覽(隱藏編輯器,F8 停止,Esc 暫停),適合需要 Esc 鍵的遊戲。

選取有 `render.camera` 的物件時,Inspector 最上方出現 **Camera Preview**(那台相機現在看到的畫面,不論是否為主相機);勾選框或 View ▸ Toggle Inspector Camera Preview 開關。它只在 Inspector 停靠且看得到時多畫一個 view;浮動視窗裡的 Inspector 不顯示預覽。

編輯相機顯示啟用場景及全域物件。停用場景的物件仍可從場景樹檢查。視口可點選方塊組合網格，其他空間物件可透過原點標記或場景樹選取。

## 選單、命令面板與快捷鍵

視窗頂端是選單列(File / Edit / View / Object / Assets / World / Play / Window / Help),最底下是狀態列(訊息、選取數、問題數、待審提案數、REV、bridge 位址、FPS)。所有選單項目、快捷鍵、命令面板(**Ctrl+Shift+P**,輸入關鍵字後按 Enter)與 AI 的遠端指令都來自同一份指令註冊表,行為完全相同。無法執行的指令會呈灰色,滑鼠停留時說明原因(例如「nothing selected」)。

常用快捷鍵:Q / W / E / R 切換工具、F 對焦、X 切換 Global / Local、Ctrl+Z / Ctrl+Y(或 Ctrl+Shift+Z)復原 / 重做、Ctrl+D 複製物件、Delete 刪除、Esc 取消選取、Ctrl+Shift+N 新增空物件、F5 Play、Ctrl+1…8 開啟 / 聚焦 Changes…AI 分頁、Ctrl+Shift+1 / 2 / 3 開啟 Console / Profiler / Event Trace、Ctrl+9 開關 World、Ctrl+, 偏好設定、F1 快捷鍵表。

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

### 共編強化(M10)

- **History 標示作者**:每筆修訂前面有標記,**You**(編輯器裡的人)、**AI**(bridge、CLI、或透過 bridge 送來的 `editor_view {command}`)、**#N**(核准的提案 N;滑鼠提示寫出核准者)、**—**(initialize、遷移、舊版資料庫沒有記錄)。作者存在 `revisions.author` 欄位(舊資料庫開啟時自動加上可為 NULL 的欄位,舊版程式照常寫入);提案由既有的 `proposals.resolved_rev` 對應,沒有新欄位。協定 `history` 的列多了 `author` / `proposal`(沒有時省略),ACL `history` 每行結尾 `by=human|agent proposal=#N`。
- **幽靈框**:Proposals 分頁 focus 的提案(新提案進來時自動 focus)或 History 點選的 revision,在 Scene 視口畫出變更物件的邊界:白框 = 原本的位置、天藍框 = 之後的位置(中間一條線)、綠 = 新增、紅 = 刪除、橘 = 位置沒變但其他欄位變了;左上角寫出來源與物件數。只是預覽,不寫入。提案的「之後」= 目前 revision 套用提案的 edit(就是核准會得到的結果)。計算(解碼、套用、編譯兩份世界)在背景 job 裡,依(來源, revision)只算一次;每幀只畫線段,最多 64 個框(其餘只計數)。沒有 `render.mesh` 零件的物件(glTF 模型、空物件)畫 0.5 m 的樞紐框。開關:Proposals 的 **Ghosts**、`view.ghosts`(偏好 `ghosts`);AI:`editor_view {command:"ghost.show", arg:"proposal:N"|"rev:N"|"off"}`,`observe` 的 `editor.ghosts`(source / state / boxes / total / drawn)。
- **逐欄衝突合併**:兩種情況。(1) 過期的提案(base 落後):Proposals ▸ **Merge…**。(2) Inspector 的草稿(批次模式或未提交)在其他寫入者提交後過期:Inspector 的「Stale draft」旁 **Merge…**。合併視窗列出雙方都改了、而且改得不一樣的欄位(Field / Base / Mine 或 Proposal / Theirs 或 Current),點格子選要保留哪一邊,All mine / All theirs,**Commit merge** = 一筆 revision(提案 = `resolve_proposal {keep}` 並標成 approved;草稿 = 一筆 apply)。只有一方改的欄位直接合併。合併單位:物件的 label / prefab / tags、元件與資源的每個頂層欄位(一方新增或刪除整個元件 / 資源 / 物件時是整個)、一條父層或關係、物件的相對順序,其餘頂層項目(場景、prefab、schema、capability、systems)整個算一個。列出衝突與建立合併都在背景 job;按 Commit 時(或背景算好、要送出寫入時)若又有新 revision、還有編輯正在存、或草稿在列出衝突後又被改過,會重新列出衝突而不是寫入;編輯正在存時不能開始合併。協定的 `keep` 一定要帶 `expected_rev`(列出衝突時的 revision),之後又有新 revision 就拒絕(`revision_conflict`),不會替人決定沒看過的衝突。AI:`acl -e "conflicts N"`,再送 `{"op":"resolve_proposal","id":N,"approve":true,"keep":[PATH..],"expected_rev":R}`(CLI `resolve N --approve --merge --keep PATH --expected-rev R`);編輯器指令 `merge.open proposal:N|draft`、`merge.pick PATH=mine|theirs|all=mine|all=theirs`、`merge.commit`、`merge.cancel`;`observe` 的 `editor.merge`。
- **把選取交給 AI**:Hierarchy 列與 Scene 視口物件的右鍵選單都有 **Copy Context for AI**(`file.copy_context`,Ctrl+Shift+C),內容與 `inspect ID` 相同的精簡描述加上編輯提示。視口右鍵(沒有拖曳環繞、這次按住期間沒有 WASD 飛行)會先選取該物件,選單另有 Copy ID、Focus、Rename、Duplicate、Delete、Create Group、Hide / Lock;Esc 或點選單外關閉。

## 外觀(M-UI 視覺改版)

- **主題**:深色底、粉紅強調色 `#ff4d6d`、圓角卡片;所有顏色集中在 `editor/theme.rs`(另有 `theme::style` 建出編輯器專用的 egui style,遊戲 UI 仍用自己的 style)。
- **圖示**:`assets/editor-icons/` 的 106 個原創 SVG,由 `scripts/build-editor-icons.ps1`(Edge 無頭模式)在建置時點陣化成一張 `atlas.png`(40 px 與 20 px 兩層,白色 + alpha)與 `atlas.json`。編輯器把 PNG 內嵌、啟動時解碼並上傳一次,以頂點色著色;查表是 `Icon` 列舉的索引。改了 SVG 要重跑腳本並提交新的 atlas(`icons::tests` 會檢查名稱與 atlas 一致)。
- **頂列**:Logo + Axiom、選單(File / Edit / View / Object / Assets / World / Play / Window / Help);第二列是圓角膠囊控制項:世界名稱 ▾(World Settings、Verify、Copy Context)、場景 ▾(Hierarchy 的場景篩選,只影響檢視)、`REV n`(Axiom 沒有「未存檔」:每次編輯都是背景寫入器提交的修訂;佇列中有 `writing N`)、Undo / Redo、Run(▾ 選 Game 分頁或 Play Maximized)/ Pause / Step / Stop、Agent API(bridge 連接埠、已處理的請求數、5 秒內有請求為 active;點一下開 AI / Protocol 分頁)、齒輪 = 偏好設定。
- **Hierarchy**:工具列 Add ▾、Duplicate、Delete、Rename、Create Group(`object.group`,Ctrl+G)、Frame,右側「…」= 場景管理與其他選取指令;搜尋框 + 篩選圖示(依場景篩選);每列有類型圖示(滑鼠提示列出全部元件類別)、眼睛 / 鎖(只在滑鼠經過、選取或已設定時顯示)。Prefab 實例以藍色顯示。
- **Scene 視口**:工具圖示(Select / Move / Rotate / Scale)、Snap 開關 + 增量下拉、Global / Local、格線、gizmo、視角下拉,分組成膠囊;3D 視圖內縮並畫上圓角外框;左下 FPS 標籤、右下 Frame 與 Fullscreen(`view.fullscreen`,F11,整個視窗無邊框全螢幕);軸向方塊移到右上;截圖鈕與 Lit / Unlit 顯示模式(見下方「外觀(工作線 E)」)。沒有 Rect 工具、Pivot 模式與 Wireframe。
- **Inspector**:Entity 卡片(名稱、ID + 複製、Prefab、Tags);Transform、Parent and scenes 為可摺疊卡片;每個元件一張卡片(展開箭頭、強調色的 Enabled 方塊、圖示、名稱;每張卡片都有 Reset(Revert)圖示與「…」選單:Reset、Copy、Paste、Remove;Play / Edit Graph 圖示與 Make Main Camera 只在展開的卡片(工作線 E 起));一次只展開一張(表單草稿一次只有一個元件)。Components ▸ Load component by ID(原本叫 Advanced,工作線 E 改名,避免和卡片內的 Advanced 摘要列混淆)可用 ID 載入任意元件。
- **Assets**:麵包屑(Assets › 類別或資料夾)、Grid / List、縮圖大小滑桿(48–144)、搜尋、類別篩選;格子標籤依寬度截斷,不再壓到鄰格。
- **Graphs**:State machine 工具列多一個 Frame(`graph.frame`):平移畫布讓全部狀態可見。沒有縮放。
- **狀態列**:狀態文字、場景(篩選中的或啟用中的)、物件數、選取數、Draw(世界 draw call + UI 網格數)、Tris(世界三角形 × instance)、VRAM(本程序在繪圖 adapter 上的專用顯示記憶體,DXGI,每 2 秒更新;見下方「外觀(工作線 E)」)、Edit / Play Mode、bridge、FPS、版本。Draw / Tris 是 renderer 錄製指令時在 CPU 上計數,不是 GPU 時間。
- **面板外框**:每個停靠區是圓角、1 px 邊框的面板,面板之間留背景色間隙。分頁標題前有圖示;同一格只有一個分頁時顯示為面板標題,有多個時是膠囊形分頁(如 Scene / Game / Graphs)。分頁上不再有 X:每個面板右上角的「…」選單有 Close 與 Reset Layout(`window.close`、`layout.reset`;浮動視窗裡的分頁也有)。
- **預設版面(版面版本 3,M8 加入 Console)**:Hierarchy | Scene、Game、Graphs 在上,Assets、Console、Problems、History 在下 | 全高 Inspector。Changes、Proposals、Scenarios、AI / Protocol、Profiler、Event Trace 預設關閉,從 Window 選單或 `window.*` / `editor_view {dock}` 開啟(開在下方分頁區);Graphs、UI Editor 開在 Scene 旁。舊版存下的目前版面會在第一次啟動時換成新 Default,原本的排列存成具名版面 **Previous**;舊的預設排列是內建的 **Classic**。Window › Layouts › Reset Layout 隨時回到 Default。
- **密度**:8 pt 間距格線、控制項 24 pt 高、Hierarchy 列高 26 pt、Inspector 欄位為「標籤欄 96 pt + 值」兩欄;收合的物件欄位在名稱後顯示前 36 個字元的摘要。
- QA:`AXIOM_WINDOW_SIZE=WxH`(實體像素)可指定編輯器 / 遊戲視窗大小,用於固定大小截圖。

## 外觀(工作線 E,2026-11-11)

- **示範場景 Sakura Plaza**(`content/plaza`,`Edit Axiom.cmd` 之外用 `axiom --db data/plaza.sqlite --pack content/plaza editor` 開):只用引擎方塊與第一方吉祥物模型(`content/mascot` 的兩個 glb,同一份檔案,git 只存一次)。Hierarchy 是分層資料夾:Environment(PlazaFloor、Buildings、Props、Vegetation)、Lighting、Characters、Cameras;資料夾就是 `group` 建立的空物件(`ops::create_group`),展開時圖示是打開的資料夾。內容由 `content/plaza/build/*.acl` 經 `scripts/build-plaza.ps1` 產生(每個階段一筆 revision,最後 verify + export;建置時模型路徑相對工作目錄,匯出時改成相對內容包)。
- **Inspector 卡片**:Components 標題列右側是 **+ Add Component…**(元件清單,下面是 Behavior 範本:FSM Agent / Patrol / Guard / NPC Dialogue);**Transform 是第一張卡片**,標題右側 Revert(`reset SEL core.transform`:原點、無旋轉、縮放 1,prefab 實例回到 prefab 的姿勢)與「…」(Reset、Copy values、Paste values);X / Y / Z 字母用 gizmo 的紅綠藍。每張元件卡片(不只展開的那張)標題都有 Revert 與「…」(Reset、Copy、Paste、Remove;Make Main Camera、Play、Edit Graph 只在展開的卡片)。**啟用勾選**只出現在元件本身有 `enabled` 欄位的卡片(例如 `render.light`);收合的卡片也能勾,等於展開它並改 `enabled`,Live 時一筆 revision(= `set SEL COMPONENT/enabled B`)。模型沒有通用的元件啟用旗標,沒有為其他元件新增(要 runtime 每個系統都尊重才有意義)。**Advanced 摘要列**:展開的卡片裡,已提交的值仍等於元件預設值(`ops::default_value`)的頂層欄位收進「Advanced (N): a, b, …」一列,點開才顯示;多選值不同、prefab 覆寫或篩選欄位時不收;至少留兩個欄位在外、至少收兩個才出現。**資源參照欄位**(模型、材質、狀態機、音效片段等 `x-ref key:` 欄位):下拉顯示資源名稱(= 挑選),旁邊 Copy(複製 `kind:id`)與 Show in Assets(`asset.reveal KIND:ID`)。Parent and scenes 移到元件卡片之後。
- **Assets 資料夾樹**:左側 `Assets/` ▸ 類別(只列有資源的)▸ 模型 / 圖片的來源資料夾(最後兩層,例如 `Models/content/plaza`)或 ID 的 `/` 前綴(例如 `Prefabs/props`),箭頭收合、資料夾圖示、數量。點資料夾 = `asset.folder PATH`;上方麵包屑 `Assets › Prefabs › props` 每段都可點。沒有圖片縮圖的格子畫類別圖示(FSM、音效、UI、輸入、shader),角落小字是類別。**Prefab 縮圖**:`render.mesh` 部件的方塊以各自顏色離屏渲染(同一個背景 job、同一個內容雜湊磁碟快取);模型 prefab 用該模型的縮圖。
- **Scene 工具列**:截圖鈕(`view.screenshot [NAME]`)在下一個畫出的幀讀回整個視窗,裁成 Scene 視圖(看不到 Scene 時用整個視窗),讀回緩衝非同步 map 後由背景 job(20 秒 watchdog、可取消、關閉時最多等 500 ms)裁切、PNG 編碼並寫到 `artifacts/captures/NAME.png`(預設 `scene-<毫秒>`),完成後狀態列顯示路徑;主執行緒不等 GPU(見下一項的量測)。**Lit / Unlit**(`view.display_mode lit|unlit`,不帶參數 = 切換):Unlit 只作用在編輯器的 Scene 視圖(Game 分頁與 Play 仍是正常光照),做法是那個 view 的 uniform:一個均勻白色環境光、沒有太陽(所以沒有陰影與高光)、沒有區域光源、沒有霧;顏色仍經過 shader 的 tone curve,所以是「平塗底色」而不是原始貼圖值,自訂材質 surface hook 收到的太陽方向與漫射項是 0、沒有陰影(hook 自己的計算照舊,所以不一定是平塗)。**沒有 Wireframe**:wgpu 的 `POLYGON_MODE_LINE` 需要為每一條世界 pipeline(方塊、網格 / 骨架、每個自訂材質 shader)各建一個線框版本,pipeline 數加倍,而且繪圖工作線同時在改這些 pipeline;CPU 畫三角形邊又是每幀大量線段。兩者都不便宜,所以只做 Lit / Unlit。
- **狀態列 VRAM**:以前的「GPU alloc n/a」是因為 wgpu 24 只有 D3D12 後端(gpu-allocator)回報 allocator report,這台 MX150 走 Vulkan(gpu-alloc)一律 `None`。現在讀 Windows 的 DXGI `IDXGIAdapter3::QueryVideoMemoryInfo`(依 PCI vendor / device 找到同一張卡):本程序在這張卡上用的專用顯示記憶體,含 swapchain 與驅動配置,不分 API。顯示 `VRAM 312 MiB`(≥ 1 GiB 顯示 `1.26 GiB`),滑鼠提示有預算、共用系統記憶體與 wgpu allocator 總數(有的話)。讀不到時顯示 `VRAM –`,原因寫在提示(非 Windows、找不到對應的 DXGI adapter、軟體 adapter …);有 allocator 報告時改顯示 `GPU alloc N MiB`。`observe` 的 `editor.vram`(`source:"dxgi"`、`local_bytes`、`shared_bytes`、`budget_bytes`,或 `available:false` + `reason`)。
- **即時截圖(loopback,缺口 E3)**:執行中的遊戲(`axiom play --port P`)或編輯器都接受 `{"op":"screenshot","path":"shots/a.png","region":"window|view"}`:不用重開程序、不用 `--smoke-frames`。`path` 必須是 `.png`、不能有 `..`,相對路徑以引擎的工作目錄為準(也可用絕對路徑),資料夾自動建立;`region` 預設 `window`(整個視窗),`view` = 編輯器的 Scene 視圖(看不到時是 Game 分頁,再不然整個視窗)。一次只能有一個截圖,第二個在前一個寫完前回 `a screenshot is already being captured`;下一個畫出的幀讀回後由背景 job 裁切 / 轉 RGBA / 寫 PNG,寫完才回覆 `{ok, path, width, height, region, ms}`(失敗時與其他錯誤同格式:`error` + `diagnostics` + `path`)。bridge 與 `--connect` 對這個 op 最多等 32 秒(10 秒等幀 + 20 秒編碼 watchdog);引擎還在畫幀但 10 秒內沒有一幀接手時回錯誤;視窗最小化而完全不畫幀時,用戶端在 32 秒後逾時。截圖只拿「請求之後」錄下的幀(`GrabOwner`),`view` 的裁切範圍在請求被排入那一幀決定。路徑只接受相對路徑或本機磁碟的絕對路徑(拒絕 `C:a.png`、`\a.png`、UNC `\\host\share`、`\\.\`、`\\?\` 與 CON / NUL / COM1 等保留名稱)。遊戲內 Console 不能送這個 op(會說明要用 loopback)。注意:舊的 `editor_view {capture}`(smoke 腳本用)仍是同步讀回,會讓那一幀多停數百 ms(debug)。主執行緒不等 GPU:那一幀只多錄一個複製到讀回緩衝的指令,緩衝以非同步 map 在之後的幀交給背景 job(裁切直接從 mapped 範圍複製)。實測(debug,2880×1620,Sakura Plaza):抓取幀 3–55 ms(同時段一般幀 19–63 ms)、交接 0.01 ms;改成非同步前同步讀回是 434–498 ms。PNG 寫完約 0.9–1.1 s(背景)。stderr 每次截圖記錄 `screenshot: grab frame rendered in N ms` 與 `frame handed over in N ms`。編輯器指令 `view.screenshot` 也接受路徑(`arg:"shots/a.png"`;只寫 NAME 時仍是 `artifacts/captures/NAME.png`)。
- AI:`editor_view {command:"view.display_mode", arg:"unlit"}`、`{command:"view.screenshot", arg:"NAME"}`、`{command:"asset.folder", arg:"Prefabs/props"}`、`{command:"asset.reveal", arg:"model:axiom-long"}`;`observe` 的 `editor.look`(`display_mode`、`screenshot {pending, last}`、`assets {folder, selected}`)。

## 除錯工具(M8)

- **Console**(下方分頁,預設在 Assets 旁;Window ▸ Console、Ctrl+Shift+1):編輯器的狀態訊息與失敗請求(Problems 的同一份診斷)、runtime 錯誤(activation、gameplay、跨套件事件)、遊戲自己的輸出(UI 指令失敗、存讀檔結果)與 Play 時的 gameplay / 跨套件事件。每列有層級圖示、tick、來源、文字;連續相同的訊息合併成「×N」。上方可依層級(info / warn / error,顯示數量)、來源(editor / runtime / game / event)、文字(也比對物件 ID,不分大小寫)篩選,垃圾桶 = 清除。列尾有「→ ID」的列點一下就選取該物件(在 Play 中也行,Inspector 顯示它的即時狀態)。分頁標題顯示目前保留的錯誤數。固定保留 512 筆,較舊的丟掉並計數。
- **Event Trace**(Window ▸ Event Trace、Ctrl+Shift+3):Play 時 runtime 的事件流,依 tick 分組(「tick N · Play #k」標頭):gameplay 事件、通過條件的規則、敘事步驟(node / scene / ending / choice / activation)、跨套件事件(含欄位)與 FSM 狀態轉換(哪個 agent 進入哪個狀態)。可依種類與文字篩選、暫停(暫停期間的事件只計數,不保留)、清除;點有物件的列 = 選取。固定保留 2048 筆。runtime 的 gameplay trace 本身只留 128 筆,同一幀內超過時標示「missed」。
- **Profiler**(Window ▸ Profiler、Ctrl+Shift+2):只在分頁看得到時量測。上方標出 debug / release 建置(debug 慢數倍,只比較 release 的數字);滾動長條圖是最近 240 幀主執行緒 CPU 時間(灰色 = 等待 swapchain,線 = 16.7 ms);表格是每個主迴圈階段(writes_bridge、poll_reload_proposals、simulate、jobs_overlay_ui、actions_history_audio、extract、render_present、observation)的本幀 / 平均 / 最大值,以及 Play 時每個 runtime 系統 `step()` 的耗時(本幀這些固定步的總和、平均、最近 2 秒的峰值)。**GPU 計時(timestamp queries)**:adapter 支援 `TIMESTAMP_QUERY` 與 `TIMESTAMP_QUERY_INSIDE_ENCODERS` 時,renderer 只在 Profiler 可見(且未暫停)時,於各組 pass 之間寫 timestamp:vfx_compute、shadows、world(每個相機視圖)、vfx_draw、post、game_ui、ui;結果用 4 格的 query set / 讀回緩衝環非同步讀回(晚 1–3 幀,主執行緒從不等 GPU;每格都還在讀回時該幀不量,計入「not measured」)。分頁另起一張「GPU (timestamp queries)」表:各組本幀 / 平均 / 最大 ms 與 frame GPU 總和。不支援時明確寫「GPU timing unavailable: 原因」(沒有 TIMESTAMP_QUERY、缺 INSIDE_ENCODERS、或 `AXIOM_GPU_TIMING=0` 關閉);`render_present` 仍是 CPU 端的編碼、送出與 present。分頁標題列另顯示 GPU 平均 / 最大 ms;寬的下方格時 CPU 階段表與 GPU 表左右並排;**矮的下方格**(本體不到 300 點,例如預設版面在 1366×768 約 110 點、1920×1080 約 240 點)改成上方一條四列的 GPU 橫表(欄 = 各組 pass + frame,列 = 名稱 / last / mean / max),下面是 28 點高的細長條圖(說明移到滑過提示)與 CPU 表,GPU 每組的數字不必捲動就看得到(太窄放不下 8 欄時仍用直式表)。`observe` 的 `editor.debug.profiler.cpu`(`frame_mean_ms`、`phase_mean_ms`)與 `editor.debug.profiler.gpu`:`available`、`reason` 或 `source:"timestamp_queries"`、`frames`、`dropped`、`mean_ms{組}`、`total_mean_ms`、`total_max_ms`、`last_total_ms`。
- **Play 中的唯讀 Inspector**:M4 的 Live 區段(transform、各系統 `*.inspect` 結果,約每 250 ms 更新)另加 health。
- **Scenario 錄製**(Play ▸ Play and Record Scenario、Play 工具列的 ● 按鈕):沒在 Play 時按下 = 開始 Play 並錄製;錄製中頂列顯示「● REC N ticks」,再按一次 = 停止錄製並寫檔(Play 繼續);Stop / 遊戲的 Restart 也會先寫檔。錄下每個固定步之前改變的動作值(放開 = 0)與成功的 runtime 指令(遊戲 UI、bridge `command`)、存讀檔,寫成 `examples/NAME.scenario.json`(預設名稱 `世界ID-recording-N`,不會覆蓋既有檔案),出現在 Scenarios 分頁。格式就是 `axiom scenario` 讀的格式;檔案帶 `finite` 與 `state_hash` 斷言(錄製結束時 Play runtime 的雜湊),所以 `axiom --db DB scenario examples/NAME.scenario.json` 重播後會自己檢查狀態是否完全相同。滑鼠視角(捕捉游標時的原始 delta)記成 `{"op":"look","deltas":[[dx,dy],…]}`,會被系統使用的螢幕指標事件(點地移動、RTS 框選 / 右鍵)記成 `{"op":"pointer","kind":"primary_down","screen":[x,y],"aspect":a,"additive":false}`(遊戲區域的正規化座標 + 長寬比,重播時由當下相機重新算射線;move 只在按住主鍵拖曳時記錄,同一 tick 連續的 move 只留最後一個),與指令同一清單、保留先後;最後一個 tick 之後才發生的操作記在 tick = ticks,重播在最後一步之後執行。設計見 `docs/adr-scenario-mouse-input.zh-TW.md`。不能精確重播時(讀取錄製開始前的存檔、錄製中 revision 改變、指標事件失敗、預算用盡)仍會寫檔,但不加 `state_hash` 斷言,並在狀態列與 Console(warn)說明原因。上限與 `axiom scenario` 相同:100 萬 tick、4096 個輸入變化、4096 個指令 / 操作、1 MiB(輸入、指令與滑鼠操作都計入檔案大小上限,可能超過 1 MiB 前就停止錄製)。
- AI 用同一批指令:`window.console|profiler|trace`、`console.filter levels=warn,error source=event text=lumi`、`console.open SEQ`、`console.clear`、`trace.pause`、`trace.filter kinds=state,event text=TEXT`、`trace.open SEQ`、`trace.clear`、`profiler.pause`、`profiler.reset`、`play.record [NAME]`;`observe` 的 `editor.debug` 回報 Console 筆數 / 錯誤數 / 最後一筆錯誤、trace 筆數 / 暫停 / 略過、Profiler 是否在量、錄製中的狀態與上一次錄製的結果(路徑、ticks、state_hash、是否精確)。

## 專用編輯器(M9)

四個分頁預設關閉,從 Window 選單、`window.input_map` / `window.localization` / `window.animation` / `window.vfx` 或 `editor_view {"dock":"…"}` 開啟(開在下方分頁區)。每個會改內容的操作都是一個 `axiom_core::ops` 函式,和 ACL 動詞共用(`input` / `loc` / `ability` / `vfx` / `shader`),registry 指令 `input.edit` / `loc.edit` / `ability.edit` / `vfx.edit` / `shader.edit` 直接吃 ACL 參數;Play 中一律停用。對照見 [editor-parity](editor-parity.zh-TW.md)。

- **Input Map**(`input.bindings`):按鈕與軸(+ / −)的表格。點按鍵格會顯示「press a key…」,下一個按下的鍵(實體鍵位置,存成 runtime 用的 winit `KeyCode` 名稱,例如 `KeyW`、`Space`、`Digit1`;單獨按 Shift / Ctrl / Alt 存成 `ShiftLeft` / `ControlLeft` / `AltLeft`)就是新綁定,一筆 revision;也可按 Mouse Left。Esc 取消;開始在別的文字欄位打字、進入 Play 也會取消,分頁被藏起來後再顯示時重新等待按鍵。擷取期間編輯器快捷鍵暫停(按 W 不會切換工具)。限制:egui 把數字鍵盤的 0–9 / Enter / - / / 回報成主鍵盤的同一個鍵,所以擷取會存成 `Digit1` 等;要綁 `Numpad1` 請用 `input.edit` / ACL 直接寫名稱(數字鍵盤 + 無法擷取)。ACL 的單一字母 / 數字(`W`、`1`)會被拒絕並提示 `KeyW` / `Digit1`。輸入名稱後 + Button / + Axis 新增動作(軸依序按正向、負向鍵,一筆 revision)。同一個鍵綁在多個動作上會標成橘色並列在上方(runtime 允許,所以是警告)。
- **Localization**(`ui.document` 的 catalogs):keys × locales 表格(虛擬清單,模型依修訂快取)。點格子編輯,Enter 或點別處提交一筆,Esc 取消。空字串或仍等於 key 的文字標成缺字(紅),和預設語系相同的文字標成未翻譯(橘);每個語系標題顯示計數。篩選框比對 key 與所有語系的文字,Missing only 只列有問題的列。+ Key(預設語系填文字,其他語系空白)、列尾刪除 key、+ Locale(空白)、語系標題 x 刪除(預設語系不能刪)。
- **Animation**:選模型(有 `animation.player` 的物件,選取物件時跟著換)與 clip。拖曳尺規或 clip 列會移動播放頭,Scene 視圖顯示模型在該時間的姿勢(只限當幀與編輯模式,不寫入,模擬的 controller 在擷取後還原);眼睛按鈕關閉預覽。事件點是引擎已有的 clip 同步事件:`action.abilities` 的 ability 把 clip 拉伸到 `duration` tick 播放,`events [{tick, sound}]` 在指定 tick 播 `audio.library` 的 cue。每個使用這個 clip 的 ability 一列:active / invulnerable / chain / dodge 視窗(唯讀色條)、事件菱形(拖曳放開一筆)、+ Event at tick N(播放頭位置)、選取事件的音效下拉與 Delete。骨架 clip 本身沒有事件格式,這裡沒有新增。
- **VFX**(`vfx.library` emitter):左側清單,右側 Size over life(`size` → `end_size` 線性,兩個把手 + 數值,底下 9 個取樣點顯示該時間的大小與顏色)與 Colour over life(`color` → `end_color` 兩色標漸層,棋盤底顯示透明度;點色標後用 RGBA 滑桿或取色器)。所有控制項先改草稿,放開滑鼠且沒有彈出視窗或打字時才提交,所以一次拖曳 = 一筆 revision。**多點曲線(後續 B)**:emitter 可選擇性地有 `size_curve`(2..8 個 `{t,size}`)與 `color_gradient`(2..8 個 `{t,color}`),取代兩點值(沒有時行為不變,CPU 參考與 GPU compute 都支援)。曲線上每個 key 都有把手:拖曳改大小(中間的 key 也能沿生命時間移動,端點固定),雙擊空白處或 + 加一個 key(最多 8),選取後 Remove 刪除(至少 2 個),Two-point 移除整條曲線回到 `size` / `end_size`;漸層同理(+ Stop / 雙擊色條、拖曳中間色標、Remove、Two-stop)。兩點 emitter 只要兩個 key 仍在生命 0 與 1 就繼續寫兩點欄位;否則寫整個清單(ACL `vfx ID size_curve T:S ..` / `vfx ID gradient T:r,g,b,a ..` / `.. none`)。曲線只作用在粒子(sprite / mesh / light);ribbon 仍用 `size` 當寬度、`color` 當顏色。
- **Camera blocker**:Inspector 的 render.mesh 卡片頂端有「Blocks follow cameras」勾選(`camera.blocker` 標記元件,取代舊 tag `camera_blocker`;舊 tag 時也顯示為勾選,取消勾選會移除 tag)。World ▸ Migrate Legacy Camera Blockers 一次轉換整個世界。
- **材質 Inspector ▸ Shader source**:材質用自訂 shader 時,顯示 WGSL 與行號欄。Check(`shader.check`)在背景 job 跑和引擎相同的驗證(`shader::validate_hook`,naga parse + validate;一次一個、10 秒 watchdog、可取消(取消在驗證開始前生效;naga 驗證本身不可中斷,受 16 KB 原始碼上限約束,實測約 1 ms;取消或逾時不會蓋掉上一次的結果)、結果記在 `observe` 的 `editor.last_job` 與狀態列耗時,編輯器關閉時最多等 500 ms join),錯誤顯示為 `line N, column M: …` 並在行號欄標紅。Apply source 寫入;無效的 hook 在 commit 時被世界驗證器以同樣的行號拒絕。內建 shader(lit / unlit / toon)沒有可編輯的原始碼。
- `observe` 的 `editor.tuning`:擷取中的動作、衝突數、篩選與每個語系的計數、Animation 的模型 / clip / 播放頭 / 預覽、VFX 選取、shader 草稿與最後一次檢查。

## 後續改善(2026-11-11)

- **Proposals / History 的差異**:路徑改成人看的寫法 `plaza/lamp-1 · core.transform/position[0]: 4 → 6`(不再顯示 `~1` 跳脫與 `@` 前綴),物件順序改變顯示 `↕ entities order: moved lamp-1 (12 items)`(以最長共同子序列找出被移動的項目),被省略的大值顯示 `‹N bytes›`。滑過一行可看到協定原本的 JSON pointer;協定與 ACL 輸出不變。
- **Shader source 卡片**:在 1366×768 也不超出 Inspector:標題過長以「…」截斷、狀態標記與按鈕列自動換行、錯誤訊息換行(長識別字也會斷行)、原始碼只在卡片內水平捲動;行號欄固定在左側,和原始碼共用垂直捲動,所以行號與行永遠對齊。
- **Graphs 縮放**:滾輪在游標位置縮放(25 %–200 %,游標下的點不動),工具列的百分比按鈕回到 100 %;Frame 縮小到全部狀態可見(不會放大超過 100 %)。點選、拖曳節點、拉轉移、右鍵選單都換算成畫布單位,拖曳放開仍是一筆 `fsm M node`。AI:`graph.zoom in|out|reset|1.5|150%`,`observe` 的 `editor.graph.zoom`。
- **UI Editor 縮放**:畫布上滾輪(或 Ctrl+滾輪 / 觸控板捏合)在游標處縮放(25 %–200 %,100 % = 塞進分頁的大小,游標下的文件點不動),中鍵拖曳平移(畫布至少留 48 點在視窗內),工具列的百分比按鈕回到 100 % 並取消平移;放大後只畫在原本的畫布範圍內。縮放 / 平移只是畫面,不寫 revision;拖曳 widget 時放開位置換算回參考解析度的文件座標(拖曳提示顯示 `widget -> region (x, y)`),放開仍是一筆 `ui place`。AI:`ui.zoom in|out|reset|fit|1.5|150%`(以畫布中心為準),`observe` 的 `editor.ui_editor.zoom`。
- **Tags 摘要**:Entity 卡片顯示一列「Tags (N)  a, b, … ›」,點一下展開原本的 chip 編輯器(輸入、建議、x 移除;多選時的「tag 2/3」),再點收合;`object.tag` 不帶參數時也會展開。
- **Prefab 覆寫標記**:prefab 實例覆寫的欄位(元件 patch 裡有的欄位;陣列整個算一個)在 Inspector 左側有藍色條——Transform 的 Position / Rotation° / Scale、元件表單欄位,含有覆寫的物件 / 陣列標頭是淡藍色。標籤右鍵 ▸ Revert to Prefab(每個選取中有此覆寫的實例)/ Apply to Prefab(單選時:這個值寫進 prefab,所有沒覆寫的實例跟著變,這個實例不再覆寫),各一筆 revision。繼承的欄位外觀不變。覆寫集合依 revision 與選取的物件快取。AI:`prefab revert SEL COMPONENT/FIELD`、`prefab apply ID COMPONENT/FIELD`(ACL),`prefab.revert` / `prefab.apply`(編輯器指令)。
- **陣列內與 JSON 模式的覆寫標記**:陣列不會和 prefab 合併,實例的陣列整個是自己的(陣列標頭的滑鼠提示會說明);陣列裡和 prefab 同位置的值不一樣的欄位(例如 `render.mesh` 的 `parts[1].color`)也有藍色條,包含它的項目索引是淡藍色,和 prefab 相同的項目外觀不變。右鍵 ▸ Revert to Prefab / Apply to Prefab 只複製這一個值(實例的陣列整個等於 prefab 的時候覆寫自動移除),**Revert Whole Array** / **Apply Whole Array** 作用在整個陣列。項目裡 prefab 沒有的鍵也能單獨處理(Revert = 從實例移除、Apply = 加進 prefab 的項目);prefab 的陣列沒有的項目(實例多出來的項目)只能整個陣列處理,選單的單值項目停用(ACL 錯誤 `no_prefab_value`)。元件切到 JSON 模式時,覆寫的鍵在文字左側 gutter 有同樣的藍色條與右鍵選單;JSON 改過還沒套用(或草稿過期)時不畫,避免標錯行。同一組 ACL:`prefab revert SEL render.mesh/parts/1/color`、`prefab apply ID render.mesh/parts/1/color`。
- **Game 分頁的遊戲 UI**:HUD、對話、提示以 Play Maximized 時的空間(整個編輯器視窗)排版,再依遊戲區大小等比縮小(0.25–1 倍;長寬比不同時短邊對齊、長邊多出空間),所以 Game 分頁看到的是最大化版面的縮小版;滑鼠位置同步換算。

## 驗證

```powershell
.\scripts\editor-smoke-test.ps1
```

腳本建立獨立 QA 資料庫，透過 bridge 驗證分頁、preview、冪等 apply、增量 observe 及 undo，並在 `artifacts/editor-*` 擷取編輯前後的原生 GPU 截圖。
