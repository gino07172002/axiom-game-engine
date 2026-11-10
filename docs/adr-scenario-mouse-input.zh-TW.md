# ADR:Scenario 錄製滑鼠視角與螢幕指標輸入

狀態:採用(2026-11-11,editor follow-up)

## 背景

M8 的 Play 錄製只記鍵盤動作值與 runtime 指令。滑鼠視角(`WorldRuntime::pointer(dx, dy)`,FPS 的 yaw / pitch、Action 的鏡頭 yaw)與螢幕指標(`WorldRuntime::screen_pointer`,Axiom Street 的點地移動、RTS 框選 / 右鍵命令)沒有 scenario 表示法,只要錄到就標成「不精確」並拿掉 `state_hash` 斷言。

兩種事件都在幀與幀之間、兩個固定步之間**立即**作用在系統狀態上:

- 視角是非線性的(pitch 夾限、`rem_euclid`、浮點累加順序),把一個 tick 內的 delta 加總後再套用,結果不一定相同。
- 指標射線由「當下」相機狀態、遊戲區域的正規化座標與長寬比算出。

## 決定

1. 新增兩個加法的 scenario 操作,和指令放在同一個 `commands` 清單,用 tick 與清單順序保留它們和指令 / 存讀檔的先後:
   - `{"op":"look","deltas":[[dx,dy],...]}`:原始裝置單位的 delta,依序各呼叫一次 `pointer`(**不加總**)。同一 tick 連續的 delta 共用一個操作,每個最多 256 個。
   - `{"op":"pointer","kind":"primary_down|primary_up|move|secondary_down","screen":[x,y],"aspect":a,"additive":b}`:記**正規化螢幕座標 + 長寬比**,重播時由當下相機狀態重新推導射線(與即時路徑同一個函式),不記射線本身。理由:射線是相機狀態的確定性函式,重播到同一 tick 時相機狀態相同,射線就相同;記座標也讓檔案和視窗大小無關、可讀、可手改。
2. `scenario` 允許 tick 等於 `ticks` 的操作:在最後一步之後、最終斷言之前執行。錄製結束前最後一步之後才發生的視角 / 點擊 / 指令因此不再讓錄製變得不精確(先前是丟掉並標不精確)。舊檔不受影響(以前這種 tick 被拒絕)。
3. 只錄會被系統使用的指標事件(`RuntimeSystem::uses_screen_pointer`);`move` 只在主鍵按住(拖曳)時記錄,同一 tick 內連續的 `move` 只留最後一個。這是 `PointerKind` 的契約:沒有拖曳時的 move 不得改變模擬狀態,一串 move 等於「游標現在在這裡」(RTS 只更新拖曳框終點,離開地面也更新;導航只看 `primary_down`)。
4. 預算:沿用 4096 個操作與 1 MiB 檔案上限。錄製器把輸入變化、指令(實際序列化長度)、存讀檔與每個 delta / 指標操作(位元組上限:f32 最多 17 字元)都計入同一個位元組預算,在檔案可能超過上限前停止錄製(標不精確,`stopped` 說明原因),而不是寫出 `axiom scenario` 讀不了的檔。持續轉視角時每幀一個 `look` 操作:60 FPS 約 68 秒到 4096 個操作上限;1 kHz 滑鼠時位元組預算約 20–40 秒先到。
5. 即時路徑的指標事件失敗(系統回傳錯誤)時,狀態可能只改了一半,重播會在錯誤處停下:這種錄製標成不精確。

## 後果

- 錄製含滑鼠視角、點地移動、RTS 框選與右鍵命令的 Play,`axiom scenario` 重播得到相同 `state_hash`(測試:`app::tests::a_recording_with_*`,並驗證拿掉這些操作後雜湊不同)。
- 仍不精確的情況:錄製預算用盡、讀取錄製開始前的存檔、錄製中 revision 改變、指標事件失敗。
- 新系統若實作 `screen_pointer` 且 `move` 不是 last-wins,錄製會失去精確度;契約寫在 `PointerKind` 的文件上。
- `state_hash` 不再包含 `nav.grid.build_ms` 與 `nav.grid.cached`(建格的實際耗時與程序內快取命中,不是模擬狀態):有導航格的世界(Axiom Street)先前即使沒有滑鼠輸入,編輯器 Play 與新的 `axiom scenario` 程序的雜湊也永遠不同。有導航格的世界雜湊值因此改變;倉庫裡沒有帶 `state_hash` 斷言的檔案。
