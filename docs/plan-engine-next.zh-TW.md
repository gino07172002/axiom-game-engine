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

## 補記(2026-11-12):納入 `docs/gaps-paladin-night.zh-TW.md` 的缺口

原計畫 → 使用者要求補上該清單的全部缺口 → 分兩批:

**併入第一批正在進行的工作線**(同一塊程式碼,順手做最省):
- A 線加 A2(clip 結束 / 標記點事件送進 behavior)、A3(不重置時間的 `animation.speed`,可漸變)、D1(clip 標記點觸發音效 / 語音,循環每圈觸發)、D2(clip / time / speed / fade 與控制器狀態納入 checkpoint)。
- E 線加 E3(loopback / editor command 即時截圖到指定路徑,不必重啟)。
- R 線原本就含 B4(蒙皮網格投射與接收陰影)。

**第二批**(第一批合併後,以新的 main 為基底):
- **W2a 角色表現 I**(`axiom-animation`):B1 glTF morph target 匯入 + 權重軌 + `animation.morph`;B6 每個 primitive / 材質群組執行期顯示開關(換裝);A5 additive 層與骨骼遮罩。
- **W2b 角色表現 II**:B2 彈簧骨(含碰撞膠囊,固定步進、確定性);B3 IK 目標綁另一個 entity 的骨骼 socket;B7 look-at 約束(目標可為鏡頭或骨骼)。
- **W2c 編排與互動**:A1 一個指令同步驅動多個演出者(編排 / 指令列表);A6 `GameAction::Call`;A7 `when` 可讀動畫狀態;C1 Slider widget;C2 滑鼠拖曳環繞 + 焦點綁骨骼(`camera.follow` 已有軸輸入,補映射);C3 骨骼膠囊命中測試回報部位 ID。
- **W2d 材質與美術管線**:B5 匯入網格的 toon ramp / 描邊 / 臉部陰影控制 / 皮膚 SSS 近似(標明近似);E2 描邊(雙面已支援,經查 `import/gltf.rs` 與 `mesh_render.rs`);E1 網格貼花 / 多層貼圖;E4 GLB 依內容雜湊熱重載(明確指令或檔案監看,背景工作);E5 依 `rigorous-perf` 量測 CPU 蒙皮佔比,有證據才做 GPU 蒙皮。

## 執行紀錄

(各工作線完成後補上)

### P 物理(worktree `wp`,分支 `work-wp`)

**ADR:自寫求解器 vs rapier3d(先 spike、量測,不用猜)**

| 項目 | rapier3d 0.23.1(採用) | 自寫小型求解器(未採用) |
|---|---|---|
| 依賴 / 授權 | **已在依賴樹**(FPS 的角色控制器與射線查詢已用,`simd-stable`),新增 crate 數 0;Apache-2.0;dimforge 持續維護 | 無新依賴 |
| 編譯時間 | 量測:見下方「補記 1」 | 未實作,無法量測;估計新增約 2–3k 行(箱-箱 SAT + 接觸流形裁切 + 熱啟動 + 島嶼睡眠),本身編譯很快 |
| 執行檔大小 | 量測:見「補記 1」 | 無法量測(未實作) |
| 確定性 | 量測:同程序兩個執行期逐位元相同(測試 `identical_runs_and_checkpoints_are_deterministic`);**兩個新程序**跑 `examples/physics-lab.scenario.json` 的 `state_hash` 相同(`dd7f29b2e5d16220`);debug 與 release 見「補記 1」。原因可讀碼確認:未開 `enhanced-determinism` 時 rapier/parry 的雜湊表是 `rustc_hash::FxHashMap`(無隨機種子),唯一的 `std::HashMap` 只在移除碰撞體時查表、不迭代。不保證跨 CPU / OS | 可完全掌握,但堆疊穩定(熱啟動、多點流形)要自己做對,是本計畫最大的風險 |
| typed storage | 本模組自己的表是型別化的(`Vec<Entry>` + slot 索引);rapier 的 arena 也是型別化結構 | 同 |
| checkpoint | rapier 的完整內部狀態只有 `serde-serialize` 特性能存(另需重編 nalgebra/parry 並存很大的接觸圖),**不採用**;改存每個剛體的速度、角速度、睡眠與睡眠計時(transform 原本就存),讀檔重建求解器:同一份存檔讀兩次逐位元相同,但不等於沒存檔一路跑下去(接觸快取重建) | 可把接觸快取一起存 |
| 每步配置 | release 量測:整個 step 15 / 21 / 27 次配置(64 / 256 / 1024 剛體,約 3 / 12 / 49 KB),來自 rapier 內部暫存;本模組的步進迴圈不配置 | 可做到 0 |

決定:用 rapier3d。理由是它已經在依賴樹裡(增量成本只有連結進來的程式碼),堆疊 / 睡眠 / 摩擦 / 彈性 / CCD 都是成熟實作,確定性在本機同一執行檔下量測成立;自寫求解器要花的時間主要在堆疊穩定度,與本計畫的風險表(「致命」)不相稱。代價:每步有少量 rapier 內部配置、checkpoint 不是逐位元延續。

**補記 1(ADR 量測,release,同一台機器同時有其他 agent 編譯)**:
- 執行檔大小 A/B:`axiom.exe` release 有 `physics.step` 34,052,487 B,只拿掉 `physics::register`(其餘相同,rapier 的角色控制器 / 查詢仍由 FPS 連結)33,412,878 B → 物理管線與本模組 **+639,609 B(+1.9%)**。
- 增量編譯(改一行後重建 `axiom-runtime` + 連結 `axiom`,release,各 1 輪):有 8.1 min、沒有 8.7 min — 差異小於機器負載造成的雜訊,量不出物理模組的編譯成本;rapier3d 本身在基準就已經編譯(0 個新 crate)。
- debug 與 release 執行檔跑同一個 scenario:`state_hash` 都是 `dd7f29b2e5d16220`(與兩個 debug 新程序相同)。
- 自寫求解器的編譯時間與大小沒有實作,因此沒有量測(上表標為估計)。

**Milestone**

- M1(walking skeleton,`eef6fb8`):`physics.body` + `physics.step`,一個動態箱子落到靜態地板:headless 測試 `a_dynamic_box_falls_onto_a_static_floor_and_sleeps`,以及 `content/physics-lab` 的 GPU 執行(`play --smoke-frames 360 --screenshot`,MX150 / Vulkan)。
- M2(同一 commit):`sphere` 形狀、`friction` / `restitution`(較大者生效)、運動學剛體、跟隨者(被其他系統移動的碰撞體推箱子:`core.motion`、`path.follow`、`nav.agent`、FPS 玩家都有測試)、睡眠、出界停用、checkpoint `systems` 狀態、熱重載、`physics.impulse|set_velocity|inspect|summary`、預算 1024。
- M3(同一 commit):`ops::add_rigid_body` + ACL `physics SEL body …` + 編輯器 Object ▸ Add Rigid Body(選項對話框)+ parity 測試;Inspector 由型別化 schema 產生表單,Play 時即時區顯示 `physics.inspect`;場景視口 gizmo 依角色著色。
- M4(同一 commit):示範內容包 `content/physics-lab`(塔、金字塔、落下的箱堆、斜坡上的球、推箱子的 porter;只用內建方塊與程序球體)+ `examples/physics-lab.scenario.json` + `Play Physics Lab.cmd`。
- 審查修正(`440811f`):見下方。

**驗證**:fmt;clippy `-D warnings` 0 警告;`cargo test --workspace` 708 passed / 0 failed / 5 ignored(基準 692 → +16:`tests/physics.rs` 16 個,另有 core、parity、對話框測試併入既有測試);editor-smoke ok:true(DB 已刪);editor-world-parity identical:true(1→13);street-editor-parity identical:true(8→16);build-street:content/street 不變;m0 GPU smoke PASS(`artifacts/m0-*.png` 已看過);示範截圖 `artifacts/captures/wp-demo-{1080,768}.png`、`wp-editor-gizmos-{1080,768}.png`(已看過)。

**Release 量測**(`perf_bench --generated-physics N --ticks 600`,每欄是 3 輪交錯執行的 p50 中位數;所有剛體 `can_sleep:false`、4 個一疊,全部醒著且有接觸;機器同時有其他 agent 在編譯,數字偏高):

| 剛體 | step p50(ms) | 3 輪 p50 | p95 中位數 |
|---|---|---|---|
| 64 | 0.28 | 0.33 / 0.28 / 0.13 | 0.32 |
| 256 | 0.74 | 0.98 / 0.51 / 0.74 | 1.15 |
| 1024 | 2.65 | 3.69 / 2.65 / 2.58 | 3.96 |

睡著的剛體幾乎不花時間(示範包 12 秒後 21/23 睡著)。1024 剛體全醒時約 2.6 ms / 步(120 Hz → 每幀最多 2 步 ≈ 5.3 ms),所以預算定為 1024。

**Fresh-context 審查**(子代理只看需求與 diff):1 個 bug + 4 個風險 + 2 個 nit,修正如下(`440811f`):
1. bug:在未啟用場景中建立的剛體,碰撞體被建成 disabled,rapier 在剛體啟用時不會重新啟用它 → 場景啟用後箱子穿過地板。改為碰撞體一律啟用、由剛體開關;測試 `bodies_built_in_an_inactive_scene_collide_once_it_is_active`。
2. 熱重載會蓋掉剛改的運動學速度 → 只有編寫的速度沒變才沿用;測試 `hot_reload_applies_edited_velocities_and_moves_unbodied_colliders_with_their_entity`。
3. 拿掉剛體後靜態碰撞體留在編寫位置、畫面在落點 → 重載 / 讀檔時靜態碰撞體跟隨實體目前姿勢(同上測試)。
4. 編譯期預算只算剛體、執行期算剛體 + 跟隨者 → 編譯期也算跟隨者。
5. nit:對話框無法要求重新擬合成箱子 → Collider 欄位預設 `keep`。編輯模式 gizmo 把跟隨者畫成靜態 → 依 movers 判斷。
未修(記錄):f32 欄位寫入 JSON 時會變長(`0.05000000074505806`),與既有元件(如 `nav.agent`)相同的慣例;checkpoint 讀檔後不是逐位元延續(見 ADR)。

**補記(原計畫 → 變成什麼 → 為什麼)**

- 「靜態 / 動態 / kinematic」:靜態不是 `physics.body` 的一種,而是「沒有剛體的碰撞體」(既有語意,向後相容,舊內容自動成為物理場景)。另外多了「跟隨者」:被其他系統移動的碰撞體 → 位置驅動的運動學剛體,角色控制器不必改程式碼就能推箱子(單向:角色不會被箱子擋)。
- restitution 合併規則用「較大者」而非 rapier 預設的平均:否則彈力球在預設地板上彈力減半(測試先失敗才改)。
- 跟隨者單步超過 0.5 m 或 0.2 rad 視為瞬移:示範包第一次跑時 ping_pong 路徑在端點瞬間轉身 180°,換算成 377 rad/s 角速度,把貼著的箱子甩到 -1268 m;加了回歸測試 `a_follower_turning_around_does_not_fling_the_crate_it_pushed`。
- FPS 競技場碰撞忽略剛體(否則動態箱子會被當成固定牆,且旋轉後違反 FPS 的軸對齊檢查)。FPS 的地板只在 FPS 模擬內,物理看不到 — 記在 docs/physics.zh-TW.md 限制。
- Action 控制器排在動畫 / 音訊之後,它的角色以上一步姿勢推箱子(差 1/120 秒),不為此改排程。

### E 編輯器外觀(2026-11-11,worktree `we`,分支 `work-we`,`rigorous-change`)

**先比較再動手**:改版前用主樹的 debug exe 在 Axiom Street 拍 `artifacts/captures/we-before-{1080,768}.png`,與設計圖並排列出差異(下表「之前」欄)。

| 項目 | 設計圖 | 之前(we-before) | 之後(we-after) |
|---|---|---|---|
| Hierarchy | 場景 ▸ Environment(Buildings / Props / Vegetation)、Lighting、Characters、Cameras 資料夾樹 | 街道是平的根物件列表(沒有群組資料) | Sakura Plaza 以 `group` 分層;資料夾圖示(展開時打開)、展開箭頭 |
| Inspector 元件標題 | Components 標題列右側 + Add Component… | Add Component… / Behavior… 兩個下拉在標題下方(768 時兩列) | 標題列右側 + Add Component…(Behavior 範本在同一清單最上面) |
| Transform | 第一張卡片,Revert / ⋯ | Components 之外的獨立區段,無動作 | Components 下第一張卡片,Revert(`reset SEL core.transform`)+ ⋯(Reset / Copy / Paste),X/Y/Z 紅綠藍 |
| 元件卡片 | 勾選、圖示、Revert、⋯ | 只有展開的卡片有 Reset / ⋯;收合卡片的勾選唯讀;值裡沒有 `enabled` 的元件不顯示勾選 | 每張卡片 Revert / ⋯;收合卡片可勾(一筆 revision);schema 有 `enabled` 的元件一定顯示勾選 |
| Advanced | 每張卡片「Advanced  Render Layers (2)…」摘要列 | 只有底部的「Advanced」(用 ID 載入元件) | 展開卡片內「Advanced (N): a, b…」收起仍等於預設值的欄位;底部改名 Load component by ID |
| 資源欄位 | 名稱 + 複製鈕 | 只有下拉 | 下拉(名稱)+ Copy + Show in Assets(`asset.reveal`) |
| Assets 左側 | 資料夾樹 | 平的類別清單(含 0 個的類別)+ FOLDERS 文字列 | `Assets/` ▸ 類別 ▸ 來源資料夾 / ID 前綴,箭頭、資料夾圖示、數量 |
| 麵包屑 | Assets › Characters › Android_01 | Assets › 類別 | 每段可點的完整路徑 |
| 縮圖 | 模型 / 骨架 / 動畫 / 貼圖 / 材質球 | 模型、圖片、材質;其他是 PFB / SND / UI / INPUT 文字 | 另加 prefab 縮圖(方塊或其模型);其他類別畫圖示 |
| Scene 工具列 | 格線、相機(截圖)、Snap、Lit ▾、Perspective ▾ | 沒有截圖鈕、沒有著色模式 | 截圖鈕 + Lit / Unlit 下拉 |
| 狀態列 | VRAM: 1.26 GB | GPU alloc n/a | VRAM 439 MiB(1080)/ 233 MiB(768),提示有預算與共用記憶體 |

**補記(原計畫 → 變成什麼 → 為什麼)**

- **Wireframe → 只做 Lit / Unlit**:`POLYGON_MODE_LINE` 要為每條世界 pipeline(方塊、網格 / 骨架、每個自訂材質 hook)各建一份線框版,pipeline 數加倍,且工作線 R 同時在改這些 pipeline;CPU 畫邊線則是每幀大量線段。Unlit 只改那個 view 的 uniform(均勻環境光、無太陽 / 區域光 / 霧,`render.rs::unlit_globals`),沒有新 pipeline、沒有 shader 變更。限制:顏色仍經 tone curve(平塗底色,不是原始貼圖值)。
- **元件啟用旗標**:沒有新增通用的 `enabled`(要每個 runtime 系統都尊重才有意義,不便宜)。勾選只出現在 schema 本來就有 `enabled` 的元件(讀 registry 的欄位提示 `"bool, default true"`;值裡省略時顯示預設值,沒動過不寫入)。
- **Advanced 列的「不常用」**:沒有逐元件的欄位分類(要求是通用卡片外框),改用資料定義:已提交值仍等於 `ops::default_value` 的頂層欄位;至少收兩個、外面至少留兩個才出現;篩選、多選值不同、prefab 覆寫時不收。
- **示範場景的角色**:用第一方吉祥物 glb(`content/mascot`,使用者自己的設計;與 axiom-town 相同的 blob,git 不重複存),外加一個方塊人偶。沒有天空 / 反射探針 / 天空光(工作線 R 的範圍),Lighting 資料夾是太陽 + 兩盞路燈的點光。
- **VRAM**:`GPU alloc n/a` 的原因是 wgpu 24 只有 D3D12 後端(gpu-allocator)回報 allocator report,這台走 Vulkan(gpu-alloc)。改讀 DXGI `QueryVideoMemoryInfo`(依 PCI vendor / device 對應 adapter):本程序用的專用顯示記憶體,含 swapchain 與驅動配置、不分 API。`windows` 0.58 本來就因 wgpu-hal 的 D3D12 後端在 lock 裡(同版本、同 feature),所以 Cargo.lock 只多一行相依,沒有新 crate。量測:一次讀取(兩個查詢)debug 6.6 µs(100 次平均,`vram::tests`)。
- **截圖**:沿用既有的讀回(`Renderer::grab`,同一個 COPY_SRC 讀回路徑),裁切、BGRA→RGBA 與 PNG 編碼在背景 job(20 s watchdog、可取消、關閉時 join 500 ms);10 秒內沒有畫出的幀就失敗(視窗最小化時不會卡住指令)。主執行緒不等 GPU(E3 改成非同步 map:審查發現同步讀回在 debug 一幀 434–498 ms;改後抓取幀 3–55 ms,和一般幀同一範圍,交接 0.01 ms;`--screenshot` 的 smoke 截圖路徑維持原本的同步讀回)。截圖包含疊在 Scene 視圖上的編輯器標示(FPS、Frame / Fullscreen 鈕),這是刻意的(看到的就是拍到的)。

**新指令 / 協定**:`view.display_mode lit|unlit`、`view.screenshot [NAME]`、`asset.folder PATH`、`asset.reveal KIND:ID`(都只是檢視狀態,`parity::view_commands_never_write`);`observe` 的 `editor.look`、`editor.vram`;截圖 job 出現在 `editor.jobs` 與狀態列。會改內容的新操作(收合卡片勾選、卡片 / Transform 的 Revert)都是既有 ops 與 ACL(`set SEL C/enabled B`、`reset SEL C`),有 parity 測試。

**證據**:見下方「驗證」段落(由最後一輪填入)。

**範圍追加 E3(使用者,缺口 `docs/gaps-paladin-night.zh-TW.md` E3:`--screenshot` 只在 `--smoke-frames` 結束時拍)**:新協定 op `{"op":"screenshot","path":"…png","region":"window|view"}`(`Request::Screenshot`,runtime 請求;store 無頭時回 `runtime request requires a live engine connection`),遊戲(`axiom play --port`)與編輯器都可用:路徑驗證(`.png`、無 `..`、相對工作目錄或絕對)、一次一個(第二個回錯誤)、寫完才回覆 `{ok, path, width, height, region, ms}`(bridge 與 `--connect` 最多等 32 s)、10 s 內沒有幀接手就回錯誤(完全不畫幀時由用戶端逾時)。審查後修正:只交給請求之後錄下的幀(`GrabOwner`)、`view` 裁切在請求時決定、拒絕磁碟相對 / 根相對 / UNC / 裝置路徑與保留名稱、失敗回覆格式統一、Console 送這個 op 時明確說明;`editor_view {capture}` 仍是同步讀回(未改)。共用 `crate::screenshot`(裁切 / BGRA 轉換 / PNG job,編輯器 `view.screenshot` 也用它;`view.screenshot` 另外接受路徑)。讀回改為非同步 map(`Renderer::take_grab`),主執行緒不等 GPU。端對端(真實 exe,連接埠 47393 / 47394,1600×900):play 與 editor 各 window / view / 相對路徑 / 壞副檔名 / `..` / 壞 region / 連續兩個 / 編輯器指令,全部符合;editor `view` 裁成 Scene 視圖 990×374;結果檔 `e3-results.json` 在工作區 scratchpad。限制:bridge 一次處理一條連線,所以「同時兩個」實際上被 bridge 排隊,拒絕路徑由單元測試 `screenshot::tests::one_capture_at_a_time_and_the_reply_carries_the_size` 覆蓋;遊戲模式的 `region:"view"` 等於整個視窗。

**驗證(最後一輪,709d5d1 之上)**:`cargo fmt --all -- --check` 乾淨;`cargo clippy --workspace --all-targets -- -D warnings` 乾淨;`cargo test --workspace --no-fail-fast` 708 passed / 0 failed / 5 ignored(基準 692;新增 16:`vram`、`chrome::vram_text`、`render::unlit_tests`、`look` ×2、`screenshot` ×4、`inspector_tests` ×3(收合卡片勾選 + Transform Revert、Add Component 標題列、Advanced 列)、`asset_browser_tests` ×2(資料夾樹 / 麵包屑 / reveal、prefab 縮圖請求)、`hierarchy_tests::sakura_plaza_is_a_folder_tree_of_groups`、`thumbs::box_tests`;調整 `thumbnail_job_reports_progress_and_cancels` 的總數 40 → 41(lab 的 prefab 現在也有縮圖)與 `parity::view_commands_never_write` 清單)。`editor-smoke-test.ps1` ok:true(資料庫已刪);`editor-world-parity.ps1` identical:true(13 = 13);`street-editor-parity.ps1` identical:true(16 = 16);`build-street.ps1` 後 `content/street` 無差異;`build-plaza.ps1` 重建後 `content/plaza` 無差異。之前一輪全量測試曾在機器滿載時失敗一次 `app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`(已知的負載敏感計時斷言,FD 記錄過),最後一輪通過。截圖(debug,已看過):`artifacts/captures/we-before-{1080,768}.png`(主樹 exe、Axiom Street)、`we-after{,-light,-unlit}-{1080,768}.png`(Sakura Plaza)、`we-viewshot-{1080,768}.png`(截圖鈕的輸出)。審查:兩輪 fresh-context(主體 14 項、E3 11 項),已修的見上;記錄未修:`advanced_keys` 與狀態列 VRAM 提示每幀格式化字串(小)、`hint_enabled` 讀的是說明文字(目前每個 `enabled` 提示都寫 `default true`)、`vram::tests` 依賴 WDDM 計數(三次重試)、截圖包含 Scene 視圖上的編輯器標示、雙 GPU 同型號時 DXGI 取第一張。
**仍與設計圖不同**:沒有天空 / 天空光 / 反射探針(工作線 R);Hierarchy 沒有「SakuraPlaza (Scene)」根列;沒有 Build、Save、場景檔分頁、Modified(unsaved)(M-UI 起的 non-goal);Advanced 列只列欄位名稱(沒有「Render Layers」這類資料);Assets 細節面板仍佔右側;角色是方塊人偶 + 吉祥物(不是設計圖的角色);1366×768 的 Inspector 仍是窄欄(Transform 標籤疊在上一行)。

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

**驗證(debug,MX150,其他工作線同時建置)**:`cargo fmt --all`;`cargo clippy --workspace --all-targets -- -D warnings` 乾淨;`cargo test --workspace --no-fail-fast` 717 passed / 0 failed / 5 ignored(基準 692;一次全跑時 `editor_kind_templates…` 因新條件數量失敗,已修正後單獨重跑通過;較早一次全跑的 `editor_scenario_runs_in_background…` 500 ms 計時斷言在高負載下失敗、單獨重跑通過,為已知負載敏感測試)。`scripts/editor-smoke-test.ps1 -Port 4917` ok:true(DB 已刪);`editor-world-parity.ps1` identical:true(1→13);`street-editor-parity.ps1` identical:true(8→16);`build-street.ps1` 後 content/street 無變化;`m0-smoke-test.ps1` PASS(`artifacts/m0-two-cameras.png` 已看過)。GPU:`--pack content/animator-lab play --smoke-frames 300 --screenshot artifacts/captures/wa-animator-play.png`(角色沿路徑行走,observe 控制器 state=move、speed≈1.2)。編輯器 Play 中的 Animator 檢視 `artifacts/captures/wa-animator-{1080,768}.png`(邏輯 1920×1080 / 1366×768;move 節點綠色 `live 1`、參數表即時 speed≈2.00),已看過。
- **配置**:`tests/animator.rs::steady_controller_steps_do_not_allocate` 以每執行緒計數配置器量 240 步:無播放器 480 次、片段播放器 480 次、控制器 480 次 → 動畫系統本身 0 次(480 = 世界步進其他部分每步 2 次,與動畫無關)。重構前 `step` 每步 clone 整個播放器(含姿勢 Vec)。retarget / twist / IK 與蒙皮輸出(`extract` 的頂點 Vec)仍會配置,未改。未做 release A/B,不宣稱效能改善。
- **審查**:兩次 fresh-context 審查。第一次:Problems 快取混用兩種圖(修)、片段清單快取不分控制器(修)、失敗步留下部分 IK 結果(修)、回滾 / 重新啟用後速度取樣跳動(修)、永不觸發的 exit_time(驗證拒絕)、缺片段只在 runtime 報錯(維持,與 `player.clip` 一致)。第二次:熱重載重放已過標記、轉移當步漏掉離開狀態的標記與 `on_end`、play+seek 重放、反向播放旗標、失敗的 play 清掉 clip_finished(全部修正並加測試);同一片段標記剛好在循環接縫的 f32 邊界、混合樹主片段切換屬已知限制。

### R 繪圖品質(worktree `wr`,branch `work-wr`)

**ADR R-1:反射走「天空環境反射(解析式,不用 cubemap)」,不做反射探針與 SSR(這一輪)**

| 方案 | MX150 成本 | 品質 | 編輯 / AI | 確定性 | 是否需要重工作 |
|---|---|---|---|---|---|
| A. 反射探針 cubemap(指令觸發擷取) | 每個探針擷取 = 6 次場景 pass(一次性);執行期每像素 1 次 cube 取樣 + box projection;每個探針 6×N² 貼圖記憶體 + mip | 看得到其他物體(靜態時) | 需要探針元件、擷取指令、快取(依內容雜湊、放在 revision 外)、失效規則、gizmo | 擷取的是「那一刻」的執行期畫面(動態物件、粒子、動畫時間會進去),同一 revision 可能擷取出不同結果,要另外定義「擷取哪一個狀態」 | 擷取必須是明確指令或背景工作;不能隱性重擷取 |
| B. SSR | 每幀全螢幕 ray march(MX150 1080p 估計數 ms);forward renderer 沒有法線 G-buffer,要加 MRT 或從深度重建 | 只反射畫面內的東西,邊緣 / 被遮住處破洞,要 fallback | 幾乎不用編寫 | 依視角變化,不影響內容 | 每幀成本,多相機 viewport 要各算一次 |
| C. **天空環境反射(採用)** | 每個 opted-in 像素約數十個 ALU(估計;實測見下)(解析天空求值 + Schlick),零貼圖、零擷取 | 只反射天空(smoothness 低時模糊到 trilight),不反射物體 | 一個元件 `render.surface {metallic, smoothness}`,Inspector 直接加 | 純函式,完全確定 | 無 |

決定:C。理由:(1)計畫允許的降級方案就是 cubemap 環境反射,C 的畫面等同「只有天空的 cubemap」但省掉貼圖與擷取;(2)A 的擷取確定性與快取失效設計是一整條工作線的量,這一輪的時間要留給驗證;(3)B 在 MX150 上成本最高,而且 forward 管線沒有法線緩衝。A 列為下一步(`render.surface` 與 `axiom_environment()` 是它的接點:之後探針只要替換 `axiom_environment` 的來源)。成本實測(release A/B)見 `docs/lighting.zh-TW.md`「量測(天空、反射、網格陰影)」:反射與網格投影在雜訊內,天空背景約 +2 ms / 幀(MX150、機器同時有其他編譯,不穩定)。

**補記(原計畫 → 變成什麼 → 為什麼)**

- 「反射探針或 SSR」→ 天空環境反射(ADR R-1)。
- 「匯入網格接收陰影」→ 接收 **並且投射**(只在編寫了燈光的世界):只接收不投射時角色腳下沒有影子,看起來更假;投射的條件和接收相同,舊內容包(沒有編寫燈光)的陰影貼圖不變。
- 天空光不是新的獨立元件,而是 `ambient` 的第三種模式 `sky`(CPU 積分天空成 trilight),沿用既有的 trilight 管線與 shader。
- 背景天空原本計畫用 vertex→fragment 傳 NDC;在 MX150 Vulkan 驅動(32.0.15.6614)上「vs 用陣列索引角點 + 輸出 varying」這個組合讓 `nvoglv64.dll` 在建立 pipeline 時 access violation(每個內容包啟動即崩潰,事件記錄可查)。改成 fragment 用 `@builtin(position)` 與新的 `sky_viewport` global 自己算 NDC,舊版背景分支一字不改。
- 第一輪 A/B 顯示天空背景約 +1.9 ms(6/7),所以把編寫的天空改到不透明幾何之後畫(只算看得到的像素);第二輪 A/B 量不出這個改動的差別(見 lighting 文件),因為不改變畫面而保留,不宣稱有改善。
- 事故紀錄:二分搜尋驅動崩潰時,一個 PowerShell `[IO.File]::WriteAllText` 用了相對路徑,寫到了**主工作樹**的 `crates/axiom-runtime/src/scene.wgsl`(.NET 的 cwd 是 D:\rust_learn,不是 worktree)。發現後立刻用 `git checkout` 還原該檔(diff 確認內容全是我的實驗碼,主工作樹之後 `git status` 乾淨)。期間若主工作樹的代理剛好在編譯,可能看過一次失敗的 shader。
- 新增第一方示範包 `content/render-lab`(程序天空 + 天空光、拋光地面、金屬方塊、鉻球 `render.geometry`、無表面的對照方塊、遮棚、`fighter.glb`——和 `content/action` 同一個本專案角色檔)。

**Milestones(每個都 commit 在 `work-wr`)**

| Milestone | 內容 |
|---|---|
| M1 walking skeleton | `render.environment.sky`(gradient / procedural)型別 + 驗證 + schema;`SkyParams`(CPU)與 `axiom_sky`(WGSL)同一公式;`ambient: sky`;Globals 加 `inv_view_proj` / 天空 4 個 vec4 / `sky_viewport`(stride 3072 → 3328,8 相機 26 KB);`ops::set_sky` + ACL `sky` + `world.sky` + World 視窗 SKY 列;parity 測試 |
| M2 網格陰影 | `shadow.wgsl` 共用;網格 camera group 加陰影貼圖 + 比較取樣器(接收);`vs_mesh_shadow` + 獨立 uniform-only group 的投影 pass(避免同一 pass 既取樣又寫入陰影貼圖);只在編寫燈光的世界生效 |
| M3 反射 | `render.surface` 元件(ADR R-1);方塊 `material.zw`、網格 `Draw.params.zw` 編碼 (metallic, smoothness+1),0 = 沒有元件 = 舊路徑 |
| M4 驗證 | 示範包、截圖、舊包逐像素比對、smoke、release A/B、fresh-context 審查與修正 |

**驗證紀錄**

- `cargo fmt`、`cargo clippy --workspace --all-targets -D warnings` 乾淨;`cargo test --workspace` 701 通過 / 0 失敗(基準 692,新增 9)。
- 舊內容包逐像素(1440×900 smoke,改動前 exe 是 base commit 重編的 debug 版):axiom-town / shader-lab / effects-tour / action / mascot / street 0 差異;light-lab 2、two-cameras 4、rts 1 個像素差 1/255;fps 132 個像素全在 HUD 右上 16×13 的數字區(同一個舊 exe 連跑兩次也有 51 個,是 HUD 數字不是渲染)。
- `editor-smoke-test` ok:true;`editor-world-parity` identical:true;`street-editor-parity` identical:true;`build-street`:content/street 不變;`m0-smoke-test` PASS;`effects-smoke-test` PASS(36 站全擷取,機器同時有其他代理在編譯,仍在時限內)。
- 截圖(實際看過):`artifacts/wr/wr-{procedural,gradient}-{1080,768}.png`、`wr-sunset-1080.png`;編輯器 `artifacts/captures/wr-editor-{1080,768}.png`(World 視窗 SKY 列 + Inspector `render.surface`)。網格接收陰影另外用「把網格輸出改成陰影值」的暫時 shader 確認(角色在柱子陰影線上 = 全藍),之後把示範場景調成只有軀幹在遮棚陰影裡。
- Fresh-context 審查:無嚴重錯誤;確認並修正 — 日盤大小在 3° 以下幾乎不變(邊緣寬度改成跟日盤成比例,加測試)、`set_sky` 經 f32 重寫整個資源(改成只改 `/sky`、`/ambient` 的 JSON)、World 按鈕與 ACL 同模式行為不一致(同模式一律空批次)、缺 Rust/WGSL Globals 大小測試(加 naga 測試)、網格投影 alpha 規則沒測試(抽成 `casts_shadow` + 測試)、文件提到不存在的量測段落(補上 A/B)、背景天空不套色調曲線(文件說明)。

### 整合補記(2026-11-12,整合者)

- 原計畫:P、E、A 各自合併。→ 變成:P 與 A 各自新增了 `Checkpoint.systems` 與 `RuntimeSystem` 存讀檔 hook(P:`checkpoint_state` / `restore_checkpoint_state`;A:`save_state` / `load_state`)。→ 合併時統一成 A 的 `save_state` / `load_state`(存檔可回報錯誤、讀檔可改 runtime state),P 的 `physics.step` 改實作這組 hook;「存檔缺少某系統狀態」改由 `WorldRuntime::load_system_states` 統一拒絕,錯誤訊息帶系統 ID。為什麼:同一份存檔不能有兩套語意。
- `editor/ui.rs` 的 Edit Graph 按鈕:保留 E 的「只在展開的卡片顯示」(收合卡片的 `draft.form` 屬於另一個元件)並加上 A 的 `animation.controller` 分支。

- R 合併:`render.rs` 的 `Globals` 同時有 R 的天空欄位(`inv_view_proj`、`sky`、`sky_viewport`)與 E 的 Unlit 改寫(`unlit_globals`)。**已知限制**:`render.surface` 反射由每個物件自己的材質參數開啟,`unlit_globals` 關不掉,所以 Unlit 檢視下金屬 / 光滑物件仍會映出天空。要修需在 shader 加全域旗標(得在 MX150 上重驗管線),列為後續。
- 四線合併後的 effects smoke:36 站截圖與所有 bridge 檢查通過,最後一步(900 幀限 60 秒)逾時。A/B(同一 effects DB、debug、各 2 輪、60 / 300 幀):先前通過 effects smoke 的 3f52dea、合併 R 前的 2806e5a、四線合併版,300 幀都是 45–47 秒(約 145 ms/幀,`render_present_acquire_wait` 佔絕大部分)。三版一致 → 不是程式退化,是機器目前 GPU 比 FE 那次(900 幀約 27 秒)慢約 5 倍;原因未查明(接電、平衡電源計畫、GPU 無其他程序)。沒有放寬腳本時限;待機器恢復後重跑。

### W2b 角色表現 II(worktree `w2b`,分支 `work-w2b`,`rigorous-change`)

範圍:缺口 B2(彈簧骨 + 碰撞膠囊)、B3(IK 目標綁另一個實體的骨骼 socket)、B7(look-at)。細節與規則見 `docs/skeletal-animation.zh-TW.md`「次級運動」。

**設計**
- 純計算 `axiom-animation/src/rig.rs`(新檔):`SpringSet` / `LookAtSet` 的型別、驗證、編譯(骨名 → 索引、鏈的走法、預算、鏈不得重疊或巢狀)與 `apply`(明確的輸入 / 輸出狀態 + 重複使用的 scratch,穩定步進 0 配置)。runtime 整合放在新子模組 `axiom-runtime/src/animation/rig.rs`(可直接用 `Controller` 私有欄位,`animation.rs` 只加幾個掛點):`Controller.rig: Option<Box<Rig>>`,沒有用到的播放器維持 `None`,行為與 metrics 不變。
- 管線位置:IK 之後依序 look-at → 彈簧 → 蒙皮;`evaluate` 成功才 `commit` 新狀態(失敗步的回滾規則不變)。彈簧與 look-at 平滑在每個固定步都推進(暫停播放也是);命令 / 讀檔 / 熱重載的 `evaluate(0)` 只重新套用狀態。
- 跨實體順序:固定步一開始對所有播放器解析目標(兩趟走訪、相同順序、可重用的清單),骨骼姿勢固定落後一步;測試精確比對。
- IK socket 沿用 `TwoBoneIk`,新增選填 `socket`(序列化時沒有就省略,舊內容 / 存檔不變);`animation.ik` 的 `target` 改成選填,但必須給 `target` 或 `socket` 其中之一(舊呼叫都有 `target`,行為不變)。`CompiledIk::apply_to` 接受解析後的目標,`apply` 委派給它(殘差仍以給定的目標計算)。
- 存檔:`SavedPlayer.rig`(選填);內容有 rig 而存檔沒有 → 明確拒絕。metrics `players.ID.rig` 只在有 rig 時出現,既有世界的 `state_hash` 不變。
- 編寫:`ops::rig_edit`(`axiom-core/src/ops_rig.rs`,只動 JSON,寫入前跑登錄的驗證器)= ACL `rig SEL spring|collider|look|ik|debug ..` = 編輯器 `rig.edit`(同一個 `parse_rig_edit`)。Inspector 表單由登錄 schema 自動產生。
- 示範:`content/rig-lab`(`scripts/build-rig-lab.py` 產生的程序化人偶;mascot / animator-lab 的 GLB 沒有頭髮 / 裙子骨,也沒有手骨可做雙骨 IK,所以另做)。

**補記(原計畫 → 變成什麼 → 為什麼)**
- 「Inspector 表單」→ 沒有手寫表單,用登錄 schema 產生的通用元件表單(與 physics 等元件相同),parity 測試改 `reset_distance`;為什麼:專用 UI 不在必做範圍,通用表單已可編輯每個欄位。
- 「debug 繪製」→ 只在元件 `debug:true` 時把方塊畫進渲染幀(青色關節、洋紅碰撞體中心與六個軸向端點、黃色面向射線與目標),不是編輯器 gizmo;碰撞體多半在身體裡面,畫面上只看得到部分標記。
- look-at 的多骨分配:第一版每根骨用同一組軸各轉 yaw·w、pitch·w,審查指出 yaw 與 pitch 同時不為 0 時乘積不等於目標(產生 roll);改成整體轉動以累積權重 slerp 分配,權重合計 1 時精確。
- 彈簧初始狀態:第一版在建置時用單位矩陣當實體矩陣,實體不在原點時第一步會甩動(審查發現);改成建置時傳入實體目前的世界矩陣,並加測試 `springs_start_at_rest_where_the_entity_stands`。
