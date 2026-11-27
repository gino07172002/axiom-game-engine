# 遊戲 UI 的樣式與版面(ui.document v2)

這份文件說明怎麼用 `ui.document` 做出有自己風格的遊戲介面,包含:

- 自訂邊框圖;
- 自由排版、跟著視窗縮放;
- 自帶字型;
- 模板;
- 選單和暫停畫面。

每一項都可以用 ACL(AI 代理)、編輯器命令或 UI Editor 編輯,三者寫出的世界完全相同。

- 細節與每個欄位的範圍:`docs/ui-effects.zh-TW.md`(「面板與邊框」以後的各節)。
- 設計與取捨:`docs/plan-ui-styling.zh-TW.md`。
- AI 代理可以先跑 `discover ui_styling`。

## 五分鐘做一個有邊框的 HUD 和暫停選單

以下用 ACL 寫。圖片和字型的路徑都是相對於內容包。

```
# 1. 參考畫面:面板用 1280x720 的座標排版,整層跟著視窗等比縮放
ui screen 1280 720

# 2. 邊框圖:內容包裡的 PNG,四邊各切 20 px(角落不拉伸)
set resources[id=ui.document] /images/card "ui/card.png"
ui frame card card 20 20 20 20
ui panelstyle card padding=16 frame=card

# 3. 自帶字型(只接受可散布的開放授權)和文字類別
ui font pixel ui/pixel.ttf license=MIT
ui textclass heading size=18 font=pixel color=66F5CD

# 4. 面板:左上角、寬 300,用 card 樣式;title 套 heading 類別
ui panel status top_left at=12,10 width=300 style=card widgets=title,coins
ui class title heading

# 5. 暫停選單:一個置中的面板,放進 pause 頁面;Esc 開關它,開著時遊戲暫停
ui panel menu center width=360 style=card widgets=resume
ui page pause panels=menu pause=on escape=on
ui button resume close pause
```

接下來確認結果。在執行中的遊戲上(`--connect`)跑 `observe`,`ui_layout` 會列出每個面板和元件在視窗上的矩形,沒有重疊、也沒有超出畫面時,診斷是空的。

## 功能一覽

| 想做的事 | 用什麼 | ACL |
|---|---|---|
| 用圖片當邊框(九宮格) | `theme.frames` + `theme.panel` | `ui frame`、`ui panelstyle` |
| 放在任何位置、指定大小 | `panels`:錨點 + 偏移、`width` / `height`(px 或 %) | `ui panel` |
| 橫排、格狀、疊放、巢狀 | `layout row|grid|stack`、`columns`、`gap`;子面板(最多 4 層) | `ui panel … layout=grid columns=3` |
| 跟著視窗縮放 | `theme.reference` + `scale` | `ui screen` |
| 文字和按鈕的樣式 | `theme.text`、`theme.button`(normal / hover / pressed / disabled) | `ui textclass`、`ui buttonclass`、`ui class` |
| 自帶字型 | `theme.fonts`(OFL / Apache / MIT / CC0) | `ui font` |
| 同一組元件用很多次 | `templates` + `uses`(`{參數}`、`{i}`、`count`) | `ui template`、`ui use` |
| 選單、彈窗、暫停 | `pages`(modal / pause / escape)、指令 `open` / `close` | `ui page`、`ui button W open|close` |
| 檢查排版 | `observe` 的 `ui_layout`(視窗像素、重疊、超出畫面) | — |
| 進度條的樣子 | `theme.progress`(底色、填色、邊框、高度、圓角、要不要顯示百分比) | `ui progressclass`、`ui class 進度條 類別` |
| 對話選項按鈕 | `classes` 裡給 choices 元件一個按鈕類別,每個選項都套用 | `ui class choices 按鈕類別` |
| 引擎的互動提示 | `theme.prompt`(文字類別、面板樣式);文件自己有 `prompt` 綁定的文字元件時不畫 | `ui prompt` |
| 引擎介面文字 | 語系表的 `axiom.save`、`axiom.load`、`axiom.paused` 取代 SAVE / LOAD / PAUSED | `loc set L KEY TEXT` |
| 角色頭上的名牌 | `theme.nameplate`(預設的文字類別、面板樣式、淡出和隱藏距離);每個 `ui.nameplate` 可以有自己的 `key`、`class`、`style` | `ui nameplate` |

## 名牌

- `ui.nameplate`(元件):
  - `text` 是固定的文字;`key` 從語系表取文字,切語系時跟著換。兩個都有時,語系表沒有這個鍵就顯示 `text`。
  - `class` 是 `theme.text` 的文字類別(字型、大小、顏色、陰影);類別沒有顏色時用名牌自己的 `color`。
  - `style` 是 `theme.panel` 的面板樣式(底色、圓角、padding、九宮格邊框)。
  - 寫了不存在的類別或樣式,世界會被拒絕,錯誤訊息指出是哪個物件。
- `theme.nameplate`(`ui nameplate [fade=M] [hide=M] [class=C] [style=S] | ui nameplate none`):
  - `class` / `style` 是沒有自己設定的名牌用的預設值。
  - `fade=M`:離鏡頭 M 公尺開始淡出;`hide=M`:超過 M 公尺不畫(預設 40)。
  - 沒有寫 `theme.nameplate` 時名牌和以前一樣:14 px 文字、深色圓角底、40 公尺外不畫、不淡出。
- 名牌用螢幕像素,不跟 `theme.reference` 縮放。
- UI Editor 畫布下方的名牌預覽用同一套外觀和目前的語系。

## 玩家設定(音量等)

- 引擎定義固定的設定鍵:`audio.master`、`audio.music`、`audio.sfx`(0–1)、`mouse.sensitivity`(0.1–5)、`mouse.invert_y`、`display.mode`(window / borderless / fullscreen)、`display.vsync`、`quality.shadows`(low / medium / high)、`quality.fxaa`。
- 作者用 `settings default KEY VALUE` 寫預設值(`game.settings` 資源)。
- 玩家改設定用指令 `{"kind":"setting","key":…,"value":…}`:按鈕直接送,滑桿寫成 `"command":{"kind":"setting","key":"audio.master"}`(值由滑桿填入)。
- 設定存在玩家自己的設定檔 `settings/<world id>.json`(資料庫旁;打包的遊戲在 `%APPDATA%\Axiom\<id>\settings\`),不進世界的修訂、也不進存檔;重開遊戲時讀回。設定檔壞掉時用預設值並回報。
- `observe` 的 `settings` 是目前的值(世界有 `game.settings` 或玩家改過時才出現);設定不影響 `state_hash`。
- 從暫停選單打開的設定頁:模態頁面打開時,在它下面的頁面不畫,只留變暗的 HUD。
- 目前接上的:
  - 音量:總音量、音樂、音效。音源用 `bus=music` 歸到音樂。
  - 滑鼠:靈敏度、反轉 Y 軸。只影響滑鼠視角,不影響重播。
  - 視窗:`display.mode`、`display.vsync`,改了立刻套用。
    - `fullscreen` 用目前螢幕最大的解析度和最高更新率;螢幕沒有列出模式時改用無邊框。
    - 關掉垂直同步時用 mailbox,不支援就用 immediate,都不支援時維持 FIFO。
    - `observe` 的 `metrics.display` 回報實際的模式、present mode 和視窗大小。
    - 編輯器裡不套用視窗模式,編輯器有自己的全螢幕切換。
  - 畫質:`quality.shadows` 把太陽陰影貼圖設成 1024 / 2048 / 4096(預設 medium = 2048,和以前一樣)。
    - `quality.fxaa` 關掉時,即使世界的 `post antialiasing fxaa` 開著也不跑 FXAA。
    - 開著時照世界的設定,不會替沒開 FXAA 的世界加上。
    - `observe` 的 `metrics.quality` 回報 `shadow_map` 和 `fxaa`。

## 在編輯器裡

UI Editor 分頁:

- 畫布用遊戲的繪製程式畫出整個 HUD(邊框、字型、縮放都一樣)。
- 拖動面板 = 移動;拖選取面板的邊緣把手 = 改寬度(對齊 8 px)。每次放開滑鼠寫入一筆修訂。
- 元件面板上方是選取的面板:padding / corner 的 − / +,以及在邊框圖上拖四條線改九宮格邊距。
- 其他編輯(顏色、文字類別、模板、頁面)用命令面板的 `ui.*` 命令,參數和 ACL 相同。

## 範例:Axiom Town

`content/axiom-town` 的 HUD 全部用這套模型做:

- 左上角的任務追蹤:`tracker` 面板,青綠邊框 `ui/tracker-frame.png`,裡面的 `coins_row` 子面板把硬幣橫排。
- 底部的對話框:`dialog` 面板,金色雙邊框 `ui/dialog-frame.png`,講者和台詞用點陣字 `ui/axiom-pixel.ttf`。
- 右側的表情按鈕:模板 `emote_button` 用一個 use 產生 6 顆,放進 3 欄的 grid;按鈕有一般、滑過、按下三種框圖。
- Esc 打開「暫停」選單:`pause` 頁面(模態、暫停、方向鍵 / 手把移動焦點)。
- 參考畫面 1280×720。

圖片和字型都由程式產生、是原創的:

```
cargo run -p axiom-runtime --example build_ui_frames
cargo run -p axiom-runtime --example build_ui_font
```

## 限制

- 舊的九宮格 `regions` 仍然可以用,但不會跟著參考畫面縮放;新的內容請用面板。
- 每份文件最多 64 個元件(模板展開後也算)、32 個面板、4 個字型、8 個頁面。
- 沒有文字輸入框、下拉選單、分頁元件、富文本。
- 中文等字型沒有的字會退回系統字型(Windows 上是微軟正黑體;系統字型不會被打包)。
- 模態頁面若沒有暫停遊戲,方向鍵同時會送進遊戲的輸入。
