<img src="../assets/branding/axiom-icon-256.png" alt="Axiom" width="96" align="right">

# Axiom 原生場景編輯器

雙擊 `Edit Axiom.cmd`，或在專案根目錄執行以下指令。首次啟動會在 `data/editor.sqlite` 建立 Assembly Yard 範例。後續開啟同一個資料庫即可繼續編輯。

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe editor
```

以不同資料庫編輯既有內容包，例如 `.\target\debug\axiom.exe --db data/adv-edit.sqlite --pack content/adv editor`。每次指定 `--pack` 都會載入包內定義。續編時保留 `--db`，省略 `--pack`。

## 編輯視窗

| 區域 | 操作 |
| --- | --- |
| Hierarchy | 依場景或名稱搜尋，展開父子樹。新增空物件、方塊及相機，建立、啟用、停用或移除場景。篩選場景後新增物件，會自動加入該場景。 |
| Scene | 左鍵選取，右鍵拖曳環繞，中鍵拖曳平移，滾輪縮放，F 對焦。Q／W／E／R 切換選取、移動、旋轉、縮放。移動使用世界軸，旋轉和縮放使用物件本地軸。 |
| Inspector | 可改顯示名稱、位置、角度、縮放、父物件、場景成員及元件。元件面板編輯物件自身的 JSON 或 prefab 覆寫。Preview 可先檢查差異，Apply 提交，Reload 載入目前修訂。 |
| Changes / Assets / AI | 檢視預覽、提交與外部修訂的局部差異，實例化 prefab，複製查詢或最近的交易 JSON。 |

每個區域都是可停靠的分頁(Hierarchy、Scene、Inspector、World 與 Changes / Proposals / History / Problems / Scenarios / Graphs / Assets / AI)。拖曳分頁標題可移到其他格或放在格子上方出現的上 / 下 / 左 / 右按鈕分割;拖到外面變成浮動視窗(Scene 除外,它由 renderer 直接畫在分頁位置)。分頁的 x 或中鍵關閉,Window 選單重新開啟(回到原本的位置附近)。**Window ▸ Layouts** 有內建的 Default(舊版的固定配置)與 Tall Inspector(Inspector 佔滿右側全高,適合 1080p 筆電),**Save Layout As…** 存成自己的名稱、**Reset Layout** 回到 Default。版面存在 `*.editor-prefs.json`(只影響這台機器的畫面,不進修訂);存檔內容有問題時改用 Default,狀態列會說明。Scene 分頁關閉或被同一格的其他分頁蓋住時不渲染 3D 視圖。

Assets 的資源按鈕會在 Inspector 載入資源草稿。可指定 JSON pointer 僅載入所需欄位，例如 `action.abilities` 的 `/abilities/slash2`。Preview、Apply、Reload 和 Copy JSON 沿用交易及修訂衝突規則。每次載入限制為 64 KiB，較大的資源可用更窄的 JSON pointer 檢查。

新增的物件有穩定 ID。名稱可更換，指向 ID 的引用會繼續有效。複製會複製單一物件與它的父層、場景成員關係。刪除物件時，子物件保留本地變換並成為根物件。交易會驗證仍指向已刪物件的型別化引用。重新掛父層時可保留世界姿態，若所需的本地變換含剪切，會回報診斷。

三軸拖曳在放開時提交一筆修訂，Esc 取消拖曳。Inspector 的欄位在 Apply 前留在草稿，選取其他物件前可先套用或 Reload。Undo／Redo 還原本視窗的提交，並追加新修訂。偵測到其他寫入者時，視窗會清除本地 Undo／Redo 堆疊，已提交的紀錄仍可透過 `history` 及 `undo REV` 存取。

F5 從已提交修訂啟動固定步進的 Play 預覽，F8 回到編輯視口。預覽期間會固定使用啟動時的版本，結束時捨棄模擬狀態並載入最新可啟用的修訂。支援的預覽存檔會寫入資料庫專用的 `*.preview-saves`。

編輯相機顯示啟用場景及全域物件。停用場景的物件仍可從場景樹檢查。視口可點選方塊組合網格，其他空間物件可透過原點標記或場景樹選取。

## 選單、命令面板與快捷鍵

視窗頂端是選單列(File / Edit / View / Object / Play / Window / Help),最底下是狀態列(訊息、選取數、問題數、待審提案數、REV、bridge 位址、FPS)。所有選單項目、快捷鍵、命令面板(**Ctrl+Shift+P**,輸入關鍵字後按 Enter)與 AI 的遠端指令都來自同一份指令註冊表,行為完全相同。無法執行的指令會呈灰色,滑鼠停留時說明原因(例如「nothing selected」)。

常用快捷鍵:Q / W / E / R 切換工具、F 對焦、X 切換 Global / Local、Ctrl+Z / Ctrl+Y(或 Ctrl+Shift+Z)復原 / 重做、Ctrl+D 複製物件、Delete 刪除、Esc 取消選取、Ctrl+Shift+N 新增空物件、F5 Play、Ctrl+1…8 開啟 / 聚焦 Changes…AI 分頁、Ctrl+9 開關 World、Ctrl+, 偏好設定、F1 快捷鍵表。

**偏好設定**(Edit ▸ Preferences…,或場景工具列 Snap 旁的「…」)可調整 Snap 的移動 / 旋轉 / 縮放間距。偏好存在資料庫旁的 `*.editor-prefs.json`(例如 `data/editor.editor-prefs.json`),不會進入世界修訂。可在該檔的 `"shortcuts"` 改綁快捷鍵,例如 `{"edit.duplicate": "Ctrl+J"}`;空字串代表取消綁定。

Play 預覽右上角有 Stop(F8)、Pause / Resume(Esc)與 Step(暫停時前進一個固定步)。

AI 可以執行同一批指令:`discover editor_commands` 列出所有 ID,再以 `axiom --connect 127.0.0.1:4252 view --command edit.duplicate`(協定:`{"op":"editor_view","command":"edit.duplicate"}`)執行。有未存草稿或指令目前不可用時,會回傳原因而不是靜默略過。
## AI 與使用者共編

視窗預設提供本機 `127.0.0.1:4252` bridge。AI 可以按需取得 schema，查詢有分頁的階層資料，並用現有的 `select` 投影單一物件。以下命令可在編輯器開啟後執行。

```powershell
.\target\debug\axiom.exe --connect 127.0.0.1:4252 discover editor
'{"op":"scene_tree","query":{"scene":"assembly-yard","limit":12,"max_bytes":2048}}' | .\target\debug\axiom.exe --connect 127.0.0.1:4252 rpc
.\target\debug\axiom.exe --connect 127.0.0.1:4252 observe-delta --field /editor --field /rev --field /activation
```

1. 用 `scene_tree` 回傳的 `rev` 作為 `expected_rev`。支援依 `parent` 查直接子物件，或以 `roots_only` 查根物件。游標綁定查詢條件與修訂。
2. 用 `select` 查穩定 ID 與需要的元件欄位。使用 `set_field` 或 `reparent`、`duplicate_entity`、`delete_entity`、`set_scene_membership` 組成小批次。
3. 先送 `preview` 檢查差異，再用 `execute` 包裝 `apply`，以固定的 request ID 安全重試。同一筆交易由視窗與 AI 共用驗證器。
4. 以 `observe_delta` 檢查 `/editor/authoring_rev`、`/rev` 及 `/activation`。有未提交草稿時，外部新修訂會使草稿標記為過期。使用者可先複製草稿 JSON，再 Reload 讀取新修訂。

父層異動會出現在 `/relations/parent/@ID`，場景成員異動會出現在 `/scenes/@SCENE/entities/@ID`。路徑中的斜線會以 `~1` 跳脫。可用 `diff FROM TO` 分頁檢查完整差異。

## 驗證

```powershell
.\scripts\editor-smoke-test.ps1
```

腳本建立獨立 QA 資料庫，透過 bridge 驗證分頁、preview、冪等 apply、增量 observe 及 undo，並在 `artifacts/editor-*` 擷取編輯前後的原生 GPU 截圖。
