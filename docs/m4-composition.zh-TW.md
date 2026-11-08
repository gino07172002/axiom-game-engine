# M4 / Beacon Alliance

同一個關卡包含六人 RTS 小隊、Action 英雄與 ADV 聯隊任務。雙擊 `Play Alliance.cmd` 開始。接受任務後派礦工交付至少 8 份晶礦，再切換英雄，以三段連擊擊敗守衛。兩個原生系統的結果經型別化事件送到任務規則，最後進入 `joined_victory` 結局。也可拒絕任務；英雄死亡會進入失敗結局。

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe --db data/alliance.sqlite --pack content/m4 play --start
.\target\debug\axiom.exe --db data/alliance-edit.sqlite --pack content/m4 editor
```

## 操作

G 或上方 CONTROL 切換操控。小隊視角使用左鍵框選、右鍵採集或移動，WASD 平移相機、Q／E 縮放。任務面板的「派出礦工」和「生產士兵」使用相同的 RTS capability commands。英雄視角使用 WASD 移動、滑鼠或 Q／E 環繞、F 鎖定、左鍵連擊、Space 翻滾。Esc 暫停。

切換由通用 `input.contexts` 資源決定，包括接收玩家輸入的系統、可見相機和滑鼠捕捉。其他模擬持續執行，離開小隊操控時會取消尚未完成的框選拖曳。UI、loopback 與 scenario 都可呼叫 `input.activate`，不需要在 app 加入遊戲類型分支。

## 內容包與版本

`content/m4` 的根包 `m4.alliance` 載入任務包 `m4.quest`，任務包再依賴 `m4.hero` 與 `m4.squad`。模組檔案為 World IR v2 片段，資產以各自的包目錄解析，最後一起編譯成一個世界。小隊、英雄與任務的配方定義位於各包，根包提供共用場地和操控情境。

既有 manifest 的 `version:1` 表示檔案格式，新增的 `content_version` 是正整數內容版本。相依採精確版本，不做範圍或自動升級。每個依賴宣告 `path`、`content_version`，可再提供來源 `fingerprint`。載入會拒絕版本不符、衝突來源、循環、超出根包的路徑及重複物件 ID。

```json
{
  "dependencies": {
    "m4.hero": {"path":"../hero","content_version":1}
  }
}
```

來源指紋採既有 FNV 識別 manifest 與 World JSON。manifest 可用 `assets` 指定相對檔名和 SHA-256，匯入時檢查檔案。M4 的兩個 GLB 都有摘要清單，清單亦包含在來源指紋內。解析結果存於 `World.packages`，隨 authoring revision、匯出、匯入和 undo 保存；runtime 使用已準備的世界，逐幀執行不讀取 manifest。

不同包的 entity、prefab、scene 及自訂 schema ID 必須唯一。資源型別可登記合併規則，例如 `input.bindings` 合併不同 action 名稱，`animation.library` 合併不同 asset／clip ID。重複 ID 會失敗。沒有合併規則的資源只允許單一定義。

## 跨包事件與資源契約

發布包以 `exports.events` 宣告版本、原生發布系統和 payload 欄位。使用包以 `imports.events` 指定直接相依的包、版本、消費系統和預期欄位。世界編譯會核對安裝的原生事件介面、實際排程及 behavior event schema。

| 事件 | 擁有者 | 內容 |
| --- | --- | --- |
| rts.delivered | rts.simulate | amount 為 int、faction 為 int |
| rts.produced | rts.simulate | entity 為 text |
| action.defeated | action.simulate | entity 為 text、faction 為 int |
| action.victory | action.simulate | 空 payload |

資源使用 `exports.resources` 宣告介面版本和 owner，`imports.resources` 宣告來源包、版本及 reader。此關卡由英雄包提供 `action.abilities`，小隊包提供 `rts.session`；任務包宣告對這些介面的相依。原生系統管理所屬的執行狀態，任務透過事件接收成果。

事件記錄包含 seq、tick、版本、publisher 和 typed fields，在固定步進邊界按發布順序分批投遞。每 tick 上限 128 筆，超額會明確失敗。外部 command 和 authored behavior 不能冒充已宣告的原生發布者。

behavior 的一批投遞使用候選狀態，整批成功才提交並移除佇列。規則失敗會保留待處理事件與原本任務狀態，並停止後續固定步進與玩法命令；修正規則後可重新啟用並重試。同一世界的未完成事件需保留相同契約；改變契約時會拒絕遺失中的批次。已投遞序號與進度在一般熱更新時保留。

## AI 與驗證

```powershell
.\target\debug\axiom.exe discover packages
.\target\debug\axiom.exe discover events
.\target\debug\axiom.exe --db data/alliance.sqlite query packages
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /input --field /events --field /gameplay/variables
.\target\debug\axiom.exe --db data/alliance-qa.sqlite --pack content/m4 scenario examples/m4-joint.scenario.json
.\target\debug\axiom.exe benchmark benchmarks/m4.json --output artifacts/m4-benchmark.json
.\scripts\m4-smoke-test.ps1
```

`query packages` 回傳版本、指紋、相依和介面名稱的摘要。使用原本的 select／set_field 調整任務或技能，配合 expected_rev、preview 和 execute receipt。檔案相依升級需要重新匯入內容包，之後的 authoring edits 由 revision 追蹤。

M4 benchmark 有技能取消窗和任務補給條件兩個任務，各自包含完整聯合作戰情境的獨立驗收。固定計畫回放記錄傳輸量、工具次數及成功率。真實模型 tokens 需由 agent adapter 提供 usage 才能計算。

## 目前範圍

根包內容版本 2 已加入 `combat.session` 與 `combat.resolve`，RTS 和 Action 共用平面碰撞、遮擋、傷害及死亡結算。小隊與英雄可攻擊同一批敵人；使用 `Play Alliance Combat.cmd` 在獨立資料庫體驗，避免覆蓋原有 authoring。詳見[共用戰鬥指南](shared-combat.zh-TW.md)。混合原型的私有模擬狀態與輸入情境尚未支援 checkpoint，存檔會明確拒絕。內容包為本機、固定相依圖的片段組合。關卡可由 `scripts/build-m4-content.ps1` 重新產生。
