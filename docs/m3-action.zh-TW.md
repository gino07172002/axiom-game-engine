# M3 Action / Ember Court

雙擊 `Play Action.cmd`，或用獨立資料庫開啟內容包。首次啟動後，續玩同一資料庫可省略 `--pack`，保留已提交的內容編輯。

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe --db data/action.sqlite --pack content/action play --start
.\target\debug\axiom.exe --db data/action-edit.sqlite --pack content/action editor
```

WASD 移動，滑鼠或 Q／E 環繞第三人稱相機，F 鎖定最近的可見敵人，再按一次解除。左鍵攻擊，於前一段後搖按下一次接第二、第三段。Space 翻滾，有方向輸入時朝輸入方向翻滾。Esc 暫停後可用按鈕操作、重開或進入控制台。敵人揮擊前的橘色弧線為預警，主角的青色弧線標示攻擊距離與有效判定。擊倒三位守衛完成關卡。

## 資料與執行

`axiom-action` 定義 `action.actor`、`action.session`、`action.abilities` 及純模擬。runtime 的 `action.simulate` 註冊輸入、相機、動畫、呈現及 `RuntimeCommand::Call`。AI、HUD 和 scenario 共用 attack／dodge／lock 命令。

每秒 120 個固定 tick。Ability IR 的窗口採半開區間 `[start,end)`，提供 clip、duration、cost、cooldown、active、invulnerable、chain、chain_window、dodge_window、damage、range、arc_degrees、travel_speed 和有 tick 的音效事件。輸入緩衝預設 16 ticks，體力成功起招時扣除。資源不足或冷卻未到的命令會失敗並保留當前技能。

同一技能啟動期間對每個目標最多嘗試一次。無敵窗會消耗該次判定，角色離開無敵後不會被同一刀的殘留有效窗再次命中。傷害判定先採樣同一 tick 的全部攻擊，再寫入傷害，可同時互相命中。有效判定是平面距離與扇形，會檢查矩形遮擋。角色碰撞使用圓形足跡，翻滾移動有子步進，避免穿過薄阻擋。

GLB 角色包含實際的 idle／run／slash1／slash2／slash3／dodge／enemy／death clips。動畫 cue 使用相同 tick，固定步進結束時消費並求值。動畫播放速率配合技能 duration。一般資料編輯保留進度，技能、角色定義或 session 變動會重新開始戰鬥。所有音效按穩定 ID 引用 `audio.library`，在固定 tick 產生事件，原生 host 以有界背景佇列播放預先生成的 PCM。

## AI 修改與驗收

```powershell
.\target\debug\axiom.exe discover action
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /action/hero --field /action/stats --field /audio
.\target\debug\axiom.exe --db data/action-qa.sqlite --pack content/action scenario examples/action-functional.scenario.json
```

使用穩定 resource ID 投影和編輯，例如 `target={"kind":"resource","id":"action.abilities"}`，欄位 `/abilities/slash2/dodge_window` 或 `/abilities/roll/invulnerable`。配合 expected_rev、preview 和 execute receipt 可檢查差異與冪等重試。無效窗口在提交前拒絕，缺失 clip 在資產準備階段回報診斷，保留上一個可執行的 revision。

在編輯器下方 Assets 點選 `action.abilities`，Inspector 的 JSON pointer 輸入 `/abilities/slash2/dodge_window` 後按 Load fields，即可只編輯該窗口。Preview 檢查差異，Apply 提交，再按 F5 驗證。也可以用 `editor --select action.abilities` 直接檢查技能資源。資源草稿與物件草稿使用相同的修訂衝突處理。

內建任務驗證三段連擊、翻滾、守衛死亡及動畫有效性。測試涵蓋取消窗、緩衝到期、體力不足、無敵邊界、同刀只判定一次、障礙遮擋、資產替換及不同固定步進批次的結果。

## 目前範圍

角色採原創低多邊形裝甲模型。靜態地面的角色和攻擊適合這個灰盒原型。跳躍、斜坡、骨骼形狀的武器掃掠及相機遮擋回縮仍屬後續能力。Action、音效和動畫私有狀態的 checkpoint 會明確拒絕。目前 Windows 音效為合成短音，播放成功與失敗會列入 observe，其他平台回報 unavailable。

模型可用 `scripts/blender-action.py` 重建。它沿用專案原創的基本人體生成器，匯出兩個造型不同但 clip 契約相同的 GLB，供敵我識別與資產替換驗證。
