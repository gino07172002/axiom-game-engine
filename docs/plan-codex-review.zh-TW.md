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

### R4 動畫

範圍:`crates/axiom-animation`、`axiom-runtime/src/animation*.rs`。15 項 finding:14 項 CONFIRMED 並修改,1 項延後(F9)。沒有改 axiom-core、CLI、app.rs 或渲染檔案。測試輸入都在程式碼裡產生,沒有加入任何第三方資產。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED | glTF image `bufferView` 的結束位置改用 `checked_add`,錯誤訊息帶 image 與 view 索引;貼圖解碼錯誤加上 `glTF image N` context | `import_limits::a_gltf_image_view_whose_end_overflows_is_an_error`(offset = u64::MAX) |
| F2 | CONFIRMED | `import::read_texture_file` 最多讀 32 MiB + 1 byte;`import::push_texture` 讓 PMX、glTF、AXRIG 共用 256 MiB 累計解碼預算;錯誤帶貼圖路徑 | `import_limits::pmx_textures_share_the_decoded_texture_budget`(5 張 4096×4000)、`an_oversize_pmx_texture_is_refused_with_its_path` |
| F3 | CONFIRMED | `crossings` 改從 `prev` 所在的那一圈開始計數(不再用絕對圈數 `u32`,所以不會飽和或溢位);`seek` 依 looping 把時間 wrap 或 clamp | `animation::review_tests::marker_crossings_after_a_far_seek_into_a_tiny_loop_do_not_overflow`、`animation_review::seeking_far_into_a_short_loop_keeps_stepping` |
| F4 | CONFIRMED | `transfer` 只在新 revision 仍有舊的 `active` 片段時才延續 time、marker 與 active;否則照作者設定從 0 開始 | `animation_review::a_revision_removing_the_clip_playing_at_runtime_still_activates` |
| F5 | CONFIRMED | 檢查移到 authoring 驗證(preview、apply、編輯器、ACL 共用同一個 world compile,parity 不變):`animation.player.clip` 必須是 clip 來源資產(有 retarget 時是 source asset)或 library 的片段,否則回 `unknown_clip` 診斷,路徑 `/entities/@ID/components/animation.player/clip`,列出最多 16 個可用片段。片段清單由 `import::clip_ids` 取得,只解析 glTF JSON chunk / `.axrig` 的 clip id,不做完整匯入;以 path + 長度 + mtime 快取。檔案讀不到或格式未知時交給 runtime 建置回報 | `animation_review::a_misspelled_player_clip_fails_validation_not_activation`、`import_limits::clip_ids_list_gltf_animations_without_importing` |
| F6 | CONFIRMED | `check_command` 的 `inspect` 分支與命令共用 `Controller::check_bones` | `animation_review::a_batch_with_a_misspelled_inspect_bone_changes_nothing` |
| F7 | CONFIRMED | 熱重載只在 `config.playing` 沒被改時才沿用 runtime 的 `playing`(比照 speed / looping) | `animation_review::an_edited_playing_flag_applies_on_hot_reload_and_undo`(含 undo、無關編輯保留 runtime 暫停) |
| F8 | CONFIRMED(部分) | `extract` 透過 `SkinCache` 保存上次的 pose、morph 權重與 `Arc` 頂點;兩者都沒變時重用同一份快照,clone 出來的 player 從空快取開始。`deform_with` 的骨矩陣暫存沒有改。**沒有做 A/B 量測,不宣稱效能改善**;代價是每個 player 多常駐一份 skinned 頂點 | `animation_review::an_unchanged_pose_reuses_the_skinned_vertices`(`Arc::ptr_eq`;morph 改變後重新 skin) |
| F9 | DEFERRED | 確實存在(依賴檔只比對長度與 mtime)。改成雜湊整個目錄的內容,每次啟用 revision 都要在主執行緒讀可能數百 MB 的檔案,違反 16 ms 規則。正確做法需要 importer 回報實際讀取的依賴清單(`Asset` 新欄位與三個 importer)、快取值存依賴的內容雜湊、熱重載 watcher 共用同一份清單,超出本輪範圍 | — |
| F10 | CONFIRMED | controller 驅動的 player 拒絕 `seek`,錯誤訊息指向 `animation.param`;schema 說明適用範圍 | `animator::commands_on_the_wrong_kind_of_player_fail_with_reasons`(新增 seek 段) |
| F11 | CONFIRMED | scrub 快取與 `Scrub` 同時保存 pose 與 morph 權重,擷取前後成對交換 | `animation::review_tests::a_timeline_scrub_draws_the_clip_morph_weights_with_its_pose`(character-lab `face` / `pulse`) |
| F12 | CONFIRMED | `ray_capsule` 改為有限圓柱側面與兩端球體分別求交,取最小有效距離;方向向量在函式內正規化 | `animation_parts::tests::oblique_rays_enter_the_capsule_side_and_rank_overlapping_parts` |
| F13 | CONFIRMED | `Controller::play` 清除 `ramp`(直接命令、stage、cue 走同一條路徑) | `animation_review::play_drops_the_speed_ramp_of_the_previous_playback` |
| F14 | CONFIRMED | `animation.inspect` 預設只回 `part_count`,加 `parts:true` 才列出完整清單;`part_names` 最多 16 項 / 1 KiB,其餘只報數量並提示 `parts:true`。`character.rs` 與 `docs/skeletal-animation.zh-TW.md` 已同步 | `animation_review::inspect_returns_a_part_count_unless_parts_are_asked_for` |
| F15 | CONFIRMED | topic 的 `actions` 由已註冊的 `animation.*` actions 產生(runtime 註冊 stage / pick 後再產生一次);格式清單改用共用常數 `import::FORMATS`(含 `.axrig`,匯入失敗的訊息也用它);controller 摘要加入 blend2d | `animation_review::discover_animation_lists_every_registered_action_and_format` |

注意:F5 第一次驗證 `.gltf` / `.axrig` 時會讀完整個 JSON(上限 128 MiB),之後走快取;`.glb` 只讀 JSON chunk。首次驗證的耗時沒有量測。

Gate(debug,worktree `r4`):`cargo fmt --all` 無差異;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 919 passed / 0 failed;`editor-smoke-test` ok;`editor-world-parity` identical(rev 13);`street-editor-parity` identical(rev 16);`m0-smoke-test` PASS(M0 + QA)。沒有改渲染檔案,所以沒做截圖 A/B;F8 只改了 `extract` 交出的 `Arc` 是否重用,渲染器拿到的頂點資料不變。新測試在修正前的失敗是推理判斷(多數依賴新 API,舊程式無法編譯)。例外:`an_oversize_pmx_texture_is_refused_with_its_path` 在舊程式也會通過,因為錯誤訊息相同,只差讀取量。
