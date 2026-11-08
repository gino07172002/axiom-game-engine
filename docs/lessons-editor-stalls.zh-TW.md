# 教訓:編輯器「讀取進度條」卡數小時(Unity)與 Axiom 的對策

> 2026-11-08 整理。來源:Unity Issue Tracker 與官方論壇(連結在文末),以及 Axiom 程式碼與量測紀錄的對照。

## Unity 為什麼會突然跳出進度條、一卡數小時

| 根因 | 典型觸發 | 為什麼會變成「好幾小時」 |
|---|---|---|
| **Domain reload**:改一行 C# 就重新編譯,並在主執行緒卸載、重建整個腳本網域 | 存檔、切到編輯器視窗、進出 Play | 編輯器在等某個執行緒結束,但使用者或套件的背景執行緒(socket、named pipe、`Socket.Poll`)不會配合結束 → 永久卡在「Reloading Domain」。Unity 的立場是無法安全地強制終止這些執行緒,部分案例標為不修 |
| **使用者程式在編輯器內執行** | `[InitializeOnLoad]`、`OnValidate`、編輯器腳本 | 無限迴圈或 O(n²) 的程式碼直接卡住主執行緒;進度條只顯示「Hold on」 |
| **隱性自動觸發的重活** | Auto Generate Lighting、AssetDatabase 自動 Refresh | 一個小改動觸發全場景烘焙或大量重新匯入;進度條卡在某個百分比,時間估計失準(Unity 曾標為 Won't Fix) |
| **整批快取失效** | 升級版本、切換平台(貼圖重新壓縮)、Library 損毀 | 整個專案重新匯入,時間與資產量成正比 |
| **組合爆炸** | Shader variants、keywords | 變體數呈指數成長,編譯時間不可預期 |
| **背景工作互相干擾、死結** | 烘焙時同時有其他背景工作;資產載入的死結 | 進度條閃爍或停住,只能殺掉程序 |
| **看不見卡在哪裡** | 模態進度條、沒有取消按鈕與逾時 | 使用者只能等;要翻 Editor.log 才知道卡在哪一步 |

共同模式:**主執行緒同步、沒有上限、沒有取消、隱性觸發、整批重做、不可觀測。**

## Axiom 現況對照(誠實盤點)

已經避開的部分:
- 沒有內嵌腳本、沒有 domain reload。行為以資料描述(FSM、規則、綁定),Rust 外掛在 IDE 中另外編譯。
- 每種內容都有明確上限(實體 4096、實例 8192、每批編輯 256、請求 1 MiB、相機 8 台等)。
- bridge 請求有截止時間;沒有自動烘焙、沒有自動全量匯入。

**已存在的同類問題(必須修)**:
1. **Scenarios 分頁在編輯器主執行緒同步執行情境**,`ticks` 上限 1,000,000(`scenario.rs`)。以除錯版實測約 0.8 ms/tick(frontier)推估,跑滿上限會凍結畫面約 13 分鐘,而且沒有進度與取消。這正是 Unity 式的卡死。
2. **每個新修訂都在主執行緒全量重建執行期**(`App::reload_after` → `WorldRuntime::reconcile_checked` → `Self::new(...)`,外加從 SQLite 讀出整個世界並複製一份給編輯器)。M6 量測(release 中位數):effects-tour 編譯 19.6 ms + 建立執行期 92.8 ms → 每次編輯約 0.1 秒卡頓;成本隨世界大小成長,大型世界會變成每次編輯卡好幾秒。
3. **Shader / pipeline 在主執行緒同步準備**(`Renderer::prepare_shaders` 在 reload 時呼叫)。目前規模小,但材質 × 混合 × 雙面的 pipeline 組合會隨內容成長,需要量測與上限。

## 規則(寫入 AGENTS.md,所有人與 agent 遵守)

1. **主執行緒不跑長工作**:編輯器與遊戲的主執行緒,單一操作預算 16 ms。超過的工作一律放到背景 job。
2. **背景 job 必備**:明確上限、可取消(協作式取消 token)、可量化的進度(已完成 / 總數,不用估計時間)、逾時(watchdog)、開始與結束都記錄耗時;關閉或重新載入時以逾時 join,絕不無限等待。
3. **不隱性觸發重活**:烘焙、導航網格、縮圖、大量匯入一律由明確指令觸發(人或 AI),可預覽成本;自動行為只能做增量且有上限的檢查。
4. **增量與快取**:內容變更只重建受影響的部分;衍生資料(pipeline、縮圖、烘焙、導航)以內容雜湊為鍵快取在修訂之外,精準失效,絕不整批作廢後同步重建。
5. **控制組合爆炸**:shader / pipeline 變體延遲建立、有上限、可列舉;新增維度前先量測。
6. **編輯器內不執行可能卡死的使用者程式**:行為維持資料驅動;Play 預覽的系統若單幀超時,要偵測並回報(狀態列 / observe),不能默默凍結。
7. **看得見**:進行中的 job 顯示在狀態列,並經 `observe` 暴露給 AI;超過門檻的操作記錄為「慢操作」並附耗時。
8. **用量測守住**:大型世界(例如 4096 實體)的編輯 → 生效延遲、情境執行、shader 準備,納入效能基準與 CI 門檻。

## 待辦(排入效能基準里程碑)

- 情境改為背景 job:進度(tick / 總 tick)、取消、結果回填 Scenarios 分頁;同時保留 CLI 的同步模式。
- 修訂生效改為增量:先量測各世界大小下的 reconcile 耗時,再依量測結果把編譯 / 系統重建拆成增量,或移到背景 job 並以「最後可用版本」繼續顯示。
- pipeline 變體:量測、上限、延遲建立並快取。

## 來源

- [Unity Issue Tracker:反覆進出 Play Mode 後卡在 Reloading Domain](https://issuetracker.unity3d.com/issues/editor-gets-stuck-on-the-reloading-domain-window-when-the-play-mode-is-entered-and-exited-a-few-times-in-a-short-period)
- [Unity Issue Tracker:Socket.Poll 導致第二次進 Play 卡死(標為不修)](https://issuetracker.unity3d.com/issues/editor-is-frozen-on-reloading-domain-when-entering-play-mode-for-the-second-time-using-socket-dot-poll-1-dot-dot-dot)
- [Unity Issue Tracker:未關閉的 named pipe 讓 domain reload 卡死](https://issuetracker.unity3d.com/issues/editor-gets-stuck-on-reloading-domain-when-re-entering-play-mode-after-starting-a-named-pipe-without-closing-it)
- [Unity Issue Tracker:資產載入死結,永遠停在 Completing Domain](https://issuetracker.unity3d.com/issues/getting-randomly-stuck-on-completing-domain-deadlock-in-asset-loading-code)
- [Unity Issue Tracker:Application.Reload 卸載網域時卡住](https://issuetracker.unity3d.com/issues/editor-hangs-on-application-dot-reload-when-unloading-domain)
- [Unity 論壇:Editor Progress Bar stalls](https://discussions.unity.com/t/editor-progress-bar-stalls-request-for-bug-reports/847630?page=2)
- [Unity 論壇:Lots of busy... Hold on](https://discussions.unity.com/t/lots-of-busy-hold-on-etc/777806)
- [Unity Issue Tracker:Auto Generate 時 GI 進度條凍結](https://issuetracker.unity3d.com/issues/ui-gi-progress-bar-is-frozen-until-hovered-on-when-auto-baking-ot-clearing-baked-data)
- [Unity Issue Tracker:烘焙在 GI 後處理階段凍結編輯器](https://issuetracker.unity3d.com/issues/lighting-generation-freezes-the-editor-during-the-global-illumination-post-processing-stage-when-baking-lighting-on-certain-systems)
- [Unity Issue Tracker:烘焙時間估計與進度條不準](https://issuetracker.unity.com/issues/9055/lighting-generation-background-task-has-inaccurate-time-calculations-and-progress-bar)
- 第三方整理(僅供參考,未經 Unity 證實):[bugnet:how to fix "Hold on (busy for…)"](https://bugnet.io/blog/how-to-fix-unity-hold-on-busy-for-in-the-editor)
- Shader variant 爆炸與切換平台後的整批重新匯入,屬於 Unity 的一般已知行為,這次搜尋沒有找到可引用的官方來源。
