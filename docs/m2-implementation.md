# M2：RTS 內容包與原生能力

v0.4 使用同一執行檔載入 FPS、ADV 與 RTS，沒有加入 Bevy。RTS 是已註冊能力與 World IR v2 內容包；authoring 交易、固定步進模擬及 RenderFrame 保持分離。

## 遊玩與資料

雙擊 `Play RTS.cmd`；初次建立 `data/rts.sqlite`，後續保留該資料庫修改。明確使用 `--pack content/rts` 會重新載入初始定義。左鍵框選、Shift 加選，右鍵地面／資源／敵人分別送出移動／採集／攻擊；WASD 平移、Q／E 縮放。HUD 可選礦工／士兵、採集、生產；Esc 暫停。

`content/rts` 初始雙方各 32 單位、各一座基地與礦點，兩種生產配方及一面有通道的牆。礦工滿載回友方 depot 交付後繼續採集；排入生產時扣款，完成後尋找空閒出生格。摧毀敵方基地獲勝，己方基地毀損則失敗。敵方以簡單原生自動警戒／戰鬥為主，沒有完整戰略 AI。

`rts.unit` 定義 faction、role、speed、radius、carry_capacity、gather_ticks 與攻擊參數；`rts.building` 定義陣營、depot 及可生產配方；`rts.resource` 定義礦量。`rts.session` 定義格網、預算、起始資源、人口上限、相機、配方及 UI metric 對應。內容中的單位只覆寫 faction，其他屬性繼承 prefab，可用 M1.5 局部編輯調整整類單位。

HUD 使用 `ui.document` 與共用型別變數綁定，按鈕送出註冊的能力命令。程式只在準備階段解析內容，步進直接讀 typed 資料與已編譯的變數 handles。

## 能力介面

```powershell
.\target\debug\axiom.exe schema rts
.\target\debug\axiom.exe schema actions
.\target\debug\axiom.exe --connect 127.0.0.1:4242 command examples/rts-select.command.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 command examples/rts-move.command.json
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /rts/summary --field /rts/selected
```

所有 native actions 都使用同一 `RuntimeCommand::Call`：

```json
{"op":"command","command":{"kind":"call","capability":"rts","action":"train","args":{"building":"base/0","recipe":"soldier","count":1}}}
```

| action | args／語意 |
|---|---|
| select | ids、可選 additive；只選存活友方 |
| select_role | role worker／soldier、可選 additive |
| select_rect | min／max [world x,z]、可選 additive；支援反向框選 |
| move | destination [x,z]、可選 units（省略用目前選取） |
| gather／attack | target 穩定 ID、可選 units |
| train | building、recipe、count 1..8；每建築佇列最多 16 |
| stop | ids；取消既有命令及路徑工作 |
| inspect | ids、fields、offset、limit；最多 128 筆，預設 metadata |

欄位型別、目標存在、陣營、職業、資源、人口與容量會檢查；不合法群組命令不留下部分修改。生產可用 execute/request_id 與 runtime_session 重試；成功 receipt 只屬於目前 host session。

## 模擬、導航及生命週期

axiom-rts 提供純模擬／schema／內容配方，axiom-runtime/rts.rs 經 generic system factory 接入 runtime。原生視窗送出經目前相機轉換的 ScreenPointer，RTS 模組自行求平面交點與下令，app 不判斷遊戲類型。

導航採四方向平面格網 A*，格網上限 128×128；預設每 4 ticks 決策一次，每次最多解決 4 個請求、每次搜尋最多展開 8192 節點，快取最多 128 條靜態路徑。每 actor 最多一個有效路徑工作，替換命令移除舊工作。群組分配不同且可通行的編隊目標，移動保留占用／目的格，阻塞時有限重排並退避；無路徑、預算耗盡與暫時阻塞分開計數。移動維持 120 Hz，不讓每個單位逐幀執行 A*。

採集、攜帶、交付、花費與戰鬥損失滿足資源守恆。已生產單位使用 `produced/faction/serial` ID，建立真正 runtime slot、generation handle、render mesh 及 typed columns。這些實體不寫回 authoring revision；restart 回初始內容。完成生產時 storage 分批準備，成功才交換，失敗恢復該步模擬。

同 world 熱更新保留單位進度、選取、經濟與生產佇列；已生產實體重新接入 storage。格網變動或刪除正在生產的配方會拒絕啟用並保留舊世界。格網修改需 restart。`max_units` 限制 resident slots，包含死亡單位，尚未回收屍體；最多 1024，範例為 512。RTS 私有狀態 checkpoint 尚未實作，沒有提供會遺失經濟／命令的假存檔。

## 驗收與量測

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --examples
cargo build --workspace
cargo build --release -p axiom -j1
.\scripts\m2-smoke-test.ps1
```

完整腳本使用隔離資料庫，預設包含 M1.5 → M1 → M0。64 單位功能 scenario 跑 7200 ticks，驗證框選 32 友方、32 全數繞牆抵達、採集交付、生產兩單位至 66；另有陣營過濾、無路徑／展開預算、命令替換、資源耗盡／守恆、攻擊勝利、真實相機射線、UI 點擊、熱更新與 storage 不變量測試。

已有同一 debug SHA256 的 M0／M1／M1.5 報告時，可用 `-SkipEarlier` 單獨補跑 M2；報告另記錄經比對的先前驗證檔與雜湊。`-BuildRelease` 可在原生畫面驗證後補建 release，再執行壓測。兩種 build 的雜湊與用途分開記錄。

原生 GPU smoke 使用隱藏視窗驗證 64 單位的即時採集／生產、128／256 單位的群組移動和渲染，留下 PNG 與 observation。`artifacts/m2-validation.json` 包含 gameplay SHA256、功能結果及 FPS／ADV 回歸是否一起執行。

`scripts/m2-stress-test.ps1` 在所有 build 結束後使用 release，每種人口各 3 個新程序；240 ticks 暖機、2400 ticks 量測（20 秒模擬時間）。雙方全體每 480 ticks 接受群組命令替換，停用戰鬥以固定人口。step 計時包含完整 WorldRuntime 更新，命令與每 120 ticks 的不變量檢查另計；檢查全部存活、人口固定與無 footprint 重疊。Windows 記憶體為整個新程序的 peak working set／private bytes，不是單一系統的配置量。

完整採樣、硬體、build profile、雜湊、分位數與記憶體見 `artifacts/m2-stress.json`。這是 headless CPU 模擬壓測，GPU 截圖是獨立功能驗證；沒有 GPU timestamp 或實際渲染 FPS 優勢宣稱。M1.5 成本 benchmark 同樣把 bytes 與真實 tokens 分開；未設定真實 agent usage 前不宣稱 token 節省。

## 本次驗證結果

105 項 workspace 測試通過，fmt 與 Clippy `-D warnings` 通過。M0／M1／M1.5／M2 gameplay 使用相同 debug SHA256；FPS 遊玩、三個 ADV 結局、中英文 UI、存讀檔、交易／熱更新與 AI 操作回歸皆通過。64 單位功能場景最終 66 單位、完成 32 次群組抵達、交付 40 礦物、生產 2 單位、花費 100。原生 128／256 單位均存活並完成群組輸入／路徑／渲染驗證，GPU 為 NVIDIA GeForce MX150／Vulkan；畫面已複檢。

2026-11-07，Windows 11、Intel Core i5-8265U（4 cores／8 logical）、rustc 1.99.1 x86_64-pc-windows-gnu，release thin LTO／codegen-units 1：

| 單位數 | 三次 step p95 中位數 | 各次 p95 範圍 | 三次最高全程序 peak working set |
|---|---:|---:|---:|
| 64 | 0.141 ms | 0.113–0.146 ms | 15.45 MiB |
| 128 | 0.314 ms | 0.276–0.322 ms | 16.90 MiB |
| 256 | 0.670 ms | 0.566–0.724 ms | 21.62 MiB |

每案暖機 240／採樣 2400 ticks，三次各為新程序；所有案例零重疊、人口固定、無模擬錯誤。表格是各次 p95 的中位數，並非合併所有樣本後重算 p95。最慢單步及 p99 保留在完整報告；壓測驗收是功能與不變量，未設立跨硬體效能門檻。

- Gameplay SHA256：`827A768EC1467C70EA0352821098707848C8A73A87F5AFCCC1F209DD47A6C38A`
- Release stress SHA256：`1EB95DCDEC140D0567B49354F57D1557EC9929FA2F92B12A300BF76D0B069E95`
- 整體驗證：`artifacts/m2-quality.json`；原生／功能：`artifacts/m2-validation.json`；量測：`artifacts/m2-stress.json`。
