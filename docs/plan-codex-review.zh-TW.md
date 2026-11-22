# Codex 分項審查計畫(2026-11-21)

來源:使用者要求「將引擎功能分項後依序給 Codex review,包括 1. 一般性 code review 2. AI 易用性 3. 人類接手可行性 4. AI 與人機協作,review 後對有問題的地方適當修改」。

## 模型

使用者指定 gpt-sol 6-2 high。舊的 npm 版 codex-cli 0.159.3 對 `gpt-6.2-sol` 回 400(not supported with a ChatGPT account),先用 `gpt-6.1-sol` 跑了 R1(保留為 `R1.gpt-6.1-sol.md`)。使用者重裝 Codex 後(`%LOCALAPPDATA%\Programs\OpenAI\Codex\bin\codex.exe`,codex-cli 0.171.0)`gpt-6.2-sol` 可用,**R1–R10 全部以 `gpt-6.2-sol` + `model_reasoning_effort=high`(唯讀 sandbox)重跑**,正式報告是 `R<n>.md`。

## 分項(依序審查)

| # | 分項 | 範圍 |
|---|---|---|
| R1 | 編寫核心與協定 | `crates/axiom-core`(IR、ops*、ACL、registry、store、schema)、`crates/axiom-cli` |
| R2 | 執行期主機與生命週期 | `axiom-runtime/src/{app,world,jobs,bridge,writer,checkpoint,scenario,events,asset_reload,benchmark,lib}.rs` |
| R3 | 渲染 | `render, lighting, shadow_local, hdr, hdr_build, mesh_render, mesh_style, shader, shader_gpu, postfx*, vfx*, gpu_timing, screenshot, vram, geometry` 與 `.wgsl` |
| R4 | 動畫 | `crates/axiom-animation`、`axiom-runtime/src/animation*.rs` |
| R5 | 物理、戰鬥、導航與通用玩法系統 | `physics, nav_agent, path_follow, trigger, interact, camera_rig, fsm, tween, combat`、`crates/axiom-combat` |
| R6 | 音效與輸入 | `audio, audio_mix, audio_file, spatial_audio, input, gamepad` |
| R7 | 類型玩法與 UI | `crates/axiom-fps` + `src/fps/`、`crates/axiom-rts` + `rts*.rs`、`crates/axiom-action` + `action.rs`、`narrative, gameplay, bound_ui, ui, ui_effects, rhythm, showcase, nameplate, presentation` |
| R8 | 編輯器 | `axiom-runtime/src/editor/` |
| R9 | 打包與玩家執行檔 | `packaging.rs`、`crates/axiom-player`、`editor/build.rs` |
| R10 | 文件、腳本與接手入口 | `AGENTS.md`、`README*`、`docs/`、`scripts/`、`examples/`、`WORKLOG.md` |

每項的四個面向:
1. **一般性 code review**:正確性、錯誤處理、資源 / 效能(16 ms 規則、每幀配置)、安全(路徑逃逸、不受信任輸入)、測試缺口。
2. **AI 易用性**:AI agent 只靠 CLI / ACL / observe 能不能發現、操作、驗證這個功能;錯誤訊息是否可恢復且可行動;輸出是否精簡。
3. **人類接手可行性**:結構、命名、模組邊界、文件是否足以讓新的人類開發者理解與修改;隱性慣例。
4. **AI 與人機協作**:AI 與人類同時編輯時的一致性(交易、parity、undo、衝突)、可觀測性、交接紀錄、可稽核性。

## 流程

1. 每項用 `codex exec -m gpt-6.1-sol -c model_reasoning_effort=high -s read-only`,報告存 `docs/reviews/codex-2026-11/R<n>.md`(原文保留)。
2. 整合者逐條讀 code 驗證 finding(沒讀過的 code 不算),分成 CONFIRMED / 不成立 / 需要更大改動(延後並說明)。
3. CONFIRMED 且範圍適當的,走 `rigorous-change` 修改;每批修改後跑 gate(fmt、clippy `-D warnings`、`cargo test --workspace`、editor-smoke、兩個 parity、m0 smoke;渲染改動加截圖 A/B)。
4. 每項在下方「處理紀錄」寫:finding 數、確認數、修了什麼(commit)、沒修什麼與理由。

**Non-goals**:大型重構(拆 `app.rs` 等)只記錄建議不在本輪做;不加新功能;不碰 `data/` 與另一個 session 的 Paladin 檔案。

## 處理紀錄

### R3 渲染

15 項 finding 全部確認屬實(不成立 0):修 14 項、DEFERRED 1 項(F3,問題屬實但需要較大改動)。每項都讀過引用的程式碼;F1 / F10 / F11 另以 base `6a9e715` 的 exe 實際重現。

| ID | 判定 | 改了什麼 / 為何不改 | 測試 |
|---|---|---|---|
| F1 相機 viewport panic | CONFIRMED(重現:base exe 對 `[0.5,0,0.500005,1]` 在 `set_viewport` panic,`Viewport has invalid rect … w: 902.50903`) | 新增 `render::pixel_rect`(共用 `hdr::viewport`,再把浮點相加超出時往下修一個 ulp):world / VFX pass 的 viewport 與 scissor、投影 aspect、`sky_viewport`、postfx uniform 與 pass 全部用同一個矩形 | `render::tests::camera_rects_stay_inside_odd_and_even_targets`;GPU:packs/f1 base panic、new 正常出圖 |
| F2 shader 預檢與實際模組不同 | CONFIRMED | `shader::material_module` 是 GPU 與 CPU 共用的組裝(`shader_gpu` 改呼叫 `material_source`);`validate_hook` 在原探針之後再以 Naga 驗證 cube 與 mesh 兩個完整模組,位置在 hook 內時換算成 hook 行號,訊息註明是哪個模組、引擎函式名稱保留 | `tests/shaders.rs::shader_preview_rejects_hook_names_reserved_by_the_engine_templates`(`scene_shadow`,preview 失敗、rev 不變、line 2)、`shader::tests::assembled_module_errors_point_into_the_hook` |
| F3 材質編譯在互動執行緒 | DEFERRED | 現況:`App::adopt` 以 `prepare_shaders` 的同步結果當啟用閘門(失敗 = 保留舊 runtime),且 wgpu 24 的 error scope 是 per-device:worker 上建的管線若有驗證錯誤,不是被主執行緒的 scope 收走就是進 uncaptured handler(panic)。`hdr_build` 能在 worker 編 HDR 變體,是因為同一 hook 已在主執行緒的 scope 下編過。要搬:(1) adopt 拆成「CPU build → GPU stage job → adopt」三段(改 `app.rs`,R2 範圍);(2) 管線驗證改靠 F2 的 Naga 完整模組驗證 + 在 worker 上用單一序列化的 scope 擁有者(主執行緒在 job 期間不得 push scope),或 `device.on_uncaptured_error` 轉成 job 錯誤;(3) job 有界、可取消、每個變體回報進度、watchdog,只安裝仍對應最新 revision 的完整結果。`observe shader_gpu.last.prepare_ms` 已可量(effects-tour debug 冷啟動 1745 ms / 12 programs,只是觀測值,非 A/B) | — |
| F4 VFX 遮罩每顆粒子重掃 | CONFIRMED | `mask_pixels` 在 factory 建一次存在 `Running.mask`;`extract` 與 `inspect` 用 `evaluate_masked`;`evaluate` 保留為公開便利函式。光源 emitter 直接沿用已算好的粒子(不再算第二次)。順序與數值逐位元相同。未宣稱幀時間改善(沒有 A/B 量測) | `vfx::tests::a_prebuilt_mask_table_matches_the_per_particle_scan`(64×64、2048 顆,bytes 相等) |
| F5 截圖逾時後讀回交錯 | CONFIRMED | `screenshot::next_id`;`Loopback` 待處理請求帶 id,`GrabOwner::{Loopback,Editor}` 記住 id,`grabbed(id, …)` 只交給同 id 的請求,否則丟棄。編輯器 `view.screenshot`(`look.rs`)同樣問題一併修 | `screenshot::order_tests::a_late_read_back_never_answers_the_next_request`、`look::tests::a_screenshot_crops_the_scene_view_in_a_background_job`(加 stale id) |
| F6 程序幾何互斥只在啟用時檢查 | CONFIRMED | `geometry::ownership_conflict` 為純函式,factory 與新的 authoring world validator 共用;啟用 `geometry.prepare` 時,對合併 prefab 後的實體回 `geometry_ownership` 於 `/entities/@ID/components/render.geometry`,訊息說要移除哪個元件 | `tests/effects.rs::procedural_geometry_ownership_is_checked_at_authoring_time`(直接、經 prefab、缺 transform) |
| F7 `editor_view {capture}` 同步讀回 | CONFIRMED | 改排入 `shots`(與 loopback `screenshot` 相同的非同步讀回 + PNG job),移除 `pending_capture`;同步讀回只剩 `--smoke-frames` 最後一幀。回覆欄位不變(`capture_ready` 文字更新);名稱型 capture 仍可覆寫。非同步後一次擷取佔用槽位約 3 s(debug、機器高負載:map + 編碼),parity 腳本相隔約 4 s 的第二張曾被拒(`capture_crossing` / `capture_resource` 為 null;舊同步路徑從不拒絕)→ 加 `Loopback::enqueue`:`editor_view` 擷取在槽位忙時排隊(上限 4,輪到時重算 10 s 等幀期限),loopback `screenshot` op 仍一次一個 | `screenshot::order_tests::queued_captures_start_when_the_slot_frees`;兩個 parity 腳本的兩張 capture 都要非 null(見 gate) |
| F8 截圖覆寫既有檔 | CONFIRMED | `write_crop` 先寫同目錄暫存檔再發布:`overwrite` 時 rename,否則 hard link(已存在即失敗)。明確路徑(loopback `screenshot`、`view.screenshot PATH`)不覆寫,在請求時與寫入時都檢查;`artifacts/captures/NAME.png` 型仍覆寫(腳本重跑依賴)。失敗不留暫存檔 | `screenshot::order_tests::explicit_paths_keep_existing_files_and_writes_are_atomic`、`one_capture_at_a_time…`(既有檔被拒) |
| F9 每個掛載 emitter 重建世界矩陣 | CONFIRMED | `VfxSystem::origin` 改收呼叫端的 lazy `Option<Vec<Mat4>>`:每次 `step` / `extract` 最多建一次、只在有實體掛載且沒有 bone socket 時建;`step` 只對未暫停的 ribbon 取位置。未宣稱幀時間改善 | 行為由既有 `bone_trails_restart_pause_and_hot_reload_preserve_unmodified_effects` 等覆蓋(配置次數無法從外部觀察,未另寫測試) |
| F10 後畫天空蓋住遠物 | CONFIRMED(重現:packs/f10 的 190 m 牆在 base 消失) | `vs_sky` z 改 1.0(legacy 天空是 `Always` 不寫深度,不受影響) | `render::tests::the_sky_triangle_sits_on_the_far_plane`;GPU A/B f10 |
| F11 色差 / 熱扭曲取樣到別的相機 | CONFIRMED(重現:packs/f11 右視圖左緣 44 px 藍條) | `postfx.wgsl` `in_view`:偏移取樣限制在本相機 viewport 內、內縮半 texel | `postfx_gpu::tests::offset_samples_stay_inside_the_camera_viewport`;GPU A/B f11(藍條消失) |
| F12 HDR 未就緒仍輸出 smoke PNG | CONFIRMED | 等待過後仍缺管線時 `render` 回 `render_post_not_ready` 診斷(含 `render_post.build` 狀態),不寫 PNG、程序非零結束 | 未寫自動測試(需要讓建置逾時的 GPU 裝置);以程式碼路徑確認,本機 smoke 都正常就緒 |
| F13 ribbon 忽略曲線 | CONFIRMED | 採「讓欄位生效」而非拒絕(拒絕會讓已存的世界無法載入):`Ribbon` 帶 `size_curve` / `color_gradient`,沿 trail(頭 t=0、尾 t=1)決定寬度 / 顏色;沒有曲線時與原本逐位元相同。schema 文字說明 ribbon 用法 | `vfx::tests::ribbon_curves_style_the_trail_and_are_optional` |
| F14 被擠掉的 VFX 燈光無指標 | CONFIRMED | renderer 記錄最後一個 world frame 的 `(local_count, dropped)`;`observe` `metrics.lighting = {local_lights, dropped, capacity}`(有燈時才出現)。`app.rs` 加 3 行 | `render::lighting_metrics_tests::dropped_local_lights_are_reported_with_the_capacity` |
| F15 vertex hook 字串替換可能零命中 | CONFIRMED | 注入行與 ABI stub 改為具名常數,`inject_once` 要求恰好一次命中(模板是編譯期常數,命中錯誤 = 引擎 bug,測試即時失敗) | `shader::tests::material_modules_route_both_vertex_entries_through_vertex_offset`(Naga 走訪 `vs_main` / `vertex_main` 確認呼叫 `vertex_offset`)、`a_template_without_the_injection_line_is_an_engine_error` |

範圍外的檔案:`app.rs`(F5 `GrabOwner` id、F7、F14,共約 30 行)、`editor/look.rs`(F5 / F8 編輯器截圖)、兩個腳本(R10 範圍):`shader-smoke-test.ps1` 與 `effects-smoke-test.ps1` 原本把 F2 的缺陷當成預期行為(「hook 單獨可編譯、與 GPU wrapper 的 `vs_main` 衝突」/「vertex hook 用 `dpdx`」必須通過 CPU 檢查、在 GPU 啟用時失敗並保留舊 revision)。F2 之後這兩個來源在 preview / apply 就回 `invalid_shader`(路徑為該 shader 的 wgsl),所以改成斷言「提交前被拒、rev 不變、activation 仍 applied」;shader-smoke 的 undo 改為回到 `$initial.rev`。代價:經由 authoring 已無法再觸發 `shader_pipeline` 啟用失敗,該保留路徑不再有 GPU 端對端覆蓋(通用的 activation 失敗保留仍由 `tests/generic_runtime.rs` 覆蓋)。

**截圖 A/B**(debug,MX150 / Vulkan,1805×950 隱藏視窗,`play --smoke-frames 120`,每個世界獨立 DB、`APPDATA` 指向工作區暫存):base = `6a9e715` 原始碼另建的 exe。base 自己跑兩次就不逐位元一致(右上角統計文字 1620–1791 × ~100–127 與 render-lab 1 階雜訊),所以以「new 與任一次 base 逐位元相同」判定:plaza、street、light-lab、render-lab、shadow-lab、two-cameras 全部逐位元相同。effects-tour:base 兩次、new 兩次各自穩定,new 與 base 差 210 px(204 px 差 1 階、3 px 差 2–3、2 px 差 22 / 46,位於 M14 高密度位移材質的邊緣);推測是所有材質模組都包含 `SCENE_WGSL`(`vs_sky` 常數改變)而由驅動重新編譯造成的浮點差,未以單獨重建隔離驗證。刻意改變的畫面:f1(base panic → new 出圖)、f10(遠牆出現)、f11(接縫藍條消失),圖都看過。

**Gate**(debug,MX150,其他 agent 同時建置、機器高負載):
- `cargo fmt --all -- --check` 乾淨;`cargo clippy --workspace --all-targets -- -D warnings` 0(最後一次在 enqueue 修改後重跑)。
- `cargo test --workspace --no-fail-fast`:915 passed / 1 failed / 6 ignored。失敗的是本輪新增的 `procedural_geometry_ownership_is_checked_at_authoring_time`(斷言用了未 escape 的實體 id,測試本身的錯);修正後 `-p axiom-runtime --test effects` 13 passed。之後新增的 `queued_captures_start_when_the_slot_frees` 連同 screenshot / look 測試 8 passed。
- `editor-smoke-test.ps1` ok:true;`m0-smoke-test.ps1` PASS(這兩個是在 enqueue 修改前的 exe 跑的;enqueue 只影響 `editor_view {capture}`)。
- `editor-world-parity.ps1` identical:true(1→13)、`street-editor-parity.ps1` identical:true(8→16),兩者兩張 capture 都寫出(enqueue 修改後;修改前第二張為 null,見 F7)。第一次全跑時 world-parity 一步回 `runtime command response timed out`;同一時間用 base exe 跑 street-parity 也出現同樣逾時,屬負載問題,單獨重跑通過。
- `cargo run -p axiom-runtime --example verify_vfx_gpu`:ok,15 cases / 960 particles,max abs error 2.1e-6。
- `effects-smoke-test.ps1`:36 站擷取、material local edit / receipt、budget rejection、vertex 拒絕(新斷言)、observe_delta 都通過,停在第 112 行最後的 900 幀 / 60 s native helper 等待(MX150 上已知會逾時,未放寬)。
- `shader-smoke-test.ps1 -SkipEarlier`:新斷言(第 57–59 行)通過,停在第 61 行 480 幀 live 程序的 60 s 等待;手動量同一命令 base exe 165 s、new exe 95 s(兩者 exit 0;只說明這台機器目前跑不進 60 s,不是效能宣稱),shader-lab 的 480 幀截圖 base / new 逐位元相同。
- 未驗證:F12 沒有自動測試(要讓 HDR 建置逾時的 GPU 環境);F9 的「每次最多一份矩陣」沒有配置次數的測試;F4 / F9 沒有幀時間 A/B,也沒有宣稱加速。
