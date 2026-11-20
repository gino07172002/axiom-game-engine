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
