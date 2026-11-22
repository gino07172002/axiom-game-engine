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
5. **複審**(使用者 2026-11-22 追加):每項修正合併後,再用同一模型(`gpt-6.2-sol` high、唯讀)複審,輸入 = 原報告 + 處理紀錄 + 該項修正的 diff,要求檢查 (a) 修正本身是否正確、有無新問題,(b) 該區域仍未處理的問題。複審報告存 `docs/reviews/codex-2026-11/R<n>.recheck.md`;有確認成立的新問題就再修一輪,直到沒有新的成立問題,或只剩明確記錄的延後項目。

**Non-goals**:大型重構(拆 `app.rs` 等)只記錄建議不在本輪做;不加新功能;不碰 `data/` 與另一個 session 的 Paladin 檔案。

## 處理紀錄

### R5 物理、戰鬥、導航與通用 gameplay 系統

15 項 finding,逐條讀 code 後 15 項 CONFIRMED,全部以有界修改處理(F9 採「每幀分段」而非背景執行緒,見下)。每項都有回歸測試;測試在修正前會失敗(依 code 推理確認,F7/F12 的測試幾何經實跑調整)。

| ID | 判定 | 修改 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED | `combat::resolve` 不再直接 `publish`:`combat.hit` / `combat.defeated` 先進有界 FIFO `outbox`(`MAX_OUTBOX` 8192 = 兩批 4096 hit 的最壞事件數),每 tick 依 `Events::room()` 分批交付,保留原 tick 與順序。HP 提交前先預留最壞容量,不足時整批拒絕(不改 HP、計入 `rejected`、`/combat/error` 說明哪個上限),不再讓事件佇列 `fault` 凍結模擬。`events.rs` 只加 `MAX_PENDING` 常數與 `room()`。observe 新增 `queued_events`、`error`;hot reload 一併繼承 outbox。combat 世界本來就不支援 checkpoint,outbox 不需存檔。 | `shared_combat::hit_burst_beyond_the_event_queue_defers_events_instead_of_freezing` |
| F2 | CONFIRMED | 新增 `fsm::validate_world`(註冊為 registry world validator,`World::compile` 執行,CLI preview/apply 與編輯器共用同一 store 請求路徑):`fsm.step` 啟用時檢查 agent 的 machine 存在、有 `core.transform`、`emit` 事件在 `behavior.program` 宣告且只含 text `agent`、`behavior.execute` 已啟用,回傳指向 `/entities/@id/components/fsm.agent/machine` 或 `/resources/fsm.machines/.../emit` 的 `Diagnostic`。`emitted_events` 由 validator 與 `FsmSystem::new` 共用;`radius` 範圍移到 component validator。 | `fsm::invalid_agent_references_fail_preview_and_apply_before_a_revision_is_written` |
| F3 | CONFIRMED | 預檢以每 zone、每方向的「本步已計」旗標套用與發送相同的 `once` 規則。 | `triggers::once_zone_counts_many_simultaneous_enters_as_one_event` |
| F4 | CONFIRMED | `SavedBody::check` 為存檔與讀檔共用:任何有限速度都合法(`MAX_SPEED` 只限制作者輸入與指令;另外原讀檔對角速度也用 1000,比作者上限 3600 還嚴)。存檔時同樣檢查,不會寫出讀不回的 checkpoint。 | `physics::a_fast_light_body_checkpoint_round_trips`(0.001 kg、2 N·s → 2000 m/s,存、讀、各走 10 步 transform 相同) |
| F5 | CONFIRMED | `interact` 的 `apply` 改回傳 `Result`,先發事件(gameplay 指令本身是 transactional),成功後才提交 `uses`/冷卻/`recent`。`interact.use` 回傳含 interactor、target、事件名的 `interact_event` 錯誤;按鍵路徑寫入 `fault`。 | `interaction::failed_use_event_keeps_a_once_target_unused_and_returns_an_error` |
| F6 | CONFIRMED | `Attack` 前檢查施術者 HP(無 `core.health` 視為存活);轉移、事件、clip 不受影響。 | `fsm::defeated_attacker_deals_no_damage`(含存活對照) |
| F7 | CONFIRMED | `navigation::straighten` 改回傳 `Result<usize, usize>`(失敗也帶出已用工作量,`axiom-core/src/navigation.rs`);`service` 先計帳再判斷結果,`budget.last/max` 不再低報,失敗後同一步不再繼續開新搜尋。超支仍以既有 `MIN_STRAIGHTEN_WORK` 為上限。 | `navigation::failed_straightening_counts_against_the_step_budget`(150 道蛇形牆) |
| F8 | CONFIRMED | `cancel` 同時移除該代理的 FIFO 項目(每代理最多一項);`track` 的重規劃改用只取消執行中搜尋的 `cancel_search`,保留原排隊位置(與修正前實際行為相同)。`transfer` 改用 `cancel`,不再留下重複項。 | `navigation::stop_and_go_requeue_at_the_back_of_the_fifo` |
| F9 | CONFIRMED,有界修正 | 編輯器路徑預覽改為跨幀分段:每次呼叫最多 `PREVIEW_FRAME_WORK` 4096 單位(A* 展開,之後直線化),同一輸入下一幀續跑,總上限仍為 65,536;輸入改變即放棄舊搜尋;`preview_pending()` 讓 viewport `request_repaint`(`editor/viewport.rs` 一處)。未做背景執行緒:搜尋狀態已在 `Mutex<Preview>` 內,分段即可滿足每幀有界,不需新的 job/watchdog。**耗時未量測**,只宣稱工作量上限由 65,536 降到每幀 4,096。快取命中仍 clone 至多 128 點的 `Vec`。 | `navigation::editor_path_preview_is_spread_over_frames`;既有 `viewport::nav_gizmo_previews_the_route_and_the_grid_overlay_toggles` |
| F10 | CONFIRMED | `Follower.authored_speed` 另存作者速度;只有新舊作者速度相同時才延續 `path.set_speed` 覆寫,位置/方向/暫停仍保留。 | `paths::an_edited_authored_speed_replaces_a_runtime_override` |
| F11 | CONFIRMED | 抽出 `target_problem`,`camera.follow.set` 與 hot reload 共用;runtime 目標在新修訂不合格(或已刪除)時回到作者目標,`camera.inspect` 的 `target_reset` 說明原因,下一次 `follow.set` 清除。 | `camera_follow::hot_reload_drops_a_runtime_target_that_became_invalid` |
| F12 | CONFIRMED | 同格網保留 waypoint 時一併保留 `goal_snapped`;`nav.inspect` 新增 `goal_snapped` 欄位。 | `navigation::unrelated_hot_reload_keeps_a_snapped_entity_goal` |
| F13 | CONFIRMED | `Shape::reach(grow)`:box 用 `(half + margin).length()`,sphere `r + margin`。 | `triggers::rotated_box_keeps_an_occupant_in_the_grown_corner_of_another_cell`(45° box,4.9 m 在舊粗篩外的格子) |
| F14 | CONFIRMED | 播放器的 `on_finish` 仍在積壓中且新 clip 有 `on_finish` 時,`tween.play` 回傳可重試錯誤(含前方事件數)。 | `tweens::replay_while_the_previous_finish_is_queued_is_a_retryable_error` |
| F15 | CONFIRMED | `fsm.machines` 的公開 schema 文字由 `editor_kinds` 產生(`schema_kinds`),補上 `clip`、`clip_loop`;`fsm.agent` schema 補 `radius`。 | `fsm::public_schema_lists_the_editor_kinds_and_state_fields` |

決定性:所有既有 replay / `state_hash` 測試未改仍通過。行為只在 finding 描述的情境改變(事件積壓、once 多重進入、旋轉 box 擴張角、零血攻擊者、stop/go 重排、失敗直線化);F8 的 hot reload 重規劃順序在「同時有新代理與承接代理重排」時會略有不同(承接代理改排在新 build 的請求之後)。

觸及他人區域:`crates/axiom-runtime/src/events.rs`(常數與 `room()`)、`crates/axiom-core/src/navigation.rs`(`straighten` 回傳型別)、`crates/axiom-runtime/src/editor/viewport.rs`(預覽重繪一處)。

連帶修改:`editor/graph_tests.rs` 的 `Harness::with_runtime` 改用可執行的世界建 store(store 現在會拒絕含未宣告 `emit` 的世界;編輯器仍開啟該草稿並顯示 graph checks)。注意:既有資料庫若最新修訂含這類無效 FSM 參照,開啟時 `World::compile` 會拒絕(與 trigger / nav 事件檢查同一政策);這類世界原本就無法啟用 runtime。

Gate(R5 worktree,debug):`cargo fmt --all --check` 通過;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 除兩個計時測試外全過 — `app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`(500 ms 門檻)與 `m15::agent_timeout_is_a_failed_task_and_never_a_zero_token_success`(10 s 門檻)在其他 agent 平行負載下失敗,單獨重跑皆通過;`axiom-runtime --lib` 全部重跑 342/343 通過(唯一失敗為同一計時測試,單獨重跑通過)。`editor-smoke-test`(port 4552)ok;`editor-world-parity`(4566)identical,rev 13/13;`street-editor-parity`(4562)identical,rev 16/16;`m0-smoke-test`(4362)PASS。無渲染管線改動,未做截圖 A/B。
