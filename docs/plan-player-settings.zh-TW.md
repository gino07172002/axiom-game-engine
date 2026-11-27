# 計畫:玩家設定、名牌主題化、UI 收尾、畫質與鏡頭參數

狀態:使用者在 2026-11-27 說「照你建議」,依建議的順序執行。基底:`ui-v2` `3df41f5`(= origin/main)。

## Phase 0 — 需求與成功樣貌

### 需求

盤點(2026-11-27)找到幾個和 UI 同類的缺口:玩家看得到、摸得到,但只能寫死。

1. **玩家設定**:音量、滑鼠靈敏度和反轉、視窗模式和垂直同步、畫質。引擎層完全沒有「設定」,打包出去的遊戲沒有地方讓玩家調。
2. **名牌** `ui.nameplate`:只有 text、color、height;字級、底色、內距、40 m 剔除距離寫死,文字不能翻譯,不能用 UI 主題。
3. **UI 收尾**:對話選項按鈕、進度條不能套類別;預設互動提示和 SAVE / LOAD / PAUSED 等介面文字寫死。
4. **畫質**:`render.settings` 只有 accent、shadow_extent;陰影貼圖固定 2048。
5. **動作鏡頭**:縮放一格、切第一人稱的距離、緩動、靈敏度都是常數。

### 最終驗收 demo(照著演)

1. 開 Axiom Town(新資料庫),按 Esc → 暫停選單多了「設定」按鈕。
2. 設定頁:
   - 總音量、音樂、音效三條滑桿;拖動後 `observe settings` 和音訊輸出的增益立刻改變;
   - 滑鼠靈敏度滑桿、Y 軸反轉開關:轉鏡頭的速度和方向跟著變;
   - 視窗 / 無邊框全螢幕 / 全螢幕、垂直同步開關:視窗立刻切換;
   - 陰影品質 低 / 中 / 高:陰影貼圖解析度改變(`observe` 回報)。
3. 關掉遊戲再開:設定都還在(存在玩家自己的設定檔,不在世界的修訂裡、也不在存檔裡)。
4. AI 代理用同一個型別化指令改設定(headless 結果相同)。
5. 名牌:套用主題的邊框和字型、文字走語系表、超過設定的距離淡出。
6. 對話選項按鈕、進度條用類別換外觀;介面文字切語系時跟著換。
7. 動作鏡頭的縮放和第一人稱距離由 `action.session` 設定。
8. 其他舊內容包的截圖和改版前逐位元相同。

### 約束

- 和 UI 計畫相同:不加 Bevy;每種內容編輯都要有 ops + ACL + 編輯器命令 + parity 測試;主執行緒 16 ms;每幀不配置;效能數字標示 debug / release。
- **設定不是內容,也不是存檔**:不進 authoring revision、不進 checkpoint;壞掉的設定檔不能讓遊戲開不起來(回到預設值並回報)。
- **重播要確定**:會影響模擬的設定(滑鼠靈敏度、反轉)在主機把輸入交給模擬**之前**套用,錄下來的輸入已經含設定的效果,重播不需要知道設定。
- 不推送第三方素材;不測試使用者的世界資料庫;寫入前先問。

### Non-goals

- **按鍵重新綁定的介面**(需要「按下要綁的鍵」的擷取流程和衝突處理):列入之後的工作;這份計畫只讓設定格式預留位置。
- MSAA(會動到所有管線的 sample count,風險和成本都高):畫質只做陰影解析度和 FXAA 開關,MSAA 另開計畫。
- 解析度清單(列舉螢幕模式):全螢幕用桌面解析度。
- 文字輸入框、下拉選單等新元件。

### 未知與假設

- 設定檔放在資料庫旁:`<資料庫所在資料夾>/settings/<world id>.json`。打包出去的遊戲資料庫在 `%APPDATA%\Axiom\<id>\`,所以設定檔也在那裡;開發時在 `data\settings\`。
- 作者在新資源 `game.settings` 宣告預設值和要開放哪些設定;沒有這個資源時用引擎預設值。

## Phase 1 — 勘查與風險

勘查(2026-11-27,讀過的程式碼):

- 音訊:`audio.rs` 的 `Output` 有 `Shared` 狀態和主執行緒 → 音訊執行緒的 `Message`;混音在 `audio_mix.rs`,沒有總增益、沒有分軌。
- 存檔位置:`app.rs` 用 `db.parent()/saves`;播放器 `axiom-player` 把資料庫放在 `%APPDATA%\Axiom\<id>\`。
- 視窗:`app.rs` 建立視窗時讀 `AXIOM_WINDOW_SIZE`;`render.rs:193` 寫死 `PresentMode::AutoVsync`。
- 滑桿元件:`Widget::Slider` 綁數字變數或 `call` 指令;指令 `slide` 一次放開送一筆。
- 動作鏡頭:`action.rs:388` 的 `dx * 0.0035`;縮放常數在 `action.rs:303-311`。
- 名牌:`nameplate.rs` 的 `paint` 和 40 m 剔除。

| # | 風險 | 若不成立 | 對策 / 驗證 |
|---|---|---|---|
| R1 | 執行中切換全螢幕、垂直同步不會讓 wgpu surface 壞掉 | 繞得過(改成重開才生效) | S3 先做小實驗:winit `set_fullscreen` + surface 重新 configure |
| R2 | 音訊執行緒能平滑套用增益,不爆音 | 繞得過 | 沿用 `Mixer` 的 gain ramp;S1 測試量測 |
| R3 | 靈敏度在主機套用後,重播仍確定 | 致命(重播是測試基礎) | ADR-3;S2 用錄製 + 重播測試證明 |
| R4 | 設定檔寫入不卡主執行緒 | 繞得過 | 只在值改變且放開時寫一次;寫入放背景工作,量測 |
| R5 | 舊內容畫面不變 | 致命 | 每個 milestone 截圖 A/B |

## Phase 2 — 方案設計(ADR)

### ADR-1:設定存在哪裡

- A 案:世界資源(authoring revision)。缺點:玩家改音量會產生內容修訂,也會跟著內容一起被別人改掉。
- B 案:checkpoint 的一部分。缺點:讀舊檔會把音量也讀回去,和玩家習慣不符。
- **C 案(採用)**:玩家自己的設定檔 `settings/<world id>.json`,和存檔分開;值改變時寫入(原子寫入:暫存檔 + 改名)。

### ADR-2:設定的形狀

- 引擎定義固定的設定鍵(型別、範圍都在程式裡),作者只能決定預設值和要不要顯示:
  - `audio.master`、`audio.music`、`audio.sfx`(0–1);
  - `mouse.sensitivity`(0.1–5)、`mouse.invert_y`(bool);
  - `display.mode`(window | borderless | fullscreen)、`display.vsync`(bool);
  - `quality.shadows`(low | medium | high = 1024 / 2048 / 4096)、`quality.fxaa`(bool)。
- 理由:設定會接到引擎的各個子系統,任意鍵無法接;固定鍵也讓 AI 代理能直接 `discover`。
- 作者在 `game.settings` 資源寫預設值(ops + ACL `settings default KEY VALUE` + 編輯器命令 + parity)。

### ADR-3:設定怎麼改、怎麼套用

- 型別化指令 `{kind: "setting", key, value}`:UI 元件、scenario、loopback 共用,和 `open` / `close` 一樣。
- 值存在 `WorldRuntime`(不在 checkpoint、不在 `state_hash`);`observe` 的 `settings`。
- 套用的位置:
  - 音量、視窗、垂直同步、陰影:主機(app / renderer / 音訊執行緒)讀設定;
  - 滑鼠靈敏度、反轉:主機在產生 `ActionMap` 時就乘進去(ADR 的重播約束)。
- UI:
  - `Widget::Slider` 加 `setting`(和 `variable` 擇一);
  - 開關和選項用按鈕送 `setting` 指令;按鈕類別的 `selected` 外觀標出目前值(M2 的按鈕類別多一個狀態)。

### ADR-4:名牌主題

- `ui.nameplate` 新增選用的 `key`(語系鍵,取代 text)、`class`(`ui.document` 的文字類別,含字型)、`frame`(`theme.frames` 的九宮格);
- 剔除與淡出距離放在 `ui.document.theme.nameplate = {fade_from, hide_beyond}`,預設維持 40 m、不淡出(舊畫面不變)。

### ADR-5:UI 收尾

- `classes` 對照表允許 choices(按鈕類別,套到每個選項按鈕)和 progress(新的 `theme.progress` 類別:fill、track、frame、vertical、show_value)。
- 引擎介面文字(SAVE / LOAD / PAUSED 等)放進每份文件可以覆寫的保留語系鍵 `axiom.save`、`axiom.load`、`axiom.paused`…;沒寫時用現在的雙語文字(舊畫面不變)。
- 預設互動提示使用 `theme.text` 的保留類別 `axiom.prompt` 和面板樣式 `axiom.prompt`(有寫才套用)。

### ADR-6:動作鏡頭

- `action.session.camera` 加 `zoom_step`、`first_person_from`、`first_person_at`、`zoom_ease`、`hide_within`、`orbit_speed`;預設等於現在的常數。

## Phase 3 — 增量拆解

| M | 範圍 | 驗收(操作 X,看到 Y) | 風險 |
|---|---|---|---|
| **S1 walking skeleton:總音量** | 設定模型 + 設定檔(讀、原子寫、壞檔回預設)+ `setting` 指令 + `observe settings` + 總音量接到音訊輸出 + 滑桿的 `setting` 綁定 + `game.settings` 預設值(ops / ACL / 編輯器 / parity)+ town 暫停選單的「設定」頁(總音量) | 拖滑桿 → `observe` 和輸出增益改變;重開後保留;headless 指令結果相同;壞掉的設定檔回到預設並回報 | R2、R4 |
| **S2 分軌與滑鼠** | 音樂 / 音效分軌(`audio.source.bus`、提示音算音效);滑鼠靈敏度、Y 反轉在主機套用 | 調音樂只影響音樂來源;靈敏度 2 倍時同樣的滑鼠移動轉兩倍;錄製重播結果相同 | R3 |
| **S3 視窗** | `display.mode`、`display.vsync` 即時套用 | 切換無邊框全螢幕、關垂直同步,截圖與 `observe` 確認,沒有 surface 錯誤 | R1 |
| **S4 畫質** | `quality.shadows`、`quality.fxaa` | 陰影品質低 / 高時 `observe` 的陰影貼圖解析度改變;截圖看過 | 重建陰影資源不能卡主執行緒 |
| **N1 名牌** | ADR-4 | 名牌套邊框和點陣字、切語系換文字、遠處淡出;舊內容截圖不變 | — |
| **U1 UI 收尾** | ADR-5 | 對話選項套按鈕類別、進度條變色和直條、介面文字切語系 | 和本機 Paladin 的進度條改動合併 |
| **C1 動作鏡頭** | ADR-6 | 改 `zoom_step` 後滾一格的距離改變;預設值和現在相同 | — |

## Phase 4 — Pre-mortem

1. **設定和內容混在一起**,AI 改內容時把玩家的音量蓋掉 → ADR-1 分開存;測試確認 `setting` 指令不產生修訂。
2. **重播壞掉** → ADR-3 在主機套用;S2 錄製重播測試。
3. **全螢幕切換讓 GPU surface 失效** → S3 先做小實驗;失敗就改成「下次啟動生效」並在 `observe` 標示。
4. **進度條和本機 Paladin 改動衝突**(Paladin 有 color / vertical / flash_at)→ U1 放最後,合併時把 Paladin 的欄位對應到新的進度條類別。
5. **範圍膨脹成完整的選項系統**(按鍵綁定、解析度清單、MSAA)→ 寫進 non-goals。

## Phase 5 — 執行協定

和 `docs/plan-ui-styling.zh-TW.md` 相同:每個 milestone 用 rigorous-change;gate = fmt + clippy `-D warnings` + `cargo test --workspace` + smoke + parity + 截圖 A/B + 新功能的 GPU 截圖(實際看過);每個 milestone 結束時把證據寫在下面的「執行紀錄」、偏離計畫先寫補記、寫 WORKLOG;推送、合併到本機 main、寫入使用者的世界,都先問。

## 執行紀錄

### S1 完成(分支 `ui-v2`)

- **模型**(`axiom-core/src/settings.rs`):9 個固定鍵,型別和範圍寫在程式裡;`merged` 讀玩家的設定檔時寬鬆(壞的值保留原值並回報),`defaults` 讀作者的 `game.settings` 時嚴格(錯誤指到 `/resources/game.settings/defaults/KEY`);數字回報到小數 4 位。
- **作者預設值**:資源 `game.settings`(能力 core);ops `set_setting_default`、ACL `settings default KEY VALUE|none`、編輯器命令 `world.setting_default`(和 ACL 用同一個值解析 `settings::parse_token`);parity 測試 3 步,parity 表加一列。
- **指令**:`{kind: setting, key, value}`,由 `WorldRuntime` 處理(不進 batch,core 驗證和執行期都擋);滑桿的 `command` 可以是 `{kind: setting, key}`(只能是數字鍵、範圍不能超出),值由滑桿填入;`discover` 的指令 schema 列出(不在 batch 的清單裡)。
- **狀態**:`WorldRuntime.player`(和 `Gameplay.settings` 的副本,給滑桿顯示);內容熱重載(同一個世界)保留;checkpoint 讀檔保留目前的設定(讀檔會重建 runtime,所以要明確帶過去);`observe settings`;`state_hash` 不含設定,設定滑桿也不放進 gameplay 的 `bindings`。
- **主機**:每幀檢查一次(只比較字串和計數,沒有配置):世界改變時讀 `<資料庫資料夾>/settings/<world id>.json`(上限 64 KB);設定改變時把總音量交給音訊執行緒、在短命的執行緒上原子寫入設定檔(`.partial` + 改名)。
- **音訊**:`Shared.master`(f32 位元),混音器在一個緩衝區內把增益漸變到新值;`observe` 的 `audio.output.master`。
- **頁面堆疊**(實作中發現):從暫停選單打開設定頁時,暫停選單的按鈕會從半透明的邊框透出來。現在最上面的模態頁面打開時,在它下面打開的頁面不畫。
- **內容**:town 的 `game.settings` 預設總音量 0.8;暫停選單加「設定」按鈕,打開 `settings` 頁面(模態、暫停):標題、總音量滑桿(0–1,間隔 0.05)、返回。
- **實作中發現並修正**:讀 checkpoint 會把設定重設成預設值;設定滑桿的值進了 bindings 讓 `state_hash` 跟著變;兩個都有測試。
- **測試**:core `tests/settings.rs` 3 個(型別與範圍、壞掉的設定檔、ACL 預設值與資源的建立 / 移除);runtime `tests/settings.rs` 3 個(指令、observe、state_hash 不變、batch 和錯誤值被擋;checkpoint 不帶設定;設定頁滑桿送 setting、顯示值、下面的暫停選單不畫);parity 1 個;M6 的焦點測試因為多了「設定」按鈕改成按兩次。
- **主機驗收**(GPU,新資料庫,1920×1080):開始時 `settings.audio.master` 和 `audio.output.master` 都是 0.8(作者預設);bridge 送 `setting audio.master 0.35` 後兩者都是 0.35,設定檔 `settings/axiom-town.json` 寫入;關掉重開後仍是 0.35;設定頁截圖看過。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1157 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(`3df41f5` 對 S1):9 個內容沒變的包逐位元相同。
- **未驗證**:沒有實際聽聲音(驗證的是送進音訊執行緒的增益);設定檔寫入的執行緒是分離的(很小的檔案,沒有在關閉時等待它)。

### 補記 2026-11-27:S2 和 ADR-3 的差異

- 滑鼠靈敏度和 Y 反轉只作用在「捕捉滑鼠的視角」(`mouse_look`,動作遊戲與 FPS 的轉視角)。`camera.follow` 的拖曳環繞是在模擬裡用畫面上的絕對位置換算角度,若在主機套用會破壞重播的確定性;這條路徑維持原樣,記為限制。

### S2 完成(分支 `ui-v2`)

- **分軌**:`audio.source.bus`(`sfx` 預設、不寫出;`music`);ACL `audio SEL source … bus=sfx|music`、編輯器「加音源」對話框的 Volume bus 欄位,同一個 `ops::AudioSourceOptions`(既有的對話框 parity 測試照樣通過)。
- **混音**:`VoiceParams.music`(和 looping 一起打包在同一個字,8 個字不變);音樂聲道混進第二個暫存區(建立時配置一次);最後輸出 =(音效 × 音效音量 + 音樂 × 音樂音量)× 總音量,三個增益都在一個緩衝區內漸變;提示音和單發音效算音效;`observe` 的 `audio.output.music` / `sfx`。
- **滑鼠**:`look_delta`(靈敏度乘上 dx、dy,反轉只翻 dy)在 `App::mouse_look` 套用,在模擬和錄製之前,重播不需要知道設定。
- **內容**:town 的設定頁加音樂、音效、滑鼠靈敏度三條滑桿和「反轉 Y 軸 / 不反轉」兩個按鈕(`invert_row` 子面板)。
- **測試**:混音 2 個(音樂只跟音樂音量、音效只跟音效音量,0.5 / 0.25 的聲道得到 5000 / 2500 / 7500;旗標經過共享區塊後還在)、`look_delta` 1 個、core 1 個(`bus` 預設不寫出、`music` 來回、錯誤的值被擋、ACL 的錯誤訊息)。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1161 passed / 0 failed / 7 ignored(第一次因為 `tests/ops.rs` 的 `AudioSourceOptions` 少了 `bus` 沒編過,補上後重跑);editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(`3df41f5` 對 S2):9 個包逐位元相同。
- **GPU**(新資料庫,1920×1080):bridge 送音樂 0.2、音效 0.6、靈敏度 1.7、反轉 Y;`observe` 的 `settings` 和 `audio.output`(music 0.2、sfx 0.6)一致,設定檔寫入;設定頁截圖看過(四條滑桿的值、反轉按鈕、返回)。
- **未驗證**:沒有實際聽聲音;滑鼠靈敏度只用單元測試驗證(GPU 腳本無法移動實體滑鼠)。

### S3 完成(分支 `ui-v2`)

- **垂直同步**:`Renderer::set_vsync`。開:FIFO;關:mailbox,不支援就 immediate,都不支援就維持 FIFO(每個 surface 都支援)。模式沒變時不重新 configure。
- **視窗模式**:`display.mode` 改了立刻套用(`App::sync_player_settings`,和音量分開記錄已套用的修訂,因為視窗可能還沒建立)。
  - `fullscreen` 用目前螢幕最大的解析度和最高更新率;螢幕沒列出模式時用無邊框。
  - 從全螢幕回到視窗時,恢復進全螢幕前的視窗大小。
  - 編輯器不套用(它有自己的 View ▸ Fullscreen)。
- **observe**:`display {mode, present_mode, size}`,從視窗和 surface 的實際狀態讀出,不是設定值。
- **內容**:town 設定頁加「視窗 / 無邊框 / 全螢幕」和「垂直同步 / 不同步」兩列按鈕。
- **R1 結論**:執行中切換沒有讓 surface 壞掉,不需要退回「下次啟動生效」。
- **GPU**(新資料庫,port 5313,debug):依序關垂直同步、無邊框、全螢幕、視窗、開垂直同步。
  - `observe` 依序是 window/mailbox 1440×900 → borderless 1920×1080 → fullscreen 1920×1080 → window 1440×900 → window/fifo。
  - stderr 只有截圖訊息,沒有 surface 錯誤;五張截圖都看過,畫面正常;設定檔寫入最後的值。
  - 第一次跑時回到視窗變成 1898×1024(Windows 沒有恢復原本大小),加上恢復大小後重跑才是 1440×900。
  - 設定頁截圖看過,`ui_layout` 沒有重疊或超出畫面。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1161 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(S2 對 S3):10 個包逐位元相同。
- **未驗證**:mailbox 是否真的讓幀率超過螢幕更新率(沒量測幀率);多螢幕;實際按 Alt+Enter 等系統快捷鍵和設定的互動。
- **已知外觀問題**:設定頁按鈕列之間的空隙偏大(S2 的反轉列就有),留給 U1 一起整理。

### 補記 2026-11-27:S4 的 FXAA 語意

- 原計畫只寫「FXAA 開關」。實作定為:`quality.fxaa` 只能關掉世界要求的 FXAA,不會替沒開的世界加上。
- 理由:
  - 預設值是 true,如果 true 代表「強制開」,所有舊內容包的畫面都會變,違反「舊內容包截圖逐位元相同」。
  - 作者沒開 FXAA 可能是刻意的(像素風、銳利的 UI)。
- 後果:沒開 FXAA 的世界(目前所有內容包),這個開關沒有效果。所以 town 的設定頁不放 FXAA 按鈕。

### S4 完成(分支 `ui-v2`)

- **陰影**:`Renderer::set_shadow_texels`。大小改變時換一張深度貼圖(只配置,不上傳),並重建場景和網格的 bind group;下一幀的深度 pass 填滿它。
  - `shadow.wgsl` 的 PCF 偏移改用 `textureDimensions(shadow_map)`,不再寫死 2048。2048 時算出的值相同。
- **FXAA**:`Renderer.player_fxaa`,在 render 開頭 `requested.fxaa &= player_fxaa`。關掉時走原本「沒有後處理」的路徑,沒有額外的管線建置。
- **observe**:`quality {shadow_map, fxaa}`。
- **內容**:town 設定頁加「陰影 低 / 中 / 高」。
- **GPU**(新資料庫,port 5314,debug,1440×900):先用 ACL `post antialiasing fxaa` 在測試資料庫開 FXAA(rev 2)。
  - 依序:medium(`shadow_map` 2048、FXAA pass 0.95 ms)→ low(1024)→ high(4096)→ 關 FXAA(`render_post` 消失,沒有 FXAA pass)。
  - stderr 只有後處理管線建置的訊息。
  - 截圖看過:low 和 high 都有正常的陰影(換貼圖後 bind group 正確)。在這個距離肉眼看不太出解析度差別;解析度改變以 `observe` 為證據。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1161 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(S2 對 S4):10 個包逐位元相同(也證明 `textureDimensions` 的改動在 2048 時不影響畫面)。
- **未驗證**:沒有量測換陰影貼圖那一幀的主執行緒時間(只有配置一張貼圖和兩個 bind group);沒有量測 4096 的 GPU 成本。

### 補記 2026-11-27:N1 和 ADR-4 的差異

- ADR-4 寫的是名牌的 `frame`(九宮格)。實作改成 `style`(`theme.panel` 的面板樣式:底色、圓角、padding、邊框)。
  - 理由:名牌只有一行字,高度約 20 px。只有九宮格的話,邊距大的邊框會被壓扁,也沒辦法調 padding;面板樣式已經有這些欄位,作者也已經會用。
- 另外加了 `theme.nameplate.class` / `style` 當預設值,不用每個名牌都寫一次。名牌自己的設定優先。
- 名牌不跟 `theme.reference` 縮放(和以前一樣是螢幕像素),記為限制。

### N1 完成(分支 `ui-v2`)

- **資料**:`ui.nameplate` 加 `key`、`class`、`style`(`text` 有 `key` 時可以省略);`theme.nameplate {fade_from, hide_beyond, class, style}`。
  - 檢查:距離 0 < hide ≤ 1000、0 ≤ fade < hide;類別和樣式要在主題裡;名牌的類別和樣式在 `Gameplay::compile` 檢查,錯誤路徑指出物件(`/entities/ID/components/ui.nameplate/class`)。
- **編輯路徑**:`ops::set_ui_nameplate`、ACL `ui nameplate …`、編輯器命令 `ui.nameplate`、parity 步驟(`fade=12 hide=30`)、parity 表一列。名牌元件本身照舊用 Inspector / `set_field` 改。
- **繪製**:`nameplate::look` 從類別和樣式組出外觀(字型用 `bound_ui` 同一份已安裝的字型、邊框用同一個貼圖快取);`paint` 加透明度。沒有主題時的外觀和以前相同。UI Editor 的名牌預覽用同一套函式。
- **內容**:town 的名牌用像素字型、按鈕邊框;18 公尺開始淡出、30 公尺外不畫;主角、Lumi、Rin 的名牌改用語系鍵。
- **測試**:core 1 個(ACL、淡出曲線、沒有主題時的 40 公尺、6 種錯誤、拒絕時不寫入修訂);runtime 2 個(語系切換和找不到鍵時的後備;類別、樣式不存在時的錯誤路徑)。
- **GPU**(新資料庫,port 5315,debug):中文截圖是「露米 ♥」「AXI · 你」「RIN · 可麗餅」,英文是「LUMI ♥」「AXI · YOU」「RIN · CREPES」;遠處的 Nova、Rin 名牌變淡;邊框和陰影正常。stderr 沒有任何訊息。
  - 像素字型只有大寫字母,所以英文名字顯示成大寫(對話框原本也是這樣)。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過(第一次 clippy 擋下 `!(distance <= hide)`,改成 `is_nan() || >`);`cargo test --workspace --no-fail-fast` 1163 passed / 1 failed / 7 ignored,失敗的是 `a_full_capture_queue_refuses_editor_view_before_its_edit`(database is locked,之前就記錄過的不穩定測試),單獨重跑通過;editor smoke 通過、world 和 street parity 結果一致、M0 PASS。
  - 截圖 A/B(S2 對 N1):town 以外 9 個包逐位元相同。town 的名牌是刻意改變的,而且舊版執行檔讀不了 `theme.nameplate`,所以 town 不在 A/B 裡,改用上面的 GPU 截圖檢查。
- **寫進使用者資料庫的順序**:town 用了舊版執行檔讀不了的欄位,必須先合併到本機 main,再寫入 `data\axiom-town.sqlite`。

### 補記 2026-11-27:U1 和 ADR-5 的差異

- 互動提示:ADR-5 寫的是保留類別名稱 `axiom.prompt`。實作改成 `theme.prompt {class, style}`,和 N1 的 `theme.nameplate` 同一種寫法。
  - 理由:保留名稱可能和作者自己的類別撞名;另外類別 ID 規則不一定允許點號。
- 進度條:ADR-5 列了 `vertical`(直的進度條)。這次沒做,記為之後再做;其他欄位(fill、track、frame、height、corner、value)都有。
- 引擎介面文字只做了 SAVE、LOAD、PAUSED 三個(`axiom.save`、`axiom.load`、`axiom.paused`)。開發用的「AXIOM / NARRATIVE」標籤只在開發模式出現,不做。
- 發現:town 和 street 都用自己的 `prompt` 文字元件,引擎預設的提示在這兩個包裡都不會畫。所以 town 沒有寫 `theme.prompt`,改用 runtime 測試驗證。

### U1 完成(分支 `ui-v2`)

- **資料**:`theme.progress`(進度條類別)、`theme.prompt`;`classes` 允許 choices(按鈕類別)和 progress(進度條類別),種類不對時錯誤路徑是 `/classes/元件`。
- **編輯路徑**:`ops::set_ui_progress_class`、`ops::set_ui_prompt`;ACL `ui progressclass`、`ui prompt`;編輯器命令 `ui.progress_class`、`ui.prompt`;parity 兩個步驟;parity 表兩列。`ui.document` 的 schema 文字也更新了。
- **繪製**:
  - 選項按鈕有類別時用 `themed_button`(和一般按鈕同一個函式);
  - 進度條有類別時自己畫(底色、填色、邊框、百分比);
  - 提示有 `theme.prompt` 時用文字類別和面板樣式(底色和邊框畫在文字後面);
  - 引擎文字用 `chrome()`:語系表有這個鍵就用,沒有就用原本的雙語文字。
  - 沒有這些設定時都走原本的程式碼。
- **內容**:town 的對話選項用 emote 按鈕,語系表加了 `axiom.save` / `axiom.load` / `axiom.paused`。
  - 原本也給好感度進度條一個類別。但多個編輯器測試和 town 的 UI 效果 scenario 會替 `affection` 加文字樣式(樣式和類別只能擇一),所以 town 不放進度條類別,進度條類別由 core 測試驗證。
- **測試**:core 1 個(進度條類別、提示、9 種錯誤、使用中的類別不能刪);runtime 2 個(選項只接受按鈕類別;street 拿掉自己的提示元件後,引擎提示畫出面板樣式的底色和文字類別的顏色)。
- **GPU**(新資料庫,port 5316,debug):
  - 中文版的工作列是「存檔 / 讀檔」,沒按 RUN 時是「暫停中」;英文版是 SAVE / LOAD。
  - 和 Lumi 對話時五個選項用 emote 按鈕的樣子。
  - stderr 沒有任何訊息。
- **未驗證**:進度條類別沒有在 GPU 截圖裡看到(town 不再使用;只有 core 測試和 parity 驗證資料和 ACL)。
- **順便修掉的不穩定測試**:`a_full_capture_queue_refuses_editor_view_before_its_edit` 和 `…_before_its_command` 用同一個資料庫檔名 `capture-full.sqlite`,同一個行程平行執行時互相刪檔、鎖檔("database is locked")。只跑這兩個時重現失敗;改成各自的檔名後一起跑 4 次都通過。之前 N1 和更早的 gate 記錄的不穩定失敗就是這個原因。
- **測試期望的更新**(內容刻意改變):`ui_panels.rs` 找對話框邊框時多加「高度 > 120」(選項按鈕現在也有邊框,但很矮);工作列文字改成 town 自己的「存檔 / 讀檔」。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1167 passed / 0 failed / 7 ignored(第一次 7 個失敗,原因和修正見上面);editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(S2 對 U1):town 以外 9 個包逐位元相同。

### 補記 2026-11-27:C1 和 ADR-6 的差異

- ADR-6 寫的是 `action.session.camera` 底下加欄位。但 `camera` 已經是相機物件的 ID(字串),不能放設定;改成新的選用物件 `action.session.camera_tuning`,欄位名稱照 ADR-6。
- 沒有新增專用的 ops / ACL 動詞:`camera_tuning` 是 `action.session` 資源裡的資料,用既有的資源編輯(ACL `set resources[id=action.session] …`、編輯器資源表單)就能改,兩邊走同一個通用路徑。

### C1 完成(分支 `ui-v2`)

- **資料**:`CameraTuning {zoom_step, first_person_from, first_person_at, zoom_ease, hide_within, orbit_speed}`,全部選用,預設等於原本的常數(0.8、1.6、0.5、0.08、0.6、1.5 rad/s)。檢查範圍與 0 ≤ at < from < `camera_distance`。
- **執行**:`action.rs` 的五個常數和寫死的 1.5 改成讀 `tuning(session)`;沒有 `camera_tuning` 時用同樣的數字、同樣的算式順序。
- **測試**:`town_camera.rs` 1 個(預設一格 = 0.8;`zoom_step 0.5` 一格 = 0.5;沒有第一人稱時停在 `first_person_from 3.0`;4 種錯誤)。原本的 6 個鏡頭測試照樣通過。
- **ACL**(獨立資料庫):`set resources[id=action.session] camera_tuning {"zoom_step":0.6,"orbit_speed":2.5}` 寫入 rev 2;`zoom_step 2` 被拒絕,訊息是 `camera_tuning: zoom_step must be 0.5..0.95`。
- **文件**:`docs/camera-follow.zh-TW.md` 的縮放一節。
- **未驗證**:沒有 GPU 截圖(這個 milestone 沒有改繪製;行為用 `observe action.camera` 的距離驗證)。
- **Gate(debug)**:fmt、clippy `-D warnings` 通過;`cargo test --workspace --no-fail-fast` 1168 passed / 0 failed / 7 ignored;editor smoke 通過、world 和 street parity 結果一致、M0 PASS;截圖 A/B(S2 對 C1):town 以外 9 個包逐位元相同。

### 計畫完成(2026-11-27)

- S1–S4、N1、U1、C1 都在 `ui-v2` 本機提交,尚未推送、尚未合併到本機 main、尚未寫進使用者資料庫。
- 寫進 `data\axiom-town.sqlite` 前必須先合併到本機 main(town 用了舊版執行檔讀不了的 `theme.nameplate` 等欄位)。
