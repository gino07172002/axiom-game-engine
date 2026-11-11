# 計畫:引擎下一階段(繪圖品質、物理、動畫控制器、編輯器外觀)

> 2026-11-11 起。流程:`rigorous-plan`(本檔)→ 每條工作線用 `rigorous-change` 執行。
> 前一份計畫 `docs/plan-editor-human-tuning.md`(M1–M10 + 後續 FA–FF)已結案。

## Phase 0 — 需求與成功樣貌

使用者要求(2026-11-11):先做「繪圖品質、物理、動畫控制器」三項引擎能力,同時讓編輯器外觀更像設計圖 `D:\newExport\Axiom Studio Sakura Plaza Editor.png`;之後做打包發佈;跨平台與大場景效能延後。

**最終驗收 demo 腳本**(全部完成那天):

1. 開編輯器載入新的第一方示範場景(只用引擎內建幾何,不用第三方素材)。Hierarchy 是分層資料夾樹(Environment / Props / Lighting / Characters / Cameras),可展開收合;Inspector 的元件卡片有啟用勾選、圖示、Revert / ⋯、Advanced 摘要列、資源欄位顯示名稱 + 選取鈕;Assets 有資料夾樹 + 麵包屑 + 真實縮圖。整體截圖與設計圖並排,版面與質感接近。
2. 場景有可設定的天空(漸層 / 程序天空)與天空光;地面與金屬物件有反射(反射探針或螢幕空間反射);匯入的 glTF 角色會接收陰影。
3. 按 Run:箱子從高處落下、堆疊、互相碰撞反彈(3D 剛體:重力、box / sphere / capsule、靜態與動態、摩擦與彈性),角色控制器可以推動箱子;同一 scenario 重播 state_hash 一致(確定性)。
4. 角色用動畫控制器(狀態機:idle ↔ walk ↔ run,依參數轉換、交叉淡入、1D 混合樹),在編輯器的動畫控制器檢視看到狀態圖、目前狀態高亮、參數即時值;AI 透過 ACL / editor command 編輯狀態與轉換。
5. 每一項都有 ACL 動詞 + 編輯器操作 + parity 測試;GPU smoke 截圖;效能宣稱都附 release A/B。

## 約束

- Rust workspace,不加 Bevy;`axiom-core`(交易式編寫)與 `axiom-runtime`(固定步進模擬)分離;內容變更走 `ops` + ACL + parity。
- 新增第三方 crate 必須在 ADR 比較(含編譯時間、確定性、授權、維護),不得引入第三方美術 / 音效素材。
- 16 ms 主執行緒預算、背景工作有界可取消、熱路徑無每幀配置、型別化執行期儲存。
- 舊內容包與舊 DB 必須能載入(向後相容 + migration + 測試);`build-street` 不變或說明差異。
- 只在本機 Windows / MX150 驗證;GPU 功能需在不支援的 adapter 上誠實降級。

## Non-goals(本計畫不做)

- 打包發佈(下一份計畫)、跨平台、大場景(1000+ 實體 / 百萬三角形)效能。
- 全域光照(GI / lightmap bake)、光線追蹤、布料 / 頭髮模擬、軟體物理、載具物理。
- 動畫:IK 編輯器、動畫重定向 UI(已有的保持不變)、2D 混合樹(只做 1D)。
- 重做整套 UI 框架;外觀改善在既有 egui + theme / panels / chrome 上進行。

## Phase 1 — 風險

| 風險 | 若不成立 | 驗證方式 |
|---|---|---|
| 物理用自寫或 crate(rapier3d 等)— 編譯時間、確定性、與 nav / combat 碰撞共存 | 致命 | 工作線 P 的 spike + ADR |
| 反射探針 / SSR 在 MX150 的成本 | 繞得過(降級成 cubemap 環境反射) | 工作線 R release A/B |
| 匯入網格 pipeline 沒有陰影貼圖 | 繞得過 | 工作線 R |
| 動畫狀態機與現有 `animation.evaluate` 姿勢混合的整合 | 致命 | 工作線 A 讀碼 + spike |
| 4 條工作線同時改 `world.rs` / `registry` / `render.rs` / 編輯器共用檔 | 合併衝突 | 只加法修改、我逐一合併並驗證 |

## Phase 2–3 — 工作線(每條在自己的 worktree,各自寫 ADR 與 milestone)

- **R 繪圖品質**:可設定天空(漸層 / 程序天空)+ 天空光;反射(反射探針 cubemap 或 SSR,ADR 決定);匯入網格接收陰影。GPU smoke + release A/B。
- **P 物理**:3D 剛體(重力、box / sphere / capsule、靜態 / 動態 / kinematic、摩擦、彈性、睡眠)、與角色控制器互動、確定性(scenario state_hash)、ACL / 編輯器 / parity、debug 繪製碰撞形狀。
- **A 動畫控制器**:`animation.controller` 元件(狀態、轉換條件、參數、交叉淡入、1D 混合樹)、執行期求值整合、編輯器狀態圖檢視(即時高亮)、ACL / parity。
- **E 編輯器外觀**:第一方分層示範場景;Hierarchy 資料夾樹;Inspector 卡片精修;Assets 資料夾樹 + 麵包屑 + 縮圖(背景工作、依內容雜湊快取);視窗顯示模式(Lit / Unlit / Wireframe 若可行)與截圖鈕;狀態列 GPU 記憶體實際數字(不能讀時說明原因)。截圖與設計圖並排比較。

每條工作線的第一個 milestone 必須是端到端最薄的一條線(walking skeleton),並可回退。

## Phase 4 — pre-mortem

1. **合併爆炸**:四條線都碰 `registry` / `world.rs` / 編輯器共用檔 → 只加法修改;我每合併一條就跑完整驗證再合下一條。
2. **物理不確定導致 scenario 重播不一致** → 固定步進、固定迭代次數、穩定排序;測試比對 state_hash。
3. **範圍爆掉**(GI、布料、2D 混合樹最誘人)→ 已寫進 non-goals。
4. **機器負載讓計時測試不穩** → 已在 FD 修過 scenario 測試;新增的計時斷言要能抵抗負載,否則標 ignored 並另做 release 量測。

## Phase 5 — 執行協定

- 每條線:`rigorous-change`;驗證 = fmt、clippy -D warnings、`cargo test --workspace`、editor smoke、兩支 parity、build-street、GPU smoke(繪圖相關)、1080 / 768 截圖(實際看過)、fresh-context 審查。
- 每條線在本檔「執行紀錄」加一節(含補記:原計畫 → 變成什麼 → 為什麼)。
- 我(整合者)獨立重跑驗證,依序合併並推送;清掉 worktree 與增量快取。

## 執行紀錄

(各工作線完成後補上)

### A 動畫控制器(工作線 A,worktree `wa`,分支 `work-wa`)

流程:`rigorous-plan`(本節 milestone 拆解)→ 每個 milestone `rigorous-change`。

**ADR A1:資源 + 元件(不是只用元件)。** 控制器定義放在 `animation.controllers` 資源(`{controllers:{ID:{parameters, initial, states, transitions}}}`),實體用 `animation.controller {controller}` 掛上。比較:(a) 純元件 —— 每個實體各存一份圖,同一套 idle/walk/run 給 20 個 NPC 要改 20 次,Graphs 也得改成編輯元件;(b) 資源 + 元件(選用)—— 與既有 `fsm.machines` + `fsm.agent` 同形,多實體共用一份圖,編輯器、ops、ACL、版面(`editor.layout` 的 `anim/ID`)都能沿用 FSM 的路徑。代價:片段名稱只能在 runtime 對各實體的 `animation.player` 解析(資源驗證時不載入資產),缺片段時 runtime 建置失敗並指出 `/entities/@ID/components/animation.controller` 與狀態名。

**ADR A2:圖形操作共用 FSM 的 ops 與畫布。** `ops_graph` 加 `GraphDoc {Fsm, Animator}`,狀態 / 轉移 / 位置 / 刪除改成 `graph_edit` / `graph_set_node` / `graph_remove`(`fsm_*` 保留為包裝,行為與錯誤碼不變,既有測試全過);兩種圖都是 `{initial, states, transitions:[{from,to,..}]}`,差別只在狀態與轉移的內容。Graphs 分頁多一個 **Animator** 檢視,同一個 Model / 畫布 / 拖曳 / 拉線 / 右鍵,右側換成動畫專用表單(`editor/anim_graph.rs`)。不另做一個圖編輯器,也不引入新 crate。

**ADR A3:執行期求值。** 純邏輯在 `axiom-animation/src/controller.rs`(無 renderer 依賴):`Compiled`(名稱→索引、片段長度)與 `State`(`Copy`:目前層、淡出層、淡入計時、16 個參數的固定陣列)。每個 120 Hz 固定步:更新 `source:"speed"` 參數(實體平面位移 / FIXED_DT)→ 推進目前與淡出中的狀態 → 未淡入時依序檢查轉移(第一個通過者觸發,消耗 trigger)→ 產生最多 4 個加權樣本(2 層 × 1D 混合兩點)。混合樹用正規化時間同步;淡入期間不打斷。runtime 的 `Controller::evaluate` 改用重複使用的姿勢緩衝(`sample_into`、`blend_toward`、`blend_from`、`globals_into`),`step` 不再每步 clone 整個播放器(失敗時回滾 `Copy` 狀態,保留上一個姿勢)。
- 參數來源:遊戲命令 `animation.param`(`RuntimeCommand::Call`,UI 按鈕、`runtime.bindings`、scenario、loopback、editor Play 都走它)、`source:"speed"`(角色控制器 / FSM 移動 / path.follow 都能驅動,不和特定 genre 綁在一起)。
- 確定性:控制器狀態在 `metrics.animation.players.ID.controller`,所以在 `state_hash` 裡;scenario 重播雜湊一致,改變 trigger 時間雜湊就不同(測試)。沒有控制器的玩家 metrics 形狀不變(不加 `controller: null`),既有 scenario 雜湊不受影響。
- Checkpoint:`animation.evaluate` 原本就不支援 checkpoint,`save` 明確失敗並指出該系統(測試);沒有做部分存檔。
- 衝突:有控制器的實體拒絕 `animation.play`(訊息指向 `animation.param`),FSM / 動作送來的片段 cue 計數在 `ignored_cues`,不默默丟掉。

**Milestones**
- M1(walking skeleton):控制器型別 + 驗證 + runtime 整合 + `content/animator-lab`(mascot 的 idle / walk / run / wave,沿 path 行走,速度驅動)+ headless 測試 + GPU 畫面。
- M2:ops(`anim_create` / `anim_param` / `anim_attach` / `anim_problems` + 共用 `graph_*`)、ACL `animctl`、編輯器 Graphs ▸ Animator(畫布、參數表、狀態 / 轉移表單、Play 即時高亮與參數值、淡入進度條)、指令 `graph.open_controller|new_controller|attach_controller|param`、`editor_view {open:"animator:ID"}`、Inspector Edit Graph、parity 測試。
- M3:驗證(全套 + GPU smoke + 截圖)、審查修正、文件。

**補記(範圍追加,2026-11-12):** 使用者要求在 A 線補上 `docs/gaps-paladin-night.zh-TW.md` 的 A2、A3、D1、D2(通用實作,不複製該內容包)。
- A2 片段結束 / 標記事件:`animation.library.markers`(片段 → `{at, event?, cue?}`)越過時送 `behavior.program` 事件(宣告了 `entity` / `clip` 文字欄位就填入),循環每圈一次;`animation.player.on_end` 每次完成循環;FSM 新條件 `clip_finished`;控制器內用 `exit_time`,轉移當步先結算離開狀態的最後一步(標記與 `on_end` 不漏)。ACL `marker CLIP add|remove|move`、op `anim_marker`、編輯器 `anim.marker`。
- A3 速度:runtime 命令 `animation.speed {entity, speed, ramp_seconds?}`(不重置時間,線性漸變,失敗步回滾);控制器狀態 `speed_parameter`(Animator 表單 Rate param)。
- D1 音效:標記的 `cue` 走既有 `audio.events`,每圈一次;未知 cue / 缺系統在建置時報錯。
- D2 存檔:`Checkpoint.systems`(新欄位,舊存檔沒有;空時不寫出)+ `RuntimeSystem::save_state / load_state`。動畫存片段、時間、速度與漸變、淡入起始姿勢、骨骼覆寫、IK 目標、twist 開關、控制器狀態(以名稱)與參數;需要狀態的系統遇到缺少該段的存檔明確拒絕。原計畫(M1 時)是「動畫 checkpoint 明確失敗」,改為完整支援,因為使用者把 D2 列為必做。
- 未做:時間軸上畫出標記(目前只有指令 / ACL / Inspector JSON 可編輯);標記所屬片段若沒有任何 player 會被忽略(不報錯,避免刪掉角色就讓整個世界無法建置);反向播放不觸發標記;混合樹的標記跟隨當步權重最大的片段。
