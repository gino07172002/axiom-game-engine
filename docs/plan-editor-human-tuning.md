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
- **M4 Game 視圖與相機(2026-11-09)**:
  - **ADR(決策 5 的落地 + Play 的介面)**:Game 視圖照 S3 結論走 viewport 合成(分頁本體透明,`allowed_in_windows=false`、`clear_background=false`),和 Scene 共用同一次抽取:`Editor::render_frame` 把 runtime 的相機 view 依 `game::compose` 換算到分頁的遊戲區(分割畫面保留各自的 viewport),Scene 可見時再加編輯器相機,選取相機時再加 Inspector 預覽;總數 ≤ `MAX_CAMERAS`(8),先 Scene、再 Game、最後預覽,超出的丟掉並計數(`observe editor.game.dropped_views`)。**遊戲 HUD 進分頁**:egui 的 Area / Panel 都以 `ctx.screen_rect()` 為準,無法把現有的遊戲 UI 關進子矩形,所以 Play 在分頁時遊戲 UI 用**第二個 egui Context**(screen_rect = 遊戲區)與第二個 `egui_wgpu::Renderer`(各自的字型貼圖,TextureId 不衝突),畫在世界 view 之後、編輯器介面之前;它只收遊戲該收的輸入(`app::game_raw_input`)。替代方案(同一個 context 先畫遊戲 UI 再畫編輯器)會讓遊戲的 Panel 吃掉編輯器版面,不採用。舊的整窗預覽保留為 **Play Maximized**(`play.maximize`,偏好 `play_maximized`),`axiom play` 單機路徑不變。**Play 期間編輯器唯讀**(計畫步驟 7 的「唯讀」):會改內容的指令(`CONTENT_COMMANDS`)回報原因,host 拒絕 Play 期間的編輯器寫入(`play_read_only`),Inspector 顯示 runtime 狀態;AI 的協定寫入不受影響。沒有新依賴。
  - **內容**:`game` 分頁(內建版面放在 Scene 同一格;reopen 時回到 Scene 旁)、工具列(長寬比 Free / 16:9 / 4:3 / 1:1 加黑邊、相機下拉 = 只改顯示、Set as Main Camera = `ops::make_main_camera` = ACL `camera main ID`、Behavior overlay、主相機名稱)。Play 自動切到 Game,Stop 時如果是 Play 把 Game 換到前面就換回 Scene。輸入:點遊戲區把鍵盤(遊戲要求時鎖滑鼠;Windows 沒有 Locked,鎖定時游標固定在遊戲區中央)交給遊戲,Esc、點編輯器其他地方、切走分頁、視窗失焦收回;滑鼠在遊戲區上方時遊戲收到滑鼠事件(含 `screen_pointer` 以遊戲區換算);遊戲有鍵盤時編輯器只保留 `play.*` 快捷鍵,Tab / Space / Enter / 方向鍵不進編輯器。指令 `play.stop`(Shift+F5)、`play.pause`(Ctrl+Alt+P)、`play.step`(Ctrl+Alt+.)、`play.time_scale 0.25|0.5|1|2`(每幀最多 12 × 倍率步,每步仍 1/120 s)、`play.maximize`、`game.aspect`、`game.camera`、`game.main_camera`、`view.camera_preview`、`window.game`;工具列 ■ Stop / Pause / Step / 倍率 / tick、Play ▸ Time Scale 與 Game View Aspect 子選單。Inspector:Play 時上方 **Live (read-only)**(runtime transform + 每個啟用的 `CAP.inspect {"id"}` 的結果,每 250 ms 且只在面板有畫時重讀,每項文字 ≤ 600 bytes、格式化一次),其餘欄位停用;Play 時 Scene 視圖顯示執行中的遊戲,沒有 gizmo 把手。選取相機時 Inspector 頂端有相機預覽洞(停靠時;整個洞看得到才畫,避免被壓扁)。`observe editor.play {state,time_scale,tick,manual_steps,maximized}`、`editor.game {visible,aspect,camera,rect,views,dropped_views,input,camera_preview}`;`discover editor_game`。CJK 字型只讀一次、以 `from_static` 共用(epaint 對 owned 字型資料會再複製一份)。遊戲 HUD 的 EXIT 在編輯器裡改為停止 Play。
  - **證據**:workspace 538 → 552 passed / 0 failed(新增 `editor::game_tests` 9、`app::tests` 4、`parity` 1;更新 dock / layout 測試:`game` 不再是未知分頁、開著的分頁 11 → 12、關 Scene 後 Game 接手)。egui 實際輸入:點遊戲區 → capture、遊戲有鍵盤時 Q 不切工具但 Ctrl+Alt+P / Ctrl+Alt+. 送出 PlayPause / PlayStep、Esc / 點 Hierarchy / 切走分頁 → release、Shift+F5 → Stop;Play 時拖 Inspector 欄位 0 筆寫入、內容指令回「stop Play」;長寬比 4:3 / 16:9 / 1:1 的遊戲區比例誤差 < 0.01 且合成的 view 剛好填滿;偏好檔保留長寬比、壞值回 Free;相機選擇只改顯示、預算超出計數 2;遊戲中把物件隱藏(場景切換)Game 分頁跟著變;預覽洞周圍有背景、洞內沒有,捲出一部分時不畫。App:pause 後 tick 不動、step 剛好 +1(兩次 +2)、Play 中 step 先暫停;60 幀 × 1/60 s:1× 119–120、0.5× 59–60、0.25× 29–30、2× 239–240 步;`editor_view {command:"play.pause"}` 同效果;Play 中 Write / Restore 回 `play_read_only`、修訂不變;Live inspect 讀到 `tween.inspect`。`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(rev 1→13);`street-editor-parity.ps1` identical:true(rev 8→16);`build-street.ps1` 重建後 `content/street` 無變化。GPU 截圖(debug,Axiom Street,隔離 DB,看過):`artifacts/captures/m4-game-16x9.png`(Play,選單 / Hierarchy / Inspector 都在,遊戲區 562×316 點 = 16:9,HUD 在遊戲區內、左右黑邊、觸發區 overlay)、`m4-game-4x3.png`(428×321 點,HUD 文字被遊戲區裁掉)、`m4-paused-toolbar.png`(PAUSED · tick、▶ Resume;暫停 1.5 s tick 1336 → 1336,Step → 1337)、`m4-inspector-readonly.png`(行人 ped/n1:Live 位置 / 角度 / `path.inspect`,下方欄位灰掉)、`m4-camera-preview.png`(編輯模式選取街道相機,Inspector 頂端預覽)。截圖發現並修正:遊戲區上的提示文字蓋到 HUD(改放工具列)、Inspector 內容比分頁寬時預覽洞被判定為「部分看不到」而不畫(寬度改取可見寬)。
  - **量測(release,MX150,每格 20 個不同幀 p50,兩輪交錯;基準 = HEAD b02ce69 從 worktree 建置的 `artifacts/perf-scratch/m4-baseline-b02ce69`,本版 `m4-current`;主執行緒 CPU 分項,不是 FPS)**:這次 swapchain 取得等待只有 ~15 ms/幀(之前 200–510 ms,機器狀態不同),所以只有超過 16 ms 的幀才有分項(兩邊同樣偏向較慢的幀)。編輯器閒置、Game 分頁在 Scene 後面:UI 0.67 / 0.79 vs 0.69 / 0.77 ms、render 扣 acquire 0.94 / 0.93 vs 1.36 / 0.95 ms、整幀 CPU 扣 acquire 1.81 / 1.94 vs 2.23 / 1.92 ms(第一輪本版 render 較高、第二輪相同,視為雜訊,未拆解)。Play:基準(整窗預覽)UI 0.15 / 0.15 ms、render 扣 acquire 0.39 / 0.40 ms、整幀 0.76 / 0.82 ms;本版 Game 分頁 UI 0.56 / 0.55 ms、render 0.87 / 0.68 ms、整幀 1.68 / 1.48 ms(**+~0.8 ms**:多畫整個編輯器介面與第二個 egui context 的 tessellation;在 16 ms 預算內);本版 Maximized UI 0.13 / 0.13、render 0.40 / 0.41、整幀 0.80 / 0.83 ms(= 基準)。程序記憶體 working set / private:基準 365–387 / 472–496 MiB,本版 342–351 / 453–459 MiB(兩輪都低約 20–40 MiB;推測是 CJK 字型不再複製一份,未單獨驗證)。第二個 egui context 第一次建立 + 第一幀(debug、測試內量):configure 23 ms(含第一次讀字型檔;app 內已讀過)、第一幀 7.5 ms、之後 0.15 ms。
  - **對抗式審查**:另開 context 的審查找到 5 個 bug、7 個風險,已修:Play 時 Game 分頁用編寫時的可見度(遊戲切場景看不到,加回歸測試)、遊戲有鍵盤時 Tab 讓編輯器元件取得焦點、Space 按下它、Windows 鎖定游標(Confined)會漂到編輯器上看不見地點擊、點編輯器收回輸入時那一下也進了遊戲(MouseLeft)、預覽洞部分捲出時影像被壓扁;Behavior overlay 每幀多一次完整抽取(改成直接算第一個啟用相機,沒有相關系統就不算)、第二個 context 又複製一份 CJK 字型、失焦時按住的滑鼠沒放開、Stop 那一幀還畫遊戲 HUD、遊戲 EXIT 後同批動作打到編輯 runtime、分頁隱藏時遊戲 context 的按住狀態殘留、observe 的 maximized / view 數過期、點黑邊也會 capture。
  - **未做 / 限制**:遊戲 HUD 用點為單位不隨遊戲區縮放(小分頁時 HUD 互相重疊,4:3 截圖可見;需要時用 Maximized);遊戲 context 的平台輸出(游標形狀、剪貼簿、IME 位置)沒有處理;Esc 保留給編輯器(需要 Esc 的遊戲用 Maximized);相機下拉選的非主相機只看不收滑鼠事件(pointer ray 用主相機),Behavior overlay 也只對主相機畫;Scene 與 Game 同時可見時共用一次抽取,編輯器隱藏的物件在 Game 也看不到(預設版面兩者同格,只會顯示一個);overlay 的相機不含 FPS 系統在 extract 時的修正;Set as Main Camera 送出後立即清除相機選擇(寫入失敗時選擇也已清除);從 Game 分頁切到 Maximized 後,遊戲 context 已釋放的貼圖要到下次分頁 Play 才從 GPU 釋放;Play 期間 Hierarchy 的「+ Object」等直接按鈕仍可按,寫入由 host 以 `play_read_only` 拒絕並顯示在 Problems;沒有人工實機操作(只有 egui 事件測試、bridge 與截圖);輸入路由的 winit 部分(鍵盤事件、鎖游標)沒有自動化測試,只有 `game_raw_input` 純函式與編輯器端測試;GPU 計時未量。
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
- **M5 Assets 瀏覽器(2026-11-10)**:
  - **縮圖 spike(決策 5 補完)**:S3 說縮圖需要真正的離屏渲染與讀回。實作 `editor/thumbs.rs::ThumbGpu`:一條小 pipeline(位置 + 法線 + 頂點色,兩道方向光)、128×128 `Rgba8UnormSrgb` 色彩 + 深度目標、`copy_texture_to_buffer`(每列 512 B,已是 256 對齊)→ `map_async` 讀回。wgpu 24 的 `Device` / `Queue` 是 `Clone`,所以渲染在縮圖 job 的 **worker 執行緒**上做,主執行緒只上傳 egui 貼圖。等待讀回用 `Maintain::Poll` + 1 ms 睡眠的迴圈(上限 10 s):審查指出 `Maintain::WaitForSubmissionIndex` 會在等待期間持有裝置 fence,讓主執行緒的 `queue.submit` 一起卡住。可重跑的檢查:`cargo run -p axiom-runtime --example verify_thumbnails_gpu -- OUT MODEL.glb…`(MX150 / Vulkan,debug):pipeline 建立 23 ms;axiom-short / axiom-long / fighter / valentina 匯入 64–162 ms、渲染 + 讀回 2–38 ms(6,760 / 7,696 / 4,512 / 5,392 三角形);材質球 2 ms。PNG 已目視(`artifacts/captures/m5-thumbs/*.png`)。**結論:Game 視圖用 viewport 合成,縮圖用離屏 + 讀回,兩條路徑分開**,縮圖不需要動到主 renderer。材質縮圖是「材質 tint 的球」,**不是**材質的自訂 WGSL(要跑材質 shader 得把 `ShaderGpu` 搬到離屏目標,這次沒做)。
  - **資產模型(core `assets.rs`,不另建資料庫)**:資產就是世界裡已有的鍵值表,包括 `animation.library/assets`(glb / gltf / pmx 模型)、`ui.document/images`、`render.shaders/materials` 與 `/shaders`、prefabs、`fsm.machines/machines`、`audio.library/clips`,以及單例 `ui.document`、`input.bindings`。參照表是 `animation.player/asset`、`animation.player/retarget/source_asset`、`render.material/id`、`fsm.agent/machine`、`audio.source/clip`、entity `/prefab`、材質的 `shader`、`ui.document` 圖片 widget。`usages`(Find Usages)與改名共用同一個走訪函式;另外列出繼承 prefab 參照的實例(`instance`)。typed schema 的 `REFS` 加上 `render.material/id`,Inspector 因此可以用下拉選單選材質,也能把材質拖進來。
  - **Ops / ACL**(`ops_assets.rs`):
    - `place_asset` = `asset place KIND:ID P`。模型會建立 `core.transform` + `animation.player {asset}`,prefab 等同 `instantiate`,材質會建立方塊 + `render.material`。缺的 capability / 系統會在套用前段 edits 之後的 scratch world 上計算,避免兩次寫 `/capabilities` 互相覆蓋。
    - `create_asset` = `asset create material|prefab|machine|ui|input`。材質會補 `render` + `shader`;UI 文件連同空的 `behavior.program` + `behavior.execute` 一起建立,因為 UI 會檢查它們。
    - `rename_asset` = `asset rename KIND:ID NEW`。新 ID 和所有參照在同一個批次修改;`ui` / `input` 不能改名;自訂 shader 不能改成內建名稱。
    - 唯讀指令 `assets [KIND]`、`usages KIND:ID`,另有 `discover assets`。
  - **編輯器**(`asset_browser.rs`;Assets 分頁改寫,舊的 prefab / 材質 / 資源按鈕移除,資源請從 World 視窗開):
    - **瀏覽**:左側是類型樹(附數量)與來源資料夾,上方有搜尋與 Grid / List 切換。格子與列表都是虛擬清單。列表、每格標籤和縮圖請求依 world version 快取,篩選結果依 (version, filter, query) 快取,比對時不配置記憶體。
    - **詳細資料**放在右側欄:Open、Place in Scene、Find Usages、Rename…;Find Usages 會再開一欄虛擬清單,點一列就跳到該物件、prefab 或資源。
    - **拖曳**:拖到 Scene 時會建立在指標下的地面上(`drop_point`:y=0;射線打不到地面就放在 6 m 處),狀態列顯示等價 ACL。拖到型別化參照欄位時,`form.rs::reference` 只接受同類資產。
    - **New… 對話框**沿用 `options.rs`,顯示 ACL 行。指令包括 `asset.new_*`、`asset.place|find_usages|rename`(有 arg)、`asset.thumbnails`、`asset.cancel_thumbnails`,新增 **Assets** 選單。
  - **縮圖 job**:
    - 一次只跑一個 `jobs::Job`,逾時 120 s,可取消,進度顯示在工具列、狀態列和 `observe` 的 `editor.jobs`。依內容雜湊(FNV,涵蓋檔案 bytes + 匯入選項 / 材質 JSON + 版本字串)存成 `<db 檔名>.thumbs/<hash>.png`,先寫暫存檔再改名,同一程序內也不會撞名。
    - 同一個檔案(長度 + 修改時間相同)的雜湊有記憶體快取,被逐出後再載回時不必重讀模型。
    - 看得到的格子缺縮圖時,會排一個最多 24 個項目的 lazy job;取消後在世界改變或按 Thumbnails 之前不會自動重排。逾時的項目標為失敗,不會無限重試。
    - 每幀最多從 worker 取 16 筆結果、上傳 4 張貼圖。像素只保留給最近 2 幀內畫過的格子,其餘留在磁碟。
    - 貼圖上限 64 張(128²,共 4 MiB),以 LRU 逐出;只有畫面上同時有超過 64 格時才會超過。
    - 沒有 GPU 時,模型 / 材質縮圖在解析模型**之前**就標為失敗,格子上顯示橘點,滑鼠提示會說明原因。`observe editor.thumbnails` 回報貼圖數 / 上限 / 失敗數 / 上次 job。
  - **證據**:
    - workspace 測試 538 → 見 WORKLOG 的通過數。core 新增 4 個測試(assets 2、ops_assets 2);parity 新增 3 個(place / create / rename,`instantiate_prefab_matches_acl` 改走 `place_asset`)。
    - egui 測試 10 個(`asset_browser_tests.rs`),全部用真實指標事件:
      - 瀏覽、類型 / 搜尋篩選、List;600 個 prefab 時只排版看得到的格子;快取不逐幀重建。
      - 把 `axiom-short` 拖到 Scene,寫入一次,結果等同 `asset place model:axiom-short at=…`。
      - 把模型拖到 `animation.player/asset`,等同 `set hero animation.player/asset "axiom-long"`;拖 prefab 到同一欄則不寫入。
      - Find Usages 列出 hero / npc/rin / pc/kiri,和 ACL `usages` 相同;點列會選取。
      - 改名 `stroller` → `wanderer`,一筆修訂、兩個 agent 都修正,等同 ACL;改成已存在的 ID 會成為 problem。
      - 5 個 New 對話框:欄位 → 顯示的 ACL → Create,結果等同該 ACL。
      - 縮圖 job:進度、`observe` 內容、取消後 lazy job 不重排、每幀上傳 ≤ 4、貼圖 ≤ 64 且有逐出、100 張圖片各一個快取檔、新的 editor 全部命中快取(made 0)、沒有 GPU 時失敗但不卡住。
    - CLI(debug,axiom-town 匯入的 DB):`asset create material glow shader=toon tint=…` → rev 2;`usages model:axiom-short` 為 3 列;`assets` 為 12 列。
    - 真 GPU 編輯器(`editor_view {command:"asset.thumbnails"}`):4 張縮圖由 GPU 產生並寫入快取,第二次啟動全部命中快取(cached 4,58 ms);`asset.place model:axiom-short 1.5,0,2` 之後,`usages` 變成 4 列。
    - 截圖(debug,已目視):`artifacts/captures/m5-browser.png`(模型 / 材質縮圖、PFB / FSM / UI 標記)、`m5-placed.png`(新的 axiom-short 角色與 gizmo)、`m5-usages.png`(Find Usages 欄列出 4 個參照)。
    - 其他檢查:`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 之後 `content/street` 無差異;clippy `-D warnings` 與 fmt 乾淨。
    - 截圖看出並已修正的問題:詳細資料原本放在底部,短 dock 時看不到格子,而且 SidePanel 內容會蓋到狀態列 → 改放右側欄、Find Usages 獨立一欄、加回歸斷言;格子標籤被切掉 → 格子改為 84×98。
  - **量測(release,headless egui 整個編輯器 frame,每格 200 幀,2–3 輪交錯;base = `b02ce69` 用 `git archive` 匯出、另一個 target 目錄建置;測量時沒有其他 agent 在編譯,但數字仍有 2 倍以上的波動,只看量級)**:
    - Axiom Town,舊 Assets 分頁(base)p50 0.39 / 0.41 / 2.41 / 1.73 ms;新瀏覽器 p50 0.43 / 0.68 / 0.61 / 0.67 ms。同一個執行檔的 Changes 分頁:base 0.37 / 0.36 / 1.42 / 1.72 ms,本版 0.38 / 0.66 / 0.84 / 0.53 ms。**沒有可量到的差異,也不宣稱變快**;base 第二輪整體變慢是機器波動。
    - 600 個 prefab + 100 張圖片縮圖(26 張貼圖在畫面上;只有本版,base 沒有這個瀏覽器):Changes p50 0.40–1.01 ms,Assets p50 0.81–1.31 ms(同一輪比 Changes 多 0.2–0.7 ms)。
    - 最大值兩版、兩個分頁都偶爾到 6–16 ms(無頭 egui 裡的偶發尖峰,沒有拆解)。快取在 400 幀內只建 1 次(`builds 1 filters 1`)。
    - 量測只涵蓋 CPU / 無頭 egui;GPU 縮圖的成本在 worker 執行緒上(見上方 spike 數字),沒有量主執行緒的 submit 是否因此受影響。記憶體:縮圖貼圖最多 4 MiB(上限常數),沒有量程序記憶體的 A/B。
  - **對抗式審查**(另開 context,只給 diff):
    - 已修:lazy job 取消後下一幀又重排、逾時項目無限重試(改為 `lazy_paused`,在途項目標為失敗);spawn 失敗時鍵永遠停在 Queued(改為 spawn 成功後才標記);逐幀 clone `Request` 與篩選鍵(改為只在沒有 job 時收集、就地比對);`WaitForSubmissionIndex` 會卡住主執行緒的 submit(改為 Poll 迴圈);讀回失敗後重用已 mapped 的 buffer(改為丟掉 `ThumbGpu` 重建);自訂 shader 可以改名成內建名稱;沒畫的格子殘留 Pixels;每次逐出後重讀重雜湊整個模型;同一個 `.part` 暫存檔名;`usages` 為了檢查覆寫而深拷貝元件;Play 中仍可拖放;每次自動重列都發通知;mod.rs 縮排。
    - 記錄、不修:RTS `Recipe.prefab` 等擴充型別的參照不在參照表內,改名 prefab 時會在 commit 驗證失敗(訊息不夠清楚);`.gltf` 外部 `.bin` / 貼圖變更不會讓縮圖失效;磁碟快取沒有清理;wgpu 的錯誤 scope 是整個裝置共用,worker 的驗證錯誤理論上可能被主執行緒的 scope 接到。
  - **未做 / 限制**:
    - 縮圖不跑材質自訂 shader;模型是靜止姿勢、每頂點取最後一個材質色、超過 20 萬三角形時均勻抽樣。
    - 拖放模型建立的物件沒有 clip(要自己在 Inspector 選)。
    - 沒有檔案匯入(把新的 glb 複製進 pack),也沒有資料夾改名 / 搬移。
    - 改名只修正參照表內的欄位。
    - Find Usages 的列在 egui 是 Label(沒有右鍵選單)。
    - 拖曳到 Scene 的實機手感只用 egui 事件測試與 bridge 指令驗證,截圖中的物件是 `asset.place` 指令建立的,不是人手拖放。
    - `measure_assets_tab_frame_cost` 是 `#[ignore]` 的量測,不是 CI 門檻。

- **M-UI 視覺改版(2026-11-10)**:依使用者的 mockup(`Axiom Studio Sakura Plaza Editor.png`,只參考版面、外框、密度與視覺語言)重做編輯器外觀;不加假功能(沒有 Build、Save / 未存檔、著色模式、Rect 工具、Pivot 模式)。
  - **ADR UI-1(圖示怎麼進 egui)**:(a) 建置時用腳本把 SVG 點陣化成 PNG atlas 並提交;(b) 啟動時用 `resvg` / `usvg` 解析;(c) 自己解析 path 子集畫成 egui shape。選 (a):這台機器沒有 SVG 函式庫,但 Edge 無頭模式可以把內嵌 SVG 的 HTML 截成透明背景 PNG(`--default-background-color=00000000`,實測 64×32 RGBA 正確);執行期零新依賴(用既有的 `png` crate 解碼)、決定性(提交的 PNG 就是輸入)、每幀只是一個帶頂點色的四邊形。(b) 要多一整串依賴與編譯時間,而且每次啟動都要解析;(c) 要自己做圓角筆畫三角化,且每幀成本高或要再做一次快取。成本:`scripts/build-editor-icons.ps1` → `assets/editor-icons/atlas.png`(640×420,94 KB,40 px 與 20 px 兩層,依實際像素大小選層)+ `atlas.json`;release exe 31,681,729 → 31,997,589 bytes(+316 KB,含所有 M-UI 程式碼,沒有拆開量 atlas 本身);GPU 貼圖 640×420×4 ≈ 1.03 MiB,每個 egui context 上傳一次(`icons::tests::atlas_uploads_once_and_draws_by_index`)。查表是 `Icon` 列舉索引,沒有字串雜湊。重建依賴 Edge 版本(不同版本的抗鋸齒可能有像素差),所以 PNG 是提交的產物而不是建置步驟。
  - **ADR UI-2(面板外框)**:egui_dock 0.16 不能隱藏單一分頁的分頁列,也不能在標題放圖片。做法:分頁標題前加空白,`panels.rs` 依 egui_dock 的分頁寬度規則(文字寬 + 16、間距 0)把圖示畫在空白上(只畫圖、不建 widget,畫在 dock 自己的 background layer,浮動視窗會蓋住它);單一分頁的格子用 `tab_style_override` 畫成一般面板標題,多分頁是膠囊分頁;分頁的 X 拿掉,改成每個格子右上角的「…」選單(Close = `window.close TAB`、Reset Layout),這個按鈕從該格作用中分頁的 body 用子 ui 畫到分頁列上(用上一幀的格子矩形)。踩坑:一開始用常駐的 `egui::Area` 放「…」,同一個面板裡的點擊全部失效;改成子 ui 後仍失效,原因是子 ui 會吃掉父 ui 的一個自動 ID,第一幀沒有分頁列矩形時不建子 ui、下一幀才建,使面板裡所有 widget 的 ID 換掉,按下與放開對不到同一個 widget。修法是每幀都建子 ui(沒有矩形時就不畫按鈕)。
  - **ADR UI-3(預設版面)**:下方分頁區從 8 個文字分頁改為 Assets / Problems / History;Graphs 移到 Scene 旁;Changes / Proposals / Scenarios / AI 預設關閉(Window 選單、`window.*`、`editor_view {dock}` 都能開,開在下方)。舊的 Default 保留為內建 **Classic**。偏好檔加 `layout_version`(欄位層級 serde default = 0):舊版存的目前版面在第一次啟動時換成新 Default,原本的排列存成具名版面 **Previous**(提示,不是錯誤);較新版本寫的檔案不會被降版。
  - **內容**:`editor/theme.rs`(調色盤、編輯器 egui style、卡片、膠囊、圖示按鈕、下拉、強調色勾選框、圓角遮罩;遊戲 UI 保留自己的 style,Play Maximized 時 app 換回)、`icons.rs`、`chrome.rs`(兩列頂列:Logo + 選單;世界 ▾、場景 ▾、REV / writing N、Undo / Redo、Run ▾(Game 分頁 / Play Maximized)/ Pause / Step / Stop、Agent API(bridge 連接埠、請求數、5 秒內 active)、齒輪 = 偏好設定;狀態列)、`panels.rs`。Hierarchy:工具列(Add ▾ 走 `object.*` 指令、Duplicate、Delete、Rename、Create Group、Frame、…)、場景篩選下拉、列高 26、類型圖示、眼睛 / 鎖。Scene:膠囊工具列、視圖內縮 + 圓角外框、左下 FPS、右下 Frame / Fullscreen(新指令 `view.fullscreen` F11)、軸向方塊移到右上。Inspector:Entity 卡片、可摺疊區段卡片、元件卡片(強調色 Enabled、Reset、Play / Edit Graph、「…」:Reset / Play / Copy / Paste / Make Main Camera / Remove)、表單兩欄(標籤欄 96 pt)、收合物件顯示 36 字元摘要(有上限的走訪,不序列化整個值)。Assets:麵包屑、Grid / List、縮圖大小滑桿、搜尋(固定 ID)、類別篩選、標籤依寬度截斷。Graphs:`graph.frame`(平移到全部狀態可見)。新內容操作 `ops::create_group` + ACL `group SEL [id=ID]` + 指令 `object.group`(Ctrl+G)。Renderer 計數 `FrameStats`(世界 draw call、三角形 × instance、UI 網格數,錄製時 CPU 計數,不配置);GPU alloc 每 2 秒一次 `generate_allocator_report`。Bridge 用兩個原子變數記請求數與最後時間。`AXIOM_WINDOW_SIZE=WxH` 供截圖。修正:編輯模式的 "Maximized" 雜項標籤(改成 Run ▾ 裡的選項,有設定時才顯示 Maximized 標記);狀態訊息裡的 ▸ 改為 ›(字型沒有 ▸)。
  - **GPU alloc n/a**:這台 MX150 走 Vulkan,wgpu 24 的 `generate_allocator_report` 回傳 `None`(和 `docs/acceptance-street.zh-TW.md` 的 `gpu_memory.available: false` 一致),所以狀態列誠實顯示 `GPU alloc n/a`,滑鼠提示說明原因;DX12 上才會有數字。不是 bug,也沒有改用其他估計值。
  - **證據**:workspace 測試 602 → 613 passed / 0 failed;新增 core `group_verb_matches_the_op_and_keeps_world_poses`、parity `create_group_matches_acl`、`chrome_tests` 6 項(頂列 Run / Maximized ▾ / Undo / 偏好設定 / Agent API;Hierarchy 工具列與 Add ▾ 的寫入等於 `run_command`;Scene 工具列;狀態列計數與 Edit / Play;Group 經工具列 = ACL)、`icons` 2 項(106 個名稱 = icons.json = atlas.json、每格兩層都有筆畫、只上傳一次)、`chrome::tests::grouped_thousands`、`dock_tests` 的版面遷移(Previous / Classic、檔案寫一次)。調整既有測試:元件卡片的 Copy / Paste / Make Main Camera 要先開「…」(`h.menu`)、tab 標題比對去掉前置空白、預設版面的分頁數與重開位置、兩個 egui 測試的畫面加高(列高變大)。`cargo fmt`、`clippy --workspace --all-targets -D warnings` 乾淨。`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 無差異。
  - **截圖(debug,已和 mockup 並排目視)**:這台螢幕是 1920×1080、150% 縮放,所以用 `AXIOM_WINDOW_SIZE` 開出邏輯 1920×1080(2880×1620 實體)與邏輯 1366×768(2049×1152)的視窗:`artifacts/captures/mui-{edit,inspector,assets,graphs,ui-editor,game}-{1080,768}.png`。改版前對照 `mui-ui-editor-base1080.png`(HEAD 95882b8 release)。
  - **量測(release,MX150,Axiom Street,選取 cafe/door/leaf,Changes 分頁在下方;每格約 65 個不同慢幀的 p50,3 輪交錯;base = HEAD 95882b8 另建的 release exe)**:UI 階段(`jobs_overlay_ui`)base 0.96 / 1.20 / 1.18 ms,本版 1.07 / 1.18 / 1.11 ms(雜訊內);整幀 CPU 扣除 `render_present_acquire_wait` base 2.68 / 3.11 / 3.02 ms,本版 3.50 / 3.38 / 3.47 ms(三輪都高 0.3–0.8 ms,沒有拆解;推測是更多圓角 / 邊框 / 圖示網格的 tessellation 與 UI 繪製,`render_present` 的 CPU 部分)。程序 private 452–454 vs 452–454 MiB,working set 341 vs 342 MiB(+~1 MiB,符合 atlas 貼圖大小,未拆解)。慢幀都由 swapchain 取得主導(這台筆電的既有現象),所以 FPS 不可比較。
  - **對抗式審查**(另開 context,只給需求與 diff):1 個 bug、5 個風險,已修 6 項:圖示畫在 Middle 層會蓋過浮動視窗 → 改畫在 dock 的 background layer;Scene 很矮時內縮矩形倒置 → 太小就不內縮;收合物件摘要每幀整個序列化 → 有上限的走訪;Hierarchy Add ▾ 直接呼叫建立函式 → 改走 `object.*` 指令(Play / 草稿時停用);版面遷移丟掉使用者自訂排列 → 存成 Previous、不降版;浮動視窗裡的分頁沒有「…」可關 → 分頁列矩形改涵蓋所有 surface。記錄未修:分頁切換時「…」少一幀;Assets 滑桿 / Grid / List 用自動 ID;21–24 px 的圖示用 20 px 層會略糊、atlas 格子沒有留邊可能有線性濾波滲色;每幀約 28 個分頁標題字串;約 650 px 以下的 Scene 寬度,提示文字會壓到 Frame / Fullscreen。
  - **未做 / 限制**:Graphs 畫布沒有縮放(只有平移到全部可見);egui_dock 分頁列沒有真正隱藏(單一分頁是樣式化的標題);浮動視窗的分頁沒有圖示;Inspector 一次只展開一張元件卡片(草稿模型一次一個元件),非展開卡片的 Enabled 是唯讀顯示;Tags 仍是原本的晶片編輯器(不是 mockup 的「Tags (7) ›」摘要);「Advanced」摘要只用在收合的物件欄位,沒有像 mockup 那樣的 Render Layers 欄位(沒有這種資料);Game 分頁沒有圓角外框;UI Editor 畫布在截圖裡是空的(改版前的 base exe 也一樣,沒有調查);整幀 CPU +0.3–0.8 ms 沒有拆解。

- **M8 除錯工具(2026-11-10)**,以 `rigorous-change` 執行,未提交(留給使用者驗證)。
  - **Step 0(1366×768 邏輯)**:Inspector 的 Transform 列以「看得到的寬度」(`available_width` 與 clip rect 取小)分配 X / Y / Z,欄寬不足 44 pt 時標籤放上一行,`interact_size.x` 跟著縮(`add_sized` 不能把 DragValue 縮到它自己的最小寬度以下);Hierarchy 工具列寬度不足時 Add 只剩圖示,放不下的選取指令移進「…」選單(同一批指令)。測試 `chrome_tests::narrow_panels_keep_the_hierarchy_toolbar_and_transform_fields_on_screen`(1366×768 與 1024×640)。截圖 `artifacts/captures/m8-step0-before-768.png` / `m8-step0-after-768.png`(已目視:Z 欄、篩選圖示、鎖圖示都回到畫面內)。
  - **內容**:新模組 `editor/debug.rs`(Console、Event trace、Profiler 與三個分頁)、`editor/record.rs`(scenario 錄製)、`editor/debug_tests.rs`。Console 512 筆、trace 2048 筆的固定容量環形緩衝(`VecDeque::with_capacity`,滿了丟最舊並計數);畫面用的過濾結果依(最新序號、篩選代數)快取,沒有新訊息時不重建。來源:`set_error` / `notice` / `problem`(problem 帶物件連結)、runtime 錯誤(activation、gameplay、跨套件事件,出現時記一次)、遊戲輸出行、Play 中的 gameplay 事件與跨套件事件。trace 只在 Play 中收集:gameplay trace(以序號接續;runtime 只留 128 筆,同幀溢出記為 missed)、`Events::recent()`(新增唯讀存取)與 FSM 狀態轉換(`agent_states()` 逐一比對字串,不配置)。Profiler 只在分頁看得到時量:主迴圈 `FramePhases` 每幀複製進 240 幀固定陣列;`WorldRuntime::set_system_profiling` 開啟時 `step()` 對每個系統計時(關閉時是 `None`,沒有 `Instant` 讀取)。GPU 計時明確標示不提供(adapter 的 `TIMESTAMP_QUERY` 支援與否由 `Renderer::timestamp_query_supported` 回報;裝置沒有要求這個 feature)。Live Inspector 加 health。版面版本 3:Default 下方加 Console。
  - **Scenario 錄製**:`play.record [NAME]` = 建立錄製器並 Play;每個固定步之前比對 `ActionMap`,只記改變的動作(放開記 0);成功的遊戲 UI 指令、bridge `command` / `execute`、Inspector ▶、存讀檔都以「目前已跑的步數」為 tick 記錄(和 `scenario::run` 的「先指令、再輸入、再 step」一致)。Stop / Restart / 再按一次 / 關視窗時寫檔,`create_new` 不覆蓋。新增 scenario 斷言 `state_hash`(加法),錄製檔帶入結束時的雜湊,重播自我檢查。不能精確重播時(系統會用到的滑鼠視角 / 點擊射線、最後一步之後的指令、讀取錄製前的存檔、超過上限而提早結束、Play 期間修訂改變)照樣寫檔,但不帶雜湊斷言,並以 warn 說明。
  - **證據**:
    - workspace 測試 613 → 626 passed / 0 failed(`cargo test --workspace --no-fail-fast`);`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 乾淨。新增:`app::tests::a_play_recording_replays_headless_to_the_same_state_hash`(Axiom Town:W+D 走到露米、E 交談、Pause + Step、bridge `choose dance`、A 鍵、Live Inspector 每幀讀取;停止後 `scenario::run`(CLI 用的同一個函式)ticks 與 state_hash 相同、`state_hash` 斷言通過;刪掉一筆輸入後斷言失敗;同名不覆蓋)、`record::tests` 3 項、`debug_tests` 8 項(Console 連結 / 篩選 / 點擊 = 指令 / 容量固定 / 閒置 60 幀不重建、trace 分組與點擊選取 agent / 暫停計數 / 閒置不重建、讀檔後仍顯示新事件(去掉修正時這項失敗,已驗證)、篩選文字保留空白、Profiler 只在可見時量、錄製指令、版本 2 版面遷移且不覆蓋既有的 Previous)、`chrome_tests` 1 項。
    - **實機 E2E(debug exe、GPU 視窗、獨立 QA DB `artifacts/m8-qa`)**:bridge 送 `play.record`、兩個 `emit`、`play.pause` / `play.step` / `play.pause`、`play.stop`,再用 `axiom --db DB scenario examples/NAME.scenario.json` 重播:最終程式碼的兩輪:邏輯 1920×1080 671 ticks、編輯器 `d43f3a6d7833c683` = 重播 `d43f3a6d7833c683`;1366×768 745 ticks、`df37e4e4afc0278c` = `df37e4e4afc0278c`;`finite` 與 `state_hash` 斷言都通過(修正前的幾輪也全部相同:507 / 509 / 763 / 841 ticks)。錄製檔複製到 `artifacts/m8-qa/` 後從 `examples/` 刪除。這條路徑沒有鍵盤輸入(bridge 不能按鍵),鍵盤由上面的 app 測試涵蓋。
    - 截圖(debug,已目視):`artifacts/captures/m8-{console,trace,profiler}-{1080,768}.png`。看出並修正:來源下拉的「▾」字型缺字顯示成方框(改用 ChevronDown 圖示);Profiler 內容沒有捲動、表格在矮的下方格被切掉(改為可捲動、圖高 84 → 64、說明移到滑鼠提示);表格欄置中(改為名稱靠左、數值靠右);圖的標籤被長條蓋住、預算內外顏色太像(標籤移到圖上方、預算內改綠色)。
    - `editor-smoke-test.ps1` ok:true(data/ 的 smoke DB 已刪);`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 無差異。
  - **量測(release,MX150)**:
    - **整個 exe 的 A/B**(base = HEAD 1e1759b 用 `git archive` 匯出、另一個 target 目錄建置;本版 = 最終程式碼;Axiom Town,editor_view `play:true` 在 Game 分頁 Play,Console 不可見(預設 Assets 在前);每輪輪詢 20 秒 `frame.last_slow` 取不同慢幀的 p50,7 輪交錯):CPU(整幀扣除 acquire wait)base 20.70 / 10.78 / 7.18 / 13.16 / 6.63 / 8.99 / 7.36 ms(中位 8.99),本版 17.36 / 7.51 / 9.76 / 11.16 / 13.76 / 7.54 ms(中位 10.46;另一輪 Play 沒有啟動,剔除);UI 階段 base 中位 2.03、本版 2.60 ms。同一個 exe 在不同輪之間就差到 3 倍,**這個方法分辨不出差異,不宣稱有或沒有變慢**。工作集 388–418 vs 388–418 MiB、private 610–632 vs 610–631 MiB(相同)。
    - **直接量新增的工作**(`debug_tests::measure_m8_debug_hooks`,`#[ignore]`,release,每項 2000 次中位數,單次執行):Play 中閒置(沒有新事件)的 `debug.collect` 0.6 µs / 幀、編輯模式 0.1 µs;每幀 `observe` 多出的 `editor.debug` JSON 5.9 µs;`set_system_profiling(false)` 0.1 µs;Axiom Town 一個固定步 53.4 µs,開啟逐系統計時(只在 Profiler 可見時)58.9 µs(+5.5 µs / 步,約 +10%)。無頭 egui 整個編輯器幀:Console 不可見 1.63 ms、可見 1.72 ms、Profiler 可見 0.75 ms(Profiler 取代了 Assets 的格子)。結論:隱藏時新增的是每幀約 6–7 µs(主要是 observe 的 JSON),在 16 ms 預算裡可以忽略;Profiler 可見時每個固定步多約 5 µs。
  - **對抗式審查**(另開 context,只給需求與 diff):4 個 bug、3 個風險、3 個 nit。已修:錄製到上限提早結束時仍可能寫入錯的雜湊(`stop` 也標成不精確);第 17 個存檔槽產生無法重播的檔案(改為在那之前結束錄製);Play 中讀檔後 runtime 序號歸零,trace / Console 看不到新事件(序號倒退時重設);篩選框每次按鍵都 trim 掉空白(只去掉開頭);版本遷移會覆蓋既有的「Previous」(改用第一個空的 Previous / Previous 2 …);Play 期間修訂改變仍標成精確(結束時比對目前修訂);同一幀多步時 trace 的 tick 標頭重複(建立列時依 tick 穩定排序);關視窗時錄製遺失(結束前寫檔並印出結果)。記錄不修:Play 事件也進 Console,事件很多時會把較舊的錯誤擠出 512 筆(錯誤數會跟著減少;可用 level 篩選看留下的);FSM 狀態轉換以「這幀結束時的 tick」標記(一幀多步時可能晚 1 個 tick);看得到的分頁每幀仍會格式化標籤字串(和其他 egui 面板一樣;隱藏 / 閒置時不配置)。
  - **未做 / 偏離**:GPU 計時(timestamp query)沒有實作,只標示不提供與 adapter 是否支援;Console 沒有收 FSM / trigger 等系統內部的 `fault`(它們只出現在各自的 summary / inspect);bridge 端讀 Console 只有 `observe` 的計數與最後一筆錯誤,沒有分頁 dump;錄製不含滑鼠視角 / 螢幕點擊(scenario 格式沒有),遇到會標成不精確;錄製檔寫在 `examples/`(相對工作目錄),名稱只能是檔名。

- **M9 其他專用編輯器(2026-11-10)**:Input Map、Localization、Animation、VFX 四個分頁 + 材質 Inspector 的 Shader source。
  - **ADR 9a(不發明格式)**:先盤點現有資料。輸入 = `input.bindings`(按鍵名是 runtime 實際比對的 winit `KeyCode` Debug 名稱,`app.rs` 的 `format!("{k:?}")`,加上 `MouseLeft`);在地化 = `ui.document.catalogs`(所有語系同一組 key);VFX = emitter 已有的 `size → end_size`(線性)與 `color → end_color`(兩色標),**沒有**多關鍵點曲線格式,所以曲線編輯器就是兩個把手、漸層就是兩個色標;材質 = `render.shaders` 自訂 shader 的 WGSL,驗證用引擎既有的 `shader::validate_hook`(naga)。**動畫事件**:骨架 clip 本身沒有事件格式;引擎唯一和 clip 同步的事件是 `action.abilities` 的 `events [{tick, sound}]`(ability 把 clip 拉伸到 `duration` tick,`action.rs::cue`),所以時間軸的事件點就是它們;選項「在 `animation.library` clip 上新增 events」被否決(要新 runtime 系統、會和 ability 事件重複,而且沒有使用者)。
  - **ADR 9b(op 與 ACL)**:core `ops_tuning.rs`(從 `ops` 重新匯出):`edit_input(InputEdit::Button|Axis|Remove)`、`input_conflicts`、`edit_localization(LocEdit::Set|AddKey|RemoveKey|AddLocale|RemoveLocale)`、`loc_issue`(和 UI Editor 的缺字規則相同)、`edit_ability_event(Add|Move|Sound|Remove)`、`edit_vfx(Size|Color)`、`set_shader_source`。`action.abilities` / `vfx.library` / `render.shaders` 是 runtime 註冊的資源,core 以 JSON 編輯並先做和驗證器相同的檢查(結構化錯誤 `action_kind`、`default_locale`、`missing_cue`、`builtin_shader`、`budget_exceeded`…),其餘由 commit 時的驗證器把關。ACL 新動詞 `input` / `loc` / `ability` / `vfx` / `shader`(`acl::tuning_command_edits` 給編輯器指令共用解析);registry 指令 `input.edit` / `loc.edit` / `ability.edit` / `vfx.edit` / `shader.edit` 直接吃 ACL 參數(列入 `CONTENT_COMMANDS`,Play 中停用),視圖指令 `input.capture`、`loc.filter`、`anim.target`、`anim.scrub`、`vfx.select`、`shader.check`、`shader.cancel_check`、`shader.draft`,`window.input_map|localization|animation|vfx`。改了什麼都沒變的 op 回傳空清單,編輯器不寫入(store 拒絕空批次)。
  - **ADR 9c(一次互動一筆)**:按鍵擷取 = 下一個按鍵一筆(新軸的正負兩鍵一筆);表格格子 Enter / 失焦一筆;事件點與 VFX 把手拖曳放開一筆;VFX 滑桿 / 取色器改草稿,放開滑鼠且沒有彈出視窗、沒在打字時才提交(size + color 同一幀的兩個寫入由 host 的 `coalesce` 合併成一筆 revision)。
  - **ADR 9d(Scrub 預覽)**:沿用 tween 時間軸的模式:`render_frame` 擷取前把選取模型的姿勢換成播放頭的姿勢、擷取後換回(`AnimationSystem::begin_scrub/end_scrub`,只交換 `pose`;播放頭沒動時重用上一次算好的姿勢,不 clone、不重算),模擬的 controller 時間 / clip 不變;只在 Animation 分頁當幀有畫、編輯模式時生效。
  - **ADR 9e(Shader 檢查)**:explicit 的 Check(`shader.check`)在 `jobs::Job` 背景執行緒跑 `validate_hook`(一次一個、10 s watchdog、進度、取消、耗時寫入 `editor.last_job` 與狀態列、`ShaderUi` drop 時 `shutdown(500 ms)`)。naga 驗證本身不可中斷,取消在驗證開始前生效;原始碼上限 16 KB,實測(debug)0.7 ms。錯誤訊息的 `line N, column M` 由 `shader::error_location` 解析(和產生它的 `format_location` 放在一起),在行號欄標紅;原始碼不換行,行號一對一。
  - **內容**:core `ops_tuning.rs`、ACL `Statement::Tuning` + GRAMMAR;runtime `animation.rs`(scrub、`as_any_mut`)、`shader.rs`(`error_location`);編輯器 `tuning.rs`(狀態、指令分派、`observe editor.tuning`、表格格子 helper)、`input_map.rs`、`localization.rs`、`anim_timeline.rs`、`vfx_editor.rs`、`shader_editor.rs`。共用檔案只做附加:`mod.rs`(模組、Dock 變體、欄位、render_frame scrub、poll / observe / jobs 各一行)、`ui.rs`(分頁分派、快捷鍵條件 `keys_captured`、`begin_frame`)、`layout.rs`(四個變體、預設版面不變,on demand 開在下方分頁區)、`commands.rs`(指令附加在表尾)、`panels.rs`(圖示)、`assets.rs`(一行呼叫 `shader_section`)。
  - **證據**:
    - workspace 測試 613 → 623 passed / 0 failed(core `tests/tuning_ops.rs` 3:每個動詞 = op、結構化錯誤碼、按鍵衝突、64 動作上限、新 key 在其他語系是空字串、儲存格只寫一個欄位、沒改就沒有 edit、單字母按鍵提示 `KeyW`、`1` = `1.0`;runtime `parity::tuning_editor_edits_match_acl`(9 種編輯器路徑 = ACL);`tuning_tests` 6 項真實 egui 事件:按 W / E / Shift / T / Q+3 / Mouse Left、Esc 取消、在其他文字欄打字取消、擷取期間 W 不切換工具、衝突標示與 `observe`;格子 Ctrl+A 輸入 Enter = 一筆、Esc 取消、+ Key 後兩個語系各 1 筆缺字、Missing only = 1 列、篩選、模型與篩選不逐幀重建;播放頭點在 clip 一半 → 場景幀的頂點和未預覽時不同、模擬的 metrics 不變、不寫入,拖曳事件點 8 幀(按住時 0 筆)放開 1 筆,+ Event、改音效、Delete;VFX 把手拖曳 6 幀放開 1 筆、色標 R 滑桿 1 筆;Shader Check 背景 job、行 3 錯誤標記、無效原始碼 commit 被拒並帶 `line 3`、有效原始碼 = ACL、Cancel 後 job 結束;沒有輸入表時 Create = `asset create input`)。
    - `cargo fmt`、`clippy --workspace --all-targets -D warnings` 乾淨;`editor-smoke-test.ps1` ok:true(產生的 data/ DB 已刪);`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 無差異。
    - GPU 截圖(debug,看過):`artifacts/captures/m9-{input-map,localization,animation,vfx,shader}-{1080,768}.png`(邏輯 1920×1080 / 1366×768,這台 150% 縮放 = 2880×1620 / 2049×1152)。input-map:attack 綁 E 與 interact 衝突(橘色 ⚠ E、上方列出);localization:新 key `greeting` 讓兩個語系各 1 missing;animation:hero 在 wave 0.7 s 的姿勢(舉手)、ability 列的 chime 事件與 active 視窗;vfx:size 曲線 + 取樣點 + 漸層並排;shader:`smoothstepp` → 行 3 標紅、錯誤 `line 3, column 18`、狀態列耗時 0.7 ms。截圖發現並修正:表格文字置中、欄頭對不齊(改成靠左的格子);VFX 漸層被擠到摺線下(改成並排);WGSL 長行換行使行號錯位(關閉換行)。
  - **量測**:沒有做 release A/B,**不宣稱效能變化**。設計上的成本:各分頁的模型依 (rev, world_version) 快取(測試確認不逐幀重建);在地化是虛擬清單;Scrub 預覽只在播放頭移動時 clone + evaluate 一次,靜止時只交換姿勢;Shader 檢查在背景執行緒(debug 實測 0.7 ms)。已知每幀小配置:各格子的 hover 文字、Animation 的列清單與標記多邊形、Shader 卡片的 `issues` 複製與 layouter 複製原始碼(egui 預設 layouter 也會複製)。
  - **對抗式審查**(另開 context,只給需求與 diff):4 bug + 5 風險 + 7 小問題。已修:Create Input Map 丟掉寫入;擷取會吃掉其他文字欄位的按鍵、藏起來的分頁回來後立刻綁、Play 中仍可啟動(→ 打字或 Play 取消、隱藏後重新等待、`input.capture` Play 中停用、Play 時不擋快捷鍵);被其他修訂刪掉的按鈕在擷取後被加回;殘留的事件點拖曳讓下一次拖曳移動舊事件;修訂後選取事件的索引可能指向別的事件(→ 依 (tick, sound) 重新確認);Scrub 每幀 clone 整個 controller(→ 快取姿勢、只交換 pose);`edit_input` / `edit_vfx` 在沒有實際變化時仍寫入(→ 型別 / 數值比較);單字母按鍵名稱(→ 拒絕並提示);取消的檢查蓋掉上一次結果;`input.capture` 對結尾是 `-` 的按鈕名稱誤判。判定不是 bug:size + color 同幀兩筆寫入(host 的 `coalesce` 合併成一筆 revision,已加註解)。記錄、不修:數字鍵盤按鍵在 egui 被回報成主鍵盤(擷取存成 `Digit1`,文件說明用 ACL 寫 `Numpad1`);naga 驗證不可中斷(取消只在開始前生效);從 ACL 對 `file` 型 shader 寫 `wgsl` 的錯誤訊息不夠清楚;同一幀按下放開到另一格時在地化可能不開新格(只有合成輸入會發生)。
  - **未做 / 限制**:鍵位沒有「重新命名動作」(會牽動 `behavior.program` / `interact` / `camera.follow` 的參照);`input.contexts` 沒有編輯器;在地化沒有匯入 / 匯出 CSV;Animation 只顯示 ability 事件(骨架 clip 沒有事件格式),Scrub 不播放音效;VFX 不是多關鍵點曲線,也沒有 emitter 新增 / 刪除;Shader 沒有語法高亮,GPU pipeline 的編譯錯誤不在這個卡片(只有 naga 驗證)。M8(Console / Profiler)在主線平行開發,合併時 `Dock` 列舉、`layout.rs` 的 `ALL` / `name` / `title`、`panels.rs`、`commands.rs` 表尾會有相鄰的附加行衝突(都是附加,逐行保留即可)。

- **M10 AI 共編強化(2026-11-10)**,以 `rigorous-change` 執行,未提交(留給使用者驗證)。
  - **ADR 10a(作者存哪裡)**:盤點既有紀錄:`revisions` 只有 `reason`(`apply` / `restore:N` / `initialize` / `migrate`),沒有作者;`request_receipts` 只有冪等收據;`proposals.resolved_rev` 已記錄「哪個 revision 是核准提案產生的」。→ 只新增 `revisions.author TEXT NULL`(`human` / `agent`),開檔時在同一個 IMMEDIATE 交易內檢查並 `ALTER TABLE ADD COLUMN`(多個連線同時開檔不會重複加);舊版程式的 `INSERT(world,reason)` 仍可寫。提案不加欄位,`history` 以 `proposals.resolved_rev` 對應出 `proposal`。誰是誰:編輯器主連線與 writer 執行緒 = human,但處理 bridge 請求(`editor_view {command}`)期間排入的寫入 = agent(`App.requester`,每筆 `PendingWrite` 帶著作者送給 writer);bridge 連線、CLI = agent;initialize / migrate = 無作者。核准提案的作者 = 核准者。
  - **ADR 10b(合併的單位與提交)**:三方合併(base = 編輯所根據的 revision、mine = base + 草稿或提案的 edit、theirs = 目前 revision)以「欄位」為單位:物件的 label / prefab / tags、元件與資源的每個頂層欄位(某一方新增 / 刪除整個元件、資源或物件時是整個)、一條父層或其他關係、物件的相對順序(只看三方都有的物件,所以單純新增不會誤判),其他頂層項目(場景、prefab、schema、capability、systems、id、title)整個算一個。只有一方改 = 直接採用;雙方改成不同值 = 衝突,預設 mine,`keep` 列出的保留 theirs。結果是一個 `ReplaceWorld` edit、一筆 revision(store 照常重算 diff / 驗證 / CAS)。選項「把合併翻成一組 `set_field`」被否決:刪除欄位、新增物件、順序都需要不同的 edit 種類,而 store 每次 commit 本來就寫整份世界,`ReplaceWorld` 只多一次序列化(在 writer 執行緒)。提案的合併由 store 在核准的交易內重算(`resolve_proposal {keep, expected_rev}`;`expected_rev` = 列出衝突時的 revision,之後若又有新 revision 就拒絕,不會替使用者決定沒看過的衝突);草稿的合併在背景 job 算出 edit,再走一般的 apply(CAS 到列出衝突時的 revision)。純函式在 `axiom-core/src/ops_merge.rs`(`merge_view` / `merge_worlds` / `merge` / `apply_edits`)。
  - **ADR 10c(幽靈框)**:提案的「之後」= 目前 revision 套用提案 edit(也就是核准會得到的);revision 的「之前」= 前一個 revision。主執行緒只讀一兩列 SQL(`proposal_edits` / `revision_json`),解碼、套用、編譯兩份世界、比較在 `jobs::Job`;結果依(來源, revision)快取(提案依目前 revision、歷史 revision 永遠不變),每幀只畫線段(最多 64 個框)。邊界 = `render.mesh` 零件的 AABB,沒有零件的物件畫 0.5 m 樞紐框;世界矩陣抽成 `world::compose_matrices`(`RuntimeState::matrices` 改用同一份,行為不變)。
  - **內容**:core `store.rs`(`Author`、欄位遷移、`set_author`、`revision_json`、`proposal_edits`、`proposal_conflicts`、`history` 的 `author` / `proposal`、`ResolveProposal {keep, expected_rev}`)、`ops_merge.rs`、`protocol.rs`(兩個可省略欄位)、ACL 讀取 `conflicts N` 與 `history` 的 `by=` / `proposal=#`;CLI `resolve --merge --keep PATH --expected-rev N`、CLI 連線作者 agent。runtime:`writer.rs`(每筆帶作者)、`bridge.rs`(agent)、`app.rs`(`requester`、`LoadGhosts` / `PrepareMerge` 動作)、`world.rs`(`compose_matrices`);編輯器 `ghost.rs`、`merge.rs`(合併視窗、`merge.*` 指令)、`ui.rs`(History 作者標記、點列 = 該 revision 的幽靈框;Proposals 的 Ghosts 切換與 Merge…;Inspector 過期草稿的 Merge…;Hierarchy 右鍵 Copy Context for AI)、`viewport.rs`(幽靈框、視口右鍵選單)、`commands.rs`(`view.ghosts`、`ghost.show`、`merge.open|pick|commit|cancel`;`merge.commit` 列入 `CONTENT_COMMANDS`;偏好 `ghosts`;`discover editor_commands` 的說明)。
  - **證據**:workspace 測試 636 → 648 passed / 0 failed(core `tests/coediting.rs` 6:衝突判定與一方獨改、`unknown_conflict`、物件新增 / 刪除與相對順序、提案逐欄合併一筆 revision + `conflicts` 輸出 + `expected_rev` 必填與不符、作者記錄與 `history by=`、舊 schema 資料庫開啟 / 再開 / 舊式 INSERT、沒改到東西的核准不掛 `resolved_rev`;runtime `coedit_tests` 5 項真實 egui 事件:新提案自動 focus → 只要求一次載入、幽靈框 1 個(Moved,x 0 → 3)、30 幀 0 次重建 0 個 action、`observe editor.ghosts`、點 History 第 2 列 = 該 revision 的框、`ghost.show off`、未知提案回報 error;Proposals ▸ Merge… → 點 Current 格 → Commit merge = 一個 `ResolveProposal {keep, expected_rev}`,結果世界 = 無頭 `conflicts` + `resolve_proposal {keep}`;Inspector 過期草稿 ▸ Merge… → 改草稿後 Commit 會重列 → pick → 一筆 apply,結果 = 在同一 base 上 `propose` 草稿 edit + `resolve_proposal {keep}`;視口右鍵物件 = 選取 + 選單 ▸ Copy Context for AI、RMB + W 飛行放開不開選單、Hierarchy 右鍵同一項;指令阻擋原因與 Play 中 `merge.commit` 停用;`app::revisions_record_the_person_the_agent_and_the_approved_proposal`(writer 執行緒:編輯器 = human、bridge `editor_view {command}` = agent、Proposals 核准 = human + proposal 1);writer / bridge 測試加作者斷言)。`cargo fmt`、`clippy --workspace --all-targets -D warnings` 乾淨;`editor-smoke-test.ps1` ok:true(data/ 的 smoke DB 已刪,0 個殘留);`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 無差異。**舊資料庫相容**:用改動前建置的 release exe(HEAD 前、無 author 欄位)建立資料庫並寫入 → 新 exe 讀 `history`(舊列沒有作者)、寫入(`by=agent`)→ 舊 exe 再寫入成功 → 新 exe `verify` ok。**demo 步驟 8、9(真實 debug exe、Axiom Town、只用 bridge)**:agent 用 `acl` 與 `editor_view {command}` 各改一筆;`acl --propose` 送出提案後編輯器自動 focus,`observe editor.ghosts` = ready 3 個(lamp-1、tree-4 移動,lamp-3 刪除);另一筆 agent 寫入讓 lamp-1 衝突,`conflicts 1` 列出 1 欄;`merge.open proposal:1` → `merge.pick …=theirs` → `merge.commit` → revision 5 `by=agent proposal=#1`,lamp-1 保留 [3,0,7]、tree-4 = 提案的 [7,0,12];`ghost.show rev:5` = 2 個框;每一步 `editor_view --capture`。截圖(debug,已目視,邏輯 1920×1080 / 1366×768 = 2880×1620 / 2049×1152):`artifacts/captures/m10-{ghost,merge,history}-{1080,768}.png`。截圖發現並修正:幽靈框的青色和觸發區 gizmo 同色(改成白 → 天藍、線寬 2)、左下角說明被 FPS 膠囊蓋住(移到左上、深色底)、合併視窗欄位顯示跳脫後的路徑且被截斷(改成 `plaza/lamp-1 · core.transform/position`)、Proposals 的過期說明在列尾被切(獨立一行)、History 作者標記太小(改成固定寬度的膠囊)。
  - **量測(release)**:同一個 release 建置內的 A/B(MX150,中位數;3 輪,第一輪與其他編譯重疊另列),`coedit_tests::measure_m10_coedit` 與 `tests/coediting.rs::measure_history_query`(`#[ignore]`):**整個無頭編輯器幀(Axiom Town、Scene 可見)**:幽靈框關 0.83 / 0.75 / 0.73 ms、最大 64 個框 0.94 / 1.43 / 0.88 ms、再關 0.80 / 1.57 / 0.73 ms → 64 個框約 +0.1–0.15 ms(第二輪三個數字都在跳,雜訊),一般提案只有幾個框;關閉時沒有成本(不畫、不配置)。**背景 job(每個來源 / revision 一次,不在主執行緒)**:Town 幽靈框計算(解碼後套用 + 編譯兩份世界 + 比較)3.9–4.9 ms;`merge_view` 1.1–1.3 ms;`merge`(建出 ReplaceWorld)1.7–2.3 ms。**`history 64`(400 revision、200 個已核准提案,主執行緒,只在 revision 改變時)**:舊查詢 52–64 µs;新查詢在加索引前 2286 µs(相關子查詢掃整張 proposals,每列都是整批 edit)→ 加 `proposals(resolved_rev)` 索引後 185–378 µs,也就是每次 revision 改變多約 0.13–0.3 ms,遠低於 16 ms 預算,但不是零。沒有做整個 exe 的 A/B(M8 已證明那種量法分辨不出這個量級)。
  - **對抗式審查**:兩輪(另開 context,只給需求與 diff)。第一輪 6 bug + 8 風險 + 4 小問題,已修:草稿合併送出前不再檢查(改為 Commit 與送出寫入前都檢查 `merge_outdated`:revision 變了、有編輯正在存、草稿在列出衝突後被改過 → 重列,不寫入)、合併寫入可能被排在使用者自己的編輯後面、被改寫 `expected_rev` 後蓋掉那筆編輯(存檔中不能開始 / 提交合併)、合併視窗開著時再改的草稿被丟掉(草稿 edit 指紋)、沒改到東西的核准仍把 `resolved_rev` 指向別人的 revision(History 會誤標提案)、bridge 請求後主連線作者停在 agent(請求結束後重設)、新增物件的插入位置是 O(n²)(往回找第一個存在的前一個物件)、`keep` 沒帶 `expected_rev` 時會替人決定沒看過的衝突(store 必填、CLI `--merge` 需要 `--expected-rev`)、存檔中列出的衝突根據的是樂觀世界(存檔中禁止開啟)、提案 edit 在主執行緒解碼(改在 job 裡,`Store::proposal_raw`)、`ghost.show off` 會被新提案打開(記住關閉)、順序單位路徑 `/entities/@order` 會和 ID 為 `order` 的物件撞名(改 `/entities#order`)、沒寫入的核准仍加 undo 與「committed」訊息。第二輪:沒有新問題;剩下第一個問題的窄窗(見限制)。另外 release 量測發現 `history` 的提案查詢沒有索引(見量測),已加索引。
  - **未做 / 限制**:合併單位是頂層欄位,巢狀的物件型欄位(例如一個資源欄位底下的多個材質)、prefab、場景、systems 整個算一個,雙方改到同一個大欄位的不同子項也算衝突,而且超過 256 B 的值在視窗裡只顯示摘要;prefab 繼承的元件:草稿改繼承欄位會變成本地覆寫(一個 Component 單位),另一方改 prefab 本身不算衝突,覆寫會固定住舊值(語意衝突沒有偵測)。`1` 與 `1.0` 視為不同值(只會多報衝突,不會漏)。草稿合併只支援 Inspector 的物件草稿(資源 / 材質 / prefab 草稿沒有 Merge…);合併寫入是整份世界的 `ReplaceWorld`(主執行緒會 clone 一次 edit 做 staging,大世界有成本,見量測);草稿合併的寫入送出後到 writer 提交前若又有別人提交,CAS 失敗、草稿已重設(錯誤有顯示,內容沒有被寫壞);透過 bridge 觸發的草稿合併,寫入在下一幀送出,作者記為 human(內容是人的草稿),bridge 回覆不等提交。幽靈框:沒有 `render.mesh` 零件的物件(glTF、空物件)只畫 0.5 m 樞紐框;只比較世界矩陣與元件,不比較 runtime 產生的物件;Play 中不畫。右鍵選單、History 標記沒有截圖驗證右鍵選單本身(bridge 無法送滑鼠事件),由 egui 互動測試覆蓋。截圖中的 History 沒有「You」標記(截圖腳本只能用 agent 路徑寫入;human 由 app / egui 測試驗證)。

### 後續 / follow-ups

- **FA 編輯器 UI 後續(2026-11-11)**,以 `rigorous-change` 執行,主線工作樹,未提交(留給使用者驗證)。六項:
  1. **差異路徑可讀**(`editor/diff_view.rs`):Proposals / History 的每行改成 `plaza/lamp-1 · core.transform/position[0]: 4 → 6`(解 `~1` / `~0`、去掉 keyed 的 `@`、陣列索引 `[i]`、關係 `a · parent` / `a · kind → b`),值被 store 省略時顯示 `‹N bytes›`;`order` 變更以兩邊都有的 ID 做最長共同子序列,顯示 `↕ entities order: moved X, Y +N (M items)`,只是新增 / 刪除造成的 `order` 不顯示(另有 `+` / `-` 行),清單被省略時寫「changed (list too long to show; ACL `diff A B` has it)」。文字在 `set_proposals` / `set_history_diff` 時算一次,不逐幀格式化;滑過一行顯示原本的 JSON pointer。協定、ACL、`diff.rs` 未改。
  2. **Shader source 卡片**(`shader_editor.rs`):卡片寬度 = 可見寬度(Inspector 其他列太寬時 `available_width` 會超過可見範圍,這是溢出的根因,M8 的 `vector()` 也是同樣處理),標題 `truncate`,狀態標記 / 按鈕列 `horizontal_wrapped`,錯誤訊息 `Label::wrap`;原始碼在 `ScrollArea::both`(捲軸固定在卡片邊),行號欄不在捲動區內,依原始碼的垂直位移畫,所以行號、錯誤行標記與原始碼對齊。
  3. **Graphs 縮放**(`graph.rs`):`zoom`(0.25–2.0)、`to_screen` / `to_canvas`,節點 / Any state / 轉移 lane / 文字 / 箭頭 / 格線都乘上縮放;滾輪(或 Ctrl+滾輪 / 觸控板捏合,二擇一,egui 兩者都回報)在游標處縮放並吃掉捲動;拖曳、拉線、平移進行中不縮放;拖曳的抓取偏移是螢幕座標、位置換回畫布單位;port 的抓取半徑隨縮放縮小(下限 0.5×)。`graph.frame` 改成縮到全部可見(上限 100 %)並置中,最小縮放也放不下時左上對齊。新指令 `graph.zoom in|out|reset|FACTOR|N%`(以上次畫布中心為準)、工具列百分比按鈕(= reset)、`observe editor.graph.zoom`。Play Maximized 的唯讀 FSM overlay 固定 100 %。
  4. **Tags 摘要列**(`world_panel.rs`):「Tags (N)  a, b, … ›」(摘要字串隨 tag 快取一起建,依 revision / 選取),點擊展開 / 收合原本的 chip 編輯器(`TagUi.expanded`,換選取時保留);單選時不再重複顯示「TAGS」標題;`object.tag` 不帶參數時展開。
  5. **Prefab 覆寫標記**:core 新增 `ops_prefab.rs`(`prefab_overrides`、`revert_prefab_field`、`apply_prefab_field`)+ ACL `prefab revert SEL COMPONENT/FIELD`、`prefab apply ID COMPONENT/FIELD`(`acl::component_field`)+ 編輯器指令 `prefab.revert` / `prefab.apply`(Object 選單,列入 `CONTENT_COMMANDS`,沒選取 / 草稿未套用 / Apply 多選時停用)。覆寫 = 實例元件 patch 提供的欄位(和 `merge_json` 一致:物件逐鍵,其他值含陣列整個取代);只看 prefab 也有的元件。編輯器 `prefab_overrides.rs`:集合依(revision, world 版本, 選取物件)快取;`form::Ctx.overrides` 讓表單欄位標籤(可右鍵)與物件 / 陣列標頭畫藍色條(含覆寫的上層較淡),Transform 三列用標籤那一行的點擊區;右鍵選單 Revert to Prefab / Apply to Prefab,一筆 revision。
  6. **Game 分頁 HUD 縮放**(`game.rs` `hud_scale` / `scale_game_input` / `hud_rect`,`app.rs` 3 行):遊戲 context 的螢幕 = 遊戲區 ÷ s、指標位置與點單位的滾輪 ÷ s、pixels_per_point = 編輯器的 × s,s = min(遊戲區 / 視窗)(以 1/32 為級距向下取整,夾在 0.25–1);也就是 HUD 用 Play Maximized 的空間排版再縮小畫進遊戲區。Behavior overlay 的矩形同步換算。
  - **證據**:workspace 測試 648 → 661(660 passed;唯一的失敗是已知的計時測試 `app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result` 在其他 agent 同時編譯時超過 250 ms,單獨重跑 3/3 通過;4 ignored)(新增:core `ops_prefab::tests` 3;runtime `diff_view::tests` 2、`followup_tests` 6(差異行與字型字元、1366×768 的 shader 卡片邊界 / 換行 / 捲動 / 錯誤行對齊、Tags 摘要展開與移除 = ACL、prefab 標記右鍵 Apply / Revert = ACL 且集合不逐幀重建、指令停用、Game 分頁 HUD 與最大化版面比例相同且像素密度 0.5)、`graph_tests::zoomed_canvas_hit_tests_and_drags_in_canvas_units`(滾輪 2× 以游標為中心、2× 與 ½× 拖曳各一筆 = `fsm M node`、點選、上下限、參數錯誤、百分比按鈕、Frame 全部可見)、`parity::prefab_revert_and_apply_match_acl`;`world_tests` 的 tag 測試先點摘要列)。`cargo fmt`、`clippy --workspace --all-targets -D warnings` 乾淨;`editor-smoke-test.ps1` ok:true(data/ 的 smoke DB 已刪,協定的 diff 路徑仍是跳脫的 JSON pointer);`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 無差異。GPU 截圖(debug,已目視,邏輯 1920×1080 / 1366×768):`artifacts/captures/fa-{diff,shader,graph-zoom,graph-frame,tags,tags-open,prefab,game-hud,game-max}-{1080,768}.png`。
  - **量測**:沒有做 release A/B,**不宣稱效能變化**。設計上:差異行、tag 摘要、覆寫集合都在資料改變時算一次;圖的縮放只多幾個乘法;Game HUD 的縮放以 1/32 為級距(egui 每換一次 pixels_per_point 會重建遊戲 context 的字型圖集,拖曳分隔線時不會每幀重建)。
  - **對抗式審查**(另開 context,只給需求與 diff):1 bug + 6 風險 + 7 小問題。已修:`order` 變更在單純新增 / 刪除時也顯示「moved」(改成只比較兩邊都有的 ID,沒移動就不顯示,fixture 修正);HUD 縮放每幀改變 pixels_per_point 會重建字型圖集(1/32 級距);Transform 標記的點擊區在窄 Inspector(標籤在上一行)會蓋住 X 欄(限制為標籤那一行);編輯器與 ACL 在混合選取上的 Revert 行為不同(op 改為略過沒有繼承此元件的物件,沒有任何一個才報錯,編輯器傳整個選取);拖曳中滾輪縮放讓節點跳開(按住時不縮放);原始碼橫向捲軸在長檔案時只在最底端看得到(改成單一 `ScrollArea::both` + 同步的行號欄);Play overlay 跟著分頁縮放並覆寫畫布尺寸(固定 100 %、只有互動畫布記尺寸);滾輪點單位位移沒換算;順序清單的 ID 被誤當成跳脫字串;port 半徑不隨縮放;`/core.transform/x` 開頭斜線的欄位。記錄、不修:帶說明的覆寫欄位有兩個 tooltip 疊在一起;`join_fields` 會去掉任何以 `@` 開頭的欄位名的 `@`、數字物件鍵顯示成 `[n]`。
  - **補記(偏離 / 假設)**:「HUD 依遊戲 UI 的參考解析度縮放」解讀為 Play Maximized 的排版空間(整個編輯器視窗的 points),不是 UI Editor 的預覽解析度(那是畫面偏好,不在世界裡);這樣 Game 分頁看到的就是最大化版面等比縮小。Apply to Prefab 只在單選時提供(多個實例的值可能不同)。標記用 prefab 藍色(紅色 accent 在編輯器表示選取 / 錯誤)。History 的差異沒有透過 bridge 截圖(沒有指令可以載入某個 revision 的差異;Proposals 與 History 用同一份 `diff_view::lines`,History 由 egui 測試涵蓋)。Changes 分頁的路徑沒有改(不在需求內)。
  - **未做 / 限制**:覆寫標記只在元件表單、物件 / 陣列標頭與 Transform 三列;陣列內的項目、JSON 模式、prefab 本身新增的元件(實例沒有的)沒有標記,實例自己新增(prefab 沒有)的元件也不算覆寫;Revert / Apply 沒有鍵盤或工具列入口(右鍵與指令)。Graphs 縮放在第一次畫出畫布前的 `graph.zoom` 以左上角為準;UI Editor 畫布仍沒有縮放。Game HUD 的鍵盤 / 滑鼠視角輸入不受縮放影響(只有位置類事件換算);窄 Inspector(標籤疊在上一行)的 Transform 標記沒有測試或截圖。

- **FB 執行期 tag interning、`camera.blocker` 型別化、VFX 多點曲線 / 漸層(2026-11-11)**,worktree 分支,以 `rigorous-change` 執行:
  - **1. Tag interning**:新模組 `axiom-core/src/tags.rs`:`TagIndex`(每個編譯後世界一份;tag 名稱依實體順序首次出現編號,`HashMap<Box<str>, u32>`)+ 每個 slot 一個 `TagSet`(id < 64 用一個 `u64`,其餘排序的 spill 清單;≤ 64 種 tag 的世界不配置)。`CompiledWorld.tag_index` 由 `compile` 建立、`RuntimeState::spawn` 依**名稱**重新 intern(RTS 的模板是在另一次編譯產生的,id 不通用),`RuntimeState::validate` 檢查「每個 slot 一組」不變量。改用 id 的查詢:FSM `nearest_slot`(near / far / move_toward / move_away / attack / face)、`trigger.zone` / `interact.target` 的 filter 掃描(每次掃描時 resolve 一次,spawn 引入的新 tag 也看得到)、selector 的 `tags`(`local::matches`)。編寫端的 tag 字串、World IR、`CompiledEntity::tags` 不變。
    - **量測(release,MX150 機器,`perf_bench --generated-fsm 256x1024`,256 個 FSM agent 每步各做 2 次 nearest-tag 掃描,1280 個實體;base = HEAD 57b6702 + 同一個 bench 模式,`git archive` 匯出另外建置;7 輪交錯)**:step p50 中位數 **base 8.05 ms → 2.64 ms(約 3.0×)**,mean 9.45 → 3.26 ms;每步配置 0 不變。第一組在背景有 debug 建置時量得 16.0 → 6.05 ms(同方向)。`--pack content/effects-tour` 5 輪:step 0.016 / 0.019 ms、render_frame 0.33 / 0.34 ms(雜訊內,沒有回歸)。
    - **記憶體(誠實)**:這是**增加**不是減少 —— 字串 tag 仍留在 `CompiledEntity`(協定輸出與跨世界模板需要),index 每個實體多 24 B(`TagSet`)+ 每種 tag 一個 map 項;1280 實體 ≈ 31 KB。`RuntimeState::clone` 配置次數 30950 → 30991(+41)。process private bytes 中位數 14.05 vs 14.96 MB,在雜訊內,不宣稱。
  - **2. `camera.blocker`(ADR)**:選**獨立標記元件 `camera.blocker`({},capability `camera`)**而不是 `render.mesh` 的欄位 —— render.mesh 在 renderer 熱路徑上、有 19 處 struct literal;元件可用既有的 add/remove、prefab 繼承、tween/mover 衝突檢查,沒有 mesh 時不擋任何東西(不當錯誤,避免刪 mesh 被拒)。舊 tag 照樣擋(`camera::mesh_blocks`),`verify` 的 `diagnostics` 回 `deprecated_tag`;`ops::set_camera_blocker` = ACL `camera SEL blocker on|off` = `object.camera_blocker on|off` = Inspector render.mesh 卡片「Blocks follow cameras」勾選(只作用在被檢視的物件);`ops::migrate_camera_blocker_tags` = ACL `migrate camera_blocker` = `axiom migrate [--preview]` = World ▸ Migrate Legacy Camera Blockers。tween 衝突訊息分成 `render.mesh with camera.blocker` / `render.mesh tagged camera_blocker`。
    - **補記**:任務說遷移後 `build-street` 會刻意改變 —— 實際上**沒有任何內容套件、範例或腳本用到這個 tag**(只有測試),所以 content 不需要遷移,`build-street.ps1` 後 `content/street` 無差異。
  - **3. VFX 多點曲線**:emitter 新增選擇性的 `size_curve`([{t,size}])與 `color_gradient`([{t,color}]),2..8 個 key、t 0..1 嚴格遞增;沒有時與舊公式位元相同(測試釘住)。CPU `vfx::size_at` / `color_at`(`piecewise`),GPU uniform 加 `curve_counts` + 打包的時間 / 值(352 B,動態 offset stride 256 → 512,bank 64 × 512 B),WGSL 同樣的運算順序。`verify_vfx_gpu`:15 組(6 組有曲線,含 8 個色標)、960 粒子讀回,最大誤差 2.1e-6。Ops `VfxEdit::SizeCurve` / `ColorGradient`(四捨五入後再驗證;空清單 = 移除欄位,寫回整個 emitter —— 沒有移除單一欄位的 edit)、ACL `vfx ID size_curve T:S ..|none`、`vfx ID gradient T:r,g,b,a ..|none`。VFX 分頁:每個 key / 色標一個把手,拖曳(中間的也能沿生命移動)、雙擊或 + 新增(最寬間隙,含兩端)、Remove、Two-point / Two-stop;仍是草稿 + 放開時一筆修訂;兩點 emitter 保持寫 `size` / `end_size`。
  - **測試** 648 → 660(tags 單元 2、trigger 1、`tests/tags.rs` 1、parity 1、camera_follow 1、tuning_ops 1、effects 1、tuning_tests 1、vfx_editor 單元 2、inspector 1,另外既有測試擴充)。
  - **驗證**:fmt / clippy -D warnings 乾淨;workspace 660 pass / 0 fail / 4 ignored;editor-smoke ok:true(DB 已刪);editor-world-parity / street-editor-parity identical:true;build-street 後 content/street 無差異;`verify_vfx_gpu` ok。`effects-smoke-test.ps1` 兩次:36 站擷取與所有 live 檢查(focus 重試、材質本機編輯、GPU emitter 不重啟、pause / inspect / restart、預算拒絕、頂點階段失敗保留可用修訂、undo 修復、delta)都通過,但最後 900 幀的原生 helper 超過腳本的 60 秒(單獨量:debug 隱藏視窗 900 幀 188 秒,機器上另有一個長時間滿載的 `find` 程序);release 對照 render_frame 沒有差異,判定是環境因素,**未取得 PASS**。擷取已目視:`fx-02-gpu`、`fx-07-smoke`(GPU / CPU 粒子正常)。
  - **截圖(debug,已目視)**:`artifacts/captures/fb-vfx-{1080,768}.png`(4 key 曲線 + 4 色標漸層)、`fb-blocker-{1080,768}.png`(render.mesh 卡片的勾選 + camera.blocker 元件)。修正:完全透明的色標標記幾乎看不見(標記改不透明)。
  - **審查(另開 context,只給需求與 diff)**:修正 —— 鄰近 key 少於 0.02 時拖曳的 clamp 上下界顛倒會 panic(`t_range` 回 None、+ Key 需要 ≥ 0.02 的間隙);`set_camera_blocker` 對 prefab 繼承的標記:on 不再寫多餘覆寫,off 一律要求改 prefab;遷移對沒有 mesh 的實體只移除 tag;Inspector 勾選只作用在被檢視的物件(不是整個多選);隱藏分頁中斷的拖曳不再卡住提交、`vfx.select` 重設拖曳 / key 索引;曲線先四捨五入再驗證;`widest_gap` 納入兩端。記錄不修:移除曲線時寫回整個 emitter(M10 逐欄合併時衝突面較大;store 沒有「移除欄位」edit);ribbon 不使用曲線(寬度 = `size`、顏色 = `color`,和以前一樣不用 end_*);VFX 分頁可見時每幀有少量 Vec 配置(只在 UI)。
  - **沒做**:編輯器 Hierarchy 搜尋的 `tag:` / `has:`(編寫端字串、屬 A 的 UI 範圍);per-domain layers;FSM 條件預先 resolve tag id(每次查一次 hash,已足夠)。

## 修訂補記

- 2026-11-08:決策 2(egui_dock)選定後,原計畫沒有把「可停靠 / 可重排面板與版面存檔」排進任何里程碑 → **併入 M2**(與 Hierarchy、視口手感一起,版面存在 `*.editor-prefs.json`)。理由:M1 先把所有入口集中到指令註冊表;重排面板需要把現有固定面板改成 TabViewer,和 M2 要改的 Hierarchy、視口剛好是同一批程式碼。
- 2026-11-09:M4 的「runtime `camera.activate` 指令」已在 camera-follow 里程碑(plan-engine-components 第 8 項)完成,M4 不重做;M8 範圍內的「Play 中的唯讀即時 Inspector」提前在 M4 做了基本版(transform + `*.inspect` 結果),M8 只剩事件 trace 與 Profiler 等。Play 改在編輯器內進行後,Play 期間編輯器唯讀(原本 Play 時沒有編輯器 UI,bridge 的 `editor_view {command}` 仍可寫入;現在一律拒絕並說明)。
- **M6 狀態機與行為圖編輯器(2026-11-09 晚)**:
  - **ADR:egui-snarl vs 自製節點圖。** S2 證實 `egui-snarl 0.7.1` 可用,依賴只有 egui、slab(加 serde feature 才有 serde;依 spike 的 Cargo.lock),原始碼約 5.6 千行。但它的連線是 `HashSet<(OutPinId, InPinId)>`(`src/lib.rs` 的 `Wires`):同一對腳位只能有一條、沒有順序。FSM 的轉移**有優先順序**(第一個通過的勝出)、同一對狀態可以有多條不同條件的轉移、還有 `*`(任何狀態)來源,用 snarl 表達要每條轉移各開腳位或插入中間節點,並且要在 snarl 的節點位置與 `editor.layout` 之間雙向同步。自製畫布只需要矩形、直線箭頭 + 條件標籤、拖曳移動 / 拉線、平移,約 1 千行(`editor/graph.rs`,含側欄與規則表),也不把版本綁在另一個 egui 套件上(pre-mortem #6)。→ **選自製,不加依賴**(所以沒有建置時間 / 執行檔大小成本可報)。
  - **資料模型**:節點位置存在新資源 `editor.layout`(`{"graphs":{"fsm/MACHINE":{"STATE":[x,y]}}}`,core 註冊、capability `core`,驗證:≤256 張圖、每張 ≤64 節點、座標有限且 ±100000 內;runtime 沒有系統讀它)。位置四捨五入到整數畫布單位。沒存位置的狀態依網格自動排(避開已存的位置)。
  - **內容**:core `ops_graph.rs`(從 `ops` 重新匯出):`fsm_create_machine`、`fsm_remove_machine`(仍被 `fsm.agent` 使用時拒絕)、`fsm_edit`(`FsmEdit`:AddState / RemoveState(連同進出的轉移與位置;initial 不可刪)/ RenameState(initial、轉移、位置跟著改)/ SetState / SetInitial / AddTransition(預設等 120 tick)/ RemoveTransition / MoveTransition / SetTransition)、`fsm_set_node`、`fsm_add_agent`(機器不存在就建立)、`add_behavior`(patrol / guard / dialogue 範本;dialogue 宣告 `near_MACHINE` 事件,已宣告且欄位相容就沿用)、`new_rule` / `set_rule` / `program_selection`、`fsm_problems`(`fsm_unreachable`、`fsm_dangling`、`fsm_undeclared_event`、`fsm_event_fields`、`fsm_needs_behavior`、`fsm_unknown_machine`)。ACL 新動詞 `fsm M create|remove`、`fsm M state add|remove|rename|set`、`fsm M initial`、`fsm M transition add|remove|move|set`、`fsm M node S x,y`、`fsm SEL agent M [radius=]`、`behavior SEL patrol|guard|dialogue [..]`;規則表沿用既有的 `append|remove|reorder resources[id=behavior.program] rules` 與 `set rules[id=..]`。runtime:`FsmSystem::agent_states()`(不配置記憶體)、`fsm::editor_kinds`(每種動作 / 條件的範本與型別化 schema,和驗證器放在一起,測試確保範本合法)。編輯器:Graphs 分頁改成 State machine / Rules / Overview(舊的唯讀總覽);畫布、側欄(狀態:改名、Make initial、Action、Emit、Clip;轉移:From / To、When、優先順序)、右鍵選單、雙擊新增;kind 下拉換成該 kind 的範本,其餘欄位走 M3 `form::show`(範圍 / 單位 / 事件與變數下拉);規則表(巢狀條件與 operand 也是 kind 下拉);Problems 的 STATE MACHINES 區段(Open graph);Inspector 的 Behavior 下拉(FSM Agent… / Patrol… / Guard… / NPC Dialogue…,選項對話框顯示 ACL)與 fsm.agent 標頭 Edit Graph;指令 `behavior.fsm_agent|patrol|guard|dialogue`、`graph.open|add_state|set_initial|delete`;`editor_view {open:"graph:M"|"rules:"}`;`observe` 的 `editor.graph`。Play:這個分支 Play 時畫面換成遊戲 UI(M4 之前),所以由 Play 預覽的 Behavior overlay 在左下角畫唯讀的機器圖並標出每個 agent 的狀態(`app.rs` 只多一行呼叫);Graphs 分頁本身在 `previewing` 時也會標(M4 合併後即可看到)。
  - **證據**:workspace 538 → 553 passed / 0 failed(最終版;clippy -D warnings、fmt 乾淨)。core `ops_graph` 單元 4 項(狀態 / 轉移 / 改名 / 刪除 / 位置、檢查、範本與規則、版面預算);runtime `fsm::tests`(每種 kind 範本通過驗證器);egui 實際輸入 `graph_tests`:+ State、拖曳節點 8 幀(按住時 0 筆、放開 1 筆)、從 port 拖到節點 = 新轉移、When kind 下拉 + 拖曳 `within`、Ctrl+A 輸入 Enter 改名、右鍵 Set as initial state、點標籤選轉移 + Delete、刪除狀態,每一步都和 ACL 結果相同,之後 30 幀 0 次重建;雙擊新增(狀態 + 位置一筆)與 Action kind;檢查顯示在節點 / Problems 並可 Open graph;Play 高亮(主角走到 Lumi 250 tick → greet `live 1`,編輯模式不亮);規則表 + Rule / event / + Action / when kind / Up / x = ACL;Inspector Behavior ▸ FSM Agent 對話框(Lumi 移除後加回 = 原本的 town)、Edit Graph、三個範本對話框 = 顯示的 ACL;town-dance 在圖上加狀態 + 轉移後仍通過;同一幀兩個圖操作 = 兩行 ACL;拖曳中收到外部修訂仍寫到同一個節點。`parity::state_machine_commands_and_templates_match_acl`。CLI:`axiom scenario examples/town-dance.scenario.json` 在 `fsm lumi state add rest; fsm lumi transition add greet rest` 前後都 ok(6 項斷言)。`editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 `content/street` 沒有變化。GPU 截圖(debug,看過):`artifacts/captures/m6-graph-lumi.png`、`m6-graph-new-state.png`(+ State 與一條 near ≤ 1.5 的轉移,側欄是新狀態)、`m6-graph-rules.png`(規則表 + talk-lumi)、`m6-graph-play.png`(Play 左下角 FSM · lumi (live),idle `live 1`)。截圖發現並修正:自動排版擠成一列、轉移標籤看不清 → 依與初始狀態的距離分欄、奇數欄錯開半列;節點副標超出節點 → 裁切;規則列不對齊、規則面板被切掉 → 等寬字 + 各自捲動。
  - **對抗式審查**(另開 context,只給需求與 diff)找到 4 個 bug、7 個風險,已修:同一幀(或寫入顯示前)的兩個圖操作各自從舊世界產生整份 `fsm.machines`,後者蓋掉前者 → 暫存世界疊加 + 合併成一筆;重建模型後拖曳 / 拉線 / 右鍵選單的索引失效(可能 panic 或寫到別的節點)→ 依名稱跟到新模型、右鍵選單清除;轉移 / 規則草稿只記索引,外部改動後會寫到別的元素 → 送出前比對載入時的值,不符就放棄並說明;換選取時未存的草稿被丟掉 → 先寫入;dialogue 範本遇到已宣告(無欄位)的事件被拒 → 欄位相容就沿用;Rules 每幀複製整個 program → 借用;檢查時複製每個物件的元件 → 只查 `fsm.agent`;放開時畫布沒看到 → 留下拖曳狀態 → 沒按鍵就清除;檢查比 runtime 寬鬆 → 加 `fields` 必須存在與 `fsm_needs_behavior`;`observe` 的問題數只在分頁畫過才更新 → 過期時現算;`fsm_unknown_machine` 的 Open graph 改選取該物件;改名 / 刪除走一般 `set` 留下的位置 → `fsm_set_node` 時清掉。
  - **未做 / 限制**:畫布沒有縮放 / 適合視窗(只有平移),小視窗要拖空白處找節點;`all` 條件裡的子條件與事件的 `fields` 對照表以 JSON / 物件欄位編輯(沒有逐項 kind 下拉);narrative.graph 仍唯讀(Overview);Play 中編輯器 UI 由遊戲 UI 取代(M4 之前),Graphs 分頁的高亮只在 egui 測試驗證,實機看到的是 overlay 小窗;Delete / F2 快捷鍵仍屬於 Hierarchy(圖上用按鈕與右鍵);側欄每幀仍會複製選取元素的 JSON(一個狀態 / 轉移,幾百位元組);新轉移預設條件是「120 tick 後」,不是空條件;模板機器 ID 取 `patrol`、`patrol_2`…,不會重用已有的同名機器。
  - **量測(release,MX150,Axiom Town,同一個版面:Graphs 分頁在下半部可見;每格 20 個不同慢幀的主執行緒分項 p50;基準 = b02ce69 在本機重建的 release,本版 = 8eb7704 之前的 review 修正版;交錯執行;**同時有其他 agent 在編譯,數字雜訊大**)**:UI(`jobs_overlay_ui`)基準(Graphs 是舊的唯讀清單)1.80 / 1.90 / 1.89 ms,本版(Graphs 畫 lumi 的節點圖 + 側欄)1.26 / 1.59 / 1.60 / 1.73 ms —— 在雜訊內,**沒有看到節點圖的額外成本,也不宣稱變快**(舊清單本來就逐行排文字)。extract 3.2–5.8 vs 2.9–3.0 ms(與本里程碑無關,雜訊)。程序 private 記憶體 447–449 MiB vs 449–479 MiB(本版四次中兩次 476–479,兩次 449,未拆解,可能是 egui 版面快取或驅動配置;working set 351–359 vs 351–365 MiB)。整合時請重量。debug 單元測試:30 幀模型重建 0 次。
  - **效能設計**:畫圖用的模型依 (world version, 已排入的圖寫入數) 快取,只有修訂改變才重建(測試:30 幀 0 次重建);每幀最多畫 64 個節點、128 條轉移,畫布外的跳過;Play 高亮用固定大小的計數陣列;檢查結果依修訂快取,取 agent 的機器時不複製元件。同一幀(或寫入顯示前)的多個圖操作疊在一份暫存世界上,後一筆整份資源寫入不會蓋掉前一筆(對抗式審查找到的問題)。
## 修訂補記

- 2026-11-08:決策 2(egui_dock)選定後,原計畫沒有把「可停靠 / 可重排面板與版面存檔」排進任何里程碑 → **併入 M2**(與 Hierarchy、視口手感一起,版面存在 `*.editor-prefs.json`)。理由:M1 先把所有入口集中到指令註冊表;重排面板需要把現有固定面板改成 TabViewer,和 M2 要改的 Hierarchy、視口剛好是同一批程式碼。

- 2026-11-09(M6):demo 步驟 5 的「Add Component ▸ Behavior ▸ FSM Agent」做成 Inspector 元件區的 **Behavior…** 下拉(與 Add component… 並排;egui 的 menu_button 無法在無頭 egui 測試展開,下拉可以),項目開啟選項對話框。Lumi 在 Axiom Town 原本就有 sm.agent,所以 demo 以「Remove 後從 Behavior ▸ FSM Agent… 選 lumi 加回」驗證(結果與原本的 town 相同),或對 Pix(沒有 agent)操作。節點位置依計畫存在 editor.layout;節點圖不用 egui-snarl(理由見 M6 ADR)。

- 2026-11-10(M8):Profiler 的「GPU 計時若可用」**未實作**:renderer 建立裝置時沒有要求 `TIMESTAMP_QUERY`,要做必須改 renderer(query set、resolve、非同步讀回)並重跑 GPU smoke,超出本里程碑;Profiler 明確顯示「GPU timing unavailable」與 adapter 是否支援。Scenario 格式新增 `state_hash` 斷言(加法)讓錄製檔自我驗證;錄製檔依計畫寫在 `examples/`,但不覆蓋、名稱只能是檔名。Console 進預設版面(版面版本 3,舊版面存成 Previous / Previous N)。

- 2026-11-10(M9):「動畫時間軸 · 事件點」落在 `action.abilities` 的 ability 事件(唯一已存在、和 clip 同步的事件格式),不在 skeletal clip 上新增事件格式;「VFX 曲線 / 漸層」是 emitter 現有的兩端點(size / end_size、color / end_color),不是多關鍵點曲線。若要多關鍵點曲線,要先擴充 `vfx::Emitter` 與 GPU 求值(`vfx_compute.wgsl`),另立里程碑。Shader source 的 Check 是 naga 驗證(和 commit 時相同),不是 GPU pipeline 編譯;GPU 端的編譯錯誤仍走既有的 `ShaderGpu` 回報。另外新增 view 指令 `shader.draft`(替換未存的草稿,供 agent / QA 截圖用)。

- 2026-11-10(M10):「History 標示作者」新增 `revisions.author`(可為 NULL,開檔時自動加,舊版程式照常寫入),提案沿用既有的 `proposals.resolved_rev`,另加 `proposals(resolved_rev)` 索引(release 量測發現沒有索引時 `history 64` 從 50 µs 變成 2.3 ms)。「逐欄衝突合併」的協定是既有 `resolve_proposal` 加兩個可省略欄位 `keep` / `expected_rev`(`keep` 必須帶 `expected_rev`),ACL 加讀取 `conflicts N`;草稿合併沒有新協定,無頭等價是「在草稿的 base 上 propose,再 `resolve_proposal {keep}`」(同一個 `ops::merge`)。合併以 `ReplaceWorld` 一筆提交(理由見 M10 ADR 10b)。「把目前選取交給 AI」沿用 `file.copy_context`,只加進兩個右鍵選單(視口右鍵選單是新的)。

## 完成總結(2026-11-10,M1–M10 全部完成)

demo 腳本九個步驟都有對應的編輯器操作、協定 / `editor_command` 等價物與互動測試;每個里程碑的證據、量測與審查紀錄在上方執行紀錄。

| 里程碑 | 完成 | 延後 / 沒做(詳見各自的「未做 / 限制」) |
|---|---|---|
| M1 外框骨架 | 指令註冊表、選單列、工具列、狀態列、命令面板、快捷鍵與偏好檔、bridge `editor_command` | 選單本身無法在無頭 egui 測試展開(以指令測試覆蓋) |
| M2 Hierarchy、視口、停靠面板 | 右鍵選單、F2 改名、顯示 / 鎖定、同層排序、飛行相機、正交視圖、框選、gizmo、可設定 Snap、視角書籤、egui_dock 版面 | `physics.collider` gizmo 沒有截圖;框選以中心點判定;`drop_to_ground` 不把 glTF 當支撐面 |
| M3 Inspector 2.0 | 型別化 schema、下拉 / 滑桿 / 參照選取、陣列、多物件編輯、即時套用與批次模式 | prefab 覆寫標記與逐欄 Revert / Apply to Prefab;History 不折疊連續修訂 |
| M4 Game 視圖與相機 | Game 分頁、Play 在編輯器內、暫停 / 單步 / 時間倍率、相機預覽、Set as Main Camera | HUD 不隨遊戲區縮放;遊戲 context 的游標 / 剪貼簿 / IME |
| M5 Assets 瀏覽器 | 類型樹、縮圖(背景離屏渲染)、拖放、建立資產、Find Usages、帶參照修正的改名 | 見 M5 限制 |
| M6 狀態機與行為圖 | 可編輯 FSM 節點圖、規則表、行為範本、Play 高亮 | 畫布縮放;narrative.graph 仍唯讀 |
| M7 UI 編輯器與動效 | WYSIWYG 畫布、樣式、在地化預覽、資料驅動動效 | 畫布縮放、圖片 widget |
| M-UI 視覺改版 | 主題、圖示圖集、面板外框、卡片 | Graphs 縮放、Tags 摘要晶片 |
| M8 除錯工具 | Console、Event Trace、Profiler(CPU)、Play 錄製 | GPU 計時(timestamp query);Console 不收系統內部 fault |
| M9 專用編輯器 | Input Map、Localization、Animation(ability 事件)、VFX 兩端點曲線 / 漸層、Shader source 檢查 | 動作改名、CSV 匯入匯出、多關鍵點 VFX 曲線、skeletal clip 事件格式 |
| M10 AI 共編強化 | History 作者、提案 / revision 幽靈框、逐欄衝突合併(提案與過期草稿)、右鍵 Copy Context for AI | 巢狀欄位與 prefab 的細粒度合併、prefab 繼承的語意衝突、草稿合併送出後的 CAS 窄窗、資源 / 材質 / prefab 草稿的合併 |

計畫至此結案;後續改善項目請另立計畫(建議優先:prefab 覆寫標記、Graphs / UI 畫布縮放、GPU 計時、合併的巢狀欄位)。

後續(2026-11-11,見執行紀錄「後續 / follow-ups」的 FA 條目):已補 prefab 覆寫標記與逐欄 Revert / Apply to Prefab(M3)、Game 分頁 HUD 縮放(M4)、Graphs 畫布縮放(M6 / M-UI)、Tags 摘要列(M-UI),並修正 Proposals / History 的差異路徑顯示與 Shader source 卡片在 1366×768 的溢出。

後續 B(2026-11-11,見 FB 條目):執行期 tag 改為 interned 位元集合(FSM 掃描 release p50 約 3×)、`camera_blocker` tag 改成 `camera.blocker` 元件並提供遷移、VFX 多點 size 曲線 / 色彩漸層(CPU / GPU 一致)。

