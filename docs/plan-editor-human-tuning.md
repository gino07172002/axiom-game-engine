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

## 修訂補記

(無)
