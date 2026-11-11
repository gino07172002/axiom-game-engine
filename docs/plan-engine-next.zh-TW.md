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
