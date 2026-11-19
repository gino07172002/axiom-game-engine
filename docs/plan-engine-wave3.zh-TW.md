# 第三批引擎功能計畫(Unity 一般性功能缺口)

建立:2026-11-19。來源:對照 Unity 常見功能的缺口盤點(對話紀錄;依據 `docs/audio.zh-TW.md`、`docs/physics.zh-TW.md`、`docs/lighting.zh-TW.md` 的「限制」章節與 code 搜尋)。使用者同意先做影響面最大、與大場景無關的六項。每條工作線走 `rigorous-change`,驗收走 `rigorous-verify`;跨平台、大場景、地形、LOD、多人連線、腳本語言不在本批。

## Phase 0 — 需求與成功樣貌

| # | 功能 | 驗收 demo(可照著演) |
|---|---|---|
| A | 音效檔播放 | pack 內放一個 WAV 與一個 OGG,`audio` 資源引用檔案;執行期播放、迴圈、音量、空間化與現有程序合成片段相同;`observe` 回報解碼後長度;缺檔 / 壞檔 / 超出預算是可恢復錯誤;打包報告把它們列為 referenced。 |
| B | 手把 | XInput 手把的按鈕與搖桿可綁到 `input.bindings`(與鍵盤並存);FPS / action 內容用手把移動與攻擊;拔插不當機;沒有手把時行為不變;無頭 scenario 可注入手把輸入。 |
| C | 物理關節 + 碰撞 / 觸發事件 | `physics.joint`(fixed / revolute / prismatic / spherical,限位、馬達選做)把兩個剛體連起來(例:鉸鏈門、吊燈鏈);`physics.collider` 的 `sensor: true` 與接觸事件送進 `behavior.program`(enter / exit,帶雙方 ID);確定性重播相同;checkpoint 支援或明確失敗。 |
| D | 2D blend tree | 動畫控制器 `blend2d`(兩個 float 參數,至少 simple-directional 或 freeform 一種),編輯器動畫圖可編輯;8 方向移動混合 demo。 |
| E | 抗鋸齒 + HDR bloom + tone mapping | 世界/相機設定可開 MSAA(4×)或 FXAA;HDR 場景色緩衝、bloom(閾值 / 強度)、tone mapping(ACES 或 AgX);預設關閉時輸出與現在逐位元相同或經 A/B 截圖確認;GPU 時間在 MX150 上量測並記錄。 |
| F | 聚光燈 / 點光源陰影 | `render.light` 的 spot `shadows: true`(單張 shadow map)與 point(cube 或雙拋物面)可用,有數量預算;牆後不再被照亮;GPU 時間量測;未支援組合仍明確拒絕。 |

**約束**:不加 Bevy;新 crate 只有必要時加並說明(優先 `windows` crate 既有 XInput、rapier 既有關節/事件);每個內容功能遵守 parity 規則(純 ops 函式 + ACL verb + parity test + `docs/editor-parity.zh-TW.md` 一列);主執行緒 16 ms;不宣稱效能而無 A/B;隔離 DB;不碰 `data/` 與另一個 session 的 Paladin 檔案。

**Non-goals**:殘響 / HRTF / 都卜勒、MP3、串流播放超長音樂(先整段解碼,有位元組預算)、力回饋震動(選做)、ragdoll 自動生成、TAA、景深 / SSAO / LUT、面光源陰影、cascade shadow map(另批)。

## Phase 1 — 風險

| 風險 | 致命? | 處理 |
|---|---|---|
| OGG 解碼需要新 crate | 可繞(只做 WAV) | 工作線 A spike:`lewton`(純 Rust)編譯時間與大小,記錄;不行就 WAV only + 明確拒絕 OGG。 |
| XInput 需要 `windows` crate 新 feature | 低 | `Win32_UI_Input_XboxController`;不加 gilrs。 |
| 關節/事件破壞確定性或 checkpoint | 中 | 工作線 C 先跑既有確定性測試;事件排序用穩定 ID。 |
| MSAA 改變所有 pipeline 的 sample_count、與 postfx/outline/VFX 衝突 | 高 | 工作線 E 先 spike:MSAA vs FXAA 的改動面積與 GPU 時間,選一個先做,另一個列後續。E 與 F 同時改 render 程式,整合者處理衝突;F 盡量把新程式放在新檔。 |
| 點光源 cube shadow 成本 | 中 | 工作線 F 量測後設預算(例:最多 1 盞點光 + 2 盞聚光陰影)。 |

## Phase 3 — 工作線(平行)

| 工作線 | 範圍 | 主要檔案(預估) |
|---|---|---|
| W3a | A 音效檔 + B 手把 | `audio.rs`、`audio_mix.rs`、`input.rs`、axiom-core audio/input 型別 |
| W3b | C 物理關節與事件 + D 2D blend tree | `physics.rs`、axiom-core 物理 ops、`axiom-animation/src/controller.rs`、`editor/anim_graph.rs` |
| W3c | E 抗鋸齒 + bloom + tone mapping | `render.rs`、`postfx*.rs`、新檔 |
| W3d | F 聚光燈 / 點光源陰影 | `lighting*.rs`、`shadow.wgsl`、新檔 |

整合順序:W3a → W3b → W3d → W3c(render 衝突最大的放最後),每合併一條跑完整 gate(fmt、clippy -D warnings、workspace 測試、editor smoke、兩個 parity、build-street/plaza 無變化、m0 smoke;renderer 改動加 effects smoke 與截圖檢查)。

## Phase 4 — pre-mortem
1. render 兩條線(E、F)合併爆炸 → F 新程式放新檔、E 先定 HDR 目標格式並寫進補記;整合者最後合 E。
2. 新 crate 拖慢建置 / 引入第三方授權 → 每個新 crate 寫明授權與建置時間增量。
3. 「順便」做殘響、TAA、cascade → 已列 non-goal。
4. 只在 headless 驗證、沒看畫面 → E、F 必須附 GPU 截圖 A/B,整合者親自看。

## Phase 5 — 執行協定
同 `docs/plan-packaging.zh-TW.md`:各工作線在本檔下方寫「### W3x 執行補記」(做了什麼、偏離、證據),整合者寫「整合補記」與 WORKLOG。

## 執行補記
