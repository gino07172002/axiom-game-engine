# 計畫:人類微調編輯器(對照 Godot / Unity 補完)

> 流程:`rigorous-plan`。本檔同時是工作檔與後續執行的依據;修訂以「修訂補記」追加,不改寫歷史。
> 建立:2026-11-08。

## Phase 0 — 需求與成功樣貌

### 需求

Axiom 的主要作者是 AI agent(CLI / bridge 交易協定),人類用編輯器做「看得到、手感好」的微調:擺位、調參數、改 UI、檢查 AI 的修改、試玩。目前的編輯器只能算原型。它缺少一般引擎使用者預期的外框,包括選單、工具列、狀態列、可重排的面板和 Game 視圖。日常微調的手感(即時套用、右鍵選單、型別化欄位、物件選取器、元件 gizmo)也還沒有,各類專用編輯器(狀態機、UI、動畫、輸入、在地化)更是付之闕如。

### 現況盤點(2026-11-08,讀 `crates/axiom-runtime/src/editor/*` 與實際截圖)

| 區域 | 已有 | 
|---|---|
| 頂部列 | Logo、世界名稱、Undo / Redo、REV、Play F5。**沒有選單列**(File / Edit / View…),也沒有 Pause / Step |
| 場景工具列 | Select / Move / Rotate / Scale(Q/W/E/R)、Frame(F)、Snap(固定 0.25 m / 15° / 0.1,不可設定)、Global / Local、Grid |
| Hierarchy | 「+ Object」只有 Empty / Cube / Camera;搜尋(`tag:` `has:` `prefab:`)、場景篩選與建立 / 啟停 / 刪除、拖放改父層、多選、Ctrl+D、Del、Ctrl+C/V(ID 清單)。**沒有右鍵選單、F2 改名、顯示 / 鎖定切換、同層排序、圖示** |
| Scene 視口 | 右鍵環繞、中鍵平移、滾輪縮放;網格 / 骨架 picking;三軸 gizmo;Play 中的行為疊層。**沒有飛行相機、正交三視圖、相機 / 燈光 / 碰撞 / 觸發區 / nav.blocker gizmo、框選、相機預覽** |
| Inspector | 名稱、Transform、父層 / 場景、元件 Form / JSON 雙模式(依值的 JSON 形狀推測控制項、取色器)、Add component(附 schema)、Duplicate / Delete。**採草稿 + Apply**;沒有元件啟用開關、右鍵選單(Reset / Copy / Paste / Remove / 上下移)、多物件同時編輯、物件 / 資產參照選取器、列舉下拉、範圍滑桿、陣列增刪、prefab 覆寫標記與逐欄 Revert |
| 底部 dock | Changes、Proposals(AI 提案審核)、History(還原)、Problems(驗證)、Scenarios(執行)、Graphs(**唯讀**:fsm / narrative / behavior)、Assets(prefab 編輯與實例化、材質滑桿、資源 JSON)、AI / Protocol |
| 其他 | 遠端 `editor_view`(AI 選取 / 對焦 / 截圖 / 開分頁 / Play)、Play 預覽、bridge 共編、外部修訂同步 |

結論:Axiom 已具備 Godot / Unity 沒有的強項,包括交易式修訂、提案審核、Scenario 驗收、AI 遙控。缺的是外框、日常手感與專用編輯器。

### 最終驗收 demo 腳本

在 Axiom Town 上,不碰 JSON、不開終端機,完成以下操作:

1. **File ▸ Open Pack…** 開啟 `content/axiom-town`。視窗有選單列、工具列(變換工具、Pivot / Center、Global / Local、可設定的 Snap、▶ ❚❚ ▶❚)和狀態列(REV、Problems 數、bridge 狀態、FPS)。版面就是上次關閉時的樣子。
2. Hierarchy 右鍵 Lumi ▸ Rename,改名後按 F2;再點「眼睛」暫時隱藏路燈群組,並鎖住地面避免誤選。
3. 在 Scene 視圖按住右鍵用 WASD 飛到廣場。切到 Top 正交視圖,把長椅 snap 到 0.5 m 格線。框選三棵樹,在 Inspector 一次把它們的縮放改成 1.2(多物件編輯)。
4. 選取另一台相機,Inspector 右上角出現相機預覽;按「Set as Main Camera」切換主相機。Game 視圖立即顯示新視角,並可切換 16:9 / 4:3。
5. 選 Lumi ▸ Add Component ▸ Behavior ▸ FSM Agent。Machine 欄位用下拉選單選 `lumi`;按「Edit Graph」開啟可編輯的狀態機,拖出新狀態與轉移。
6. 開 UI Editor 分頁,把「好感度」文字從左上拖到右上區域,改用 `value` 樣式;加一個「進場淡入 + 數值跳動」效果,畫布即時預覽 zh-TW / en。
7. ▶ Play。Console 顯示事件與錯誤;Profiler 顯示各系統耗時;在 Play 中選取 NPC,看即時 FSM 狀態與欄位(唯讀)。❚❚ 暫停後 ▶❚ 單步。
8. 每一步都在 History 產生可讀的修訂(拖曳、滑桿在放開時才提交,不逐幀產生)。AI 同時送來的提案出現在 Proposals,視口以幽靈框顯示差異,核准後套用。
9. 整個過程中,AI 只用 `editor_command` / 既有協定就能做出同樣的事,並可用 `editor_view --capture` 截圖驗證。

### 約束

- 不加入 Bevy;維持 egui + wgpu。`axiom-core`(authoring)與 `axiom-runtime`(simulation)分離。
- **人機同權**:每個人類操作都要有協定等價物(既有 op 或新的 `editor_command`),經同一驗證器與 CAS。編輯器不得產生 AI 無法重現的內容狀態。
- 編輯器偏好(版面、快捷鍵、視角書籤、最近開啟)屬於使用者,存在 `data/*.editor-prefs.json`,**不進**世界修訂。
- 協定輸出保持精簡;新增 schema 中繼資料不得讓 `discover` 預設輸出膨脹(依需投影)。
- 不呼叫外部模型 API(AI 是外部 agent 操作 CLI / bridge)。
- 沒有量測就不宣稱效能。截圖驗證 UI。

### Non-goals

- 2D tilemap、地形雕刻、光照烘焙、程式碼編輯器 / 偵錯器(行為一律資料驅動,Rust 外掛走正常 IDE)。
- 多人即時協作游標(已有 bridge + 提案機制)。
- 外掛市集、編輯器內購買 / 下載素材。
- 完整複製 Unity / Godot 的全部選單;只做 Axiom 已有資料模型能支撐的項目。

### 未知與假設

- 假設 `egui 0.31` 有相容的 docking 與節點圖 crate(→ Spike S1、S2)。
- 假設可在同一 wgpu 裝置上做離屏渲染,供縮圖與相機預覽使用(→ Spike S3)。
- 「即時套用」會讓修訂數量暴增,需要合併策略(→ ADR 3)。

## Phase 1 — 勘查與風險引爆

### 風險清單

| # | 風險 | 若不成立 | 驗證成本 | 處理 |
|---|---|---|---|---|
| R1 | 可停靠 / 可重排面板(egui_dock 與 egui 0.31 相容) | 繞得過(自製分頁) | 低 | Spike S1 |
| R2 | 節點圖編輯(egui_snarl 或同類 crate 相容) | 繞得過(自製簡易節點圖) | 中 | Spike S2 |
| R3 | 離屏渲染:Game 視圖 / 相機預覽 / 資產縮圖共用 renderer | **致命**(Game 視圖與預覽做不出來) | 中 | Spike S3 |
| R4 | 型別化欄位 schema(enum / range / ref / asset)需改 core registry,影響 AI `discover` | **致命**(Inspector 品質的根基) | 中 | Spike S4 + ADR 4 |
| R5 | 即時套用造成修訂暴增、Undo 粒度過細 | 繞得過 | 低 | ADR 3 |
| R6 | 多物件編輯 × prefab 覆寫 × 混合值的寫回語意 | 繞得過(先不支援 prefab 混合) | 中 | M3 內處理 |
| R7 | CJK IME 在 egui 文字框的輸入(改名、UI 文字) | 繞得過 | 低 | M2 抽驗 |
| R8 | 大型世界(數千物件)Hierarchy / 選取效能 | 繞得過(虛擬清單) | 低 | M2 量測 |

### Spike 清單(拋棄式,不進正式 code;結論與證據追加到本檔)

- **S1 docking**:最小 egui_dock 範例接上 4 個既有面板,拖曳分頁並序列化版面。答案:可用 / 不可用,以及版本。
- **S2 節點圖**:egui_snarl 或同類 crate,在 egui 0.31 顯示 3 節點、2 連線,可拖曳並輸出結構。答案:用 crate,或自製最小節點圖(只需矩形節點、貝茲線、拖拉連線)。
- **S3 離屏渲染**:用現有 renderer 對指定相機渲染到 texture,在 egui 中以 Image 顯示,並量測 1 個額外 view 的 GPU / CPU 成本。答案:Game 視圖、相機預覽、縮圖能否共用一條路徑。
- **S4 型別化 schema**:挑 3 個元件(`render.camera`、`fsm.agent`、`ui.nameplate`),以 JSON-Schema 子集描述(type / enum / minimum / maximum / `x-ref`:entity / asset / resource)。驗證能驅動表單,並量測 `discover` 輸出位元組的變化(預設不展開)。

**GATE 1**:S3、S4 有結論與證據才開始 M2 以後的里程碑;S1、S2 若失敗,改走自製方案並寫修訂補記。

## Phase 2 — 方案設計(ADR)

**決策 1:編輯器指令系統(Command Registry)**
- 背景:選單、工具列、命令面板、快捷鍵和 AI 都需要觸發同一組操作。
- 選項 A:各 UI 元件各自寫 click 處理(現況)。選項 B:單一 `EditorCommand` 註冊表(id、標題、分類、預設快捷鍵、`enabled(ctx)`、`run`),選單 / 工具列 / 命令面板 / 快捷鍵都從它生成;bridge 新增 `editor_command {id, args}`。
- 決定:B。
- 理由:人機同權(AI 透過同一指令操作)、快捷鍵可自訂、選單不會和實作脫節。
- 後果:既有按鈕要逐步改成走註冊表(M1 完成主要項目)。指令只能包裝既有交易或純視圖狀態,不得繞過驗證器。

**決策 2:面板框架**
- 選項 A:egui_dock(可拖曳分頁、浮動、序列化)。選項 B:自製固定四區加分頁切換。
- 決定:依 S1 結果;偏好 A。
- 後果:版面存在編輯器偏好檔。重置版面要有指令。

**決策 3:Inspector 套用模式**
- 背景:Unity / Godot 改了即生效,Ctrl+Z 可退;Axiom 現在是草稿 + Apply。
- 選項 A:維持草稿。選項 B:即時套用,每個「提交點」(Enter、失焦、拖曳或滑桿放開、勾選、選單選擇)產生一筆修訂;連續編輯同一欄位、間隔 < 1.5 s 時,以 `undo` 語意合併成一筆。選項 C:B 為預設,保留「批次模式」開關(草稿 + Preview + Apply,給大改或審慎修改用)。
- 決定:C。
- 理由:手感對齊主流引擎;批次模式保留 Axiom 預覽差異的優勢;修訂仍然只追加、不改寫。
- 後果:需要「合併最近修訂」的協定支援(以 `undo REV` 加新修訂實作,不改寫歷史);History 顯示時折疊同欄位的連續修訂。

**決策 4:型別化欄位 schema**
- 背景:表單目前從 JSON 形狀推測控制項,做不出列舉、範圍、參照選取器。
- 選項 A:編輯器端為每個元件寫 UI(Unity 的 CustomEditor 風格)。選項 B:在 core registry 的元件 / 資源註冊中加入 JSON-Schema 子集,外加 `x-ref`、`x-unit`、`x-color`、`x-range-step`。表單、AI 驗證與 `discover {schema: ID}` 共用;`discover` 預設只列 ID,需要時才投影 schema。
- 決定:B,並允許少數專用 Inspector(相機預覽、材質、動畫)疊加在通用表單上。
- 理由:同一份 schema 同時服務人和 AI;新元件自動得到好用的表單;維持 token 精簡。
- 後果:要為現有約 60 個元件 / 資源逐步補 schema(M3 先補最常用的 20 個,其餘落回推測模式)。

**決策 5:Game 視圖與預覽的渲染路徑**
- 依 S3 結果。偏好:renderer 支援「render view → texture」,Game 視圖、相機預覽、資產縮圖共用。縮圖以快取檔存放(`data/*.thumbs/`),不進修訂。

**決策 6:編輯器偏好存放**
- `data/<db 檔名>.editor-prefs.json`:版面、快捷鍵覆寫、Snap 數值、視角書籤、最近開啟、主題 / UI 縮放。可刪除、不影響世界;bridge 可讀不可寫(避免 AI 改動使用者環境),除非明確指令。

### 資料模型 / 介面紙上推演(對照 demo 腳本)

- 步驟 2 的改名 / 隱藏 / 鎖定:改名 = `set_field label`。「隱藏 / 鎖定」只屬於編輯器視圖(偏好檔內的集合),不改世界 → 不影響遊戲。
- 步驟 3 的多物件編輯:`set_where` / 多筆 `set_field` 在同一交易。
- 步驟 4 的 Set Main Camera:同一交易內把這台 `render.camera/active=true`、其他設為 false(新增 `camera.make_main` 編輯器指令,底層是一般 set_field)。
- 步驟 5 的 FSM 圖:`fsm.machines` 資源的 `set_field`(狀態 / 轉移);節點位置是編輯器中繼資料,存 `editor.layout` 資源。這是決策:節點位置進修訂,和隊友、AI 共享版面;它只是一個 runtime 不讀取的資源。
- 步驟 6 的 UI 編輯:`ui.document` 的 regions / styles,外加新增的 `effects`(資料驅動動效)。
- 步驟 7 的 Play 內唯讀即時檢視:已有 `observe` / `inspect` 呼叫。
- 步驟 8 的提案幽靈框:提案 preview 的差異 → 視口疊加;已有 proposals + diff。

**GATE 2**:以上決策在 S1–S4 完成後定案;資料流與既有協定吻合,無新增「只有 UI 能做」的狀態。

## Phase 3 — 增量拆解

每個里程碑結束時都可執行、可展示、可回退;每項都要有單元測試、egui 互動測試,以及截圖目視確認。

**M1 外框骨架(walking skeleton)**
- 範圍:`EditorCommand` 註冊表;選單列(File / Edit / View / Object / Component / Play / Window / Help);主工具列(變換工具、Pivot / Center、Global / Local、Snap 下拉含數值、▶ ❚❚ ▶❚、Undo / Redo);狀態列(REV、Problems 數、bridge 狀態、選取數、FPS);命令面板 Ctrl+Shift+P;快捷鍵表(View ▸ Shortcuts)與偏好檔;bridge `editor_command`。
- 驗收:從選單、工具列、命令面板、快捷鍵與 `axiom --connect … editor-command object.duplicate` 執行同一指令,得到相同修訂;重開後 Snap 數值保留。
- 依賴:無。風險:現有按鈕散落,遷移時遺漏 → 以指令清單測試覆蓋。

**M2 Hierarchy 與視口手感**
- 範圍:右鍵選單(新增子物件、Rename F2、Duplicate、Delete、Copy ID / 路徑、Select Children、Create Prefab、Focus);同層拖曳排序;顯示 / 鎖定切換(編輯器視圖);型別圖示;Hierarchy 虛擬清單。視口:右鍵 + WASD 飛行與速度、正交 Top / Front / Side 與 View Cube、框選、相機 / 燈光 / nav.blocker / 觸發區 / 碰撞 gizmo 與圖示、可設定的 Snap、對齊地面、視角書籤。
- 驗收:demo 步驟 2、3 的視口部分。1000 物件世界的 Hierarchy 捲動與選取量測寫入紀錄。
- 依賴:M1。

**M3 Inspector 2.0(型別化 schema)**
- 範圍:決策 4 的 schema 擴充(前 20 個元件 / 資源);列舉下拉、範圍滑桿、單位、陣列增刪與排序、物件 / 資產參照選取器(搜尋加拖放)、元件標頭(啟用開關、摺疊、右鍵:Reset / Copy / Paste Values / Remove / Move Up / Down)、多物件編輯(混合值顯示「—」)、prefab 覆寫標記與逐欄 Revert / Apply to Prefab;決策 3 的即時套用與批次模式。
- 驗收:demo 步驟 3 的多物件編輯;拖曳滑桿放開只產生 1 筆修訂;連續 Undo 回到原值;`discover {schema:"render.camera"}` 輸出位元組有紀錄。
- 依賴:M1、Spike S4。風險:最高(core 改動)→ 提前在 S4 驗證。

**M4 Game 視圖與相機**
- 範圍:Game 視圖分頁(啟用相機、解析度 / 長寬比預設、Play 時接收輸入);選取相機時的 Inspector 預覽;「Set as Main Camera」;runtime `camera.activate` 指令(FSM / 規則 / UI 可切換相機);Play 工具列 ❚❚ 與單步、時間倍率。
- 驗收:demo 步驟 4、7(暫停 / 單步)。
- 依賴:M1、Spike S3。

**M5 Assets 瀏覽器**
- 範圍:資料夾 / 類型樹、縮圖(網格、glb、圖片、材質)、搜尋、拖放到場景或參照欄位、建立資產(材質、prefab、狀態機、UI 文件、輸入表)、Find Usages、帶參照修正的改名。
- 驗收:把 `axiom-short.glb` 拖進場景建立物件;Find Usages 列出所有參照。
- 依賴:M3(參照欄位)、S3(縮圖)。

**M6 狀態機與行為圖編輯器**
- 範圍:可編輯的 FSM 節點圖(新增 / 刪除狀態、拖出轉移、條件 / 動作表單走 M3 schema)、Play 中高亮目前狀態;behavior.program 規則表格編輯;「Add Component ▸ Behavior」範本(巡邏、守衛、NPC 對話)。
- 驗收:demo 步驟 5;編輯後 `town-dance.scenario.json` 仍然通過。
- 依賴:M3、Spike S2。

**M7 UI 編輯器與 UI 動效**
- 範圍:`ui.document` WYSIWYG 畫布(拖放到區域、樣式下拉、在地化切換預覽、按鈕指令選取器);新增資料驅動 UI 動效 `ui.document.effects`(進場 / 離場淡入、滑入、縮放、脈衝、震動、數值跳動,以 `when` 觸發);`ui.nameplate` 預覽。
- 驗收:demo 步驟 6;動效有 egui 測試與截圖。
- 依賴:M3。

**M8 除錯工具**
- 範圍:Console(log / 錯誤 / 事件,可過濾、可點回物件)、Profiler(各系統每幀耗時、GPU 計時若可用,並註明量測範圍)、Play 中的唯讀即時 Inspector、事件 trace 檢視、Scenario 錄製(Play 時錄輸入與指令,存成 `examples/*.scenario.json`)。
- 驗收:demo 步驟 7;錄下的 scenario 重播後 state_hash 一致。
- 依賴:M4。

**M9 其他專用編輯器**
- 範圍:輸入對應表(按鍵擷取)、在地化表格(缺字警告)、動畫時間軸(clip 預覽、事件點)、VFX 曲線 / 漸層、材質編輯延伸(shader 錯誤行號)。
- 驗收:各自的互動測試與截圖。
- 依賴:M3、M5。

**M10 AI 共編強化(Axiom 特色)**
- 範圍:History 標示作者(人 / agent / 提案)、視口幽靈框顯示提案或修訂差異、逐欄衝突合併 UI、「把目前選取交給 AI」的上下文包(已有,整合進右鍵選單)。
- 驗收:demo 步驟 8、9。
- 依賴:M1、M3。

排序理由:M1 是所有入口的骨架;M3 是 Inspector 品質的根基、風險最高,所以提前到第三;M4 解決使用者已提出的相機問題;M6、M7 解決使用者已提出的腳本與 UI 問題。

**GATE 3**:每個里程碑都是「操作 X、看到 Y」的行為驗收;M1 是 walking skeleton。

## Phase 4 — Pre-mortem

1. **整合炸在指令系統與既有按鈕的接縫**:新選單與舊按鈕各做一套,行為分歧。→ 對策:M1 以測試列舉所有指令,舊按鈕改為呼叫指令;新增 lint 測試,禁止在 editor UI 直接組交易。
2. **型別化 schema 拖垮 AI token 預算**:`discover` 變大、AI 每次都讀。→ 對策:預設不展開;記錄 `discover` 前後位元組;schema 以 ID 投影。
3. **即時套用讓修訂爆量、History 不可讀**:→ 對策:提交點規則、同欄位合併、History 折疊;以「拖曳 10 秒只產生 1 筆修訂」作為測試。
4. **範圍蔓延成「做一個 Unity」**:→ 對策:Non-goals 寫死;每個里程碑只服務 demo 腳本中的步驟。
5. **離屏渲染成本讓編輯器在 MX150 筆電上卡頓**:→ 對策:S3 先量測;Game 視圖與預覽只在可見時渲染、可降低解析度;沒有量測不宣稱效能。
6. **egui 外部 crate 綁死版本**:egui 升級時 dock / 節點圖跟不上。→ 對策:封裝在單一模組;S1、S2 留下自製方案的退路。

## Phase 5 — 執行協定

1. 每個里程碑以 `rigorous-change` 執行、以 `rigorous-verify` 驗收(fmt、clippy -D warnings、workspace tests、`editor-smoke-test.ps1`、新互動測試、截圖目視)。
2. 里程碑結束:演示並記錄證據到下方執行紀錄;檢查計畫是否需要修訂(寫補記);更新 `WORKLOG.md`;更新 `docs/scene-editor.zh-TW.md`(目前停在 M8 以前,需同步)。
3. 偏離計畫(介面改變、重排)先改本檔再繼續。
4. 每個新的人類操作都要附上對應的協定或 `editor_command` 測試(人機同權)。

**GATE 5**:計畫已回報使用者;使用者確認優先順序後從 Spike S1–S4 開始。

## 執行紀錄

- **Spike S1–S4(2026-11-08)**,拋棄式程式碼放在 `.tools/spikes/`(git 忽略),不進正式程式碼庫。
  - **S1 docking:可用。** crates.io 版本對照:`egui_dock 0.16.0` 是最後一個支援 egui ^0.31 的版本(MIT)。無頭測試以真實指標事件拖曳 Console 分頁,版面改變(分頁被拉成浮動 surface);`DockState` 序列化 2409 B,往返相等。
  - **S2 節點圖:可用。** `egui-snarl 0.7.1`(egui ^0.31.0,MIT/Apache-2.0)。3 節點 2 連線可渲染;拖曳節點標題,節點正好移動 (120, 80),連線保留;序列化往返一致。**注意**:egui 0.31 的標籤預設可選取,在標題文字上拖曳會選取文字而不是拖曳節點,圖編輯區必須設 `selectable_labels = false`。
  - **S3 Game 視圖 / 相機預覽:不需要離屏渲染。** renderer 已支援同一幀多個 view,各自畫到自己的 `viewport` 區域,再疊上 egui。編輯器的場景視口本身就是這樣做的(`Editor::render_frame` 推入 `editor.orbit`)。暫時加入子母畫面 view,實際截圖顯示 Follow camera 畫在場景視口右下角;Axiom Town、除錯版 300 幀交錯 3 次,中位數 10.1 s → 11.0 s(約 +9%,機器波動大,僅供量級參考)。→ 決策 5 改為「viewport 合成」:UI 在對應區域保持透明(Area 只畫邊框),不另建離屏貼圖。**資產縮圖**仍需真正的離屏渲染與讀回,移到 M5 內再做 spike,不阻擋 M4。spike 程式碼已還原。
  - **S4 型別化 schema:可用。** 以 JSON-Schema 子集加上 `x-unit` / `x-widget` / `x-ref` / `x-discriminator` 描述 `render.camera`、`fsm.agent`、`ui.nameplate`。`content/` 內 41 個實例(16 / 19 / 6)全部符合,0 違規。生成的表單:投影方式用下拉選單加對應欄位、viewport 用 rect、radius / height 用滑桿並標示單位、machine 用資源選取器、color 用取色器。大小:typed schema 為 874 / 235 / 340 B,現有說明文字為 ~180 / ~40 / ~100 B,約 3.5 倍。→ 決策 4 定案:`discover` 預設與 `discover <component>` 維持精簡說明,typed schema 只在 `discover <component> --typed`(或 `{"typed":true}`)時投影;編輯器在程式內直接讀 registry,不經協定。另發現說明與實作已不同步:`fsm.agent` 的說明沒有列出已存在的 `radius`。typed schema 讓這類漂移可以用測試抓到(M3 加入「實例符合 schema」的測試)。
  - **GATE 1 通過**;決策 2(egui_dock)、5(viewport 合成)、4(typed schema 依需投影)定案。長期風險:專案的 egui 0.31 落後最新版 0.36 五個大版本,dock 與節點圖套件都要跟著升級(已列在 pre-mortem #6)。

- **M1 外框骨架(2026-11-08)**:`editor/commands.rs` 指令註冊表,共 32 個指令(File / Edit / View / Object / Play / Window / Help),每個有 ID、標題、選單、預設快捷鍵與「不可用原因」;選單列、命令面板(Ctrl+Shift+P,關鍵字過濾、方向鍵、Enter、Esc)、快捷鍵分派、快捷鍵表(F1)、偏好視窗都從同一張表產生。原本寫死在 `draw` 的快捷鍵全部改走註冊表(Ctrl+C / V 仍走 egui 的 Copy / Paste 事件,但執行同一指令)。偏好 `*.editor-prefs.json`(Snap 開關與三種間距、Grid、Global / Local、dock 分頁、快捷鍵覆寫;載入時夾限範圍並丟棄未知指令;未變更不寫檔)。Snap 間距由偏好提供(場景工具列 Snap 旁有「…」可調)。狀態列顯示訊息、選取、問題、提案、REV、bridge、FPS。`editor_view {command}` 與 CLI `view --command`;`discover editor_commands`(1449 B)。Play 預覽浮層新增 Pause / Resume 與 Step。File ▸ Exit。
  證據:`editor::commands` 單元測試 5 項(註冊表一致性與所有預設快捷鍵可解析、無重複綁定;每個指令要嘛執行、要嘛回報原因;以 egui 鍵盤事件驗證 Ctrl+D、Ctrl+3、改綁 Ctrl+J 後 Ctrl+D 失效、命令面板開啟後輸入 R 不會切換工具;面板輸入「create cube」+ Enter 產生寫入;偏好往返與錯值夾限)。原有「只按 Ctrl 不切換工具」的測試仍通過(修正:比對時 Ctrl 與 Command 視為同一個鍵)。clippy exit 0;workspace 257 passed;`editor-smoke-test.ps1` ok:true。即時驗證:經 bridge 執行 `object.cube`(rev 1→2)、`edit.undo`(→3,以追加修訂還原)、未知指令與「nothing selected」都回報原因、`file.exit` 讓編輯器以 code 0 結束。截圖確認選單列、狀態列、命令面板(整列寬、快捷鍵靠右、不可用項目灰色)。修正一處缺字:Snap 下拉的「▾」改為「…」。
  未做 / 延後:預覽期間編輯器 UI 被遊戲 UI 取代(看不到選單),留給 M4 的 Game 視圖;選單無法用 egui 測試展開截圖,選單項目以 `run_into` 與指令測試覆蓋。

- **M2a Hierarchy 工作流程(2026-11-08)**:右鍵選單(Create Child、Rename、Duplicate、Delete、Copy ID、Select Children、Create Prefab From Selection、Focus、Hide / Lock),F2 列內改名(Enter / 點別處提交,Esc 取消,中文可用),拖到列上 / 下緣同層排序(青色細線)、拖到列中間改 parent,每列畫出的眼睛 / 鎖頭(不依賴字型符號),型別標籤 `CAM MESH GEO SKIN FSM PFB`。會改內容的都走 `axiom_core::ops`(`rename`、`move_entity`、`create_prefab`)與 ACL `rename`、`move`、`prefab create`,新協定 edit `move_entity`;同層順序 = `world.entities` 順序(原本依 ID 排序)。隱藏 / 鎖定只存在編輯器偏好檔(依 world ID),不影響 Play 與內容。新指令 9 個(`edit.rename` F2、`edit.select_children`、`edit.toggle_hidden` H、`edit.toggle_locked` L、`object.child_*`、`object.create_prefab`)。
  證據:對等測試 5 項、hierarchy egui 互動測試 7 項、視口隱藏 / 鎖定測試 2 項、core ops / ACL 測試。量測(debug build,無頭 egui,含整個編輯器 frame):2000 物件只排版 13 列,frame 中位數 7.3–8.2 ms(3 次)。順手修正:`dnd_drag_source` 讓 Hierarchy 列無法點選(egui 0.31 hit test 把點擊丟給疊在上面的純拖曳元件)。詳見 `docs/editor-parity.zh-TW.md`。

- **M2b 場景視口(2026-11-08)**:飛行相機(按住右鍵 + WASD / QE,Shift 3 倍;按住右鍵時滾輪調速度並在視口下方顯示,速度存偏好檔 `fly_speed`;按住右鍵期間單鍵快捷鍵暫停;沒按移動鍵的右鍵拖曳仍是環繞,開始飛行後滑鼠改為繞眼睛轉)。正交軸向視圖 Top / Bottom / Front / Back / Right / Left 與 Perspective(`view.*`,Alt+1..7、場景工具列 View 選單、右下角座標軸可點;picking、gizmo 拖曳、框選共用 `Orbit::matrix`,和 renderer 的 projection · look_at 同式)。框選(左鍵在非握把處拖曳;Shift 加選、Ctrl 反選、空框清除;跳過隱藏 / 鎖定)。元件 gizmo(`view.gizmos`,egui painter,只在編輯器視圖:相機視錐、`nav.blocker`、`physics.collider`、`fsm.agent` 半徑)。相機書籤 `view.bookmark.save|recall.1..4`(Ctrl+F6..F9 / F6..F9,依 world 存偏好檔)。內容 op `ops::drop_to_ground` + ACL `ground SEL` + `object.drop_to_ground`(End)。`ops::duplicate` 的複本緊接在來源之後。編輯器隱藏旗標也作用於附著在物件上的 VFX(粒子、緞帶、VFX 光源);`render.geometry` 與骨架網格本來就遵守,補測試。共 17 個新指令。
  證據:workspace 測試 292 → 302 passed(core ops 2、viewport 5、parity 2、commands 1;ACL 對照測試多 2 例);clippy exit 0。無頭 ACL(scratch DB,axiom-town):`ground` 把抬高的長椅 / 路燈落回 y=0(只改 `position/1`),重跑回 `unchanged`,錯誤回 `no_match` / `acl_syntax`。GPU 截圖(debug build,`artifacts/captures/m2b-*.png`):Perspective 有 nav.blocker 橘框、FSM 紫圈、非啟用相機灰藍視錐;Gizmos 關掉後只剩原有 pivot 標記;Top / Front 正交場景正確,啟用中的相機視錐為黃色。
  修正:正交視圖眼睛原本放在 1 km 外,scene shader 依眼睛距離加霧,整個場景被霧蓋掉只剩 overlay(截圖發現)→ 眼睛距離改為 `max(distance, 30 m)`,加回歸斷言。場景工具列加了 Gizmos / View 後超出寬度,把 CentralPanel 撐寬、視口右側被 Inspector 蓋住(截圖發現)→ 改 `horizontal_wrapped`。
  未做 / 限制:`physics.collider` gizmo 沒有截圖驗證(axiom-town 沒有 physics capability),也沒有單元測試;框選以投影中心點判定(不是包圍盒相交);Ctrl 框選不會取消主選取;`drop_to_ground` 不把 glTF / 骨架模型 / `render.geometry` 當支撐面;沒有 attachment 的 VFX 無法隱藏;無頭 egui 測試無法模擬真實滑鼠速度或 IME;飛行 / 框選的實機手感只用截圖與 bridge 驗證,沒有人工操作。

- **M2c 可停靠面板與版面(2026-11-09)**:
  - **ADR**:egui_dock vs 自製分割 / 分頁。crates.io 查證 `egui_dock 0.16.0` 依賴 `egui ^0.31`(0.17.0 起 `^0.32`),MIT,rust-version 1.81。選 egui_dock(S1 已證實拖曳可用;自製要寫拖放 overlay、分隔線、分頁列約 1–2 千行)。**不開 `serde` feature**:它會讓 egui / emath / epaint / accesskit 改帶 serde 重編,而且把偏好檔綁死在 egui_dock 內部格式;改用自己的版面 spec v1(分頁名稱字串樹,`editor/layout.rs`),升級 egui_dock 只動這一個模組。依賴成本:Cargo.lock 多 3 個套件(egui_dock、duplicate、proc-macro2-diagnostics);這 3 個 debug 單獨編譯 9.4 s(`-j1`);release 執行檔 29,625,729 → 29,903,626 B(+271 KiB)。M4 Game 視圖:同 Scene 的做法(`allowed_in_windows=false`、`clear_background=false`、用分頁本體的矩形走 `camera.viewport` 合成),分頁名稱 `game` 留給 M4(現在的建置讀到會當未知名稱丟掉並提示)。
  - **內容**:每個面板都是分頁(Hierarchy、Scene、Inspector、World、Changes、Proposals、History、Problems、Scenarios、Graphs、Assets、AI / Protocol;Timeline 在 Inspector 裡,不是獨立面板)。拖曳分割 / 移動 / 浮動視窗(Scene 不可浮動)、分頁 x 或中鍵關閉(拿掉「整組關閉」鈕)。新指令:`window.hierarchy|scene|inspector`、`window.close [TAB]`(無參數 = 目前聚焦的分頁)、`layout.save_as NAME`(選單開命名小視窗)、`layout.load NAME`、`layout.delete NAME`、`layout.reset`;Window ▸ Layouts 子選單(內建 Default = 舊固定配置、Tall Inspector = Inspector 佔右側全高,加上使用者存的)。`window.*` = 開啟並聚焦(關著就回到預設位置附近);`window.world` 保留原本的開關語意,預設浮動在原 World 視窗位置。偏好檔多 `layout`(spec,以原始 JSON 讀,壞掉只影響版面)與 `layouts`(最多 32 個);`dock` 保留(舊檔沒有 `layout` 時聚焦它;新檔寫入底部那格的作用分頁)。壞的 / 未知版本 / 非物件的 layout → Default + 狀態列橘色說明,其他偏好照舊;未知分頁名稱個別丟棄並列出;分數夾在 0.05–0.95、巢狀 ≤ 12、浮動視窗 ≤ 8。版面只在 fingerprint 改變且沒有按著滑鼠鍵時寫檔(拖分隔線不會每幀寫檔);沒動過的 Default 不產生偏好檔。Bridge:`editor_view {dock}` 接受全部分頁名稱(舊名稱不變,開啟並聚焦,未知名稱列出可用值),回覆與 `observe` 的 `editor.tabs`(`{tab, active, focused?, floating?}`,依 fingerprint + 聚焦快取)與 `scene_visible`;`discover editor_layout`。Scene 分頁沒畫(關閉、被同格分頁蓋住)時:`RenderFrame.draw_world=false`,不抽取、不跑陰影 / 世界 / VFX compute / post pass,介面 pass 先清成黑色;`viewport` 設為 NOTHING、框選 / 飛行 / 路徑點拖曳清掉、進行中的 gizmo 拖曳取消。另:renderer 記錄 swapchain 取得等待,慢幀分項多 `render_present_acquire_wait`(是 `render_present` 的一部分)。
  - **證據**:workspace 511 → 518 passed(新增 layout 單元 3、dock egui 4:真實指標事件把 History 拖到 Scene 右分割鈕 → 成為 Scene 的右鄰、視口變窄、放開後才寫偏好檔、新編輯器讀回同版面;拖分隔線按住期間不寫檔、放開後寫入;以名稱 / 無參數 / `window.*` / bridge 名稱關閉與重開;Scene 關閉 → `render_frame` 無 view 且 `draw_world=false`、viewport NOTHING、`observe` 反映;縮小視窗視口跟著縮;Scene 被同格分頁蓋住也不渲染;儲存 / 載入 / 刪除 / 重置 / 內建名稱拒絕 / 中文名稱;四種壞 layout 回退並保留其他偏好;舊 `{"dock":"history"}` 檔聚焦 History 且不重寫;200 幀穩態不重建 spec、節點數 / pending 容量 / fingerprint 不變);clippy exit 0;`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(rev 1→13);`street-editor-parity.ps1` identical:true(rev 8→16)。GPU 截圖(debug,看過):`artifacts/captures/m2c-default.png`(與舊配置相同的三欄 + 底部分頁,Scene 正常合成在分頁內)、`m2c-tall-inspector.png`(Inspector 全高,Tween 時間軸按鈕不再被切掉)、`m2c-custom-split.png`(偏好檔存的自訂版面:Scene | History 並排、Inspector 下方 Graphs)、`m2c-scene-closed.png`(Scene 關閉,History 佔滿,沒有 3D 畫面)。
  - **量測(release,MX150,每格 20 個不同慢幀的 p50,兩輪交錯,HEAD 32b23a2 的基準 exe vs 本版;主執行緒 CPU 分項,不是 FPS)**:street UI(`jobs_overlay_ui`)基準 0.95 / 1.09 ms vs 本版 1.05 / 0.99 ms,extract 0.064 / 0.062 vs 0.059 / 0.054 ms;生成 4096 UI 3.75 / 4.83 vs 4.08 / 4.96 ms,extract 0.467 / 0.537 vs 0.481 / 0.547 ms —— 差異在兩輪的雜訊內,**沒有可量到的 dock 成本,也沒有宣稱變快**。Scene 隱藏(只有本版):street extract 0.059 → 0.000 ms、render 扣掉 acquire 1.30 / 1.47 → 0.58 / 0.66 ms、UI 1.05 / 0.99 → 0.62 / 0.75 ms;4096 extract 0.48 → 0.001 ms、render 扣 acquire 1.16 / 1.13 → 0.76 / 0.53 ms。swapchain 取得等待約 250 ms / 幀(兩版相同,環境問題)。程序記憶體 working set / private:street 369.3 / 472.7(基準)vs 368.5 / 471.2 MiB,4096 405.3 / 519.4 vs 404.8 / 519.8 MiB,相同。基準 exe 的 render_present 含 acquire(沒有拆分),所以 render CPU 只比較本版的顯示 / 隱藏。
  - **未做 / 限制**:egui 自己每幀仍有配置(文字版面、egui_dock 的 `valid_surface_indices` 等),只驗證「不增長」,沒有用計數配置器量每幀配置數;`save_prefs` 既有的「每幀序列化整份偏好比對」沒改(現在版面在裡面,約多 1 KB 文字);分割沒有最小寬度(可以把 Inspector 拖到很窄);浮動視窗位置存的是螢幕座標,換解析度後可能在畫面外(egui 會夾回視窗範圍);「Scene 隱藏時取消進行中的 gizmo 拖曳」只有程式路徑,沒有測試(正常操作下按著滑鼠鍵無法切換分頁);沒有人工實機拖曳,只有 egui 事件測試與截圖;Game(M4)/ Assets 瀏覽器(M5)只保留名稱位置,沒有實作。
- **M3 Inspector 2.0(2026-11-09 深夜)**:
  - **ADR(決策 4 的落地)**:型別化 schema 不另寫一份(會和說明文字漂移),而是在 core 由**註冊時的說明文字 + 型別預設值**推導(`axiom-core/src/typed_schema.rs`:`a | b` → enum、`LO..HI` → 範圍 / 項數、`default X`、單位、`entity ID [with C]` → `x-ref: entity` + `x-has`、`behavior.program event` → `x-ref: event`、rgb → 取色器、quaternion → 歐拉角),加一張小表補文字說不出的參照(`audio.source/clip` → `key:audio.library/clips`、`fsm.agent/machine`、`animation.player/asset`、`interact.target/action`、`nav.grid/pointer_agent`、`path.follow/path`);世界自訂 record 直接轉。所有已註冊型別都有 schema,文字推不出的欄位依值的形狀畫,畫不了的(null、過深)用行內 JSON。協定不變:`discover typed:ID`(`render.camera` 522 B;預設 `discover` 2,999 → 3,029 B,只多了 `typed:ID` 與 `editor_inspector` 兩個名稱)。測試 `shipped_content_fits_the_typed_schemas` 確認 content/ 所有元件與資源都落在推導出的範圍 / enum / 項數內(推導不會擋掉真實資料)。
  - **內容**:編輯器 `form.rs` 重寫(`Spec` 依型別快取於 `SpecCache`,registry 換了才清;世界 record 依內容比對;不每幀重建)。欄位:範圍 + 單位拖曳(`clamp_existing_to_range(false)`)、整數欄位整數拖曳(整數值整數寫回)、enum 下拉、取色器、四元數歐拉角(記住輸入的角度,拖一軸其他軸不跳)、vec3 / rect、物件參照(按鈕;不存在紅、缺 `x-has` 元件橘;點開搜尋清單,依 (query, world version) 快取、`show_rows` 虛擬清單;Hierarchy 列拖上來)、事件 / 資源鍵下拉、巢狀物件摺疊、陣列 ▲ ▼ + x 與 + Add item(32 項一頁)、欄位提示、Filter fields。**Live**(預設,`prefs.live_inspector`,可關回草稿 + Apply):沒有按著滑鼠、沒有在打字時提交 = 一次拖曳 / 勾選 / 下拉 / Enter 一筆 revision,送出後 `Draft::mark_committed` 以送出的值為基準(不重送);前一筆還在存時等(`saving > 0`),`apply_staged` 不重設進行中的編輯;Esc(Live、沒有開著的下拉 / 選擇器 / 對話框)取消並結束拖曳;燈光 / 網格欄位拖曳中在場景逐幀預覽(換入 compiled entity 再換回),其他元件提交後顯示;提交後元件保持開啟、換物件時 Inspector 捲回頂端。多選混合值「—」(數字、下拉、勾選不確定、文字、取色、Transform 各軸;依 world version / 選取 / 元件快取)。元件標頭:Enabled、Reset(新 `ops::reset_component` = ACL `reset SEL COMPONENT`;prefab 實例移除覆寫;相機保留 active;沒有有效預設值時停用)、Copy / Paste(= `set SEL /components/C {…}`)、Remove、Make Main Camera、▶ Play(新 `Action::PlayClip`:進 Play 再呼叫 `tween.play` / `audio.play`)。陣列走 `ops::edit_array`,新增 `ArrayEdit::Move` = ACL `reorder SEL F FROM TO`。Resource inspector 同一套表單 + Live + 陣列(例如 `runtime.bindings.events`)。建立指令選項對話框(`editor/options.rs`,選單 / 指令面板開,快捷鍵與 bridge 仍直接用預設):Make Interactable、Add Tween Clip(含 play_on)、Add Audio Source、Audio source ▸ New clip…(自訂 ID)、Follow Path、Add Nav Agent、Camera Follow Selected;只傳改過的選項,下方顯示等價 ACL。時間軸 Play on(Bind = `tween … set play_on=E`;x = `remove resources[id=runtime.bindings] events I` 或 `events/I/calls J`)。Bridge `editor_view {open:"component:ID"}`(開啟並捲到該元件)、`{open:"options:COMMAND"}`;`discover editor_inspector`。
  - **證據**:workspace 518 → 538 passed / 0 failed(core typed_schema 3、core ops 2:reorder 與 reset;form 4、inspector egui 12、parity 1 等)。egui 實際輸入:拖曳 12 幀 = 1 筆 revision + Undo 還原;Esc 取消拖曳 / 打字(0 筆);Live 關閉時 Esc 不丟草稿;前一筆存檔中的編輯等待並保留;陣列 append / insert / reorder / remove = ACL;300 項陣列只畫 32 項;混合值「—」與一次改兩個物件 = `set entities[id=a,id=b]`;參照選擇器搜尋 + Hierarchy 拖放 = `set`;標頭 Copy / Paste / Enabled / Reset / Make Main Camera = ACL;Play on Bind / x = ACL;對話框(中文 prompt,egui 輸入 + Create)與其他 5 個對話框 = 顯示的 ACL 行。4096 物件世界(debug,整個編輯器 frame):閒置最大 7.1 ms、開選擇器 6.6 ms、輸入搜尋 8.0 ms、過濾後最大 7.6 ms;spec 不重建。clippy -D warnings、fmt 乾淨;`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(rev 1→13);`street-editor-parity.ps1` identical:true(rev 8→16);`build-street.ps1` 重建後 `content/street` 無變化。GPU 截圖(debug,看過):`artifacts/captures/m3-inspector-mesh-array.png`(bld/n1 render.mesh:標頭、parts [5] 每項 ▲ ▼ + x、取色器)、`m3-inspector-light.png`、`m3-inspector-mixed-mesh.png`(兩棟建築:不同的顏色旁「—」、emission「—」)、`m3-inspector-mixed-light.png`(路燈 + 夕陽:intensity / kind / shadows「—」、Transform 各軸「—」)、`m3-timeline-play-on.png`(咖啡廳門:Play on door_opening x / door_closing x、event … Bind)、`m3-resource-bindings-array.png`(runtime.bindings events [13] 陣列)、`m3-options-interactable.png`(Make Interactable 對話框與 ACL 行)。截圖發現並修正:「✕」「▾」在內建字型顯示成方框(改 x / …)、開啟元件時表單在畫面外(加捲動)、取色欄位的混合值沒標出(陣列整體也記混合)。
  - **量測(release,MX150,每格 20 個慢幀 p50,兩輪交錯;基準 = HEAD 4c02c00 的 `artifacts/perf-scratch/m2c-current`;主執行緒 CPU 分項,不是 FPS)**:street 選取物件(不開元件,兩版同狀態)UI 0.85 / 0.85 vs 0.92 / 0.92 ms(兩輪一致 +~0.07 ms,**可量到的小增加**,未拆解);M3 開 `render.mesh` 型別表單 1.10 / 1.47 ms(最大 2.31)。4096 世界選取 3.36 / 4.83 vs 4.88 / 3.94 ms(兩版互有高低,雜訊內),開型別表單 3.32 / 5.39 ms(最大 7.03)。extract / render 扣 acquire 不變(雜訊內)。working set / private:street 360.7–360.9 / 474–476 vs 361–371 / 471–473 MiB;4096 405 / 520 vs 401–406 / 520–524 MiB,相同量級。基準無法遠端開元件,所以「開表單」只有本版數字。
  - **對抗式審查**:另開 context 的審查找到 4 個 bug、3 個風險,已修:整數欄位被寫成浮點(`33.0` 被 `u32` 拒;整數值整數寫回,沒有樣本值且範圍是整數的欄位推為 integer)、前一筆仍在 staging 時第二筆 live 編輯被拒而遺失(改為等待 + `apply_staged` 保留)、Live 關閉時 Esc 丟掉草稿、主相機 Reset 變成沒有主相機;schema 推導把 `{…}` 物件當數字、`dual_quaternion` 當歐拉角、tween tracks 的項數當數值範圍、數值陣列的範圍當項數;空陣列沒有項目 schema 時 Add item 寫入 `""`(改為停用);選擇器可能寫進另一個元件的同名欄位(加 owner);混合勾選第一下改為全部開;Play on 列每幀重建(改快取)。
  - **未做 / 限制**:prefab 覆寫標記與逐欄 Revert / Apply to Prefab(計畫 M3 範圍,延後);History 不折疊連續修訂(一次互動一筆已滿足「拖曳 10 秒 1 筆」);schema 沒有、值裡也沒有的選填欄位不能從表單加(用 JSON);元件陣列只作用在主選取、自己編寫的元件(prefab 繼承的陣列要先改 prefab);Paste 只讀編輯器內的剪貼簿(egui 0.31 讀不到系統剪貼簿;Copy 會寫到系統剪貼簿);▶ Play 會進 Play 預覽(預覽時 Inspector 被遊戲 UI 取代,M4);觸發區建立時沒有尺寸選項(ACL `create trigger` 也沒有);Live 模式下打了未宣告的事件名稱,提交被拒後草稿重載,Problems 的 Declare event 之後要重打一次;`render.mesh` 預覽在拖曳期間每幀複製一次元件(上限 1024 parts);推導 schema 仍有不準的欄位(例如 `vfx.library` 的 enum),表單遇到型別不合時改用值的形狀,`discover typed:` 的使用者要知道它是推導值;release 的 4096 選擇器過濾沒有單獨量測(只有 debug 單元測試與 release 整體 UI 分項)。
- **M7 UI 編輯器與 UI 動效(2026-11-09 晚)**:
  - **ADR 7a(動效的資料形狀)**:選項 A:每個 widget 上加 `effects` 欄位;B:`ui.document.effects` 平坦清單 `{id, widget, kind, on?, event?, when?, ticks?, ease?, direction?, amount?, from?}`;C:沿用 `tween.clips` 動 UI 屬性。選 **B**:和既有的 `regions` / `styles`(widget ID → 值)同一層、不改 widget 的 serde 形狀(既有內容逐位元相同,`effects` 空時省略);每個效果有穩定 `id`(熱重載、`ui effect set/remove`、`select` 都靠它);一個 widget 可疊多個效果。C 不選:tween 目標是實體元件與變數,UI 元素不是實體,而且 tween 沒有「顯示 / 隱藏 / 值改變」觸發。**共用的部分**:easing 直接用 `axiom_core::tween::Ease`(同一組曲線與 `parse`)。觸發:`show` / `hide`(與區域面板同一個可見性判斷)、`change`(顯示的變數改變)、`event`、`when`(條件上升緣);省略時依種類給預設。
  - **ADR 7b(執行位置)**:新的排程系統 `ui.effects`(capability ui,phase 96,after behavior.execute),不放進 `Gameplay`:`Gameplay` 每步會被複製(既有),放進去會讓動效狀態跟著每步配置;獨立系統有自己的 transfer / checkpoint / metrics 掛勾。編譯在 core(`CompiledGameplay::effects`,和 widget、事件、條件同一次驗證);`ui.effects` 建立時配置固定槽,`finish_step`(事件遞送之後)更新。世界有動效但沒排程系統 = 驗證錯誤並提示 `system add ui.effects`;`ui effect add` / Add effect 自動補。系統的 `reads` 寫成 `ui.document/effects`,刻意不寫 `ui.document`:否則 `resource create ui.document` 會連帶排程它(會改變街道建置的輸出)。
  - **ADR 7c(存檔)**:閒置時動效狀態可由 gameplay 推得 → 允許存;播放中 → `RuntimeSystem::checkpoint_ready` 明確拒絕(新的預設掛勾,`checkpoint()` 才檢查,讀檔不檢查);讀檔用新掛勾 `restored` 重建基準。不把動效進度寫進存檔:存檔格式 v1 沒有系統區段,而動效最長 10 秒。
  - **ADR 7d(畫布)**:畫布不另寫一套繪製:`bound_ui` 把區域面板抽成 `draw_regions(Surface)`,遊戲用整個視窗、畫布用 `Area::constrain_to(虛擬 1280x720 等)` + `Context::set_transform_layer` 縮放進分頁(egui 0.31 公開 API)。畫布的 Area 不可互動,點選 / 拖曳由編輯器讀原始指標並對照 widget 矩形(第一版用 `ui.interact`,但轉換過的圖層裡可選取的文字會吃掉點擊,egui 測試抓到)。`styles` 現在也作用在 `progress`(有樣式時多顯示「值 / 上限」),否則 demo 的「改用 value 樣式」在進度條上沒有效果;沒有內建內容對進度條設樣式。
  - **內容**:core `ops_ui.rs`(`set_ui_region` / `set_ui_style` / `set_ui_button_command` / `set_ui_widget_fields` / `add_ui_effect` / `set_ui_effect` / `edit_ui_effect` / `remove_ui_effect` / `free_ui_effect_id`)+ ACL `ui place|style|button|effect add|set|remove`(`acl::ui_command_edits` 給編輯器指令共用解析);runtime `ui_effects.rs`(9 種效果、預算 16、丟棄計數、熱重載依 ID、`observe ui_effects`、`discover ui_effects`)、`bound_ui` 套用動效(`multiply_opacity` + 圖層形狀轉換)與 `draw_canvas`、`Gameplay::read` 改為借用(條件比較文字不再複製);編輯器 `ui_editor.rs`(UI Editor 分頁:畫布拖放、Region / Style / Command / Fields / Effects 面板、▶ Preview、語系預覽與缺字、名牌預覽、參考解析度)、`ui.*` 指令 11 個。文件 [ui-effects.zh-TW.md](ui-effects.zh-TW.md)。
  - **證據**:workspace 測試 538 → 556 passed / 0 failed(core `tests/ui_ops.rs` 4:每個 `ui` 動詞 = op、結構化錯誤 24 種、編譯預設 / 索引 / 64 上限、內建內容序列化不變;runtime `ui_effects` 9:每種效果在指定步的值、觸發(show / hide / change / event / when、滾動中再改值從顯示值接續)、兩次執行狀態相同、預算 16 溢出丟棄計數、300 步緩衝區位址 / 容量不變、熱重載依 ID 保留進度且重排 widget 不誤觸、播放中存檔明確失敗 + 讀檔後立即可存且第一步照常觸發、進退場互相取代、缺系統的世界被拒;`tests/ui_effects.rs` 跑 `examples/town-ui-effects.scenario.json`(第 10 步淡入在播、第 340 步滾動已播 9 步、結束全部停止且未丟棄)並比對兩次 state_hash;editor egui 3:demo 步驟 6 全程(真實指標事件把 `affection` 從左上格拖到右上格 = 1 筆 revision = `ui place`、Style 下拉 value、Add effect 兩次、▶ Preview 中途顯示跳動數字、語系切換畫布顯示英文 / 中文且不寫入、名牌、預覽每個 revision 只建一次)、效果表單觸發下拉 / x 移除 / 按鈕 Command 下拉、缺字計數;parity 1(10 種編輯器路徑 + 6 個 `ui.*` 指令 = ACL);layout 測試更新)。clippy -D warnings、fmt 乾淨。`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(rev 1→13);`street-editor-parity.ps1` identical:true(rev 8→16);`build-street.ps1` 重建後 `content/street` 無變化;CLI:town-dance 6/6、street 21/21 + 中途 20/20、town-ui-effects 5/5 + 中途 7/7。GPU 截圖(debug,看過):`artifacts/captures/m7-ui-editor-zh-TW.png`(畫布中文、選取框、名牌列)、`m7-ui-editor-en.png`(英文)、`m7-ui-editor-moved.png`(好感度在右上、value 樣式「0 / 100」、兩個效果)、`m7-ui-editor-preview.png`(▶ Preview 中「36 / 100」)、`m7-play-number-roll.png`(Play 中和露米聊天 +5,600 步的滾動播到第 213 步顯示「3 / 100」,observe `number 2.92`)。截圖發現並修正:畫布被元素面板擠得太小、名牌疊在 HUD 上(改為面板依空間放旁邊或下方、名牌移到畫布下方一列),效果列文字被切掉(縮短)。
  - **量測(release,MX150;基準 = M4 agent 從 b02ce69 建的 release exe;其他 agent 同時在編譯,數字雜訊大)**:無頭 Axiom Town 100,000 步 + 40 次 emote 事件、5 輪交錯、每輪一個新程序的總時間中位數:基準 10.83 s、本版 9.60 s、本版 + `town-ui-effects.acl`(2 個效果)10.70 s、本版 + 64 個脈衝效果(每次事件 16 個開始、48 個丟棄)11.04 s;各組範圍 7.5–13.8 s 重疊,**每步差異 < 0.015 ms,在雜訊內,不宣稱變快或變慢**。編輯器 UI 分項(`jobs_overlay_ui`,每格 20 個慢幀 p50,兩輪交錯):Scene 分頁 基準 1.16 / 1.21 ms vs 本版 1.21 / 1.72 ms(第二輪有 53 ms 的尖峰,雜訊);UI Editor 分頁 1.44 / 1.49 ms(Scene 不畫,extract 0.001 ms)。working set / private:基準 365.3 / 478–480 MiB,本版 365.5–366.2 / 479–480 MiB,相同。
  - **對抗式審查**:另開 context 的審查找到 6 個 bug、4 個風險、1 個小問題,已修:換世界時沿用舊 trace 序號(`on: event` 失效 → 視為新開場)、讀檔後立即存檔被拒且進場效果重播、讀檔後第一步的觸發被吞(→ 新掛勾 `restored`,開場改為「待命」而非建立時就開始)、熱重載新效果以初始狀態判斷可見(→ 用即時狀態)、退場中的按鈕仍可點、進退場效果疊加、UI 編輯器換選取 / Deselect 丟掉未提交的表單(→ 先提交)、`progress` 的 `on: hide` 永遠不會觸發(→ 編寫錯誤)、表單選 `on: when` 必失敗(→ 無條件時不列出)、長 widget ID 產生超過 128 字的效果 ID。保留並記錄:有樣式的進度條多顯示數字(刻意)、`event=default` 保留字、`on_enter` 事件不觸發。
  - **未做 / 限制**:見 ui-effects 文件的限制段;另外 Play 預覽時編輯器的 Stop Preview 浮層與右上角區域重疊(既有,M4 的 Game 視圖處理);畫布縮小時文字較糊;外部修訂會丟掉 UI 編輯器未提交的表單草稿。

## 修訂補記

- 2026-11-08:決策 2(egui_dock)選定後,原計畫沒有把「可停靠 / 可重排面板與版面存檔」排進任何里程碑 → **併入 M2**(與 Hierarchy、視口手感一起,版面存在 `*.editor-prefs.json`)。理由:M1 先把所有入口集中到指令註冊表;重排面板需要把現有固定面板改成 TabViewer,和 M2 要改的 Hierarchy、視口剛好是同一批程式碼。
