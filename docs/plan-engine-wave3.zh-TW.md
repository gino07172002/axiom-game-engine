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

### W3c 執行補記

**做了什麼**:`render.environment.post`(新檔 `axiom-core/src/post.rs`):`antialiasing: none|fxaa`、`hdr {exposure, tonemap: aces, bloom {threshold, intensity, radius}}`,預設全關。執行期新檔 `axiom-runtime/src/hdr.rs`、`hdr.wgsl`(bloom prefilter / 降採樣 / 升採樣 + 曝光 + Hill ACES fitted)、`fxaa.wgsl`(仿 FXAA 3.11 quality 的後處理邊緣濾波,**不是 MSAA**)。`ops::set_post` + ACL `post …` + 編輯器 World 視窗 POST 列(切換鈕、勾選、滑桿放開時寫入一次)+ `world.post` 指令;parity 測試 `post_settings_match_acl`、egui 實際操作測試 `world_window_post_row_toggles_and_slider_write_through_the_shared_op`、核心 `set_post_toggles_settings_and_keeps_the_rest_of_the_environment`。`observe` 有設定開啟時回報 `render_post`(設定、目標大小、bloom 層數、pipeline 建立毫秒、各 pass 平均 GPU 毫秒);編輯器 Profiler 多 bloom / tonemap / fxaa 三欄(`gpu_timing` 的 timestamp 上限 32 → 48)。說明見 `docs/post-processing.zh-TW.md`。

**Spike(MSAA 4× vs FXAA)**:現況 color target = surface 格式(第一個 sRGB 格式,本機 `Bgra8UnormSrgb`);pass 順序 = VFX compute → sun depth → 每個 view {世界(舊背景 / 方塊 / 網格 + 描邊 / 編寫天空 / 透明)→ VFX → postfx(讀這個 view 的深度)} → 遊戲 egui → egui → 截圖複製。MSAA 需要:每條顏色 pipeline 的 `sample_count = 4` 版本(內建 14 條 + 每個自訂材質程式 16 條)、多重取樣顏色 + 深度目標與 resolve、VFX 軟粒子 `textureLoad(scene_depth)` 改 multisampled 深度、postfx 讀深度需要自寫深度 resolve(wgpu 不能 resolve 深度)。FXAA = 一個全畫面 pass + 一張 LDR 中介目標、零 pipeline 變體。**先做 FXAA,MSAA 列為後續**。

**HDR 目標格式**:`Rgba16Float`(給整合者 / W3d:HDR 開啟時世界 pipeline 用這個格式的變體;`Globals.ambient_ground.w = 1` 表示寫線性輻射,shader 用 `axiom_tone()` 代替原本的 `x/(x+0.7)`)。

**偏離 / 注意**:
- HDR 需要內建世界 pipeline 的 `Rgba16Float` 版本(方塊 + 兩種天空、網格 4 + 描邊、VFX 6 = 14 條),第一次開 HDR 時在主執行緒同步建立;自訂材質程式以(HDR, WGSL)為快取鍵重新編譯。量到 28–546 ms(debug,GPU 與其他代理共用;一次性,只在明確開啟時),超過 16 ms 預算 → 列為已知限制(建議後續改背景建立)。後處理自己的 5 條 pipeline 6–395 ms(同樣一次性)。
- 直接輸出顯示值的路徑(自訂材質、無燈光網格平光、VFX、舊背景)HDR 時也經過 ACES:整體中間調變暗、飽和的自發光(effects-tour M06 青色球)往白色壓縮。這是 tone mapping 的本質,文件已寫明;需要時調 `exposure`。
- Effects Atlas `mat/06-emissive` 預設沒開 `post`,所以 inventory 的「不宣稱 HDR bloom」仍為真,**沒有改**。
- 多相機:bloom / tone map 以每個 view 的矩形為 viewport 且取樣夾在 view 內;FXAA 對整個畫面做一次(view 邊界會被當成邊緣處理)。

**證據**(debug build、NVIDIA GeForce MX150 / Vulkan、隱藏視窗 1805×950、80 smoke 幀、每次隔離 DB `artifacts/w3c/…`):
- 預設關閉 A/B(改動前 exe vs 新 exe,逐像素):plaza、street、axiom-town、two-cameras、light-lab、effects-tour 總覽 / M06 自發光 / P03 粒子光源 / M20 postfx 共 9 張 **0 像素差**;fps 129 像素差,全在右上 HUD 數字框 (1712,45)-(1728,57)——舊 exe 自己連跑兩次同一區也差 141 像素,是 HUD 計數雜訊。
- 開啟:FXAA 單開時邊緣像素改變(plaza 36,945 / emissive 47,145 / two-cameras 11,818 像素),放大裁切可見角色頭部描邊的鋸齒變平滑;bloom(預設值)時 M06 格紋球的橘色在黑色縫隙中暈開,`intensity 1.2 threshold 0.7` 時青色球外圈有清楚光暈;HDR 讓青色球往白色飽和壓縮、背景天空較暗。two-cameras 兩個 view 各自 tone map、沒有互相光暈;M20 postfx 在 HDR + FXAA 下仍正確做通道偏移。
- 每個 pass 的 GPU 時間(timestamp):
  - 1920×1080 無頭量測(`hdr::tests::measure_post_passes_1920x1080`,8 px 棋盤格 = FXAA 最壞情況,299 幀中位數,debug test build;shader 執行與 build 型態無關):bloom(6 層)0.61 ms、tone map 0.48 ms、FXAA 2.44 ms。
  - 遊戲內(1805×950、smoke 74 幀平均,含啟動幀,GPU 與其他代理共用):bloom 0.69–2.8 ms、tone map 0.55–1.9 ms、FXAA 1.2–4.4 ms(同一場景兩次之間差到 2 倍,只能當量級)。沒有做 release 量測。
- Gate:`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` **856 passed / 0 failed / 6 ignored**(基準 848 / 0 / 5;新增 8 個測試 + 1 個 ignored 量測);`editor-smoke-test` ok:true;`editor-world-parity`、`street-editor-parity` identical:true;`m0-smoke-test` PASS;`verify_vfx_gpu` ok(max error 2.1e-6);`effects-smoke-test` 36 站截圖、live edit / 重試 / vertex pipeline 復原都過,最後 900 幀 / 60 秒那一步照已知情況在 MX150 逾時(`Effects native helper timed out or failed`,`live-errors.log` 無錯誤),沒有放寬。
