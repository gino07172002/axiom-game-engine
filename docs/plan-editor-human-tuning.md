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
## 修訂補記

- 2026-11-08:決策 2(egui_dock)選定後,原計畫沒有把「可停靠 / 可重排面板與版面存檔」排進任何里程碑 → **併入 M2**(與 Hierarchy、視口手感一起,版面存在 `*.editor-prefs.json`)。理由:M1 先把所有入口集中到指令註冊表;重排面板需要把現有固定面板改成 TabViewer,和 M2 要改的 Hierarchy、視口剛好是同一批程式碼。
