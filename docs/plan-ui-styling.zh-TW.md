# 計畫:遊戲 UI 的樣式、自由版面與模板(ui.document v2)

狀態:草案,待使用者確認後從 M1 開工。基底:origin/main `3768ea8`。

## Phase 0 — 需求與成功樣貌

### 需求

使用者回報兩個問題:
- UI 做出來都在畫面下方展開。
- 樣式單調。

使用者想要:
- 自己設計 UI 模板;
- 用底圖做邊框。

### 盤點結果(現況,origin/main)

UI 由 `ui.document`(`axiom-core/src/gameplay.rs`)描述,由 `axiom-runtime/src/bound_ui.rs` 用 egui 畫出。下表的限制都寫在格式或繪製程式裡,不是 egui 或 wgpu 做不到:

| 現象 | 原因 |
|---|---|
| 沒放位置的元件都在下方 | 一律進 `TopBottomPanel::bottom`(最高佔螢幕 87%) |
| 位置只有 9 種 | `regions` 只有九宮格錨點,每格寬度寫死(Top 760 px、TopLeft 300 px…) |
| 所有面板長一樣 | 面板背景寫死:RGBA(12,16,24,205)、圓角 8、內距 10 |
| 文字樣式少 | 只有 6 種 `TextStyle`,顏色和字級寫死 |
| 圖片不能當邊框 | 圖片只能當獨立元件,高度上限 170 px,最多 16 張 PNG |
| 字型不能換 | 只讀 Windows 微軟正黑體(`ui.rs::configure`) |
| 只有一個畫面 | 沒有頁面、選單、彈窗的概念 |
| 編輯器只能放九宮格 | UI Editor 只能拖到 9 格、選 6 種樣式 |

本機 main 另有 Paladin session 的未提交改動(`bound_ui.rs` +260 行):
- 進度條的 `color`、`vertical`、`flash_at`、`label`;
- 全螢幕淡出幕。

### 最終驗收 demo(照著演)

1. 開 Axiom Town(新資料庫,從 `content/axiom-town` 建立)。
2. **對話框**:靠近 Lumi 按 E。
   - 畫面底部中央出現一個用**自訂九宮格底圖**做邊框的對話框;寬度隨解析度等比縮放,邊角不變形。
   - 左側是講者立繪,右側是講者名牌和台詞。
   - 字型是內容包自帶的字型。
3. **任務追蹤**:左上角,另一種邊框底圖,沒有深色預設底。
4. **表情按鈕**:右側一個 2×3 格狀容器。
   - 每個按鈕都有 normal / hover / pressed 三種底圖。
   - 滑鼠移上去換圖,按下再換圖。
5. **暫停選單**:按 Esc 開出置中的模態視窗,遊戲暫停。
   - 用鍵盤方向鍵或手把就能在按鈕間移動並確認。
   - 選「繼續」後關閉,遊戲恢復。
6. **AI 改樣式**:AI 代理用 ACL 改主題色和對話框底圖。
   - 遊戲即時套用;`observe ui.layout` 回報每個元件的實際矩形。
   - 結果沒有重疊,也沒有超出畫面。
7. **人在編輯器做同一個改動**:在 UI Editor 拖動、縮放對話框,結果和 ACL 寫出的世界完全相同(parity)。
8. **舊內容不變**:其他舊內容包(plaza、adv、fps…)的截圖和改版前逐位元相同。

### 約束

- 不加 Bevy;繼續用 egui 0.31 + wgpu 24。
- 每種內容編輯都要有:純 `ops` 函式、ACL 動詞、編輯器命令、parity 測試,並在 parity 表加一列。
- 主執行緒每項操作 16 ms 以內:
  - 字型和圖片的讀取與解碼放背景工作(有界、可取消、有 watchdog);
  - 每幀不配置記憶體;
  - 貼圖依內容 hash 快取。
- 每項都要有預算:圖片數、像素總量、字型數與大小、元件數、巢狀深度、模板展開數。
- 協定輸出要精簡;錯誤要可恢復,路徑要指到 `/resources/ui.document/...`。
- 舊的 `ui.document` 必須照常運作:沒用到新欄位的文件,畫面不變。
- 不推送第三方素材:
  - 示範用的邊框底圖用程式產生(`scripts/` 下的生成器,原創);
  - 字型只收開放授權(OFL 等),並在 manifest 註明授權。

### Non-goals(這個計畫不做)

- HTML/CSS 排版引擎、文字輸入法(IME)、富文本標記語言。
- 3D 世界空間 UI(現有 nameplate 以外)、小地圖。
- 自己寫一套 wgpu UI renderer(egui 已足夠,見 ADR-2)。
- 動畫時間軸編輯器(沿用 `ui.effects`)。
- 下拉選單、文字輸入框、分頁元件 → 列為之後的 backlog,不在這份計畫內。

### 未知與假設

- **參考解析度**:假設預設 1920×1080,縮放模式預設 `fit_height`;舊文件不縮放。
- **Paladin 改動的協調**:假設等 Paladin session 提交後才開始改 `bound_ui.rs`。
  - 在那之前,M1 只碰 `axiom-core` 的格式;
  - 或者先請使用者讓 Paladin 提交。見 pre-mortem 第 1 項。
- **手把焦點導覽**:假設需要(demo 第 5 步),放在 M6。

## Phase 1 — 勘查與風險引爆

勘查來源:
- `gameplay.rs`(`UiDocument`、`Widget`、`Region`、`TextStyle`,預算在 `compile` 約 1175 行);
- `bound_ui.rs`(`draw`、`draw_regions`、`anchor`、`draw_widgets`、圖片貼圖快取 `state.textures`);
- `ui.rs::configure`(字型);
- `ui_editor.rs`;
- `acl.rs`(`ui place|style|button|effect|slider`);
- `docs/editor-parity.zh-TW.md` 第 91–95、129、164、182 列。

### 風險表

| # | 風險 | 若不成立 | 驗證 | 結論 |
|---|---|---|---|---|
| R1 | egui 能畫九宮格邊框,並當子內容的底圖、大小隨內容變 | 致命(核心需求) | Spike S1 | **成立** |
| R2 | 能把整層 UI 依參考解析度等比縮放 | 繞得過(改成逐元件乘比例,但很麻煩) | Spike S2 | **成立** |
| R3 | 在 runtime 換成內容包自帶的字型,成本可接受 | 繞得過(只能用系統字) | Spike S3 | **成立**,注意讀檔 |
| R4 | 和 Paladin 未提交的 `bound_ui.rs` 改動合併 | 繞得過但很痛 | 協調 | 見 pre-mortem 1 |
| R5 | 新格式向後相容,舊截圖逐位元不變 | 致命(會破壞所有內容包) | M1 起每個 milestone 做截圖 A/B | 用測試把關 |
| R6 | 每幀成本(多面板、多貼圖)守住預算 | 繞得過(收緊預算) | M3 量測 debug/release | 待量測 |
| R7 | parity:新的 ops、ACL 和編輯器結果相同 | 致命(專案規則) | 每個 milestone 都要有 parity 測試 | 既有模式 |

### Spike 證據

2026-11-26 用 headless egui 跑過(debug),spike 程式已刪除,沒有進 codebase。

- **S1 九宮格:**
  - 作法:先在畫筆上預留一個 `Shape::Noop` 位置,內容畫完後用 `painter.set(slot, mesh)` 換成 9 個 `add_rect_with_uv` 四邊形(36 個頂點)。
  - 繪製順序:這個網格排在文字前面,所以是底圖。
  - 大小:矩形 = 內容 + 內距(648×79),會跟著內容大小變。
- **S2 整層縮放:**
  - 作法:`set_transform_layer(layer, TSTransform::from_scaling(0.5))`。
  - 結果:800,400–1000,600 的矩形 tessellate 後是 399.5,199.5–500.5,300.5。
  - 注意:縮放是以原點為中心。錨點換算要自己處理:先在參考座標排版,再整層縮放。
- **S3 換字型:**
  - 測試字型:21.4 MB 的 msjh.ttc。
  - `set_fonts` 呼叫本身 0.00 ms。
  - 換字型後的第一幀 5.90 ms;對照組(換字型前的第一幀)5.10 ms。之後各幀都在 1 ms 以下。
  - 結論:成本在**讀檔**,不在換字型。讀檔要放背景,在世界啟用時完成。

**GATE 1**:所有「致命 × 沒把握」的項目(R1、R5、R7)都有對策或證據;R5 和 R7 由每個 milestone 的測試把關。

## Phase 2 — 方案設計(ADR)

### ADR-1:擴充 `ui.document` 還是另開新資源

- **背景**:舊文件很多(town、adv、rts HUD…),ACL `ui …` 動詞和 UI Editor 都綁在 `ui.document` 上。
- **A 案:擴充 `ui.document`**
  - 加選用欄位 `theme`、`panels`、`templates`、`screens`;
  - 舊欄位 `regions`、`styles` 保留,當成語法糖編譯成新模型。
- **B 案:新資源 `ui.layout`**,和 `ui.document` 並存。
- **比較**:
  - A 的優點:一份文件、一套驗證、既有綁定可以重用。缺點是格式變大。
  - B 的缺點:兩份文件互相參照,ACL 和編輯器的操作會分成兩邊,parity 面積更大。
- **決定:A。**
  - 新欄位全部是 `serde(default, skip_serializing_if)`,所以舊文件序列化後完全不變。
  - 渲染時,舊文件走「預設主題 + 舊版面」路徑。
- **後果**:`gameplay.rs` 的 UI 型別要拆到新模組 `axiom-core/src/ui_document.rs`,否則檔案太大。
- **回頭重審的條件**:文件大小或驗證時間超出預算時。

### ADR-2:繼續用 egui,還是自寫 wgpu UI renderer

- **決定:egui。**
- **理由**:S1–S3 證明九宮格、整層縮放、換字型都做得到。自寫 renderer 等於重做文字排版、輸入、無障礙和編輯器整合,成本高一個數量級。
- **代價**:egui 的版面是立即模式;自由定位要用 `Area` 加固定大小來排。
- **回頭重審的條件**:同一幀上百個面板時,egui 的成本超出預算。

### ADR-3:版面模型(資料模型,最難改,最先定)

```
panels: [Panel]                      // 根面板,依 z 排序
Panel {
  id,
  anchor: top_left|top|...|bottom_right|center,   // 9 錨點 + 偏移
  offset: [x, y],                    // 參考座標(px)
  size: {w: px|"%"|"auto", h: ...},
  layout: column|row|grid{cols}|stack,
  gap, padding: [l, t, r, b],
  style: STYLE_ID,                   // 主題裡的面板樣式
  when?,                             // 條件顯示(沿用 Condition)
  children: [PanelChild],            // 元件 ID 或子面板(深度 ≤ 4)
  z?,
}
```

- **為什麼是錨點 + 偏移 + 大小,不是絕對座標**:解析度改變時,貼角落的 HUD 仍要貼角落。這和 Unity UGUI、Godot 的錨點模型同類。
- **參考解析度**:`theme.reference = [1920, 1080]`,縮放模式 `fit_height | fit_width | fit | none`,以 S2 的整層縮放實作。
- **舊 `regions` 的處理**:編譯成「該錨點上的一個 column 面板」,寬度沿用舊值,樣式 `legacy`。
- **舊的底部面板**:只有文件裡沒有任何 `panels` 時才使用。有 `panels` 的文件,沒放進面板的元件算錯誤,所以 UI 不會再莫名掉到下方。
- **紙上推演**:demo 第 2–5 步的每個畫面(對話框、任務、表情格、暫停選單)都能用 panel 樹表達。暫停選單 = 一個 screen 加 `modal`(見 ADR-6)。

### ADR-4:主題與樣式

```
theme: {
  reference, scale,
  fonts: {ID: {path, license}},
  text:  {CLASS: {font?, size, color, outline?, shadow?}},
  panel: {CLASS: {fill?, corner?, border?, frame?: FRAME_ID, padding?}},
  button:{CLASS: {normal, hover, pressed, disabled}},   // 每種狀態 = {frame?, fill?, text?}
  frames:{ID: {image, slice: [l, t, r, b], center: stretch|tile|hidden}},
}
```

- 元件新增 `class`(取代 `styles` 對照表)。舊的 6 種 `TextStyle` 變成內建預設主題裡的 6 個 class:數值和現在寫死的一模一樣,所以舊截圖不變。
- **九宮格**:`slice` 以原圖像素為單位;角落不縮放,邊和中央拉伸(`tile` 和 `hidden` 列在 M2 驗收範圍內)。
- **Paladin 的進度條欄位**(`color`、`vertical`、`flash_at`):合併時對應成 progress class 的屬性,不再是一次性的專用欄位。

### ADR-5:模板

- **作法**:`templates: {ID: {params: [..], panel: Panel}}`。使用端寫 `{use: TEMPLATE, params: {...}, id_prefix}`,在**編譯時**展開。
- **理由**:runtime 沒有額外成本;展開後就是普通的 panel 和元件,所以綁定、效果、parity 都不用改。
- **預算**:展開後的元件總數 ≤ 256、深度 ≤ 4、模板不能遞迴。
- **清單綁定**:`repeat: {count: VARIABLE|N, max ≤ 32}`,展開出 `ID/0..N` 的實例,依數量變數決定顯示幾個。資料陣列綁定是後續擴充。

### ADR-6:多畫面與焦點

- **作法**:`screens: {ID: {panels: [...], modal: bool, pause: bool}}`,加上新的命令種類 `ui.open SCREEN`、`ui.close`。
  - 這兩個是型別化的 gameplay 命令,UI、scenario、loopback 共用。
  - HUD 就是預設畫面。
- **焦點**:依面板的排列順序,做方向鍵和手把導覽。焦點狀態對應到按鈕的 hover 樣式。
- **checkpoint**:記錄目前開著哪些畫面。UI 狀態屬於 gameplay `Progress`,要明確存讀,不能默默丟掉。

### ADR-7:素材與字型

- **圖片**:每份文件 ≤ 32 張 PNG,總像素 ≤ 16M。
  - 解碼在背景工作進行,結果依內容 hash 快取。
  - 世界啟用前就準備好(沿用 `bound_ui::prepare_assets` 的路徑)。
- **字型**:每份文件 ≤ 4 個,單檔 ≤ 24 MB。
  - 必填 `license` 欄位,值必須是 OFL / Apache / MIT 之一。
  - 背景讀檔;字型組合不變時,不重新呼叫 `set_fonts`。
  - 微軟正黑體保留為 CJK 後備字型(不重新散布)。
- **示範素材**:用 `scripts/build-ui-frames.py`(或 Rust 範例)程式產生原創邊框 PNG,納入 repo。

## Phase 3 — 增量拆解

| M | 範圍 | 驗收(操作 X,看到 Y) | 依賴 | 風險 |
|---|---|---|---|---|
| **M1 walking skeleton:一個帶九宮格邊框的面板** | 格式:`theme.frames` + `theme.panel` + `panels`(只支援錨點、偏移、寬度,column 排列);渲染:九宮格底圖;ops `set_ui_panel` / `set_ui_frame`;ACL `ui panel …`、`ui frame …`;UI Editor 的面板表單;parity 測試;生成器產出 1 張原創邊框;town 的任務追蹤改用它 | GPU 截圖裡任務追蹤有自訂邊框;ACL 和編輯器寫出的世界逐位元相同;其他舊內容包截圖不變 | — | R4、R5:把 `regions` 改編譯成面板時,舊截圖最容易變 |
| **M2 主題與 class** | text / panel / button class;按鈕 normal、hover、pressed、disabled 四態;文字描邊和陰影;舊 `TextStyle` 對應成內建 class;九宮格中央 `tile` / `hidden` | town 的表情按鈕滑過、按下會換底圖;舊截圖不變 | M1 | 按鈕狀態在編輯器裡怎麼預覽 |
| **M3 自由版面與縮放** | `size`(px / % / auto)、row、grid、stack、gap、padding、巢狀(≤ 4 層)、z 順序;參考解析度整層縮放;`observe ui.layout`(每個元件的矩形,含重疊和出界診斷) | 1280×720 和 1920×1080 下對話框比例一致;`observe` 列出矩形且沒有重疊診斷;量測 30 個面板的每幀成本(debug 和 release 分開標示) | M2 | R6 |
| **M4 內容包字型** | `theme.fonts` + 授權驗證、背景載入、預算;text class 可以指定字型 | town 對話用 OFL 字型;啟用期間主執行緒不超過 16 ms(有量測) | M2 | 授權把關;字型檔大小 |
| **M5 模板與重複** | `templates`、`use`、`repeat`(數量綁定)、展開預算 | 對話框模板定義一次,用在 Lumi 和 Rin 兩處;表情格用 `repeat` 產生 | M3 | 展開後的元件 ID 和 parity 測試 |
| **M6 畫面、模態與焦點** | `screens`、`ui.open` / `ui.close` 命令、modal + pause、鍵盤和手把焦點導覽、checkpoint 存目前畫面 | demo 第 5 步:Esc 開暫停選單、方向鍵移動、確認後關閉;讀檔後選單狀態正確 | M3 | 和遊戲本身的輸入衝突(Esc、WASD) |
| **M7 編輯器自由畫布** | UI Editor 拖動和縮放把手、對齊格線、主題編輯器、九宮格邊距編輯(在圖上拖四條線) | demo 第 7 步:編輯器拖動、縮放對話框,結果和 ACL 相同(parity);每次放開滑鼠只寫一筆 revision | M3、M5 | 編輯器的主執行緒預算 |
| **M8 town 改版與文件** | town 的 HUD 全部改成新模型;`docs/ui-styling.zh-TW.md`;AI 代理用的 discover topic | 完整跑完 Phase 0 的 demo | M1–M7 | 範圍膨脹 |

每個 milestone 結束時系統都能跑:新欄位都是選用的,不用的話系統保持原樣。

**GATE 3**:M1 是端到端的 walking skeleton(格式 → 驗證 → 渲染 → ACL → 編輯器 → parity → 實際內容);每個驗收都是可演示的行為。

## Phase 4 — Pre-mortem(假設專案失敗,原因是什麼)

1. **和 Paladin 的未提交改動合併時爆炸。** 兩邊都大改 `bound_ui.rs` 和 `gameplay.rs` 的 UI 型別。
   - 對策:開工前請使用者讓 Paladin session 先提交,把 origin/main 合併進本機 main,這個計畫以合併後的結果為基底。
   - 若無法等:M1 先只做 `axiom-core` 的格式和 ops(不碰 `bound_ui.rs`),渲染等合併後再做。
   - Paladin 的進度條欄位依 ADR-4 對應成 class。
2. **舊內容包的畫面悄悄改變。** `regions` 和 `styles` 改編譯成新模型時,任何一個像素差都算回歸。
   - 對策:M1 先建立 `scripts/ui-screenshot-ab.ps1`,對所有用到 `ui.document` 的內容包做新舊 exe 截圖 A/B,要求逐位元相同,每個 milestone 都要過。
3. **範圍膨脹成「整套 UI 框架」。** 下拉選單、輸入框、富文本、動畫時間軸都很誘人。
   - 對策:這些都寫在 non-goals,只做 demo 需要的東西;新元件另開計畫。
4. **最樂觀的預估是 M3(自由版面)。** egui 的立即模式要做 auto 大小、百分比和巢狀,很容易需要兩階段排版(第一幀量大小,第二幀才畫)。
   - 對策:M3 先只支援「固定大小 + auto 高度」;百分比只算相對螢幕,不算相對父容器。真的需要相對父容器時再拆成 M3b。
5. **AI 看不到排版結果**,又重演這次的朝向問題。
   - 對策:`observe ui.layout` 和重疊、出界診斷提前到 M3,每個 milestone 的驗收都用它,再加 GPU 截圖。
6. **素材授權出事。**
   - 對策:邊框圖只用程式產生;字型欄位強制填授權,只接受白名單;推送前檢查新增的二進位檔。

**GATE 4**:6 個失敗原因都已寫出對策,並放進計畫。

## Phase 5 — 執行協定

1. 每個 milestone 用 `rigorous-change` 實作。驗收用 `rigorous-verify`,依改動範圍選快速或完整模式。
2. 每個 milestone 的 gate:
   - fmt、clippy `-D warnings`、`cargo test --workspace`;
   - editor smoke、world 和 street parity、M0 smoke;
   - UI 截圖 A/B(舊內容逐位元相同);
   - 新功能的 GPU 截圖,而且要實際看過;
   - 效能數字一律標示 debug 或 release。
3. 每個 milestone 結束時做三件事:
   - 照驗收實際演示一次,證據寫在這份文件的「執行紀錄」;
   - 回頭看計畫要不要修訂(用補記,不覆寫原文);
   - 寫 WORKLOG。
4. 實作中發現必須偏離計畫時,先停下來修改這份文件(附補記),再繼續。
5. 使用者的 `data\axiom-town.sqlite` 不做測試用途;要寫入前先問,並先備份。

## 執行紀錄

### 補記 2026-11-26:M1 拆成 M1a / M1b(原計畫 → 現在 → 原因)

- 原計畫:M1 一次打通格式、渲染、ACL、編輯器和 town 內容。
- 現在:
  - M1a 只做格式、驗證、ops、ACL、編輯器命令、parity;
  - M1b 做 `bound_ui` 渲染、town 內容、GPU 驗收和截圖 A/B。
- 原因:開工時 Paladin session 仍有未提交的改動(`bound_ui.rs` +260、`gameplay.rs` +109),這就是 pre-mortem 第 1 項。M1a 不碰 `bound_ui.rs`,新型別放在新模組 `axiom-core/src/ui_theme.rs`,`gameplay.rs` 只加兩個欄位和一個驗證呼叫,讓之後的合併衝突最小。

### M1a 完成(分支 `ui-v2`,基底 `7451a04`)

- **格式**:`ui.document.theme`({frames, panel})和 `ui.document.panels`,兩者都是選用欄位、空的時候不序列化。
- **驗證**:`ui_theme::validate`,在 `compile` 和 ops 都會執行。錯誤路徑會指到欄位,例如 `/resources/ui.document/theme/panel/box/frame`。
  - 擋下的情況:圖片不在 `images` 裡、邊距超過 1024 px、圓角或內距超出範圍、樣式引用不存在的邊框、刪除仍被使用的邊框、面板引用未知元件、同一元件同時在面板和 region、同一元件在兩個面板、未知的面板樣式、寬度超出範圍、id 重複。
  - 預算:邊框、面板樣式、面板各 32 個,每個面板最多 64 個元件。
- **ops**:`set_ui_frame`、`set_ui_panel_style`、`set_ui_panel`(修改時保留面板在清單中的位置,也就是 z 順序;內容沒變時不產生編輯)。
- **ACL**:`ui frame|panelstyle|panel`;語法表已更新。
- **編輯器**:`ui.frame`、`ui.panel_style`、`ui.panel` 命令都經過同一個 ACL parser;Play 期間和檢查器有草稿時都會擋下;parity 表加了一列。
- **測試**:
  - `axiom-core/tests/ui_theme.rs` 3 個:建立流程、11 種拒絕(檢查路徑,且不寫入)、加上再全部移除後文件逐位元回到原樣;
  - `editor::parity::ui_theme_and_panel_commands_match_acl`:逐步比對編輯器命令和 ACL 的結果。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace` 1120 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS。
- **尚未做(M1b)**:畫面上還看不到面板和邊框,放進面板的元件照舊畫在原本的位置(discover 的說明有寫)。截圖 A/B 腳本、town 內容、原創邊框生成器也都留到 M1b。

### M1b 完成(分支 `ui-v2`)

- **渲染(`bound_ui.rs`)**:新增 `draw_panels`。
  - 每個面板是一個 egui `Area`:錨點沿用 region 的 `Align2`,偏移使用 `offset`;錨在上方的面板會再往下讓開頂端的 chrome。
  - 背景:先預留一個繪製位置,內容畫完後依實際矩形填入「`fill` 圓角矩形 + 九宮格 mesh」,所以背景會在元件下方(Spike S1 的作法)。
  - 子元件依 `children` 的順序畫出。
  - 遊戲畫面和編輯器畫布(`draw_canvas`,面板圖層套用畫布縮放)走同一條路徑。
  - 放在面板裡的元件不會掉到底部文件面板。
- **九宮格**:`nine_slice(rect, tex, size, slice)`。四個角保持原像素大小;矩形比兩個角加起來還小時,角等比縮小;寬或高為零的格子略過。貼圖和圖片元件共用 `state.textures` 快取。
- **回歸(實作中發現並修正)**:元件全部放進面板或 region 後,底部面板不再出現,原本只看 region 的判斷讓語系、存檔、讀檔控制項消失。現在判斷「已放好位置」時也算入面板。
  - 回歸測試 `ui_panels::session_controls_stay_on_screen_when_every_widget_is_in_a_region_or_panel`。
  - 把修正還原後測試會失敗,恢復修正後通過。
- **原創素材**:`crates/axiom-runtime/examples/build_ui_frames.rs` 產生 `content/axiom-town/ui/dialog-frame.png`(96×96,切割 32)。連跑兩次 SHA-256 相同。
- **內容**:Axiom Town 的 `speaker`、`line`、`choices` 從 bottom region 移到 `dialog` 面板(底部置中、`offset [0,-14]`、寬 900、`padding 26`、`frame dialog`)。
- **測試**:
  - `bound_ui::panel_tests` 2 個:邊角像素與 UV、小矩形的邊角縮小。
  - `tests/ui_panels.rs` 2 個:
    - 走到 Lumi 按 E 之後,只畫出一個九宮格,寬 900、置中、離底 14 px,所有選項按鈕都在框內;
    - session 控制項仍在畫面上。
- **截圖 A/B**(`scripts/ui-screenshot-ab.ps1`;新 exe 對 M1a exe,debug,1805×950,120 幀,每次用新的資料庫,第一個包重跑兩次確認截圖本身穩定):
  - adv、animation、animator-lab、character-lab、duet-lab、rts-bench、rts、street、style-lab 共 9 個包**逐位元相同**;
  - axiom-town 預期會變:截圖已看過,對話框在金框內,右上角控制項還在。
- **未驗證**:沒有在 GPU 上截對話框開啟中的畫面,這部分由 headless 測試涵蓋;出生點的截圖已顯示框內的「廣場」說明。沒有做 release 量測,也沒有效能宣稱。

### 補記 2026-11-26:M2 和 ADR-4 的差異(原計畫 → 現在 → 原因)

1. **類別放在哪裡**
   - 原計畫:每個元件加一個 `class` 欄位。
   - 現在:改用 `ui.document.classes` 對照表(元件 ID → 類別),和 `regions`、`styles` 同一種形式。
   - 原因:`Widget` enum 的六種元件和所有比對它們的程式碼都不用改;序列化方式一致;編輯器和 ACL 可以沿用 `map_entry`。
2. **舊的 `TextStyle`**
   - 原計畫:對應成內建預設主題裡的 6 個 class。
   - 現在:舊的 `styles` 維持原本的程式路徑;同一個元件不能同時有 style 和 class,兩者只能擇一。
   - 原因:不碰舊路徑,舊內容的截圖才能保證逐位元相同,A/B 已驗證。改成內建 class 留到需要時再做。
3. **文字描邊**:這次只做陰影。描邊在 egui 上要畫 8 次 galley,先列為 backlog。

### M2 完成(分支 `ui-v2`)

- **格式**:
  - `theme.text`:size、color、shadow;
  - `theme.button`:size、padding,以及 normal / hover / pressed / disabled 四種狀態,各自可設 frame / fill / text,沒設的欄位沿用 normal;
  - `theme.frames.*.center`:stretch / tile / hidden;
  - `classes`:元件 → 類別。
- **驗證**:
  - 文字元件只能用 text class,按鈕只能用 button class,其他元件不能用類別;
  - 同一個元件不能同時有 style 和 class;
  - 範圍:字級 8–96、padding 0–64、文字和按鈕類別各 ≤ 32 個;
  - 刪掉仍被使用的邊框或類別會被擋下,錯誤指出完整路徑。
- **ops / ACL / 編輯器**:
  - ops:`set_ui_text_class`、`set_ui_button_class`、`set_ui_class`;
  - ACL:`ui textclass|buttonclass|class`,`ui frame … center=`;
  - 編輯器命令:`ui.text_class`、`ui.button_class`、`ui.class`(同樣在 Play 中、檢查器有草稿時擋下);
  - parity 測試延伸三個步驟,parity 表加一列。
- **渲染**:
  - `themed_text`:陰影先畫,本體疊在上面;
  - `themed_button`:依狀態(停用 > 按下 > 滑過 > 一般)畫底色、九宮格和置中的文字;點擊和 target 登記和原本的按鈕相同;
  - `nine_slice` 支援 center 模式:tile 最多 1024 片,超過改用 stretch;
  - 沒有類別的元件走原本的程式。
- **原創素材**:`build_ui_frames` 新增 `button-normal/hover/pressed.png`(48×48,切割 14)。dialog 框圖的 hash 不變;連跑兩次結果相同。
- **內容**:town 的 6 個表情按鈕使用 `emote` 類別;`speaker` 使用金色加陰影的 `speaker` 類別。
- **測試**:
  - core `ui_theme.rs` 新增 1 個(建立流程、8 種有路徑的拒絕、6 種解析錯誤、style 和 class 互斥、全部移除後逐位元回到原樣);
  - `frame_center_tests` 3 個(hidden、tile 的片數和部分片的 UV、超過預算改用 stretch);
  - `ui_panels.rs` 新增 3 個(滑過和按下會換成不同的框圖貼圖、停用字色、講者陰影位在右下 2 px)。
  - 注意:測試時遊戲沒在跑,egui 會把停用的 UI 整體乘上透明度,所以顏色以「每個通道 ≈ 設定值 × alpha」比對。
- **截圖 A/B**(M1b exe 對 M2 exe):9 個內容沒變的 UI 內容包逐位元相同。town 的內容使用了新欄位,舊 exe 讀不了,所以這次不做 A/B;GPU 截圖已經看過:表情按鈕是酒紅底金邊,講者名字是金字加陰影。
- **未驗證**:沒有在 GPU 上截滑過和按下狀態(由 headless 測試涵蓋);沒有 release 量測,也沒有效能宣稱。

### 補記 2026-11-26:M3 和 ADR-3 的差異(原計畫 → 現在 → 原因)

1. **內距**
   - 原計畫:面板有自己的 `padding: [l, t, r, b]`。
   - 現在:內距仍由面板樣式(`theme.panel.*.padding`)決定,面板本身不加欄位。
   - 原因:M1 已經把內距放在樣式裡;同一件事有兩個來源會讓 ACL、編輯器和驗證都多一層優先順序。真的需要四邊不同時再加。
2. **百分比**
   - 依 pre-mortem 第 4 項:`"N%"` 只相對參考畫面(沒有 reference 時相對視窗),不相對父面板。
3. **巢狀的寫法**
   - 原計畫:`children: [PanelChild]`(元件或子面板的物件)。
   - 現在:`children` 仍是 ID 字串清單,ID 是另一個面板時就是子面板。面板 ID 不能和元件 ID 重複。
   - 原因:不改既有欄位的形狀,舊文件和 M1、M2 的 ACL 都不用動;子面板忽略自己的錨點、偏移和 z。
4. **縮放範圍**
   - 只縮放面板層;舊的九宮格 `regions` 維持螢幕像素、不縮放,舊內容的截圖才能逐位元相同。
5. **`when`(條件顯示)**:ADR-3 列了面板的 `when`,M3 沒做。面板沒有可見的子元件時本來就不畫,目前的內容不需要;留到 M6(畫面與模態)一起考慮。
6. **`observe ui.layout` 的位置**:放在 `ui_layout`,只有透過 `--connect` 觀察執行中的遊戲才有(版面是 egui 在視窗裡排出來的);headless 的 `observe` 沒有這一項,測試則用 `WorldRuntime::ui_layout_observation`。
7. **town 的參考畫面是 1280×720,不是 1920×1080**
   - 原計畫:`theme.reference` 預設 1920×1080。
   - 現在:格式不設預設值(沒有 reference 就不縮放);Axiom Town 用 1280×720。
   - 原因:實測(150% DPI 的螢幕,1920×1080 視窗)用 1920×1080 時,一個參考像素等於一個視窗像素,但舊 regions 和頂端列跟著 DPI 放大 1.5 倍,對話框和表情按鈕反而比 M2 小,字級和旁邊的 HUD 不一致。1280×720 剛好等於這個視窗的 point 大小,面板和 regions 一致,不同視窗大小下比例仍固定。
   - 同時發現:`ui_layout` 一開始回報的是 egui point,改成視窗像素(乘上 pixels_per_point),`scale` 是「每個參考像素對應幾個視窗像素」。

### M3 完成(分支 `ui-v2`)

- **格式**:
  - 面板:`width`、`height`(px 或 `"N%"`)、`layout`(column / row / grid / stack)、`columns`(grid,1–16)、`gap`(0–128)、`z`(-100–100);`children` 可以寫面板 ID 來巢狀(最多 4 層)。
  - 主題:`reference: [w, h]`、`scale`(fit_height 預設 / fit_width / fit / none)。
  - 全部是選用欄位、預設值不序列化,舊文件不變。
- **驗證**:長度範圍、`columns` 只能配 grid、gap / z 範圍、面板 ID 和元件 ID 不能重複、面板不能裝自己、不能在兩個面板裡、循環、超過 4 層、沒有 reference 卻設 scale、reference 範圍;錯誤都指到欄位路徑。
- **ops / ACL / 編輯器**:`set_ui_screen`;ACL `ui screen W H [scale=] | none`、`ui panel` 的新選項;編輯器命令 `ui.screen`;parity 測試多兩步(reference 和巢狀 grid 面板),parity 表加一列。
- **渲染**:
  - 根面板依 z 排序;子面板畫在父面板裡;row、grid、stack 用 egui 的 horizontal、Grid、同原點的 scope 排。
  - 有 reference 時,面板在參考座標排版,整個面板層用一個 transform 縮放(Spike S2);點擊命中由 egui 依 transform 換算,`targets` 也換成螢幕座標。
  - 沒有 reference 時不設 transform(identity 會移除),和 M2 完全相同。
  - 每幀用的排序和矩形緩衝放在 `UiState` 重複使用。
- **`observe` 的 `ui_layout`**:每個 region、面板、面板內元件的視窗像素矩形,加上 `ui_overlap`、`ui_offscreen` 診斷(最多 32 筆)。只有透過 `--connect` 觀察執行中的遊戲、而且文件有面板時才有。
- **內容**:Axiom Town 的參考畫面 1280×720;6 個表情按鈕移到 `emotes` 面板(右側、3 欄 grid、gap 8、`hud` 樣式),從 regions 移除。
- **測試**:
  - core `ui_theme.rs` 新增 1 個(格式來回、深度、11 種有路徑的拒絕、4 種解析錯誤、5 層被擋、全部移除後逐位元回到原樣);
  - `ui_panels.rs` 新增 4 個:720p 和 1080p 下對話框占視窗的比例相同;表情按鈕是 2 列 3 欄、`ui_layout` 沒有診斷;故意放錯的面板會報出重疊和超出畫面;1280×720 縮放後滑過和點擊仍然有效。
  - 既有的 M1b、M2 測試改用參考座標換算後的數值(寬 900 → 1080 視窗下 1350;陰影 2 → 3 px)。
- **實作中發現並修正**:
  1. `ui_layout` 原本回報 egui point;150% DPI 的螢幕上 1920×1080 視窗回報成 1280×720。改成乘上 `pixels_per_point` 的視窗像素。
  2. town 原本用 1920×1080 當參考畫面,實際截圖裡對話框和表情按鈕比 M2 小、和旁邊的 HUD 字級不一致,改成 1280×720(見補記第 7 點)。
  3. `ui_layout` 的重疊診斷抓到真的問題:和 Lumi 對話時對話框變高,右上角伸到表情格底下(headless 測試 `emotes_sit_in_a_three_column_grid…` 失敗)。表情格往上移 60 參考像素後沒有診斷。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace` 1137 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS。
  - 中途有一次 `app::tests::a_full_capture_queue_refuses_editor_view_before_its_edit` 報 database is locked(兩個 cargo test 同時跑);單獨重跑和最後的完整重跑都通過,和 M3 改動無關,沒有改它。
- **截圖 A/B**(M2 exe 對 M3 exe,debug,每次用新的資料庫):9 個內容沒變的包逐位元相同;第一個包重跑一次確認截圖穩定。town 使用新欄位,舊 exe 讀不了,不做 A/B。
- **GPU 截圖**(實際看過):1920×1080 和 1600×900 兩種視窗,`ui_layout` 都是 2 個面板、9 個元件、0 個診斷;對話框寬 1350 / 1125(都是視窗寬的 70.3%)、置中、離底 21 / 17.5 px(14 參考像素 × 縮放)。
- **量測**(`examples/ui_layout_bench.rs`,1280×720 視窗,egui pass + tessellate 的 CPU 時間,300 幀):

  | 設定 | 面板 | debug 平均 / p95 | release 平均 / p95 |
  |---|---|---|---|
  | town 原樣 | 2 | 0.34 / 0.42 ms | 0.31 / 0.45 ms |
  | town + 30 個有九宮格框的 grid 面板 | 32 | 1.28 / 1.75 ms | 1.15 / 1.71 ms |

  - debug 和 release 接近,因為這個 workspace 的 dev profile 把相依套件(含 egui)編成 `opt-level = 2`。
  - 文件元件上限 64 個,所以這 30 個面板每個只放 1 個文字元件。
  - 這是 CPU 時間,沒有量 GPU。
- **編輯器畫布**:`ui_editor_tests::canvas_panels_scale_with_the_reference_screen`:畫布在 1280×720 和 1920×1080 下,面板元件占畫布的比例相同(第一次寫測試時忘了等 grid 欄寬穩定,量到前一幀的位置;多畫兩幀後通過)。
- **未驗證**:沒有在 GPU 上截對話中(對話框變高)的畫面,這部分由 headless 測試涵蓋;編輯器畫布沒有截圖檢查。

### 補記 2026-11-26:M4 和 ADR-7 的差異(原計畫 → 現在 → 原因)

1. **示範字型**
   - 原計畫:town 對話用一套 OFL 字型。
   - 現在:用 `examples/build_ui_font.rs` 產生的原創 5×7 點陣字(MIT,和 repo 相同)。
   - 原因:使用者的規則是不推送第三方素材;OFL 字型雖然可以散布,仍是第三方素材。格式和驗證照樣接受 OFL / Apache / MIT / CC0。
2. **授權寫在哪裡**:寫在 `theme.fonts.ID.license`(和字型路徑同一處),不另外寫進 manifest;驗證只接受白名單裡的 SPDX ID。
3. **button class 也能指定字型**:原計畫只寫 text class;按鈕標籤是同一種文字,一起支援。

### M4 完成(分支 `ui-v2`)

- **格式**:`theme.fonts.ID = {path, license}`(.ttf / .otf,≤ 4 個,每個 ≤ 24 MiB,license ∈ OFL-1.1 / Apache-2.0 / MIT / CC0-1.0);text class 和 button class 加 `font`。
- **驗證**:授權不在白名單、副檔名不對、類別引用不存在的字型、超過 4 個、刪除仍被使用的字型,都回報欄位路徑。
- **ops / ACL / 編輯器**:`set_ui_font`;ACL `ui font ID PATH license=SPDX | ui font ID none`、`textclass` / `buttonclass` 的 `font=`;編輯器命令 `ui.font`;parity 測試多兩步,parity 表加一列。
- **內容包與打包**:匯入時字型路徑和圖片一樣換成內容包裡的絕對路徑(不能跑出內容包、≤ 24 MiB);`axiom package` 的檔案清單包含字型,打包後的 SHA-256 和原檔相同。
- **執行期**:
  - 世界啟用時(背景)讀檔並用 `ttf-parser`(egui 字型堆疊原本就有的 crate)檢查格式;壞檔案讓啟用失敗並寫出字型 ID。
  - 主執行緒的 `ensure_fonts` 只在 context 缺字型時呼叫一次 `set_fonts`(基底 = egui 預設 + 微軟正黑體後備,`ui::base_fonts`);family 名稱含檔案指紋,換檔就是新 family;被別的 `set_fonts` 換掉時兩個 pass 後補回。
  - egui 下一個 pass 才套用新字型,這之前類別用預設字型畫,不會引用還不存在的 family(egui 會 panic)。
- **原創素材**:`examples/build_ui_font.rs` 產生 `content/axiom-town/ui/axiom-pixel.ttf`(11,724 bytes、51 個字符;5×7 點陣大寫、數字、常用標點與 `·`,小寫對應大寫)。連跑兩次 SHA-256 相同(D8D29382…)。
- **內容**:講者名字、對話台詞(新類別 `dialog_line`)、表情按鈕使用 `pixel` 字型;中文退回系統字型。
- **實作中發現並修正**:編輯器命令的 op 沒有任何改變時,原本會送出空的 Apply,被 store 以 `batch must contain 1..256 edits` 拒絕(M1a 起的 UI 主題 op 對「設成相同的值」回傳空的編輯)。改成不寫入、顯示「no change」;parity 測試加了這個情況。
- **測試**:core `ui_theme.rs` 新增 1 個;`ui_panels.rs` 新增 2 個(安裝後台詞改用內容字型、被別的 set_fonts 換掉後補回;不是字型的檔案讓啟用失敗並寫出 ID)。既有的滑過 / 按下測試要多等幾幀:字型換上後按鈕變寬,grid 下一幀才重新算欄寬。
- **量測**(`examples/ui_font_bench.rs`,1280×720,context 先載入遊戲的基底字型;每幀 = egui pass + tessellate 的 CPU 時間):

  | | 第 0 幀 | 第 1 幀(套用字型) | 之後 |
  |---|---|---|---|
  | debug,有內容字型 | 3.72 ms | 5.89 ms | 0.47–0.84 ms |
  | debug,沒有內容字型 | 3.53 ms | 0.71 ms | 0.53–2.84 ms |
  | release,有內容字型 | 2.98 ms | 6.79 ms | 0.40–0.93 ms |
  | release,沒有內容字型 | 3.80 ms | 0.85 ms | 0.52–0.66 ms |

  - 套用字型的那一幀多約 5–6 ms(egui 重建字型與 atlas,含 21 MB 的微軟正黑體後備),在 16 ms 預算內。
  - debug 和 release 接近,原因同 M3(相依套件 opt-level 2)。
- **GPU 截圖**(實際看過,1920×1080):台詞裡的 WASD、Z/C、SPACE、數字和表情按鈕的數字、YA 是點陣字,中文是系統字型;`ui_layout` 0 個診斷。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace` 1140 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS。
  - 中途一次 `inspector_on_a_4096_object_world_stays_within_budget`(時間預算 96 ms)在 GPU 截圖同時跑的時候失敗;單獨的完整重跑通過,這個測試和 M4 的改動無關。
- **截圖 A/B**(M3 exe 對 M4 exe):9 個內容沒變的包逐位元相同,第一個包重跑一次確認穩定。
- **打包**:`axiom package --pack content/axiom-town` 的輸出包含 `content/ui/axiom-pixel.ttf`,SHA-256 和原檔相同,打包的重新載入檢查通過;沒有實際啟動打包出來的遊戲。
