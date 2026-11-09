# 效能基準(回歸參考)

> 2026-11-09 建立。之後任何效能或記憶體宣稱,都以本檔的方法重量、並與本檔數字對照(AGENTS.md「Responsiveness and performance rules」)。

## 機器與建置

- Intel Core i5-8265U(4C/8T,筆電,插電),24 GB RAM,NVIDIA GeForce MX150(Vulkan)+ Intel UHD 620,Windows 11。
- 本機噪音大(其他 GPU 負載、降頻):每個 A/B 都**交錯**執行(A、B、A、B…),報中位數與範圍,單次數字不可比。
- 下表除特別註明外皆為 **release**(`lto=thin`、`codegen-units=1`)。debug 版慢 5–20 倍,只拿來除錯。

## 方法(一鍵重跑)

| 工具 | 量什麼 | 指令 |
|---|---|---|
| `crates/axiom-runtime/examples/perf_bench.rs` | 單一程序、無 GPU:固定步長模擬(120 Hz)每步耗時 p50/p95/p99/max;每步與每次 `WorldRuntime::render_frame` 的配置次數/位元組(**只在此 binary 裝計數 global allocator**);觀測組裝;`RuntimeState`/`World` 複製成本;編輯→生效(走編輯器真實寫入路徑 `editor::Action::Write → Store::execute → App::reload_after`,檔案型 SQLite,隔離 DB,第一筆暖機捨棄);工作集 | `cargo build --release -p axiom-runtime --example perf_bench -j1` 後 `powershell -File scripts/perf-bench.ps1 -Repeats 3` |
| `scripts/perf-bench.ps1` / `scripts/perf-summary.ps1` | 每包一個新程序、跨包交錯重複;摘要取各重複中位數與 [最小..最大] | `powershell -File scripts/perf-summary.ps1 artifacts/perf-scratch/bench-*.jsonl` |
| `scripts/perf-live-edit.ps1` | 真實編輯器(隱藏視窗、GPU renderer)經 bridge 送 `set_field`,讀 `observe` → `reload`(主執行緒各階段,含 GPU shader/pipeline 準備)。`-ShaderProgram/-ShaderFile` 改為交替編輯 WGSL(模擬 undo/redo) | `powershell -File scripts/perf-live-edit.ps1 -Exe target/release/axiom.exe -Pack content/effects-tour -Entity atlas/floor -Position "0,-0.34,-25"` |
| `perf_bench --generated 4096 [--dump-pack DIR]` | 4096 實體生成世界(1 相機、1 地板、4094 方塊,1/4 旋轉);`--dump-pack` 輸出成內容包供 live 腳本用 | |
| 執行期計時 | `WorldRuntime.build_timings`/`system_build_ms`/`reconcile_timings`;App `observe` 的 `reload`(`last`、`pending`、`slow_reloads`、16 ms 預算)與 `shader_gpu`(編譯/快取程式數、pipeline 數、預算) | |

注意:
- 負載是各內容包的預設狀態、無輸入(fps 玩家站立、frontier 由 AI 對手推進生產)。`light-lab` 沒有模擬系統,步長≈0。
- 「編輯→生效」只量**同一程序內**的寫入到執行期切換;外部寫入(AI 經 bridge)另有編輯器 0.25 s 輪詢延遲,live 腳本的 client 端數字含 PowerShell 每次 TCP 連線的開銷,只能當上限看。
- commit 時間被 SQLite WAL `synchronous=FULL` 的 fsync 主導(連 22 實體的 light-lab 都約 100–120 ms);這是耐久性設定,不是 CPU 成本。

## 基準線(優化前,release,3 次交錯重複的中位數)

| 內容包 | 實體 | 步長 p50 / p95 / p99 / max (ms) | 每步配置 p50 / max | render_frame ms / 配置 / KiB | 編輯→生效 p50 / p95 (ms) | 其中 commit p50 | 主執行緒 reload p50 / p95 | 工作集 MiB |
|---|---|---|---|---|---|---|---|---|
| fps | 16 | 0.344 / 0.530 / 0.665 / 0.77 | 432 / 529 | 0.011 / 11 / 107 | 168.7 / 198.1 | 160.4 | 8.4 / 14.8 | 13.6 |
| rts-bench | 88 | 0.020 / 0.028 / 0.055 / 0.11 | 77 / 78 | 0.006 / 10 / 54 | 146.9 / 229.1 | 136.1 | 11.7 / 19.1 | 14.0 |
| rts (frontier) | 159 | 0.011 / 0.020 / 0.028 / **3.93** | 36 / **24 867** | 0.015 / 11 / 154 | 200.4 / 287.2 | 157.0 | 37.7 / 58.6 | 19.9 |
| axiom-town | 26 | 0.023 / 0.024 / 0.034 / 0.12 | 71 / 84 | **1.94** / 47 / **1 904** | 147.5 / 252.9 | 125.8 | 22.4 / 29.2 | 18.5 |
| effects-tour | 148 | 0.009 / 0.011 / 0.016 / 0.04 | 21 / 22 | **4.57** / 1 438 / **7 577** | 212.8 / 276.6 | 153.2 | 52.9 / 63.1 | 48.5 |
| light-lab | 22 | ≈0 | 0 / 1 | 0.002 / 16 / 9 | 124.4 / 256.3 | 123.1 | 1.5 / 1.7 | 10.2 |
| 生成 4096 | 4096 | 0.068 / 0.085 / 0.100 / 0.15 | 0 / 1 | 0.150 / 14 / 1 024 | 430.7 / 679.2 | **347.6** | **76.4 / 103.3** | 54.2 |

主執行緒 reload 拆解(基準線,p50 ms):

| 內容包 | 讀 SQLite+解碼 | 複製給編輯器 | 編輯器 update | IR 編譯 | shader 函式庫驗證 | 系統工廠 | 狀態移轉 | 其他(舊執行期釋放) |
|---|---|---|---|---|---|---|---|---|
| axiom-town | 0.61 | 0.19 | 0.27 | 1.55 | 0 | **17.96**(GLB 匯入 ~15) | 0.05 | ~1.8 |
| effects-tour | 1.38 | 0.41 | 0.31 | 9.02 | 6.66 | **28.9**(程序幾何 ~29) | 0.06 | ~5 |
| rts (frontier) | 2.85 | 0.83 | 0.83 | 5.83 | 0 | **24.9**(每個 prefab 各編譯一次整個世界) | 0.08 | ~2 |
| 生成 4096 | **17.1** | **7.7** | 3.4 | **29.9** | 0 | 0.56 | 1.46 | ~14 |

真實編輯器(GPU,外部 `set_field`,2 輪 × 10 次,release):主執行緒 reload p50 / p95 — effects-tour 78.7 / 107.4 ms、axiom-town 33.7 / 49.1 ms、生成 4096 142.5 / 264.5 ms(其中編輯器 diff 摘要約 56 ms、讀取解碼約 21 ms)。位置編輯的 GPU 準備 ≤0.1 ms(既有程式依 hook 比對重用)。WGSL 交替編輯(effects-tour `wire`,16 次):GPU 準備 p50 20.8 ms(max 32.7)、主執行緒 p50 154 ms;每個程式 16 條 pipeline(cube/mesh × 4 混合 × 2 剔除),effects-tour 12 個程式 = 192 條。

其他量測(基準線,判定為**不熱**,未改):
- 每幀觀測組裝 `observation()` ≤0.06 ms、≤403 次配置;序列化 ≤0.01 ms。
- Renderer 每幀 `snapshot.instances.clone()`:4096 世界 0.027 ms(4095 個 instance),其他包 ≤0.007 ms。
- 編輯器每幀複製可見性向量:長度 = 實體數的 `Vec<bool>`(≤4096 bytes,單次 memcpy),遠低於量測噪音。
- 情境(scenario)在編輯器主執行緒同步執行:frontier 除錯版約 0.8 ms/tick,上限 1 000 000 tick ⇒ 最壞凍結約 13 分鐘、無進度、無取消。

## 目標與結果

目標(任務設定):編輯器不被情境卡住;axiom-town / effects-tour 的編輯→生效不比基準差;4096 實體世界每次編輯的主執行緒阻塞 ≤16 ms(release)。

### 總結(release)

| 指標 | 基準 | 現在 | 判定 |
|---|---|---|---|
| 情境執行時編輯器主執行緒 | 同步跑完全部 tick(最多 1 000 000) | 背景 job;測試在 1 000 000 tick 情境執行中繪製編輯器,最慢一幀 8.7 ms(debug),可取消、watchdog 20 分鐘、結果回填 | 達成 |
| 4096 世界,編輯器本地編輯的 reload 主執行緒(無 GPU,p50 / p95) | 76.4 / 103.3 ms | **2.2 / 3.2 ms** | 達成(≤16) |
| 4096 世界,外部(bridge)編輯的 reload 主執行緒(GPU 編輯器,p50 / p95) | 134.7 / 147.0 ms | **3.3 / 27.2 ms** | p50 達成;p95 為偶發的跨連線快取未命中(主執行緒解碼 17–38 ms) |
| 4096 世界,編輯器本地編輯的 **commit**(主執行緒) | 347.6 ms | 328.6 ms(未改) | **未達成**:fsync + 兩次編譯 + JSON 往返,見「剩餘熱點」 |
| axiom-town / effects-tour 編輯→生效(無 GPU,p50;交錯 A/B 的基準組 vs 最終組) | 138.9 / 181.2 ms | 134.8 / 170.5 ms | 不比基準差(commit 主導,±10 ms 噪音) |
| axiom-town / effects-tour 外部編輯→生效(GPU 編輯器,client 端) | 見下表 | 見下表 | 見說明 |

GPU 編輯器(release,外部 `set_field` 經 bridge,最終版 vs 基準交錯 2 輪 × 10 次,p50 / p95 ms):

| 內容包 | 主執行緒 reload 基準 → 現在 | client 端編輯→生效 基準 → 現在 | bridge commit 基準 → 現在 |
|---|---|---|---|
| effects-tour | 73.0 / 94.2 → **0.8 / 1.9** | 810 / 1 082 → 763 / 1 048 | 176 / 340 → 185 / 303 |
| axiom-town | 27.7 / 33.9 → **0.7 / 1.9** | 755 / 1 228 → 781 / 1 267 | 159 / 347 → 166 / 357 |
| 生成 4096 | 134.7 / 147.0 → **3.3 / 27.2** | 801 / 1 574 → 989 / 1 837 | 392 / 589 → 404 / 655 |

- 主執行緒 reload 不含第 12 項「本幀剩餘預算內等待」(每幀最多 12 ms 減去本幀已用時間,不超出 16 ms 預算)。
- client 端數字含編輯器 0.25 s 輪詢、PowerShell 每次 TCP 連線、以及本機每幀 200–475 ms 的 swapchain 阻塞,四次獨立 A/B(第 5 項、第 10 項、第 12 項的兩個步驟後各一次)的 p50 差在 −250…+250 ms 之間來回,屬噪音範圍;無 GPU 的同程序量測(上表第 5 列)顯示編輯→生效不變。4096 的 client p50 在最後一輪較基準慢一幀左右(建置約 30 ms > 本幀剩餘預算,於幀尾才交換)。

### 每項改動(交錯 A/B,release,3 次重複中位數 [範圍])

| # | 改動 | 量測點 | 前 → 後 | 保留 |
|---|---|---|---|---|
| 1 | Store 解碼修訂快取(`Store::current_shared`,commit 後直接保留新 `Arc<World>`)+ 編輯器改持 `Arc<World>`、`WorldRuntime::new` 改收 `Borrow<World>` | reload 讀取 / 複製(4096) | 17.4 → 0.016 ms;8.1 → 0 ms | 是 |
| 2 | 動畫資產快取(依路徑、選項、檔案位元組與資料夾內檔案大小/時間雜湊) | axiom-town 系統工廠 | 17.1 → 4.7 ms | 是 |
| 3 | 程序幾何快取(依參數文字) | effects-tour 系統工廠 | 28.1 → 1.4 ms | 是 |
| 4 | RTS 範本一次編譯(原為每個 prefab 各編譯整個世界;失敗時退回逐一編譯以保留原錯誤) | frontier 系統工廠 | 19.5 → 9.3 ms | 是(測試比對兩種結果逐欄相同) |
| 1–4 合計 | | 主執行緒 reload p50:town / effects / frontier / 4096 | 21.4 / 51.0 / 31.0 / 77.5 → 7.9 / 18.5 / 18.1 / 49.5 ms | |
| 5 | 背景啟用:新修訂的執行期在 worker 建好,主執行緒只做狀態移轉+GPU 準備+交換(`WorldRuntime::adopt`);被取代的執行期在背景釋放;執行期請求前先完成啟用(協定語意不變);Play 預覽時捨棄背景建置 | 主執行緒 reload p50:town / effects / frontier / 4096 | 7.6 / 18.2 / 15.6 / 42.2 → 0.67 / 0.96 / 1.37 / 9.7 ms | 是 |
| 6 | 生產單一單位時不複製整個 `RuntimeState`(單次 spawn 本身已是先驗證後寫入) | frontier 步長 max / 每步最大配置 | 3.87 ms / 24 867 → 0.14 ms / 678 | 是(frontier 腳本勝利測試與 m2 smoke 行為一致) |
| 7 | 沒有任何系統消費待送事件時,不做交易性狀態複製(`receive_events` 空集合本為 no-op) | 同上(與 6 同一 build,合併觀察) | — | 是 |
| 8 | 靜態網格頂點以 `Arc` 共享;renderer 遇到同一份頂點不再轉換/上傳 | effects-tour `render_frame` | 3.39 ms、7 577 KiB/幀 → 0.23 ms、931 KiB/幀 | 是 |
| 9 | shader pipeline 依 WGSL 內容快取並跨修訂保留(LRU,上限 512 條 pipeline,超出回報 `shader_variant_budget`) | effects-tour 交替編輯 `wire` 的 GPU 準備 p50 / max | 20.8 / 32.7 → 0.1 / 0.3 ms | 是 |
| 10 | 外部修訂:bridge 與主程式共享解碼快取;編輯器 Changes 清單的 diff 與舊世界釋放移到背景 | 4096 外部編輯主執行緒 p50(GPU) | 78.3(第 5 項後)→ 4.9 ms | 是 |
| 11 | 本地編輯時舊編輯器世界於背景釋放 | 4096 reload 主執行緒 p50(無 GPU) | 9.5 → 2.2 ms | 是 |
| 12 | 背景建置在本幀剩餘預算內(≤12 ms)等待,並在幀尾再輪詢一次 | GPU 編輯器外部編輯→生效 | 見上方 GPU 表 | 是 |

被量測、判定不熱而**未改**:每幀觀測組裝(≤0.06 ms)、renderer `instances.clone()`(≤0.03 ms)、編輯器可見性向量複製(≤4 KiB memcpy)。

沒有改動被回退:每項都在對應的計時階段有超過噪音的改善;唯一的取捨是第 5 項(見下)。

### 第 5 項的取捨(誠實說明)

背景啟用讓「新執行期上線」至少晚一幀才發生(交換只在幀之間)。在 60 Hz 的機器上是 ≤16 ms;在這台機器上,每幀的 swapchain 取得(`get_current_texture`)會阻塞 200–475 ms(基準與新版都有,見剩餘熱點),一幀就是數百毫秒,所以 GPU 編輯器的 client 端編輯→生效在 effects-tour 曾比基準慢約一幀(加入第 12 項之前:p50 884 vs 621 ms)。第 12 項讓小型修訂在同一幀上線、大型修訂在同一幀的幀尾上線來縮小此差距。

## 剩餘熱點(依量測排序)與下一步

1. **(已處理:2026-11-09 改為背景寫入器,主執行緒 p95 ≤2.6 ms,見文末;下列為當時紀錄)本地編輯的 commit 在主執行緒**(4096:~330 ms;小世界:~110–160 ms)。組成(4096,debug 探針比例):fsync(WAL `synchronous=FULL`,約 110 ms 底線)、兩次 IR 編譯(新+舊,約 1/3)、`Target::edit` 的整個世界 JSON 往返、diff 摘要、序列化。要達 16 ms 只能:(a) 編輯器寫入改為非同步(worker 自有連線,草稿在結果回來前鎖定),或 (b) 由擁有者決定是否放寬耐久性(`synchronous=NORMAL`);(c) 可另外快取最新修訂的 `CompiledWorld` 省掉一次編譯。未做:超出本次範圍且 (b) 是需求變更。
2. **swapchain 取得阻塞**:本機(MX150/Optimus,Vulkan,AutoVsync)`Surface::get_current_texture` 每幀阻塞 200–475 ms,隱藏或可見視窗、基準或新版皆同(rts-bench 300 幀:基準 60.2 s、新版 59.5 s)。新的 `observe` → `frame.last_slow.phases_ms.render_present` 可直接看到。這也讓 `scripts/m2-smoke-test.ps1` 的 900 幀 GPU 段超過 60 s 逾時。下一步:試 `desired_maximum_frame_latency`、Mailbox/Immediate、DX12 後端或內顯,逐一量測。
3. **外部修訂偶發快取未命中**:bridge commit 與寫入共享快取之間的極短空窗被主執行緒輪詢撞到時,主執行緒仍會解碼一次(4096:17–38 ms,約 15% 樣本)。
4. **CPU 蒙皮每幀配置**:axiom-town `render_frame` 1.5 ms、1.9 MB/幀(`skin::deform` 產生新頂點);可改為重用緩衝或 GPU 蒙皮。
5. 新 shader 內容第一次出現時仍在主執行緒編譯 16 條 pipeline(約 20 ms/程式);可改為背景建立或延遲建立變體。

## 2026-11-09:編輯器 commit 移到背景寫入器(剩餘熱點第 1 項)

決策(擁有者):選 (a) 編輯器寫入非同步;**耐久性不變**(`synchronous` 維持預設 FULL,未改任何 PRAGMA)。(c) 快取已編譯世界:目標已達成,依 rigorous-perf「達標即停」未做(見下方「編輯→生效」)。

### 設計(`crates/axiom-runtime/src/writer.rs`、`app.rs`)

- **寫入器執行緒**:`axiom-editor-writer`,自己的 SQLite 連線(與主連線/bridge 共享已解碼修訂快取),依序執行 `Store::execute`。編輯器所有 commit 都走它:`Action::Write`(Inspector 套用、gizmo 拖曳、階層操作、時間軸、所有 `ops::*` 編輯器路徑、Preview)、Undo/Redo、History 還原、提案核准/拒絕。bridge/AI 與 CLI 仍用自己的同步 Store,協定輸出不變。
- **佇列**:主執行緒 FIFO,上限 `MAX_PENDING = 8`,同時只有一筆在寫入器上;下一筆在前一筆結果回來後才送出,CAS `expected_rev` 在送出時解析為前一筆產生的修訂(外部寫入夾在中間仍會衝突)。
- **樂觀世界**:每筆 apply 在 worker job(`jobs::Job`,watchdog 30 s)上做「複製 + 套用 + 編譯驗證」,完成後編輯器改顯示這個世界、後續編輯建立在它之上;`editor.rev` 維持最後採用的修訂,佇列清空才採用 commit 結果(`Editor::commit_landed`:保留 undo、編輯中的草稿與拖曳)。驗證失敗時草稿保持原樣(和舊版一樣可修正再套用)。
- **何時拒絕(附原因)**:佇列滿;前一筆仍在準備樂觀世界(4096 世界約 75 ms,小世界 2–15 ms);佇列裡有 undo/還原/提案(其結果世界要 commit 後才知道)。Play 在有待存寫入時拒絕。原因寫進狀態列與 Problems(`editor_saving`)。bridge `editor_view` 遇到這些情況改到下一幀處理(順序不變),回覆在該筆 commit 完成(或失敗)後才送出,語意同舊版。
- **失敗/衝突**:該筆的錯誤(例如 `revision_conflict`)進 Problems;排在後面的寫入全部丟棄並列出(`write_discarded`),編輯器回到資料庫內容(含先落地的外部修訂),絕不覆蓋。
- **watchdog**:單筆 commit 超過 30 s → SQLite interrupt(交易回滾);再 5 s 不回應 → 寫入器脫離、未開始的請求一律拒絕、回報 `writer_stalled`、改回主執行緒同步寫入。
- **關閉**:`run()` 結束時 `close_writes`(上限 30 s)送完佇列並以剩餘時間 join;逾時或 join 失敗 → stderr + 程序錯誤結束,訊息列出未確認筆數。
- **可觀測**:`observe` → `editor.pending_commits`、`editor.last_commit_ms`、`editor.writes{pending, in_flight{what, elapsed_ms, interrupted}, committed, failed, discarded, last_commit_rev, recent_ms[16]}`;狀態列 `saving N…`。幀剖析器現在從幀開頭計時,第一段 `writes_bridge` 含寫入結果與 bridge 請求(以前這段不在 `frame.last_slow` 裡)。
- 寫入排隊中時不做 0.25 s 外部輪詢 reload(避免和寫入器快取寫入競速而在主執行緒解碼);外部修訂會讓下一筆寫入衝突、佇列清空時被採用。

### 量測(release,`perf_bench` 編輯器路徑,5 輪交錯 HEAD 1bf44ac ↔ 本版,每輪每包 20 次編輯捨棄第 1 次;中位數 [各輪範圍])

「編輯器主執行緒/commit」:本版 = 送出 + 顯示樂觀世界 + 處理結果三段主執行緒時間總和(`edit_live.editor_main_ms`);HEAD = 整個同步 `editor_action`(commit + reload 請求,p95 為兩者 p95 相加,上限估計)。

| 內容包 | 編輯器主執行緒/commit p50 HEAD → 本版 | p95 HEAD → 本版 | 單段最大 p95(本版) | 編輯→生效 p50 HEAD → 本版 | 編輯→生效 p95 HEAD → 本版 | 寫入器 commit p50(本版) | 樂觀世界就緒 p50 |
|---|---|---|---|---|---|---|---|
| 生成 4096 | 313.4 [302..319] → **1.18** [0.94..1.47] | 551 [513..594] → **2.62** [1.98..3.47] | 2.39 | 347 [341..355] → **375** [367..405] | 581 → 592 | 334 | 75.0 ms |
| light-lab | 112.3 → **1.29** | 252 → **1.78** | 1.50 | 113.5 → 113.5 | 253 → 147 | 110 | 2.0 ms |
| axiom-town | 120.5 → **1.35** | 232 → **2.12** | 1.85 | 132.5 → 128.6 | 246 → 181 | 117 | 4.2 ms |
| fps | 128.1 → **1.41** | 149 → **2.07** | 1.81 | 136.8 → 136.4 | 161 → 224 | 128 | 9.6 ms |
| effects-tour | 141.0 → **1.31** | 185 → **1.99** | 1.80 | 171.1 → 169.6 | 220 → 273 | 139 | 14.5 ms |
| rts (frontier) | 147.2 → **1.58** | 339 → **2.07** | 1.87 | 168.0 → 164.0 | 358 → 354 | 141 | 14.4 ms |

- 噪音底線(HEAD 兩輪,`noise-head.jsonl`):小世界 commit p50 兩輪差最多 ~34 ms、p95 差最多 ~190 ms;4096 commit p50 差 ~9 ms。
- **目標**:編輯器主執行緒每次 commit ≤16 ms p95 → **達成**(4096:2.6 ms;小世界 ≤2.1 ms;最差單輪 3.5 ms)。剩下的主執行緒成本約 1 ms 是處理結果時啟動背景執行期建置(`reload.request_ms`)。
- **編輯→生效**:小世界在噪音內不變。**4096 p50 變慢約 28 ms(347 → 375,超出 HEAD 各輪範圍)**:寫入器 commit 本身 313 → 334 ms,推測是同時在另一核心做樂觀世界(複製 + 編譯 ~75 ms)搶 CPU(4C 筆電),未進一步拆解。若要收回,下一步是 (c)(快取最新修訂的 `CompiledWorld` 省一次編譯)或讓樂觀世界與 commit 共用一次編譯。
- 被量測但不改:無(只有一個變因)。

重跑:`powershell -File scripts/perf-bench.ps1 -Exe <perf_bench.exe> -Repeats 1 -Edits 20 -Ticks 300 -Packs ...` HEAD 與本版交錯各 5 次;`perf_bench --sync-writes` 量舊的主執行緒 commit。GPU 編輯器路徑:`scripts/perf-live-edit.ps1 -EditorCommand object.cube`(每次編輯 = `editor_view` 跑一個編輯器指令,回覆在 commit 後)。
GPU 編輯器(release,隱藏視窗,MX150/Vulkan;`perf-live-edit.ps1 -EditorCommand`,每包每版 1 輪 × 10 次,HEAD 與本版依序執行;client 端數字含 PowerShell TCP 連線與本機每幀 200–475 ms 的 present 阻塞,單輪,只當參考):

| 內容包 / 編輯 | client 編輯→生效 p50 / p95 HEAD → 本版 | `editor_view` 回覆(= commit 完成)p50 HEAD → 本版 | 寫入器 commit p50(本版) | 修訂順序 / 失敗 |
|---|---|---|---|---|
| light-lab / `object.cube` | 517 / 539 → 768 / 1040 ms | 389 → 504 ms | 146 ms | 2..12 皆 `apply` 依序;11 committed / 0 failed |
| 生成 4096 / `edit.delete` 選 `box/i` | 827 / 1040 → 1053 / 1829 ms | 555 → 766 ms | 317 ms | 2..12 依序;11 committed / 0 failed |

- **GPU 路徑的編輯→生效多約一幀**(這台機器一幀 ≈ 250 ms):舊版在處理 bridge 請求的那一幀內同步 commit、同幀啟用;本版 commit 在 present 阻塞期間於寫入器完成,幀尾收到結果才開始背景建置,下一幀才上線。60 Hz 機器上這一幀是 ≤16 ms。可再改進:幀尾收到結果後在剩餘預算內等候建置(像第 12 項),未做。
- 經 bridge 無法造出多於 1 筆的佇列(bridge 一次處理一個連線,`editor_view` 回覆又等 commit 完成),所以「快速連續編輯依序落地」在 GPU 上驗證的是連續 10 次、每次前一次完成後立即送出;佇列深度 >1 的順序、衝突、undo、關閉 flush、watchdog 由 `app::tests` 的 7 個測試覆蓋。
- 寫入器版的 `frame.last_slow` 例:4096 一幀 19.8 ms,其中 12.1 ms 是既有的「本幀剩餘預算內等候背景建置」(第 12 項),`writes_bridge` 0.006 ms。HEAD 的幀剖析器不含 bridge 請求處理,看不到當時的 commit 卡頓(主執行緒成本以上方 headless 數字為準)。
## 3D 音訊(`audio.spatial`,plan-engine-components 第 7 項,2026-11-09)

release,`perf_bench --generated-audio N`(N 個循環聲源、4 種程序片段、聽者繞圈、聲源移動;32 voice),每組 3 次:

| N | step p50 / p95 / max | step 配置 p50 / max | `audio.spatial` 建置 | voiced / virtual / 累計 steals | 混音 512 frame(23.2 ms 聲音)p50 / max | 混音配置 |
|---|---|---|---|---|---|---|
| 256 | 0.029–0.030 / 0.032–0.038 / 0.07–0.12 ms | 0 / 1 | 10.1–10.4 ms(含 4 段片段合成,冷快取) | 32 / 224 / 81 | 0.100–0.104 / 0.23–0.45 ms | 0(2000 次中最大 0) |
| 1024 | 0.102–0.104 / 0.12–0.13 / 0.19–0.33 ms | 0 / 1 | 10.0–10.2 ms | 32 / 546 / 84 | 0.100 / 0.22–0.37 ms | 0 |

- step 時間含 `motion.integrate`(N 個移動聲源);1200 步中 max 1 次配置與先前各項目相同(無頭量測中既有的現象,未追查來源;`tests/audio.rs` 以本執行緒計數器量 200 步為 0)。
- 片段 PCM 260,190 bytes(4 段,依內容 hash 共用)。
- 原生視窗實測(debug,MX150,`editor` Play):waveOut 裝置 open,`rendered` 持續增加、`underruns` 0、`mix_us_max` 232 µs、`voices_mixed` 1。

axiom-town(不用 `audio.spatial`)A/B,release,HEAD `9094f2c` 與本版交錯 5 輪:step p50 中位數 0.0178 → 0.0180 ms(各輪 0.0176–0.0178 vs 0.0178–0.0183)、p95 0.0184 → 0.0187、配置 69/82 不變、observation 0.0346 → 0.0352 ms。本版略高約 1%(+0.2 µs),未拆解;推測與 `CompiledEntity` 多兩個欄位有關。
