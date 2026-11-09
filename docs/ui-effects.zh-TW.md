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
ui button WIDGET emit EVENT [{..}] | locale L | choose NODE CHOICE | call CAP ACTION [{..}]
ui effect add ID WIDGET KIND [on=..] [event=..] [when={..}] [ticks=N] [ease=..] [direction=..] [amount=X] [from=X]
ui effect set ID [kind=..] [widget=..] [同上;OPTION=default 清回預設]
ui effect remove ID                   # ui.effects 系統保留
```

每個動詞都是 `axiom_core::ops` 的純函式(`set_ui_region`、`set_ui_style`、`set_ui_button_command`、`add_ui_effect`、`set_ui_effect`、`edit_ui_effect`、`remove_ui_effect`、`set_ui_widget_fields`),編輯器呼叫同一組函式。`discover ui_effects` 列出種類、觸發、預設與預算。

## UI Editor 分頁

Window ▸ UI Editor(`window.ui_editor`,和 Scene 同一格的分頁;`editor_view {dock:"ui_editor"}`)。

- **畫布**:用遊戲的 HUD 程式碼(`bound_ui::draw_canvas`,與 Play 時相同的區域錨點、面板、樣式、動效)在參考解析度(1280x720 / 1920x1080 16:9、1024x768 4:3、2560x1080 21:9)排版,再整體縮放進分頁。完整 chrome 的頂部除錯列以一條同高的色帶代表;沒放進區域的 widget 畫在畫布底部(遊戲裡是跨整個視窗的底部面板)。畫布不可操作遊戲:按鈕不會送指令。
- **拖放**:按住 widget 拖到九宮格的另一格,放開時一筆修訂(`ui place`),拖曳中不寫入,格子會高亮。
- **元素面板**:Region、Style 下拉;按鈕的 Command 下拉(無欄位的事件 `emit`、各語系 `locale`;`choose` / `call` 用 Fields 修改);Fields(Inspector 的型別化表單,放開滑鼠且沒有在打字時提交,= `set widgets[id=W] F V`);Effects 清單(點選開表單、x 移除)、Add effect(種類下拉 + 按鈕)、▶ Preview(在畫布上用 egui 時間播放這個 widget 的所有動效;數值跳動在編寫值等於起點時示範跳到進度條上限或 +100)。效果表單:種類、觸發、事件、步數、ease、方向、amount,放開滑鼠時提交(`ui effect set`)。
- **語系預覽**:工具列的語系按鈕只改預覽(不寫入世界),缺字警告 = 該語系中空白或與鍵名相同的字串(滑鼠停留列出鍵名),另列出與預設語系文字相同的數量。
- **名牌預覽**:畫布左下角以遊戲同一個 `nameplate::paint` 畫出世界裡的 `ui.nameplate`(最多 16 個;可關閉)。
- 編輯器指令(人機同權,`editor_view {command, arg}`):`ui.place` / `ui.style` / `ui.button` / `ui.effect_add` / `ui.effect_set` / `ui.effect_remove`(參數與 ACL `ui` 動詞相同,經同一個解析器),檢視用 `ui.select WIDGET`、`ui.locale L`、`ui.resolution 1920x1080`、`ui.preview_effects`。

選取另一個 widget / 效果、Deselect 之前,未提交的表單草稿會先提交;但若有**外部**修訂(AI、bridge)在編輯中途到達,畫布重建,未提交的草稿會被丟棄。效果表單不能編輯 `when` 條件,`on: when` 只在效果已有條件時出現在觸發下拉(條件用 ACL `ui effect set ID when={..}` 或編輯器的 `ui.effect_set` 指令)。ACL 的 `OPTION=default` 一律代表清回預設,所以名為 `default` 的事件無法用 `event=default` 指定(用 JSON / `set`)。

限制:畫布不載入 `image` widget 的 PNG(避免在主執行緒讀檔;圖片 widget 在畫布中不顯示);縮放後的文字是同一份字形放大縮小,小尺寸時較模糊;動效只改畫面,不改點擊區域;名牌預覽不是 3D 投影位置。
