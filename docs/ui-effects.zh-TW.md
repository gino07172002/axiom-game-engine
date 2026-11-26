# UI 動效(`ui.document.effects`)與 UI Editor

`ui.document.effects` 讓 HUD 元件(widget)在出現、消失、數值改變、事件發生或條件成立時播放資料驅動的動效:淡入 / 淡出、滑入 / 滑出、縮放進場 / 退場、脈衝、震動、數值跳動(count-up)。動效寫在文件裡、經過驗證、在建立執行期時編譯成型別化的索引,固定步長執行,不需要改 Rust。

執行期系統是 `ui.effects`(capability `ui`,phase 96,排在 `behavior.execute` 之後)。文件有動效時世界必須排程這個系統(`system add ui.effects`;`ui effect add` 與編輯器的 Add effect 會自動加上)。

## 欄位

```json
"effects":[
  {"id":"affection.fade_in","widget":"affection","kind":"fade_in"},
  {"id":"affection.number_roll","widget":"affection","kind":"number_roll","ticks":120},
  {"id":"quest.pulse","widget":"quest","kind":"pulse","amount":0.2},
  {"id":"coins.shake","widget":"coins","kind":"shake","on":"event","event":"thanks"},
  {"id":"title.slide","widget":"title","kind":"slide_in","direction":"up","amount":40}
]
```

| 欄位 | 說明 |
|---|---|
| `id` | 穩定 ID(熱重載依 ID 保留進度;`select` / `ui effect set` 用它)。文件內唯一 |
| `widget` | 套用的 widget ID |
| `kind` | `fade_in` / `fade_out` / `slide_in` / `slide_out` / `scale_in` / `scale_out` / `pulse` / `shake` / `number_roll` |
| `on` | 觸發:`show`(元件變為可見,含開場時就可見)/ `hide`(變為不可見)/ `change`(元件顯示的變數改變)/ `event`(`behavior.program` 事件發生)/ `when`(`when` 條件由假變真)。省略:`*_in` = show、`*_out` = hide、`pulse` / `shake` / `number_roll` = change |
| `event` | `on: event` 的事件(只寫 `event=E` 等於 `on=event`) |
| `when` | 條件(與規則同語法)。其他觸發時是閘門(觸發當下為真才播);`on: when` 時是觸發本身 |
| `ticks` | 長度,固定步數(120 步 = 1 秒),1..1200;預設 30(`number_roll` 60) |
| `ease` | `linear` / `step` / `ease_in` / `ease_out` / `ease_in_out`(與 `tween.clips` 共用 `axiom_core::tween::Ease`)。預設:進場類與 number_roll = ease_out,退場類 = ease_in,脈衝 / 震動 = linear |
| `direction` | 只有滑動:`left`(預設)/ `right` / `up` / `down`,從哪一側進來 / 往哪一側出去 |
| `amount` | 滑動 = 像素(預設 48,0..2000);`scale_in` / `scale_out` = 起始 / 結束縮放(預設 0.6,0..4);`pulse` = 峰值多出的縮放(預設 0.15,0..2);`shake` = 左右像素(預設 6,0..200)。淡入淡出與 number_roll 不接受 |
| `from` | 只有 `number_roll`:起點。省略時 `change` 從舊值、其他觸發從 0 |

驗證(提交時,錯誤帶 `/resources/ui.document/effects/I` 路徑):ID 合法且唯一、widget 存在、`change` 需要顯示變數的 widget(文字 `variable` / `localized_variable`、`progress`)、`number_roll` 需要顯示 number / int 變數的 widget(文字 `variable`、`progress`)、事件已宣告、條件可編譯、範圍、每份文件最多 64 個。

### 效果的值(u = 已播步數 / ticks,k = ease(u))

| kind | 效果 |
|---|---|
| `fade_in` / `fade_out` | 不透明度 k / 1 − k |
| `slide_in` / `slide_out` | 位移 方向 × amount × (1 − k) / × k |
| `scale_in` / `scale_out` | 以元件中心縮放 amount + (1 − amount)k / 1 + (amount − 1)k |
| `pulse` | 縮放 1 + amount · sin(πk) |
| `shake` | 水平位移 amount · (1 − k) · sin(2π · 5k)(五次衰減擺動,無亂數) |
| `number_roll` | 顯示 from + (目標 − from)k;目標每步跟著變數走。播放中再次改變時從「這一步顯示的值」接續 |

`*_out` 播放期間,即使元件的 `when` 已變假,仍會畫出來(淡出要看得到);這時按鈕只畫不收點擊、不登錄 target。同一元件的進場效果開始時會結束它正在播的退場效果,反之亦然(淡出到一半又出現 = 從頭淡入,不會疊成先淡入又淡出)。`progress` 永遠可見,所以對它用 `on: hide` 是編寫錯誤。開場節點 `on_enter` 發出的事件早於系統建立,不會觸發 `on: event`。文字綁定變成空字串(例如提示消失)就沒有東西可畫。多個動效可以同時作用在同一元件上(相乘 / 相加),例如淡入 + 數值跳動。

**樣式與進度條**:`styles` 現在也作用在 `progress`:有樣式的進度條會在條上方以該樣式顯示「值 / 上限」(`number_roll` 時跳動),沒有樣式的照舊只有百分比條。

## 執行期(效能與記憶體)

- 建立執行期時,`axiom_core::gameplay::CompiledGameplay::effects` 已把 widget、事件、條件解析成索引並套好預設值;`ui.effects` 在建立時配置每個動效一個槽、每個 widget 一個基準值(可見、變數指紋、數值)與兩個小緩衝區,之後**每步不配置**:可見性用不建字串的判斷(`ui_effects::widget_visible`,區域面板也改用它),變數改變用 FNV 指紋(不保留文字副本),事件用 gameplay trace 的序號掃描,條件求值改為借用(`Gameplay::read` 回傳參照,比較文字不再複製)。測試 `steps_reuse_the_storage_built_with_the_runtime` 確認 300 步後各緩衝區位址與容量不變。
- 預算:同時播放最多 `MAX_ACTIVE_UI_EFFECTS` = 16 個;超出的觸發**丟棄並計數**(`observe ui_effects.dropped`),重新觸發已在播放的動效不佔額度(從頭播)。
- 決定性:進度以固定步計;沒有亂數(震動是公式)。同樣的輸入與指令得到同樣的 `ui_effects` 狀態與 `state_hash`。
- 繪製:`bound_ui` 每個 widget 問一次 `EffectVisuals::visual`(最多 64 個定義的線性掃描),以 `multiply_opacity` 與 egui 圖層的形狀轉換(位移、以中心縮放)套用;只影響畫面,不影響排版(滑入時不會推擠其他元件)。

## 熱重載、存檔、觀察

- **開場**:建立執行期時,可見元件的 `show` 動效先「待命」(第一幀就以第 0 步的樣子畫出,例如淡入從透明開始),第一步才正式開始並計入預算。
- **熱重載**:相同 `id` 的動效保留進度(改長度不會重來;改短到小於已播步數,下一步結束);widget 依 ID 對應基準值,所以重排 widget 不會誤觸 `change`。新加入的 `show` 動效若元件在**目前的**遊戲狀態下可見,重載時播一次(Play 中即時看到剛加的進場效果)。切換到另一個世界(world id 不同)視為新的開場。
- **存檔**:閒置時的動效狀態完全由 gameplay 狀態推得,可以存;**有動效播放時存檔會明確失敗**(`system ui.effects: N UI effect(s) are running (...); their progress is not saved in checkpoints`),不會默默丟掉進度。讀檔後(`RuntimeSystem::restored`)以讀入的狀態重建基準,不播進場效果,馬上可以再存;讀檔後第一步的改變照常觸發。
- **觀察**:`observe` 的 `ui_effects`:`{defined, active:[{id, widget, kind, elapsed, ticks, alpha, offset, scale, number?}], started, dropped, budget}`。Scenario 可以直接斷言,例如 `examples/town-ui-effects.scenario.json`(Axiom Town 套用 `examples/town-ui-effects.acl` 後,第 10 步淡入在播、第 340 步數值跳動已播 9 步、最後全部結束且未丟棄)。

## 協定 / ACL

```
ui place WIDGET REGION|none           # 九宮格區域;none = 底部文件面板
ui style WIDGET STYLE|none            # title|heading|banner|value|label|muted(text、progress)
ui button WIDGET emit EVENT [{..}] | locale L | choose NODE CHOICE | call CAP ACTION [{..}] | open PAGE | close [PAGE]
ui effect add ID WIDGET KIND [on=..] [event=..] [when={..}] [ticks=N] [ease=..] [direction=..] [amount=X] [from=X]
ui effect set ID [kind=..] [widget=..] [同上;OPTION=default 清回預設]
ui effect remove ID                   # ui.effects 系統保留
ui frame ID IMAGE L T R B | ui frame ID none        # 主題九宮格邊框:images 裡的圖 + 四邊切割像素
ui panelstyle CLASS [fill=RRGGBB[AA]] [corner=N] [padding=N] [frame=ID] | ui panelstyle CLASS none
ui panel ID ANCHOR [at=X,Y] [width=W|N%] [height=H|N%] [layout=column|row|grid|stack] [columns=N] [gap=N] [z=N] [style=CLASS] [widgets=A,B] | ui panel ID none
ui screen W H [scale=fit_height|fit_width|fit|none] | ui screen none   # 參考畫面:面板層整體縮放
ui font ID PATH license=OFL-1.1|Apache-2.0|MIT|CC0-1.0 | ui font ID none   # 內容包字型
ui page ID panels=A,B [modal=on|off] [pause=on|off] [escape=on|off] | ui page ID none   # 頁面(選單、彈窗)
ui template ID {JSON} | ui template ID none                       # 模板:元件(+ 面板、類別)
ui use ID TEMPLATE [into=PANEL] [count=VAR] [[{..},..]] | ui use ID none   # 每個參數物件一個實例
ui textclass CLASS [size=N] [font=ID] [color=RRGGBB[AA]] [shadow=RRGGBB[AA]] | ui textclass CLASS none
ui buttonclass CLASS [size=N] [font=ID] [padding=N] [STATE.frame=ID] [STATE.fill=..] [STATE.text=..] | ui buttonclass CLASS none
ui class WIDGET CLASS|none            # 文字元件 → text class;按鈕 → button class
```

**類別(plan M2)**

- `theme.text.CLASS = {size, color, shadow}`:文字大小(8–96 px)、顏色,以及往右下偏 2 px 的陰影色。
- `theme.button.CLASS = {size, padding, normal, hover, pressed, disabled}`:
  - 每個狀態可以各自設定 `{frame, fill, text}`,分別是九宮格框圖、底色、字色;
  - 狀態沒設定的欄位沿用 `normal` 的值;
  - 遊戲沒在跑或主控台開著時,按鈕是停用狀態;
  - 按鈕點擊後的行為和沒套類別的按鈕完全相同。
- `classes.WIDGET = CLASS`:讓元件套用類別。
  - 文字元件只能用 text class,按鈕只能用 button class;
  - 已經用 `styles` 的元件不能同時用 class,兩者只能擇一;
  - 沒有套用類別的元件,畫法完全不變。
- `theme.frames.ID.center`:框圖中央的畫法,有三種:
  - `stretch`(預設):拉伸;
  - `tile`:依原圖中央的像素尺寸平鋪,最多 1024 片,超過時改用拉伸;
  - `hidden`:不畫中央,只畫邊框。
- 示範:Axiom Town 的 6 個表情按鈕使用 `emote` 類別,一般、滑過、按下各有一張原創框圖(`ui/button-*.png`,由 `build_ui_frames` 產生);講者名字使用 `speaker` 類別,金色並帶陰影。

**面板與邊框(`ui.document` v2,見 `docs/plan-ui-styling.zh-TW.md`)**

- `panels`:每個面板是一個錨定在九宮格方位(ANCHOR = REGION 名稱)加偏移 `at`(px,+x 往右、+y 往下)的直欄,元件依 `widgets` 的順序由上而下排列。
- 面板依清單順序疊放:後面的面板畫在上層。
- 面板可以指定 `width`;沒指定時寬度跟著內容,上限是該錨點原本的寬度。
- 同一個元件只能放在一個面板或一個 region 裡,不能兩處都放。
- 所有元件都放進 region 或面板時,不再畫底部的文件面板;語系、存檔、讀檔控制項會移到右上角。
- `theme.panel.CLASS`:面板外觀。
  - `fill`:RGBA 底色。
  - `corner`:圓角。
  - `padding`:內距,預設 10。
  - `frame`:引用 `theme.frames` 裡的一個九宮格邊框,畫在元件後面。
- `theme.frames.ID = {image, slice:[l,t,r,b]}`:四個角保持原像素大小,四邊和中央拉伸。面板比兩個角加起來還小時,角會等比縮小。
- 示範:Axiom Town 的對話框是 `dialog` 面板(底部置中、寬 900,使用 `ui/dialog-frame.png`)。這張圖由 `cargo run -p axiom-runtime --example build_ui_frames` 產生,是原創、不含第三方素材的圖,而且每次產生的結果都相同。
- 回歸檢查:`scripts/ui-screenshot-ab.ps1 -Old OLD.exe` 對每個有 `ui.document` 的內容包做新舊截圖逐位元比較。

**頁面、模態與焦點(plan M6)**

- `pages.ID = {panels, modal?, pause?, escape?}`:頁面打開時才畫出 `panels` 裡的根面板(沒有放進任何頁面的面板就是一直顯示的 HUD)。
  - `modal`(預設開):底下的 HUD 變暗、按鈕不能用,頁面拿走鍵盤 / 手把焦點;
  - `pause`:頁面開著時固定步長的模擬停止(UI、scenario、loopback 都一樣);指令照樣能用,所以頁面上的按鈕可以關掉它;
  - `escape`:Esc(手把 Start)開關這個頁面;最多一個。文件沒有 escape 頁面時,Esc 照舊暫停整個遊戲。
- 開關頁面是型別化的遊戲指令:`{"kind":"open","page":ID}`、`{"kind":"close","page"?:ID}`(沒有 page = 關最上面的);按鈕用 `ui button W open PAGE` / `ui button W close [PAGE]`。
- 焦點:最上面的模態頁面打開時,方向鍵 / Tab 在它的按鈕間移動(到底會繞回),Enter / Space 按下;手把十字鍵、A 一樣,B 關掉最上面的頁面。被選中的按鈕用 hover 的外觀(沒套類別的按鈕畫金色框)。
- 存檔:開著的頁面存在 checkpoint 的 gameplay 進度裡;讀檔時頁面不存在會明確失敗,不會默默丟掉。
- 觀察:`observe` 的 `gameplay.pages` 是開著的頁面(由下到上)。
- 示範:Axiom Town 按 Esc 打開置中的「暫停」選單(對話框邊框、點陣字):繼續遊戲 / English / 中文;遊戲在選單開著時暫停。

**模板與重複(plan M5)**

- `templates.ID = {params, widgets, panel?, classes?}`:
  - `widgets` 是一般的元件 JSON,字串裡可以寫 `{參數名}` 和 `{i}`(實例的序號,從 0 開始);
  - `panel`(選用)是裝這些元件的面板(不要寫 `children`,會自動填入元件);
  - `classes` 是元件 ID → 類別,也可以用參數。
- `uses = [{id, template, instances, into?, count?}]`:
  - `instances` 每個參數物件產生一個實例(1–32 個);參數值只能用英數字和 `_ - . /`,最長 64 字;
  - `into`:把實例放進哪個面板(模板有面板時放面板,沒有時直接放元件);沒寫時,每個實例的面板都是根面板;
  - `count`:數字變數;第 i 個實例在變數 > i 時才顯示(例如「顯示幾顆星」)。
- 展開發生在編譯時:實例接在原本的元件和面板後面,綁定、效果、類別、面板、`ui_layout` 看到的都是一般元件。展開後的元件總數仍然 ≤ 64。
- 錯誤會指到欄位:未知模板、少參數、多參數、參數值不合法、`into` 的面板不存在、模板面板寫了 `children`、參數取名 `i`、超過元件預算、刪除還在使用的模板。
- 編輯器:UI Editor 畫布照常畫出實例,但實例不能點選或拖動(要改模板或 use)。
- 示範:Axiom Town 的 6 個表情按鈕由模板 `emote_button`(參數 `name`)加一個 use(6 個實例,放進 `emotes` 面板)產生,元件 ID 仍是 `emote_wave` 等。

**內容包字型(plan M4)**

- `theme.fonts.ID = {path, license}`:內容包裡的 .ttf / .otf(路徑相對於內容包,匯入時換成絕對路徑),每份文件最多 4 個,每個 24 MiB 以內。
- `license` 必須是可以隨遊戲散布的開放授權:`OFL-1.1`、`Apache-2.0`、`MIT`、`CC0-1.0`。
- text class 和 button class 用 `font=ID` 指定字型;字型沒有的字(例如中文)會退回預設 UI 字型,再退回系統的微軟正黑體(系統字型只在本機讀取,不會被打包)。
- 載入:
  - 世界啟用時(不在主執行緒)讀檔並檢查是不是 TrueType / OpenType;壞掉的檔案讓啟用失敗,錯誤寫出字型 ID。
  - 主執行緒只把檢查過的字型交給 egui;egui 在下一幀才套用,所以第一幀用預設字型畫。
  - 字型組合沒變時不會重新安裝;別的 `set_fonts`(例如編輯器主題)把字型換掉時,會自動補回。
- `axiom package` 打包遊戲時會一起複製字型檔。
- 示範:Axiom Town 的 `ui/axiom-pixel.ttf` 是 `cargo run -p axiom-runtime --example build_ui_font` 產生的原創 5×7 點陣字(大寫、數字、常用標點,小寫對應到大寫;MIT),用在講者名字、對話台詞和表情按鈕;每次產生的結果都相同。

**自由版面與縮放(plan M3)**

- **大小**:`width`、`height` 可以是像素(`900`)或參考畫面的百分比(`"40%"`);沒設高度時跟著內容。
- **排列** `layout`:
  - `column`(預設):由上而下;
  - `row`:由左而右;
  - `grid`:每列 `columns` 格(預設 2),由左而右再往下;
  - `stack`:所有子項疊在同一個原點,後面的在上層。
- `gap`:子項間距(0–128 px)。
- **巢狀**:`widgets` 也可以寫另一個面板的 ID,那個面板就畫在這個面板裡面,忽略自己的錨點、偏移和 z。最多 4 層,不能形成循環;面板 ID 不能和元件 ID 相同。
- `z`:根面板的上下順序(-100–100),大的在上;沒設時依清單順序。
- **參考畫面** `theme.reference = [w, h]`、`theme.scale`:
  - 面板在 w×h 的參考座標裡排版,整個面板層再縮放到視窗;
  - `fit_height`(預設)依高度、`fit_width` 依寬度、`fit` 取兩者較小者、`none` 不縮放;
  - 只有面板會縮放,舊的九宮格 `regions` 維持螢幕像素。
- **觀察**:透過 `--connect` 觀察執行中的遊戲時,`observe` 的 `ui_layout` 列出每個 region、面板、面板內元件的螢幕矩形 `[x, y, w, h]`,以及診斷:
  - `ui_overlap`:兩個元件重疊,或兩個根面板 / region 重疊(同一個 stack 面板裡的元件不算);
  - `ui_offscreen`:超出視窗。
  - 只有文件有面板時才會出現。headless 的 `observe` 不排版,所以沒有這一項。
- 示範:Axiom Town 的參考畫面是 1280×720;6 個表情按鈕在右側的 `emotes` 面板(3 欄的 grid,往上偏 60,讓開和 Lumi 對話時變高的對話框)。
- 單位:參考座標以「整個視窗」為準,和螢幕的 DPI 縮放無關;`ui_layout` 回報的是視窗像素(egui point × DPI 縮放)。舊的 regions 仍跟著 DPI 縮放,所以參考畫面最好選接近「視窗 point 大小」的尺寸(例如 150% 縮放的 1920×1080 視窗是 1280×720 point),面板和 regions 的字級才會一致。

每個動詞都是 `axiom_core::ops` 的純函式(`set_ui_region`、`set_ui_style`、`set_ui_button_command`、`add_ui_effect`、`set_ui_effect`、`edit_ui_effect`、`remove_ui_effect`、`set_ui_widget_fields`),編輯器呼叫同一組函式。`discover ui_effects` 列出種類、觸發、預設與預算。

## UI Editor 分頁

Window ▸ UI Editor(`window.ui_editor`,和 Scene 同一格的分頁;`editor_view {dock:"ui_editor"}`)。

- **畫布**:用遊戲的 HUD 程式碼(`bound_ui::draw_canvas`,與 Play 時相同的區域錨點、面板、樣式、動效)在參考解析度(1280x720 / 1920x1080 16:9、1024x768 4:3、2560x1080 21:9)排版,再整體縮放進分頁。完整 chrome 的頂部除錯列以一條同高的色帶代表;沒放進區域的 widget 畫在畫布底部(遊戲裡是跨整個視窗的底部面板)。畫布不可操作遊戲:按鈕不會送指令。
- **拖放**:按住 widget 拖到九宮格的另一格,放開時一筆修訂(`ui place`),拖曳中不寫入,格子會高亮。
- **面板(plan M7)**:
  - 按住根面板(或面板裡的元件)拖動 = 移動面板(改 `at`);點選的面板有外框和一個邊緣把手,拖把手 = 改寬度(把手在錨點的對邊;置中的面板在右邊,兩側一起長)。
  - 位置和寬度對齊 8 個參考像素;拖曳中畫出虛線位置和數值,放開時一筆 `ui panel` 修訂,拖曳中不寫入。百分比寬度拖過以後會變成像素。
  - 元件面板上方固定顯示選取的面板:位置、樣式的 padding / corner(− / + 每按一次一筆 `ui panelstyle` 修訂),以及樣式邊框圖的九宮格邊距:在圖上拖左、上、右、下四條線,放開時一筆 `ui frame` 修訂。
  - 預覽沿用執行中遊戲已解碼的圖片(同一個檔案才用),編輯器自己不讀檔、不解碼。
- **元素面板**:Region、Style 下拉;按鈕的 Command 下拉(無欄位的事件 `emit`、各語系 `locale`;`choose` / `call` 用 Fields 修改);Fields(Inspector 的型別化表單,放開滑鼠且沒有在打字時提交,= `set widgets[id=W] F V`);Effects 清單(點選開表單、x 移除)、Add effect(種類下拉 + 按鈕)、▶ Preview(在畫布上用 egui 時間播放這個 widget 的所有動效;數值跳動在編寫值等於起點時示範跳到進度條上限或 +100)。效果表單:種類、觸發、事件、步數、ease、方向、amount,放開滑鼠時提交(`ui effect set`)。
- **語系預覽**:工具列的語系按鈕只改預覽(不寫入世界),缺字警告 = 該語系中空白或與鍵名相同的字串(滑鼠停留列出鍵名),另列出與預設語系文字相同的數量。
- **名牌預覽**:畫布左下角以遊戲同一個 `nameplate::paint` 畫出世界裡的 `ui.nameplate`(最多 16 個;可關閉)。
- 編輯器指令(人機同權,`editor_view {command, arg}`):`ui.place` / `ui.style` / `ui.button` / `ui.effect_add` / `ui.effect_set` / `ui.effect_remove`(參數與 ACL `ui` 動詞相同,經同一個解析器),檢視用 `ui.select WIDGET`、`ui.locale L`、`ui.resolution 1920x1080`、`ui.preview_effects`。

選取另一個 widget / 效果、Deselect 之前,未提交的表單草稿會先提交;但若有**外部**修訂(AI、bridge)在編輯中途到達,畫布重建,未提交的草稿會被丟棄。效果表單不能編輯 `when` 條件,`on: when` 只在效果已有條件時出現在觸發下拉(條件用 ACL `ui effect set ID when={..}` 或編輯器的 `ui.effect_set` 指令)。ACL 的 `OPTION=default` 一律代表清回預設,所以名為 `default` 的事件無法用 `event=default` 指定(用 JSON / `set`)。

限制:畫布不載入 `image` widget 的 PNG(避免在主執行緒讀檔;圖片 widget 在畫布中不顯示);縮放後的文字是同一份字形放大縮小,小尺寸時較模糊;動效只改畫面,不改點擊區域;名牌預覽不是 3D 投影位置。
