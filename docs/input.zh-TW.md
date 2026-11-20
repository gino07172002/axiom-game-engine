# 輸入綁定與手把(`input.bindings`)

`input.bindings` 把**動作名稱**對到輸入;系統只讀動作(`input.axis("move.forward")`、`input.pressed("jump")`),從不寫死按鍵,所以換按鍵、加手把都不用改 Rust。

```json
"input.bindings":{
  "axes":{"move.forward":{"positive":"KeyW","negative":"KeyS"}},
  "buttons":{"jump":"Space","fire":"MouseLeft"},
  "capture_pointer":true,
  "gamepad":{
    "axes":{"move.forward":{"positive":"PadLStickUp","negative":"PadLStickDown"}},
    "buttons":{"jump":"PadA","fire":"PadRT"},
    "deadzone":0.24
  }
}
```

| 欄位 | 說明 |
|---|---|
| `axes` | 動作 → `{positive, negative}` 鍵名(winit `KeyCode` 名稱 `KeyW` / `Space` / `ArrowUp` …、`MouseLeft`、滾輪的 `WheelUp` / `WheelDown`) |
| `buttons` | 動作 → 一個鍵名 |
| `capture_pointer` | Play 時鎖定滑鼠 |
| `gamepad` | 選填。**既有動作的第二組輸入**(手把),見下;沒有這個欄位的舊世界完全不變(序列化也不會多出欄位) |

## 手把(XInput)

- 後端:Windows XInput(`windows` crate 既有依賴加 `Win32_UI_Input_XboxController` feature,沒有新 crate)。最多 4 支,**所有連線中的手把合併**(同一個輸入取最大值),所以任何一支都能玩單人遊戲。
- 原生視窗(`axiom play` / 編輯器 Play / 打包後的遊戲)每個畫面在主執行緒讀一次;測試、無頭指令(`verify`、`scenario`、`call`)與編輯器內嵌的自動化宿主都**不讀**硬體,所以有沒有接手把不影響確定性。
- 拔插:連線中的槽每個畫面讀;空槽每 1 秒才探測一次(XInput 對空槽很慢)。拔掉的手把下一個畫面就變成未連線,數值歸 0;不會當機。沒有手把時輸出和以前逐位元相同(`ActionMap` 只在有手把時才合併)。
- `observe` 的 `gamepad`(只有原生視窗):`backend`(`xinput` / `disabled`)、`connected`(4 個槽)、`polls`、`poll_us_last` / `poll_us_max`(每次輪詢的主執行緒耗時)、`plugged` / `unplugged`、`probe_interval_ms`。

### 手把輸入名稱

| 名稱 | 來源 | 數值 |
|---|---|---|
| `PadA` `PadB` `PadX` `PadY` | 面板鍵 | 0 / 1 |
| `PadLB` `PadRB` | 肩鍵 | 0 / 1 |
| `PadBack` `PadStart` | | 0 / 1 |
| `PadLS` `PadRS` | 按下搖桿 | 0 / 1 |
| `PadDpadUp` `PadDpadDown` `PadDpadLeft` `PadDpadRight` | 十字鍵 | 0 / 1 |
| `PadLT` `PadRT` | 板機 | 0..1(低於 XInput 門檻 30/255 為 0,之上線性重新縮放) |
| `PadLStickUp/Down/Left/Right`、`PadRStickUp/Down/Left/Right` | 搖桿的四個方向 | 0..1(圓形死區 `deadzone` 內為 0,之外重新縮放到 0..1,邊緣沒有跳階;斜推長度仍是 1) |

### 合併規則

- `gamepad.buttons`:動作必須已經是 `buttons` 裡的按鈕;手把數值 ≥ 0.5 算按下。結果 = 鍵盤與手把取大值(任一個按下就按下)。板機與搖桿方向也能當按鈕。
- `gamepad.axes`:動作必須已經是 `axes` 裡的軸;手把值 = 正向數值 − 負向數值(-1..1,類比)。結果 = 鍵盤值 + 手把值,夾到 -1..1。
- `deadzone`:0..0.9,預設 0.24(≈ XInput 左搖桿建議值 7849/32767)。
- 驗證(錯誤是可恢復的請求錯誤):名稱不在上表 = 錯誤;動作不存在或種類不符(把軸綁成手把按鈕)= 錯誤;**不支援只有手把的動作**(要先有鍵盤綁定,其他檢查按鈕 / 軸是否存在的地方因此不用改)。

### 內容

- `content/fps`:左搖桿移動、右搖桿視角(新的 `look.x` / `look.y` 軸,鍵盤是方向鍵)、RT 開火、A 跳、LS 衝刺、X 換彈。FPS 每個固定步讀 `look.x` / `look.y`(全推 = 每秒 3 弧度水平、上推 = 抬頭),和滑鼠視角相加;沒有綁定時為 0,行為不變。
- `content/action`:左搖桿移動、右搖桿左右 = `orbit`、X 攻擊、A 翻滾、RB 鎖定。動作遊戲本來就只讀綁定,沒有改 Rust。

## 無頭注入(scenario)

`inputs[]` 的每一筆可以帶 `pad`(一支虛擬手把,寫入後一直保持到下一筆帶 `pad` 的輸入):

```json
{"ticks":120,"inputs":[
  {"tick":0,"actions":{},"pad":{"left_stick":[0,1],"buttons":["PadRT"]}},
  {"tick":60,"actions":{},"pad":{}}
],"assertions":[{"kind":"position","entity":"player","axis":2,"min":-20,"max":-1}]}
```

`pad` 欄位:`buttons`(按鈕名稱,只能是 0/1 的那些)、`left_stick` / `right_stick`([x, y],-1..1,y 向上,**套用死區之前**的原始值)、`left_trigger` / `right_trigger`(0..1)。數值經過與實機相同的死區與合併規則變成動作值;同一筆的 `actions` 在合併之後覆寫同名動作。沒有任何 `pad` 的 scenario 走原本的路徑(雜湊不變)。

編輯器 Play 的錄製(`play.record`)記錄的是合併後的動作值,所以手把操作也會被錄下並能無頭重播。

## 編輯器與無頭等價

| 編輯器 | 無頭(ACL) |
|---|---|
| Input Map 分頁每列的 Pad 下拉選單(按鈕一格、軸的 + / − 各一格,`—` = 移除) | `input pad ACTION NAME`(按鈕)/ `input pad ACTION POSITIVE NEGATIVE`(軸)/ `input pad ACTION off` |
| Input Map 分頁標頭的 Deadzone 數值 | `input deadzone V` |
| 刪除動作 | `input remove ACTION`(連同它的手把綁定) |

## 限制

- 只有 XInput(Xbox 相容手把);沒有 DirectInput / HID / PlayStation 原生協定、沒有震動(non-goal)。
- 多支手把合併成一個玩家;沒有「第 2 位玩家」的綁定。
- 手把數值每個畫面讀一次,同一畫面內的多個固定步看到同一份狀態。
