<img src="../assets/branding/axiom-icon-256.png" alt="Axiom" width="96" align="right">

# Axiom v0.4：World IR v2 與執行邊界

## Authoring

`axiom-core` 只描述世界資料：entities、components、prefabs、relations、resources、scenes、schemas、capability versions 及 system IDs。世界不需要玩家、生命或物理。原生 schema、驗證器、系統宣告、遷移與相容指令由 Registry 安裝。

SQLite 保存不可變 revision 快照。BEGIN IMMEDIATE 與鎖內 expected_rev 比較提供交易與 CAS；在暫存世界套用整批編輯，最終編譯驗證成功才 commit。Preview 使用相同路徑並回滾交易。Undo 追加新的恢復版本。

M1.5 的 select 以穩定 target ID、欄位投影及回覆 byte budget 操作上述資料；修改不依賴節點／選項陣列位置。Diff 按 ID 比對記錄，回傳精確路徑，較大的值按需查詢。Execute 的 authoring receipt 與 revision 在同一 SQLite 交易提交；runtime receipt 則綁定 host session，兩者具有不同生命週期。詳見 [M1.5 合約](m15-implementation.md)。

## Compilation

World compiler 執行能力版本、schema、欄位、component dependency、引用、parent cycle、scene、system dependency 與資源預算檢查，再呼叫已安裝能力的世界驗證器。

Prefab 採單層組件繼承、物件欄位遞迴覆寫，陣列整體替換。編譯結果包含已展開組件、穩定 ID 到 slot 的索引、parent 拓樸順序、scene enable mask、原生 typed structs、自訂 component 的 SoA columns，以及依 phase／after 排序的系統清單。

動態欄位支援 bool、i64、f32、string、vec3、entity ref、asset ID。M1 的 behavior.program 另有具型別共用變數、事件 payload、條件與動作；變數與節點引用編譯為索引。這些變數不等同於任意 component 欄位寫入介面；新增原生演算法仍需要實作並註冊能力。

## Runtime

`WorldRuntime` 依 RuntimeRegistry 建立系統物件；shared gameplay、core motion、FPS 與 RTS 都走相同的 factory／step／transfer／extract 介面。執行順序固定，尚未平行排程；reads／writes metadata 目前提供探索與契約說明。

引擎維持 120 Hz 固定步進。輸入由內容中的 input.bindings 映射成 ActionMap，情境 runner 使用相同的 action 名稱。RuntimeState 保存位置、生命、自訂 columns、runtime IDs 與時鐘，原生系統直接使用具型別資料。

`RuntimeCommand::Call` 包含 capability、action 和 typed args；Registry 提供 schema／驗證，啟用的 RuntimeSystem 接受命令。UI／scenario／loopback 共用此介面。原生視窗將游標依相機投影轉成 ScreenPointer 射線，能力模組自行定義互動語意。核心不判斷 genre。RuntimeState.spawn 使用預編譯 template 建立穩定邏輯 ID、generation handle 與各型別欄位；執行期生產不新增 authoring revision。

低頻更新先建構新的 compiled world 與系統，再依邏輯 ID 遷移狀態，完成驗證後交換。失敗保留上一個世界並記錄 requested／applied revision。自訂欄位定義不變時保留值；entity refs 按邏輯 ID 重新連結，失去的 runtime target 回退到新 authoring 值。改變 schema 的欄位使用新的定義初始化。

## Presentation

RenderFrame 包含共享 cube instances、獨立 camera views、view-local instances 與場景材質設定。Renderer 不接觸 FPS Simulation。多相機使用獨立的動態 uniform offsets、viewport／scissor 與 depth clear；透視與正交可並存。

通用 WGSL 提供材質、方向光、PCF 陰影、格線與霧。FPS 的競技場、標記與外觀由內容資料提供；武器後座、步行姿勢及 HUD 由 FPS 原生能力模組產生。M1 的 ui.document 經編譯後由 egui 呈現文字、對話、按鈕、圖片、選項與進度；點擊輸出 typed RuntimeCommand。PNG 由內容包解析路徑並於準備階段解碼；語系與資料綁定讀取相同 Gameplay 狀態。

RTS 的資源／人口／選取／佇列 HUD 使用同一 ui.document，flow 版面與可選 session_controls 擴充均保留 ADV 預設行為。RTS 在建構時把 UI metric 對應到 typed variable handles，步進不解析 JSON 路徑；選取外框及拖曳框由能力抽取呈現。

## FPS 邊界與遷移

`axiom-fps` 包含舊 schema 的強型別模型、v1→v2 轉換器、FPS 組件／資源與相容指令。`axiom-runtime/fps` 保留原 Rapier 膠囊控制、射擊、裝填、有限狀態機與 HUD。能力註冊決定是否建立 FPS 系統，通用核心不依遊戲類型分支。

v1 資料庫開啟時，轉換後的世界追加為新 revision，歷史原文保留；讀取舊 revision 或 import v1 也使用同一個 adapter。資料定義 undo 與遊玩存檔為不同功能。M1 checkpoint 不存入 revision 表；native FPS 的物理／戰鬥進度尚未加入 checkpoint 合約，會明確拒絕儲存。

## 共用事件與敘事

behavior.execute 在 phase 5 處理 rising-edge 輸入、具型別事件及規則。事件依佇列順序、規則依作者順序消費；延遲事件以固定模擬 tick 計時。條件支援 typed 比較與布林組合，動作可更新變數、發事件、等待 tick 後發事件、換節點、換場景或設定結局。每批執行最多 512 指令、128 個 queued events、16 層 node entry；條件深度最多 16。

命令與 fixed-step 的玩法變更先在暫存狀態執行，成功才交換。錯誤命令不留下部分變更；step 出錯則保留舊進度並發布 fault。UI、scenario、ActionMap 與外部命令使用相同執行器；沒有 ADV 特例分支。敘事選擇同時驗證來源 node 與條件，拒絕過期／不可見選項。

相同內容世界熱更新時，未改型別／預設值的變數保留 runtime 值；敘事 node、場景、語系與待處理事件使用穩定 ID 遷移。刪除正在使用的 node 或不相容的 pending event，會使啟用失敗並保留舊世界。換成另一個 world ID 時敘事重新初始化。

## Checkpoint

Checkpoint v1 分開保存 engine version、來源 authoring revision、完整內容定義及 runtime progress。Transform、health、custom columns 以穩定 entity ID 擷取；entity refs 以邏輯 ID 序列化並於恢復時重新編譯。敘事 node、變數、結局、active scenes、locale、held input 與 pending events 亦保存。恢復在全新 runtime 驗證所有資料，再交換，且不重播 on_enter。

內容定義須相同，authoring revision 編號可以不同；v0.4 checkpoint v1 reader 只接受目前版本與明確支援的 v0.3.0／v0.3.1，尚未做存檔內容遷移。只有明確宣告支援的無私有持久狀態原生系統可以 checkpoint。FPS／RTS 的私有模擬狀態尚未支援，會明確拒絕。檔案上限 8 MiB，使用限定 slot、分段編碼 world ID、暫存檔 flush 和上一版備份；損壞資料不會改變當前 runtime。

Loopback bridge 使用有界 mailbox 將 command／save／load 交給主執行緒，回覆成功代表操作已在 runtime 完成；到期未開始的命令不會晚到執行。authoring requests 維持原交易介面，observe 讀取主執行緒的快照。

## Observation 與測試

Observe 的通用欄位包含 applied／requested revision、啟用狀態、tick、entities、cameras、systems 與 CPU 端呈現 wall time。各系統提供命名統計；無 FPS 系統的世界不帶玩家／槍械統計。

ObserveDelta 保存有界的投影快照，以 cursor 回傳精確 set／remove changes；預設不含時間噪音、全 UI 或 trace。runtime_session 區分 host、runtime_epoch 區分恢復／切換；過期與投影變更明確 reset。Trace 具序號與缺口標記，沒有永久事件日誌承諾。

## RTS 能力

axiom-rts 保存 schema、內容配方與純模擬；axiom-runtime/rts.rs 透過同一系統註冊連接 typed storage、ScreenPointer、動態生產與呈現。平面格網使用四方向 A*；導航有每次展開預算、每決策步解決數、有限快取、每 actor 最多一個有效工作。路徑替換取消舊工作，動態阻塞以格子預留及重排處理。預設導航 30 Hz、移動 120 Hz。

採集、攜帶、交付與生產扣款具有資源守恆檢查。生產先扣款排隊，完成時建立真實 runtime entity；storage 準備失敗會回復該步模擬，避免只存在模擬內的單位。熱更新按穩定 ID 保存既有與已生產單位、經濟和佇列，格網更動或失去排隊配方會拒絕啟用。細節與驗收見 [M2](m2-implementation.md)。

Scenario 輸入有固定 tick、action values、typed commands、數量與時長限制；支援 checkpoint／restore、中途斷言、觀察值、位置、統計及選項可見性。narrative-check 在狀態／記憶體預算內探索 authored choices、UI button 與 delayed event；回傳結局路徑及 complete，外部事件來源不在證明範圍。自動測試覆蓋交易、引用、遷移、狀態保存、FPS 碰撞／射擊、ADV 與系統組合。GPU smoke 另外驗證真實後端與畫面。

重現性目前限同 build／平台與相同輸入；固定步進不保證跨平台浮點物理或多人 lockstep。效能與 AI token 成本需分開量測。
