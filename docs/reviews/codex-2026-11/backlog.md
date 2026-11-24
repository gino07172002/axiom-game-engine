# Codex review 2026-11 — Backlog(未修項目總表)

收斂規則(使用者決定):第三輪起只修「我們的修正造成的回歸」與「high」;其餘(medium/low、修得不完整但沒有比修前差、測試缺口)一律列在這裡,不在本次 review 中修。確認複審之後的「最終輪」只修回歸與 high,之後不再送 Codex 複審。

各項的原始證據在同目錄的報告裡;處理紀錄(判定、修改、測試、gate)在 `docs/plan-codex-review.zh-TW.md` 的「處理紀錄」。

## 0. 沒有經過 Codex 複審的修正

下列修正只經過修正者的 gate 與整合 gate,**沒有再送 Codex 確認**(依「確認一次後收尾」的決定):

- 第三輪:R2(r2c)、R4(r4c)、R8(r8c)、執行期 R3/R5/R6/R7(r3x)。
- 最終輪:C0(c0f)、R9/R10(r9f)。

若要再確認,下次 review 應以這些 merge commit 的 diff 為範圍。

## 1. 第三輪分流的 BACKLOG(36 項)

完整表格與一行證據見 `triage-round3.md`(各區「BACKLOG」列)。摘要:

| 區 | 項目 |
|---|---|
| R1 core / protocol | 前輪 N1(浮點與整數混合比較仍走 f64)、前輪 N2(超過 256 bytes 的 diff 值只回長度與指紋)、6.1-F10(`rpc`/`serve` 先開 Store 再解析請求)、6.2-F14 / 6.1-F9(animation operand schema 沒表達 `field:"param"` 與 `param` 互斥)、N1(跨世界 `import` 後舊提案仍可依 base_rev 套用)、N2(待審上限 256 但列表最多 64)、N3(`observe` 可投影整個 `/gameplay`) |
| R3 rendering | F8(截圖 `hard_link` 退回 `rename` 的 TOCTOU 空窗)、F11(postfx 分數像素邊界仍混色)、F10(天空 z=1.0 + `LessEqual` 蓋掉深度恰為 1.0 的材質)、F13(`vfx.inspect` 對 ribbon 用粒子生命週期)、F12(`render_post_not_ready` 缺可注入失敗的測試)、F14(`local_lights` 在 acquire 前更新)、N1(fxaa/hdr shader 既有問題)、N2(`vfx.rs` 熱路徑 `e.clone()`) |
| R5 physics / nav / gameplay | F9(A* 只限展開次數,移動目標下長路徑預覽可能永遠不完成,待確認)、F7(回溯與轉角掃描未計入預算)、F11(目標被刪時 camera socket 清空)、N1(physics checkpoint 不存 `events_halted`)、N2(trigger 先計數再 emit)、N3(nav 以原始位置判斷重排)、N4(fsm `clip` 只做 `valid_id`) |
| R6 audio / input | F4(短音效 Shot 先於 Clips 送出)、N1(編輯器用當下的 `expected_rev`)、N2(修飾鍵固定寫成 Left)、N3(gain 固定 `None`) |
| R7 genre / game UI | F6(作者改場景旗標時留下初始 `on_enter` 結果)、F13(熱載入後無變數滑桿只改 `slider_values`,GUI 是否不同待確認)、F8(RTS 熱載入對 goal 仍 `request_path`)、F15(Showcase 首幀多轉約 0.004 rad)、F7(FPS 跨世界移轉測試在舊碼也會過)、F9(交付事件 envelope 缺斷言)、F12(閒置固定步測不出 clone 被加回)、N1(RTS 只比較單位數)、N2(narrative `Op::Call` 累加不清空)、N3(fps editing `*old = e`) |

## 2. 第二輪確認複審之後的 BACKLOG

| 來源 | 項目 |
|---|---|
| R2.recheck2 | A-F11(只有測試缺口,行為正確);`inexact` 錄影重播的低嚴重度問題 |
| R4.recheck2 | A-F14(selector echo 可到 67 bytes);前輪 N3(熱重載 `playhead` 時間可能與姿勢取樣不同);`animation.stage` 丟掉相同的 skin cache;gameplay bindings 解析 primary clip 兩次;F3 缺 checkpoint 到事件送達的端對端測試 |
| R8.recheck | N2 [medium](遠端切換頁籤會取消人類正在做的拖曳) |

## 3. 最終確認複審(C0 / R9 / R10)之後的 BACKLOG

| 來源 | 項目 |
|---|---|
| C0.recheck | `unknown_clip` 缺實際 Store 開庫、修復、undo 的整合測試(目前以同一個 `World` 當前後值) |
| R9.recheck2 | Windows 預設資料庫以原始 package ID 命名,只差大小寫的兩個 ID 會共用 `world.sqlite`;`TEMP` 不同的程序拿到不同的鎖;非 Windows 的 check-then-rename;R9.recheck 的 F5、F6、F8、F9、F12、F14、F15、N3 |
| R10.recheck | 兩支 parity 腳本的 `Wait-Editor` 以嘗試次數限制,沒有總牆鐘期限 |

## 4. DEFERRED(需要較大改動)

| 區 | 項目 | 需要什麼 |
|---|---|---|
| R2 第一輪 | F7 | 見處理紀錄 R2 |
| R3 第一輪 | F3 | 見處理紀錄 R3 |
| R4 第一輪 / 第二輪 | F9;F5 部分 | 見處理紀錄 R4 |
| R6 第一輪 | F2、F3;音檔在主執行緒同步建置 | 建置移到背景工作 |
| R7 第一輪 / 第三輪 | F10 部分;F2:共用導航網格仍在固定步內建置 | 改 `RtsSystem` factory 與 `set_combat` 呼叫點,在啟用/準備階段建好,step 只交換(`rts.rs` 每步 clone 整個 scene) |
| R8 第一輪 | F8、F9 | 見處理紀錄 R8 |
| R9 第一輪 | F10 部分 | 見處理紀錄 R9 |
| C0 最終輪 | 提案 diff 逐頁產生(目前以固定 64 MiB 上限擋下);編輯器 ghost / merge 預覽仍用共用的 16 MiB 預算;舊陣列覆寫完全省略某欄(依 serde 預設)時退回一般物件形式 | `diff::changes` 改成可中止的串流;編輯器預覽改走 `ops::review_edits` |

## 5. 修正者自報的未驗證事項與取捨

- **R2 第三輪**:熱重載建置期間,若執行中的 runtime 還原 checkpoint 或產生同 ID 實體,`transfer_state` 保留其當下 handle,factory 存下的 handle 可能失效(罕見交錯)。`replacement()` 每次背景啟用在主執行緒複製實體 handle 表,未量測。
- **R3 第三輪**:同一幀內的多個截圖請求仍拍到同一幀;次像素 viewport 只有單元測試,沒在 GPU 上實跑(1805 px 視窗下 0.001 寬為 1.8 px)。
- **R4 第三輪**:A-F5 clip index 命中時,含內嵌 base64 的大 `.gltf` 每次整檔讀取,未量測;新的資產級上限 `MAX_ANIMATION_KEYS`=16M 可能拒絕極端資產;A-F2 沒有證明只建一個 decoder 的測試。
- **R8 第三輪**:F11 視窗關閉路徑沒用真實視窗測;F2 每次空合併多一次 `store.revision()` 查詢,未量測。
- **R9 最終輪**:`--force` 會把舊成品雜湊兩次(最多各 4 GiB),未量測;第三輪格式 1 標記的成品不能再用 `--force` 取代,要手動刪除(使用指南已寫)。
- **R10 最終輪**:另外 10 支 smoke 腳本(animation、combat、duet、effects、m1、m15、m2、m3、m4、street)透過 bridge 寫入前沒有檢查連接埠擁有者;`Get-NetTCPConnection` 權限受限帳號、`shader-smoke-test` 在 pwsh 7 的完整執行、`setup.ps1` 實際下載都未驗證。
- **整合**:`axiom-runtime --lib` 在一次高負載的 workspace 測試中卡住數小時,之後無法重現;同時期發現並修正了 `screenshot::order_tests::queued_captures_start_when_the_slot_frees` 的測試競態。若再出現卡住,先用 `--test-threads=1` 找出是哪個測試。
