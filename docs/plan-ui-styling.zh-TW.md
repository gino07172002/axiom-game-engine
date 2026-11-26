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

(尚未開工)
