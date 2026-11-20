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

### W3a 執行補記

**做了什麼**

- A 音效檔:`audio.library.clips.ID` 可寫 `{"file":{"path":"sounds/x.wav|.ogg","gain":0..1}}`(不能同時寫合成欄位)。WAV 解析器自己寫(PCM 8/16/24/32-bit、float32、EXTENSIBLE、未知 chunk 跳過);OGG 用 `lewton`。單 / 雙聲道、8..192 kHz,串流降混成單聲道並重新取樣到 22,050 Hz(移動平均 + 線性內插),混音器與 voice 參數**沒改**。解碼走既有的程序片段快取(key = 檔案 SHA-256 + gain,32 MiB 預算不變),新增 package resolver(`spatial_audio::resolve_files`,和 shader 檔同一個機制):pack 相對路徑 → pack 內絕對路徑、限制在 pack 內、匯入時就解碼一次(壞檔 / 超預算 / 缺檔 / 逃出 pack = 帶 `/resources/audio.library/clips/ID/file` 與檔名的可恢復錯誤),執行期建立時命中快取。預算:檔案 16 MiB、解碼後 120 s、一個執行期用到的片段 PCM 合計 8,388,608 樣本。`observe audio.clips.files` 回報 clip / path / format / channels / rate / seconds / bytes / decode_ms;`audio.inspect.length` 是解碼後長度。Ogg 解碼器的 panic 會被攔下變成錯誤。
- 權益(parity):`ops::add_audio_file_clip` + ACL `audio clip ID file=PATH [gain=G]` + 編輯器 New clip… 對話框的 File 欄位 + Inspector 顯示檔名 / 長度 / 聲道 / 取樣率;parity 測試 `audio_sources_listeners_and_clips_match_acl`,`docs/editor-parity.zh-TW.md` 一列。
- B 手把:`input.bindings.gamepad {buttons, axes, deadzone}`,24 個輸入名稱(`PadA`…`PadRStickRight`,`axiom_core::PAD_INPUTS`),是**既有動作的第二組輸入**(種類必須相同);舊世界不變(欄位不存在就不序列化)。`ActionMap::from_input`:按鈕 ≥0.5 按下、軸加到鍵盤值並夾到 -1..1,沒有手把時與 `from_bindings` 完全相同。XInput 走 `windows` 0.58 的 `Win32_UI_Input_XboxController` feature(沒有新 crate),原生宿主每畫面在主執行緒輪詢一次,空槽每 1 秒才探測;測試 / 無頭 / scenario 不讀硬體。Scenario `inputs[].pad` 注入虛擬手把。FPS 新增 `look.x` / `look.y` 軸(3 rad/s);`content/fps` 與 `content/action` 加了手把綁定(FPS 另加方向鍵視角)。ACL `input pad ACTION PAD|POS NEG|off`、`input deadzone V`,Input Map 分頁 pad 下拉與 deadzone;parity `tuning_editor_edits_match_acl`。說明在新檔 `docs/input.zh-TW.md`(原本沒有輸入文件)。

**偏離**

- 檔案片段一律降混成單聲道(混音器只吃單聲道 PCM;改立體聲 voice 會動到 seqlock / 混音熱路徑)。2D 立體聲音樂會失去左右資訊 → 列為已知限制。
- 檔案增益是 `file.gain`(物件內),不是片段層級的 `gain`:片段層級的 `gain` 預設 0.5 且代表峰值正規化,語意不同。
- 不支援「只有手把」的動作(必須先有鍵盤綁定),這樣 interact / camera / behavior.program / scenario 檢查按鈕或軸是否存在的地方都不用改。
- 沒有 pack 的世界(`apply` / ACL)寫的相對路徑以工作目錄解析(和 `animation.library` 資產相同),不受 pack 限制。

**證據**(debug build,MX150 筆電,其他 agent 同時在編譯)

- `axiom.exe --db artifacts\w3a\lab.sqlite --pack content\audio-lab play --port 0 --start --smoke-frames 120 --screenshot ...`:`audio.voiced = 2`(hum.wav 左、chime.ogg 右,pan ±0.338),`clips.files` = hum wav 1 ch 22,050 Hz 1.0 s(decode 12.0 ms)、chime ogg 2 ch 44,100 Hz 2.0 s(80.2 ms);`output.device = open`、`rendered 208`、`underruns 0`、`voices_mixed 2`。聽不到聲音,只能看 observe。截圖確認三個喇叭方塊。
- 30 秒片段解碼時間(兩次各一個新程序):WAV float32 立體聲 44.1 kHz 10.6 MB = 946 / 1665 ms;Ogg Vorbis 立體聲 44.1 kHz 118 KB = 1054 / 1477 ms。解碼發生在載入 pack 的執行緒(CLI 啟動時,在事件迴圈之前)與 revision 啟用的背景執行緒;**不在每個畫面**,但 debug 下 30 s 檔案超過 16 ms,所以不要在主執行緒的互動路徑上匯入大音檔(release 未量測)。
- 手把:`gamepad` 觀測 `backend xinput`、`connected [false×4]`、`polls 120`、`poll_us_last 1`、`poll_us_max 366`(第一次探測 4 個空槽)。**實機手把測試:NOT RUN**(這台機器沒有接手把)。
- 無頭手把:`examples/fps-gamepad.scenario.json`(左搖桿右推 + RT)與鍵盤版 `fps-regression` 的 `state_hash` 相同;拿掉 `gamepad` 綁定後同一個 scenario 斷言失敗;右搖桿 0.5 s 轉 1.5 rad;動作遊戲左搖桿上推 30 步主角移動 > 0.5 m(`tests/gamepad.rs`)。
- 新 crate:`lewton` 0.10.2(MIT OR Apache-2.0)、`ogg` 0.8.0(BSD-3-Clause)、`tinyvec` 1.13.3(Zlib OR Apache-2.0 OR MIT);`byteorder` 原本就在 lock 裡。axiom.exe(debug)95,522,262 → 96,211,387 bytes(+689 KB,含本工作線全部程式碼與 XInput)。編譯時間:`lewton` + `ogg` + `tinyvec`(+ 已有的 `byteorder`)在空 target 目錄單獨建置 7.2 s(debug,依賴 opt-level 2;`byteorder` 單獨 1.4 s),所以新 crate 約 +6 s。`windows` 加 `Win32_UI_Input_XboxController` feature 會讓 `windows` → `wgpu-hal` → … → `axiom-runtime` 一次性重編;本工作線加完後的 `cargo build -p axiom-runtime` 花了 19 分 11 秒,但其他三個 agent 同時編譯,無法拆出單獨的增量(未隔離量測)。
- Gate:`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings` 乾淨;`cargo test --workspace --no-fail-fast` = **868 passed / 0 failed / 5 ignored**(基準 848 / 0 / 5;第一次全跑時 `app::tests::editor_scenario_runs_in_background…`(<250 ms)與 `m15::agent_timeout…`(<10 s)兩個計時測試在多 agent 同時編譯的負載下失敗,單獨重跑與第二次全跑都通過;另一個是我自己的重新取樣測試門檻寫錯,已修)。`scripts\editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` 與 `street-editor-parity.ps1` identical:true;`m0-smoke-test.ps1` PASS。

**已知限制 / 注意**

- 每次建立執行期(每個 revision 啟用)都會重讀檔案片段並算 SHA-256 來查快取(在背景啟用執行緒上;16 MiB 上限)。若要省掉,之後可改成以路徑 + 大小 + 修改時間做第一層 key。
- 匯入 pack 時的解碼在載入 pack 的執行緒上做;debug 下 30 秒檔案約 1–1.7 秒。
- 無頭 scenario 的手把每個步驟會配置一個 `ActionMap`(只有注入手把時;沒有手把的 scenario 路徑不變)。

### W3b 執行補記

**完成(C 關節與接觸事件)**
- `physics.joint`(`axiom-core/src/physics.rs`):fixed / revolute / prismatic / spherical,`to`(穩定 ID,省略 = 世界)、`anchor`(本地、乘縮放)、`axis`、`limits`(revolute 度 / prismatic 公尺)、`motor`(target_velocity / target_position / stiffness / damping / max_force)、`collide_connected`。嚴格驗證:未知欄位、範圍、兩邊必須是 `physics.body`、不能連自己、至少一邊動態、每個世界 256 個(`invalid_physics` / `physics_budget`)。框架取自編寫姿勢,不必寫對方錨點。
- 執行期(`axiom-runtime/src/physics.rs`):rapier impulse joint;被連的實體是 rapier 的 body 2,所以角度 / 位移 / 限位 / 馬達描述的是它自己的運動(第一次實作把它放在 body 1,測試抓到 prismatic 位移與馬達轉向都反號,已修正)。連到世界的關節共用一個固定剛體,只在需要時建立,沒有關節的世界模擬不變。`physics.inspect` 回報 `joint {kind,to,angle|offset,motor}`;新動作 `physics.set_motor`。
- `physics.collider` 新增 `sensor`、`on_enter`、`on_exit`(預設值不序列化,舊內容不變)。事件 payload 只能宣告文字欄位 `self` / `other`,沿用 trigger / nav 的 `RuntimeCommand::Emit` 機制與 world 驗證。每步在 rapier 步進後以 `contact_pairs_with` / `intersection_pairs_with` 計算,事件碰撞體與對方都依實體 ID 排序(與編寫順序無關),每步上限 64 個事件(超過停止事件並在 observe 說明)。感應器加上 KINEMATIC_FIXED / KINEMATIC_KINEMATIC 碰撞類型,靜態感應區看得到角色(跟隨者);不配對靜態對靜態。感應器不是相機阻擋、導航障礙或 FPS 掩體。
- Checkpoint:執行期改過的馬達目標、每個事件碰撞體的接觸名單(ID)、已送出數都存檔;缺少時明確拒絕;讀檔後不會重送已存在接觸的 `on_enter`。熱重載依 ID 保留接觸名單,編寫未變的關節保留馬達目標。
- 編寫:ACL `physics SEL joint KIND [to=] [anchor=] [axis=] [limits=] [collide=] [velocity=] [position=] [stiffness=] [damping=] [max_force=]`(`ops::parse_joint` + `ops::add_joint`),編輯器 `object.joint ARGS`(Object ▸ Add Joint…)共用同一個解析器;Add component / Inspector 表單由 schema 產生(`to` 是實體參照)。
- 示範包 `content/joint-lab`(只用引擎內建方塊與程序球體)+ `Play Joint Lab.cmd` + `examples/joint-lab.scenario.json`:被 path 角色推開、彈簧馬達拉回的鉸鏈門;四節鏈條 + 擺錘;角色與落球進出的感應區(事件計數進 `behavior.program` 變數)。文件:`docs/physics.zh-TW.md` 新增「關節」「感應器與接觸事件」「跟 trigger.zone 怎麼選」,限制段移除「沒有關節 / 觸發事件 / 碰撞事件」。

**完成(D 2D blend tree)**
- `Motion::Blend2d {parameters:[X,Y], points:[{at:[x,y],clip}], speed?, looping?}`,最多 9 點(8 方向 + 中心)。權重採 **simple directional**:選它是因為 8 方向移動正是其用途,O(點數) 角度查找加 2×2 解,最多 3 個樣本、不配置記憶體;超出外緣沿徑向夾到外緣,無中心時只看方向。驗證:兩個不同的 float 參數;方向 1 個或 ≥3 個且相鄰夾角 < 180°;同方向兩點、兩個中心拒絕。時間同步與 blend1d 相同(一圈 = 依權重加總的片段長度)。`MAX_SAMPLES` 4 → 6(兩個 2D 狀態淡入淡出)。2D 狀態的 observe 多 `blend`(其他控制器觀測不變)。
- ACL 說明、`animation.controllers` schema 文字、`ops_anim` 參數使用檢查(2D 使用中的參數不能刪)、編輯器 Animator 狀態表單(2D blend tree:X / Y 參數、點清單 x / y / 片段、+ Point 依序填 N、E、S、W、斜向、中心、x 移除)、節點副標題;`docs/skeletal-animation.zh-TW.md`、`docs/editor-parity.zh-TW.md` 兩列新增 / 一列更新。

**偏離**
- 關節的「連到對方錨點」不提供手寫欄位(自動由編寫姿勢求出);沒有可斷裂關節、多體關節。
- 實心(非感應器)碰撞體的事件沿用 rapier 預設碰撞類型:至少一邊是動態剛體才有接觸。
- 2D 混合只做 simple directional(freeform / cartesian 未做)。編輯器點清單的加入 / 移除以 `next_direction` 純函式與 parity 測試驗證,沒有寫 egui 點擊測試。
- 沒有量測 N 個關節的步進成本,因此不做效能宣稱。

**證據**(debug,本機 MX150)
- 新測試:`tests/joints.rs` 10 項(鉸鏈門被推開且鉸鏈點不動、限位;球窩鏈不分離且擺動;fixed 抗重力、prismatic 只沿軸且限位;馬達、`set_motor`、checkpoint 往返與缺狀態拒絕、熱重載;感應器進出各一次且球穿過落地;靜態感應區看到運動學角色、同一步兩球依 ID 排序;讀檔 / 熱重載不重送;joint-lab scenario 兩次 state_hash 相同;編寫錯誤;感應器不是相機阻擋)。`controller.rs` 3 項 2D 權重 / 時間同步 / 驗證單元測試;`tests/animator.rs` 2 項(mascot 第一方片段的 8 方向 + idle 權重、scenario 重播雜湊相同、穩定步進配置數等於無播放器基準);parity `joints_and_sensors_match_acl`、`blend2d_states_match_animctl`。
- GPU 截圖 `axiom.exe --pack content\joint-lab play --start --smoke-frames 240/480 --screenshot`:240 幀時角色正推著門、門繞門柱轉開,鏈條與擺錘斜向擺動且各節相連,球停在綠色感應區內的地上;480 幀時門被馬達拉回中途、鏈條擺到另一側。observe:`joints 6`、`contact_events.emitted 3`(tick 79 角色進入、tick 90 球進入、tick 306 角色離開)、`touching {zone:[ball]}`。
- Gate(debug):`cargo fmt --all -- --check` 通過;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` **865 passed / 0 failed / 5 ignored**(基準 848 + 新增 17)。中途一次全量執行時 `app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`(500 ms 牆鐘門檻)在機器滿載時失敗,單獨重跑通過,最終全量執行也通過。`scripts\editor-smoke-test.ps1` ok:true;`editor-world-parity.ps1` 與 `street-editor-parity.ps1` identical:true;`build-street.ps1` / `build-plaza.ps1` 之後 `content/street`、`content/plaza` 無變化;`m0-smoke-test.ps1` PASS。

### W3d 執行補記

- **做了什麼**:`render.light` 的 spot / point `shadows: true` 可用(area 仍拒絕)。spot = 一張 1024² 透視陰影貼圖;point = cube 陰影(6 個 90° 透視面,每面 512²,存成 2D 陣列層,shader 依主軸選面);3×3 PCF,法線 + 朝燈光方向的線性偏移。新檔 `shadow_local.rs`(投影燈光挑選、矩陣、GPU 貼圖 / pass、observe)、`shadow_local.wgsl`、`shadow_local_none.wgsl`(VFX 粒子不接收);共用檔只做小改:`lighting.rs`(pack 填 `params.zw`)、`lighting.wgsl`(局部光乘可見度)、`render.rs`(globals buffer 多 8 個槽、bindings 3–5、陰影面 pass、`scene_bind` helper)、`mesh_render.rs`(camera group 多 3 個 binding、`draw_shadow` 帶 offset)、`gpu_timing.rs`(新區段 `local_shadows`)、`app.rs`(observe `local_shadows`、`AXIOM_GPU_TIMING=1` 的 Play 計時)。
- **預算**:`World::compile`(`light::validate_budget` 改收 `(kind, shadows)`)最多 2 spot + 1 point 投影;`RenderLight::validate` 拒絕投影 spot 的 `outer_degrees > 160`。錯誤是可恢復的交易錯誤(`light_budget`)。renderer 另外在執行期上限(編輯器草稿預覽不經 compile),超出的燈光照明不投影並計入 `observe` `local_shadows.ignored`。
- **選擇 cube 而非雙拋物面**:雙拋物面的非線性投影只在頂點上做,本引擎多為低面數方塊(大三角形),陰影邊會錯位;cube 6 個透視面是線性的。量到 point 的 6 個 512² 面 GPU 時間和 1 張 1024² spot 同一量級(見下),沒有理由為省 pass 而接受錯誤的陰影。
- **偏離 / 限制**:沒有新增 `shadow_resolution` / bias 欄位(固定解析度,所以不需要新 ops / ACL / parity test,`docs/editor-parity.zh-TW.md` 加了說明列);不偵測靜態場景,有投影燈光時每幀重畫(正確偵測需比對全部 instance、網格變形、燈光矩陣,成本與風險都不低,量到的 pass 時間 0.1–0.3 ms);沒有逐面剔除(大場景每面畫全部投影物);陰影貼圖第一次需要時才配置(spot 8 MiB + point 6 MiB),之後不釋放;VFX 粒子不接收也不投射局部陰影;timestamp 是 pass 之間的時間點;新增 QA 環境變數 `AXIOM_GPU_TIMING=1`(只影響沒有編輯器的 Play,最後 JSON 多 `gpu_timing`)。`effects-smoke-test` 最後 900 幀 / 60 s 步驟在這台 MX150 逾時(已知,未放寬);前面 36 站截圖、live edit、retry、vertex pipeline 復原都通過。
- **證據**:新內容包 `content/shadow-lab`。1920×1080 GPU 截圖(debug):shadows on —— 聚光燈下柱子與骨架角色在地上投出陰影,點光源讓柱子、兩個箱子投出放射狀長影;off —— 同場景沒有任何局部陰影。牆測試(俯視牆兩側):off 時牆後地面有暖色光暈、牆後箱子頂面被照亮;on 時牆後地面與箱子全暗,只有牆端以外的地面受光。預設內容 A/B(改動前 exe vs 改動後,1920×1080,60 幀):plaza、street、light-lab 0 像素不同;fps 只有右上角 FPS 數字區 151 像素(每次執行都會變)。`observe` 回報 `local_shadows {casters:[spot/key spot, point/lamp point], faces:7, ignored:0}`;關掉陰影後該欄位消失。
- **量測**(debug,MX150,1920×1080,GPU timestamp,`content/shadow-lab`,4 設定交錯 3 回合,每次 295 幀;同時有其他代理使用 GPU):`local_shadows` 中位數 spot + point 0.34 ms、只 spot 0.14 ms、只 point 0.24 ms、關閉 0;world pass 回合間差到 3 倍,取樣端成本無法宣稱。和 `docs/lighting.zh-TW.md` 先前的牆鐘量測(2048² 太陽陰影約 +0.9 ms / 幀,rts-bench)方法不同,不直接比較。沒有量 release。
- **Gate**:fmt 乾淨;clippy `-D warnings` 乾淨;`cargo test --workspace --no-fail-fast` 855 過 / 0 失敗 / 5 ignored(基準 848 + 7 新測試);m0-smoke PASS;editor-smoke ok:true;editor-world-parity identical:true(1→13);street-editor-parity identical:true(8→16);effects-smoke 見上(已知逾時)。

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

#### W3c 追補:後處理 pipeline 改在背景建立(分支 `w3c-async`)

- **問題**:第一次開 HDR / bloom / FXAA 時,14 條 HDR 世界 pipeline、5 條後處理 pipeline 與材質 HDR 版本在主執行緒同步建立(之前量到 28–546 ms),違反 16 ms 規則。
- **做法**:新檔 `hdr_build.rs`,用既有的 `jobs::Job`(一次一個 job、每組 pipeline 之間檢查取消、done/total 進度、30 秒 watchdog、關閉時限時 join)。wgpu 24 的 `Device` / layout / module 都是 `Clone`,直接複製到 worker。建好之前畫面走沒有後處理的舊輸出,全部建好的那一幀一次切換;設定在建置中改變 → 停止目前 job(已建好的保留)、結束後用最新設定再開(不排隊);關掉後處理也會取消。失敗 / 逾時 → `render_post.build.state = failed` + `error`、編輯器狀態列紅字,設定改變前不重試。無頭 `--screenshot` 只有截圖那一幀會等(上限 watchdog)。
- **順帶發現**:主執行緒上一次 `eprintln!` 在隱藏視窗 + stderr 導向檔案的 headless 跑法量到 150–290 ms(比整個建置還久),所以完成紀錄改在 worker 印。主執行緒的其他 `eprintln!`(例如截圖交付的紀錄)可能有同樣問題,這次沒有改。
- **量測**(debug、MX150、plaza、headless 1805×950、執行中用 ACL 開 `post antialiasing fxaa` + `post bloom on`,`render_post.build` 讀數 = `Renderer::render` 在主執行緒的毫秒,不含等 swapchain):
  - 改動前(05cba45):開啟那一幀在 `render()` 裡同步建立 pipeline:HDR 世界 24.3 / 25.9 / 25.0 ms + 後處理 6.1 / 5.8 / 5.6 ms(3 次;這幾次驅動的 pipeline 快取是熱的,冷快取時之前量到最多 546 ms),再加上平常的那一幀。
  - 改動後:要求的那一幀 1.47 / 2.92 / 1.40 ms(前一幀 1.53 / 2.08 / 1.35 ms);切換的那一幀 7.9 / 4.9 / 4.5 ms(含建立 25 MB 目標與第一次 HDR 繪製);worker 花 67 / 83 / 69 ms;期間 2–4 幀照舊輸出。沒有做 release 量測。
- **證據**:預設關閉時 plaza、street、light-lab、render-lab、shadow-lab 與 05cba45 的 PNG **檔案位元組完全相同**;shadow-lab / plaza 只跑 3 個 smoke 幀就截圖、post 開啟,截圖仍是 HDR + bloom + FXAA 的畫面(截圖幀等到建置完成)。Gate:fmt、clippy `-D warnings` 通過;`cargo test --workspace` 903 passed / 0 failed / 6 ignored(新增 `hdr_build` 3 個測試:取代 / 取消 / 失敗不重試);editor-smoke ok:true;editor-world-parity、street-editor-parity identical:true;m0 smoke PASS。

## 整合補記(2026-11-20,整合者)

**合併順序與結果**(每次合併後在合併樹上跑完整 gate:fmt、clippy `-D warnings`、`cargo test --workspace --no-fail-fast`、editor-smoke、兩個 parity、m0 smoke):

| 合併 | commit | 測試 | 其他 |
|---|---|---|---|
| W3a 音效檔 + 手把 | 06e1c72 | 868 / 0 / 5 | 自跑 `content/audio-lab`:`voiced 2`(hum.wav 1 ch 22,050 Hz 1.0 s、chime.ogg 2 ch 44,100 Hz 2.0 s)、`underruns 0`,截圖正常(聲音本身無法聽) |
| W3b 關節 / 事件 + blend2d | 4b98aed | 885 / 0 / 5 | 自跑 `content/joint-lab` 480 幀截圖:鏈條各節相連擺動、角色推開門、球停在感應區;observe `joints 6`、`emitted 3`、`touching {zone:[ball]}` |
| W3d 聚光 / 點光陰影 | bdd9cb7 | 892 / 0 / 5 | 合併前後 exe 的 plaza、street、light-lab、render-lab PNG **位元組相同**;shadow-lab 開 / 關陰影截圖親自比對(柱子、角色、箱子投影,牆後轉暗) |
| W3c FXAA / HDR / bloom | 05cba45 | 900 / 0 / 6 | 合併前後 exe 的 plaza、street、light-lab、render-lab、shadow-lab、audio-lab PNG **位元組相同**;shadow-lab + plaza 開 HDR + bloom + FXAA 截圖親自看(局部陰影保留、對比提高、無破圖);observe 同時有 `local_shadows` 與 `render_post` |
| W3c 追補(背景建 pipeline) | ce7032f | 903 / 0 / 6 | 預設關閉時五個 pack 的 PNG 與 05cba45 位元組相同;只跑 3 幀的 shadow-lab post 截圖已是完整 HDR 畫面 |

- **W3d × W3c 衝突解法**:`gpu_timing.rs` 區段合成 11 個(local_shadows + bloom / tonemap / fxaa),timestamp 每幀最多 47 個(上限 48);`app.rs` 的 observe 同時寫 `local_shadows` 與 `render_post`;沒有編輯器的 Play 計時器在 `AXIOM_GPU_TIMING=1` 或任一後處理開啟時啟用,同一個 frame 同時累計 `gpu_timing` 與 `render_post.gpu_ms`;`render.rs` 先畫局部陰影面,再依 W3c 的規則選世界顏色目標。HDR 世界 pipeline 用與一般版本相同的 `pipeline_layout`,所以含 W3d 的 bindings 3–5。
- **16 ms 規則**:W3c 原本第一次開 HDR 在主執行緒同步建 pipeline(28–546 ms),整合者退回請 W3c 追補改背景建立後才合併。整合者自己量(debug、MX150、shadow-lab、post 從啟動就開、120 幀,2 次):要求幀 12.0 / 12.0 ms、切換幀 10.1 / 9.1 ms、worker 32 ms、期間 7 幀走舊輸出。只跑 3 幀就截圖時切換幀 233.8 ms——那是 headless 截圖幀刻意等建置(只有 `--screenshot` 會等),不是互動路徑。沒有 release 量測。
- **effects-smoke**:四條合併後(05cba45)跑一次:36 站截圖、live edit、粒子預算診斷、vertex pipeline 失敗保留 + 復原都過,最後 900 幀 / 60 s 步驟照已知情況在 MX150 逾時(第 112 行),未放寬;抽看 P03 粒子光源、M06 自發光截圖正常。追補合併後(ce7032f)再跑一次:**全部 PASS**(含最後 900 幀步驟,`effects-20261120-191330-488/report.json`)——這一步的逾時一向隨機器負載而定,不代表變快。
- **未驗證**:實機手把(沒有手把)、聲音實際聽感、release 下的音檔解碼與後處理時間、N 個關節的步進成本、主執行緒其他 `eprintln!` 在 stderr 導向檔案時的延遲(W3c 追補發現,未處理)。
