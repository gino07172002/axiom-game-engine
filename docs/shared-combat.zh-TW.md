# RTS / Action 共用碰撞與傷害

雙擊 `Play Alliance Combat.cmd` 使用獨立的 `data/alliance-combat.sqlite` 遊玩更新後的 M4。原本的資料庫與存檔不會被覆蓋。既有 `Play Alliance.cmd` 已建立的資料庫維持原本內容；要驗證新功能請使用新啟動器。

接受聯隊任務、派礦工採集，再按「小隊支援守衛戰」。兩名士兵會前往攻擊庭院守衛。G 切換英雄、F 鎖定、左鍵攻擊、Space 翻滾。英雄和士兵攻擊同一個守衛、共用血量；任何一方擊殺都能完成 ADV 任務。小隊視角也能直接右鍵敵人下達攻擊命令。

## 架構與資料

`axiom-combat` 是不依賴遊戲類型的純 Rust 平面核心。`combat.session` 宣告 XZ 邊界與矩形障礙，`combat.resolve` 啟用共用戰鬥。每個控制器提供身體快照及攻擊意圖，由核心在固定步進末端統一結算；控制器接收結果並處理動畫、選取、採集損失、生產佇列及既有任務事件。app 不需要加入遊戲類型分支。

身體以穩定 ID 和 runtime generation 識別，包含 owner、陣營、半徑、位置、血量和無敵狀態。攻擊必須符合來源擁有者、generation、距離、扇形和遮擋條件。友軍傷害停用，友軍與敵軍都參與實體碰撞。移動分段檢查以避免快速翻滾或單位穿過障礙。RTS 格網導航保留，並納入共用靜態障礙；英雄、士兵與建築的身體碰撞使用相同場景。

同一步進的攻擊先依同一份存活與幾何快照檢查，允許同時互換傷害；死亡只在血量首次降至零時產生。Action 的翻滾無敵窗會防禦 RTS 攻擊，一次技能對每個目標只嘗試一次。敵人可選擇最近的異陣營角色，因此可攻擊英雄或其支援士兵。RTS 的採集、資源守恆與生產不變。

M4 根包 `m4.alliance` 的 `content_version` 升為 2，仍由四個包組成。根包發布 `combat.hit`（source、target、amount）與 `combat.defeated`（entity、source、faction）契約；既有 `action.defeated`、`action.victory` 持續支援 ADV 任務。後者在共用模式下考慮所有異陣營身體，不會只因 Action 自己的敵人死亡就提早宣告勝利。

## AI 查詢與編輯

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe discover combat
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /combat --field /gameplay/ending
.\target\debug\axiom.exe --db data/combat-qa.sqlite --pack content/m4 scenario examples/m4-support.scenario.json
.\scripts\combat-smoke-test.ps1
```

使用通用 command 呼叫 `combat.inspect`，以最多 128 個穩定 ID 查詢身體，不需要取回整個世界：

```json
{"kind":"call","capability":"combat","action":"inspect","args":{"ids":["hero","enemy/guard","squad/soldier/4"]}}
```

`/combat` 提供命中、閃避、拒絕、死亡計數及最近 12 筆結算紀錄。透過 `set_field` 編輯 `combat.session`、既有單位或技能資料，仍使用 expected_rev、preview、execute 及 observe_delta。相容熱更新會保留血量、生成單位及結算序號；會令現有身體重疊的變更會拒絕啟用。

## 範圍

這是目前 RTS 與 Action 的 XZ 平面碰撞及直接命中傷害，並非完整 3D 剛體、彈道或骨骼碰撞箱系統。FPS 保留既有 Rapier 物理。單獨的 FPS、ADV、RTS、Action 原型不需啟用共用模式。混合私有狀態 checkpoint 仍明確回報未支援。

每步進最多 4096 筆共用命中意圖，近期紀錄上限 64；跨包事件仍保留每步進 128 筆的明確上限。CPU 壓測與原生 GPU 畫面驗證分開記錄。
