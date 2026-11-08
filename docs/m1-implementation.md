# M1 實作與驗收

M1／v0.3 在 World IR v2 上加入共用玩法執行器，保留 axiom-core 的 authoring 交易邊界及 axiom-runtime 的固定步進邊界，沒有新增 Bevy 或依遊戲類型分支。

| 交付 | 實作與證據 |
|---|---|
| 共用事件、條件、動作 | core/gameplay.rs 型別與引用編譯；runtime/gameplay.rs 有界執行、typed payload、延遲事件、trace |
| 敘事圖 | 條件選項、on_enter、節點、場景、結局；來源 node 驗證避免過期選擇 |
| UI 資料綁定 | 文字、變數、本地化變數、角色／對話、按鈕、選項、圖片、進度；實際 egui 點擊測試 |
| 中文／英文 | zh-TW 與 en catalogs 相同 keys；Windows 系統 CJK fallback；換行與捲動 |
| 圖片 | 原創程式繪製 PNG；scripts/build-adv-art.ps1 可重建；包內路徑及影像大小驗證 |
| 存讀檔 | Checkpoint v1；旗標、物品、場景、node、延遲事件、語系、模擬欄位與時鐘；file roundtrip 與損壞／備份測試 |
| ADV 包 | content/adv：兩位角色、兩個場景、可調查箱子、鑰匙與三種結局 |
| AI 資料編輯閉環 | examples/adv-branch-edit.json 經 preview／CAS apply／observe／undo；不重建引擎 |
| 情境驗證 | 三個結局 scenario、存讀檔重播、中途可見性與狀態斷言；narrative-check 有預算可達性探索 |
| FPS 回歸 | 原有交易／遷移／物理／射擊測試及完整 M0 GPU smoke；另測 shared gameplay 與 FPS 同世界組合 |

## 執行

雙擊 `Play ADV.cmd`；使用專屬 data/adv.sqlite，既有內容修改保留。選項用滑鼠；工坊按 E 亦可取得鑰匙；上方控制台維持原 JSON 協定。

先載入 scripts/env.ps1，執行 cargo fmt、workspace tests、Clippy -D warnings 及 workspace build，再執行 scripts/m1-smoke-test.ps1。Smoke 預設包含 M0／FPS 回歸，使用獨立 QA 資料庫和隱藏原生視窗，結束會清理自己的 helper process。報告為 artifacts/m1-validation.json；同一執行檔 SHA256 在所有內容操作前後一致。

## 行為合約

事件消費採確定的佇列順序與作者的規則順序；每批命令／每個 fixed-step 上限 512 指令、128 個排隊事件、16 層 node entry。敘事、條件、UI 和事件參照在編譯階段驗證；錯誤命令、溢位及超額不會提交部分玩法變更。

Preview 不改 authoring revision。authoring commit 與 runtime activation 分開；同一 world ID 的進度依 node／variable／scene IDs 遷移，無法遷移時保留原 runtime。讀存檔不執行 on_enter，也不回復／追加 authoring revision。

## 明確邊界

- 共用變數支援 bool、i64、f64、text；尚未提供任意 component field 寫入、任意程式腳本、完整任務圖或取消任務 API。
- UI 目前是有順序的資料元件與單一可捲動文件版面；不是完整通用排版編輯器。PNG 是首個圖片格式；未提供完整資產資料庫或字型散布。
- checkpoint 要求相同內容定義與引擎版本；不相容時明確拒絕，尚未實作內容版本間的存檔遷移。FPS 私有物理／戰鬥進度不宣稱可以 checkpoint。
- narrative-check 證明範圍是 authored choices、UI buttons、delayed events；未知外部事件、所有實數／無界整數狀態不自動成為可證明範圍。預算不足回傳 complete:false。
- RTS 導航／經濟與 Action 動畫／技能時序仍屬 M2／M3。
