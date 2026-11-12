# 打包發佈計畫(Axiom Packaging)

建立:2026-11-12。依使用者排程「先做 1,2,3 跟編輯器外觀,然後再做 4,跨平台跟大場景之後再說」,本計畫是第 4 項。執行協定見文末;每個 milestone 用 `rigorous-change` 做、`rigorous-verify` 驗收。

## Phase 0 — 需求與成功樣貌

**需求**:把一個 content package(例如 `content/plaza`)變成一個能交給別人的 Windows 資料夾:雙擊執行檔就能玩,不需要 repo、Rust、cargo、PowerShell 或 `data/`;存檔寫在使用者可寫的位置;開發者用一個指令(CLI)或編輯器選單產生它,兩者結果相同。

**最終驗收 demo 腳本**
1. `target\release\axiom.exe package --pack content\plaza --out D:\dist\plaza`,輸出一份 JSON 報告:檔案清單(相對路徑、大小、SHA-256)、總大小、警告。
2. 把 `D:\dist\plaza` 整個複製到另一個路徑(例如 `D:\elsewhere\Sakura Plaza`),在檔案總管雙擊 `Sakura Plaza.exe`。
3. 視窗出現:標題為作品名、工作列圖示為 Axiom 圖示、沒有黑色主控台視窗、沒有開發工具列(CONSOLE/RESTART/世界資訊列)、沒有開著的 loopback 連接埠。
4. 遊玩、存檔,關掉再開,讀檔成功;存檔位於 `%APPDATA%\Axiom\<package id>\`,遊戲資料夾內沒有寫入任何檔案。
5. 在編輯器開 plaza,選 `File > Build Game…`,狀態列顯示進度且可取消,完成後輸出與步驟 1 的檔案清單雜湊相同。
6. 對一個引用 pack 外部素材(`external: true`)的 pack 執行 package:可恢復的錯誤,列出外部檔案,不產生半成品。

**約束**
- Windows x64 單平台(跨平台延後)。MX150 筆電、`-j1` release 建置。
- 不加 Bevy;不加需要網路下載工具鏈的步驟。圖示用 `.tools/mingw64/bin/windres.exe`(已確認存在)。
- 主執行緒 16 ms 預算:打包是長工作 → 背景 job,可取消、有進度、有 watchdog、記錄耗時。
- 不碰使用者的 `data/`;測試用隔離 DB 與 `D:\` 下的暫存輸出,做完刪除。
- 不 push 含第三方素材的 pack 或輸出。

**Non-goals**:安裝程式(MSI/NSIS)、程式碼簽章、自動更新、Steam、跨平台、素材壓縮/加密/打包成單一封存檔、執行檔瘦身以外的大小優化、zip 由引擎產生(smoke 腳本用 PowerShell 壓縮即可)、多 pack 選單啟動器。

**未知與假設**(最保守解讀)
- 「打包」指可攜資料夾,不是安裝程式。
- 目標機器有支援 wgpu(DX12/Vulkan)的 GPU;不打包驅動或 runtime。

## Phase 1 — 勘查與風險引爆

### 勘查結果(實際讀 code / 實際執行)
- 執行檔只有一個:`crates/axiom-cli`(package 名 `axiom`)。`play` 經 `initial(--pack)` 載入 pack,`replace_world` 時每次啟動重新匯入。預設 DB `data/world.sqlite`,存檔在 `db.parent()/saves`(`app.rs:696`)。
- `ContentPackage::load`(`axiom-core/src/package.rs`)與 package resolver(animation、shader、UI 影像)在匯入時把素材路徑**改寫成絕對路徑**並存進 DB。
- 動畫素材支援 `external: true`:路徑可以在 pack 外(`axiom-animation/src/authoring.rs:439`)。
- shader resolver 讀 `.wgsl` 檔(路徑限制在 pack 內)。
- pack 目錄含不該出貨的建置原料:`content/plaza/build/*.acl`、`content/street/build/`。
- 沒有 `windows_subsystem`,執行檔會帶主控台視窗;沒有嵌入圖示(只有視窗執行期圖示 `branding.rs`)。
- `.tools/mingw64/bin` 有 `windres.exe`、`strip.exe`。

### 風險與 spike
| # | 風險 | 若不成立 | 結論 / 證據 |
|---|---|---|---|
| S1 | 搬到 repo 外能跑嗎?(寫死的相對路徑、絕對路徑 DB) | 致命 | **成立**。把 debug `axiom.exe` 與 `content/plaza` 複製到 `D:\axiom-pkg-spike`,`--db save\w.sqlite --pack content\plaza play --start --smoke-frames 120 --screenshot` 從該目錄執行:截圖正常(角色、場景),10.4 s 含啟動。DB 內 0 個 `rust_learn` 字串,GLB 路徑全是 `D:\axiom-pkg-spike\...`。⇒ 每次啟動重新匯入 pack 就能搬家;**出貨 DB 不行**(路徑寫死)。 |
| S2 | 每次重新匯入會不會破壞讀檔? | 致命 | 未驗證 → 列入 P1 驗收(存檔 → 重開 → 讀檔)。 |
| S3 | 圖示嵌入不需新工具鏈 | 繞得過 | `windres.exe` 存在;PNG 可直接包進 ICO 容器。P2 實作時確認。 |
| S4 | 玩家模式隱藏開發工具列 / 關閉連接埠 | 繞得過 | spike 截圖可見工具列 `AXIOM / WORLD IR v2 … CONSOLE [TAB] EXIT`;`--port 0` 已可關閉 loopback。P2 處理工具列。 |
| S5 | 外部素材 / 第三方素材混入輸出 | 繞得過但嚴重 | 打包器偵測 `external` 與越界路徑 → 可恢復錯誤(P3)。 |

## Phase 2 — 方案設計

**決策 #1:出貨的內容形式**
- A:出貨 SQLite DB。B:出貨 pack 目錄,玩家執行檔每次啟動重新匯入到 `%APPDATA%` 的 DB。C:烘成單一二進位世界檔。
- 比較:A 被 S1 否決(絕對路徑)。C 需要改所有讀絕對路徑的系統,是難逆的格式決策。B 重用既有載入器、SHA-256 驗證與 budget,啟動多一次匯入(plaza 實測整體 10 s 內,含 debug 建置與 120 幀)。
- **決定 B**。後果:啟動時間隨 pack 大小增加;若大場景時代需要,再以 C 取代(屆時重審)。

**決策 #2:玩家執行檔**
- A:同一個 `axiom.exe` 加 `.cmd` 啟動器。B:新 bin `axiom-player`(同 workspace,重用 `axiom_runtime::run`),`windows_subsystem = "windows"`、嵌入圖示、啟動時讀旁邊的 `game.json`。
- 比較:A 有主控台視窗、開發指令全開、需要 cmd;B 多一個小 crate,但符合 demo 第 3 步。
- **決定 B**,輸出時重新命名為 `<作品名>.exe`。開發工具列與 loopback 以 `RunMode::Play` 的新欄位 `player: bool` 控制(不新開 genre 分支)。錯誤寫 `%APPDATA%\Axiom\<id>\crash.log` 並以訊息框告知(沒有主控台)。

**決策 #3:要複製哪些檔案**
- A:整個 pack 目錄減去排除規則(`build/`、`*.acl`、`*.blend`、`*.py`…)。B:精確引用閉包(manifest.assets ∪ 解析後世界中的 pack 內路徑 ∪ resolver 讀過的檔案)。
- 比較:B 精確但要改每個 resolver 回報讀檔,漏一個就在玩家機器上壞;A 可能多帶檔案但不會少。真正的安全網是「搬家後實際載入」的驗證。
- **決定**:P1 用 A + 搬家驗證;P3 加上 B 的「引用集合」只作報告(列出未引用檔案、可用 `--exclude`),不自動刪檔。

**決策 #4:存檔位置**:`%APPDATA%\Axiom\<package id>\`(DB + saves + crash.log)。遊戲資料夾視為唯讀(可放在 Program Files)。

**決策 #5:編輯器整合**:`File > Build Game…` 啟動背景 job,呼叫與 CLI 相同的 `package` 函式(放在 `axiom-runtime` 或新模組 `packaging`,CLI 與編輯器共用);不是內容編輯,不寫 revision,不走 ACL。

**接口(紙上推演 demo)**
```
axiom package --pack DIR --out DIR [--title T] [--exe-name N] [--exclude GLOB]... [--force]
→ {"ok":true,"id","title","files":[{"path","bytes","sha256"}],"bytes","warnings":[...],"ms"}
輸出:
  <N>.exe            axiom-player(release)
  game.json          {"format":1,"id","title","pack":"content","engine":"<version>","window":{...}}
  content/...        pack 檔案(排除規則後)
  README.txt         作品名、存檔位置
```
demo 第 1 步 → package;第 2–4 步 → player 讀 `game.json` → `%APPDATA%` DB → 匯入 `content` → `run(Play{player:true})`;第 5 步 → 同一函式在 job 執行;第 6 步 → 匯入前掃描 external。每步資料都拿得到。

## Phase 3 — 增量拆解

**P1 walking skeleton:CLI package + 搬家可玩**
- 範圍:`axiom package`(複製 pack 目錄 + 排除規則 + 複製目前的 `axiom.exe` + `game.json` + 產生 `Play.cmd`);輸出前以 `ContentPackage::load` 驗證;報告 JSON。
- 驗收:package plaza → 複製到另一路徑 → 從該路徑以 smoke 截圖執行,截圖正常;世界中沒有輸出目錄以外的路徑;存檔、重開、讀檔成功(S2)。
- 風險:讀檔與重新匯入不相容。

**P2 玩家執行檔**
- 範圍:`crates/axiom-player`、`windows_subsystem`、windres 圖示、`game.json` 讀取、`%APPDATA%` DB/saves、player 模式隱藏開發工具列與 Console、預設不開連接埠、crash.log + 訊息框;package 改為複製 player 並重新命名。
- 驗收:雙擊執行沒有主控台;截圖無開發工具列;`netstat` 無監聽埠;存檔在 `%APPDATA%`;遊戲資料夾在執行後沒有新檔案;壞掉的 `game.json` → 訊息框 + crash.log。
- 風險:在無主控台下 smoke 截圖的自動化(player 也要支援 `--smoke-frames --screenshot`)。

**P3 檔案閉包、外部素材防護、報告**
- 範圍:external / 越界素材 → 可恢復錯誤;引用集合報告(未引用檔案警告);`--exclude`;輸出決定性(相同輸入 → 相同檔案雜湊清單);寫入用暫存目錄再改名(失敗不留半成品);`--force` 才覆蓋既有輸出。
- 驗收:同一 pack 打包兩次清單相同;external pack → 錯誤且輸出目錄不存在;中途失敗不留檔。

**P4 編輯器 Build Game**
- 範圍:`File > Build Game…` 對話框(輸出路徑、作品名)、背景 job(進度 = 已複製檔案數/總數、可取消、watchdog、耗時記錄)、狀態列與 `observe` 顯示 job、完成後「開啟資料夾」。
- 驗收:編輯器截圖(進行中與完成);取消後無半成品;輸出清單雜湊與 CLI 相同;主執行緒單幀不超過 16 ms(以 job 計時紀錄佐證)。

**P5 smoke 腳本與文件**
- 範圍:`scripts/package-smoke-test.ps1`:release 建置後打包 plaza、adv、fps、action,各自搬家執行 smoke 截圖並檢查路徑;external 負面測試;使用者手冊章節。
- 驗收:腳本 ok:true;截圖已看過;文件更新。

依賴:P1 → P2、P3(可平行)→ P4 → P5。

## Phase 4 — pre-mortem

1. **整合炸在「玩家機器上缺檔」**:resolver 讀了排除規則刪掉的檔案。對策:每個 milestone 的驗收都是「搬家後實際載入 + 截圖」,不是只看清單;P5 對 4 個不同類型 pack 做。
2. **讀檔壞掉**:重新匯入後 checkpoint 的世界雜湊對不上。對策:P1 就驗 S2;壞了在 P1 修(或改決策 #1)。
3. **相依卡死:release 建置在 MX150 上太慢**,讓每次驗收都要很久。對策:P1–P3 用 debug 執行檔驗證行為,只有 P2 的視窗屬性與 P5 用 release。
4. **範圍爆炸:「順便」做安裝程式、zip、壓縮、多 pack 啟動器**。對策:已寫入 non-goals。
5. **第三方素材外洩**:輸出資料夾被 commit 或 push。對策:輸出預設在 repo 外(`D:\dist`);若 `--out` 在 repo 內則警告;`.gitignore` 加 `dist/`。

## Phase 5 — 執行協定
1. 每個 milestone 走 `rigorous-change`,驗收走 `rigorous-verify`(快速模式,P5 完整)。
2. Milestone 結束:演示並記錄證據到本檔「執行補記」;回顧計畫,要改就寫補記(原計畫 → 變成什麼 → 為什麼);寫 WORKLOG。
3. 偏離計畫先改本檔再繼續。
4. 每批合併前跑整合 gate(fmt、clippy -D warnings、workspace 測試、editor smoke、兩個 parity),並清理工作樹、暫存輸出與 incremental。

## 執行補記

### P1 完成(2026-11-12)
- `axiom package --pack DIR --out DIR [--title] [--exe-name] [--exclude]... [--force]`;實作 `crates/axiom-runtime/src/packaging.rs`(CLI 與之後的編輯器 job 共用,`Progress` 提供計數與取消)。輸出先寫到同層 `*.partial`,複製後的 pack 以 `ContentPackage::load` 重新載入,並掃描解析後世界中所有絕對路徑都在輸出內,才改名成正式目錄。`--force` 只取代帶 `game.json` 的目錄;輸出與 pack 重疊直接拒絕。
- 偏離:原計畫用 `absolute()` 比較路徑;`canonicalize` 在 Windows 回傳 `\\?\` verbatim 形式,`starts_with` 永遠不成立 → 測試抓到「輸出在 pack 內」沒被拒絕。改為 `normalized()`(最深的既有祖先 canonicalize 後接上其餘)。
- 作品名清理:去除保留字元並合併空白(`Last Signal / 最後的訊號` → `Last Signal 最後的訊號`);啟動器加 `chcp 65001`,否則 cmd 以 OEM 碼頁讀不到中文檔名。
- 證據(debug):plaza 打包 1.5 s、5 檔 96.6 MB(執行檔 95 MB)、`build/` 被排除;複製到 `artifacts/pkg/elsewhere/Sakura Plaza` 執行 smoke 截圖正常(S1)。ADV 打包後搬家:執行中 `save live`(tick 1983)→ 關閉 → 重開(重新匯入 pack)`--load-slot live` 成功,tick 2103,`assets/station.png` 從搬家後路徑載入,截圖已看過(**S2 成立**)。存檔在 `--db` 的父目錄 `saves/`,遊戲資料夾內沒有新檔案。
- 單元/整合測試 `packaging::tests` 3 個:排除規則、名稱清理、決定性(兩次打包清單雜湊相同)/未加 --force 拒絕/--force 不刪非遊戲目錄/取消後不留輸出與 partial/輸出在 pack 內拒絕。Gate:fmt 乾淨、clippy -D warnings 乾淨、workspace 840 passed / 0 failed / 5 ignored。
- 仍未做(屬 P2/P3):主控台視窗與開發工具列仍在;external 素材沒有專門偵測(只靠路徑越界掃描,尚無負面測試)。

### P2 執行補記(2026-11-12)
- 完成:新 workspace bin `crates/axiom-player`(`#![windows_subsystem = "windows"]`,debug/release 皆無主控台)。讀執行檔旁 `game.json`(format 1;`id` 限英數與 `-_.`、`pack` 必須是遊戲資料夾內相對路徑、檔案上限 64 KiB),DB 為 `%APPDATA%\Axiom\<id>\world.sqlite`(saves 落在同層 `saves\`),`ContentPackage::load` 後以 `RunMode::Play { player: Some(title) }`、`replace_world:true`、`start_play:true`、預設 `port:0` 呼叫 `axiom_runtime::run`。自動化參數 `--smoke-frames --screenshot --db --port`。任何錯誤(含 panic)寫 `crash.log`(`%APPDATA%\Axiom\<id>\`,id 未知時為系統暫存目錄的 `axiom-player-crash.log`)並跳 `MessageBoxW`,結束碼 1;`AXIOM_PLAYER_NO_DIALOG=1` 略過訊息框。
- 圖示:`build.rs` 讀三張 branding PNG(驗證 IHDR 尺寸),手寫 ICO 標頭(PNG 直接當 entry),`windres -O coff` 編成物件檔以 `cargo:rustc-link-arg-bins` 連結;找不到 windres 只發 cargo warning。沒有新 crate:`windows` 0.58 只多 `Win32_UI_WindowsAndMessaging` feature,Cargo.lock 只新增 `axiom-player` 本身。
- 玩家外觀:`RunMode::Play` 改為 `Play { player: Option<String> }`(計畫寫 `player: bool`;改成帶作品名,因為視窗標題要用 `game.json` 的 title,而 `--title` 可與世界標題不同)。`UiState.player` 讓 `ui::draw_generic` 不畫開發工具列/compact pill,Tab 不開控制台、`Action::Apply` 被忽略;暫停時上方顯示 `PAUSED · ESC`;初始狀態列不顯示 `OK world compiled rev=N`。`axiom play` 仍是 `player: None`,外觀不變。
- `packaging.rs`:新增 `default_player_exe()`(`current_exe()` 旁的 `axiom-player.exe`,缺少時提示 `cargo build -p axiom-player`),CLI 改用它;不再產生 `Play.cmd`;P1 測試補上「沒有 Play.cmd」與 `game.json` 欄位斷言。
- 證據(debug,`artifacts/pkg-p2/`,驗完已刪):
  - `axiom package --pack content\adv` → 4 檔(3 個 content + `Last Signal 最後的訊號.exe`,大小等於 `axiom-player.exe`),複製到 `elsewhere\Last Signal Copy` 後以 `--smoke-frames 300 --screenshot --db <隔離>` 執行:exit 0,截圖無開發工具列,ADV 介面(語言切換、SAVE/LOAD、對話、選項、進度條)完整;同 pack 的 `axiom play` 截圖仍有 `AXIOM / WORLD IR v2 … CONSOLE [TAB] EXIT`。
  - 監聽埠:執行中(已確認行程存活)`Get-NetTCPConnection -State Listen -OwningProcess` 為 0 筆;對照組 `--port 4299` 列出 4299,證明查詢有效。
  - `APPDATA` 指向 artifacts 下隔離目錄、不帶 `--db`:`Axiom\last-signal\world.sqlite` 建立;經 `--port` 執行 `save quick`(`tick 860`)後出現 `saves\…\slot-quick.json`;遊戲資料夾前後檔案清單相同(0 個新檔)。
  - 視窗標題 = `Last Signal / 最後的訊號`(`MainWindowTitle`)。
  - 壞掉的 `game.json`(截斷 JSON)→ exit 1,`axiom-player-crash.log` 寫在 `%TMP%`(Windows 暫存目錄先看 TMP 再看 TEMP);合法 game.json 但 pack 不存在 → `%APPDATA%\Axiom\broken-game\crash.log`;不設 NO_DIALOG 時行程停在標題為 `The game could not start` 的訊息框。
  - `objdump -p`:player `Subsystem 00000002 (Windows GUI)`(CLI 仍為 3 CUI);`.rsrc` 0x186d0 bytes(三張 PNG 合計 99,056);`ExtractAssociatedIcon` 取出的圖為 Axiom 圖示,CLI 為 Windows 預設圖示。
- Gate:`cargo fmt --all` 乾淨、`cargo clippy --workspace --all-targets -- -D warnings` 乾淨、`cargo test --workspace --no-fail-fast` 842 passed / 0 failed / 5 ignored(基準 840 + player 新增 2)、`scripts/editor-smoke-test.ps1` ok:true。
- 未做/限制:沒有 release 建置驗證(視窗屬性與圖示在 debug 已確認);ADV 敘事列左側的 `AXIOM / NARRATIVE` 字樣屬共用 UI,未在玩家模式隱藏;存讀檔按鈕的回應仍以原始 JSON 顯示在狀態列(玩家看得到);Esc 暫停提示未截圖驗證。
