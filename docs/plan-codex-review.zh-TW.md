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

### R1 編寫核心與協定

兩份報告:`R1.md`(gpt-6.2-sol,F1–F15)與 `R1.gpt-6.1-sol.md`(gpt-6.1-sol,F1–F11,下表記為 6.1-Fn)。重疊:6.2-F6 = 6.1-F4(set_where 放大)、6.2-F14 = 6.1-F9(discovery 漂移)。共 26 條,去重後 24 條:全部確認,修 22 條(其中 6 條是整合者先在 worktree 修好的,已複查並一起提交),延後 2 條。逐條讀過引用的程式碼才下結論。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| 6.1-F1 | CONFIRMED,已修(整合者) | `acl::run` 的讀取分支在 `--preview`/`--propose` 時拒絕 `undo`(它會提交恢復版本) | `tests/acl.rs::undo_is_refused_in_preview_and_proposal_mode` |
| 6.1-F2 | CONFIRMED,已修(整合者) | `local::matches` 的 nodes/choices/widgets/rules 不再 `unwrap()` 沒驗證的 `id`,改回 `invalid_resource` 診斷(`item_id`,帶清單與位置) | `tests/selectors.rs::selecting_from_a_malformed_intermediate_resource_is_an_error_not_a_panic` |
| 6.1-F3 | CONFIRMED,已修(整合者) | `scenario` 預設只在記憶體留 checkpoint,不寫 `<db 目錄>/saves`;要寫磁碟需 `--save-dir DIR`。`scripts/m1-smoke-test.ps1` 的 scenario 之後要用 `play --load-slot` 讀這些 slot,所以改成明確傳 `--save-dir`(QA 資料庫同層 `saves/`,行為與修前相同) | m1 smoke(見 gate);手動:無 `--save-dir` 不建立 saves |
| 6.1-F5 | CONFIRMED,已修(整合者) | `export` 遇到 `ok:false` 或回應沒有合法 World 時不寫檔、不覆寫既有檔並以失敗結束 | 靠閱讀確認(需要連線 bridge 才能重現遠端錯誤) |
| 6.1-F6 | CONFIRMED,已修(整合者) | `import --expected-rev N` 傳進 Apply 做 CAS | 沿用 Apply 的 `revision_conflict` 測試;CLI 參數由 clap 檢查 |
| 6.1-F7 | CONFIRMED,已修(整合者) | `serve` 用 `read_bounded_line` 逐行有界讀取,超長行丟到換行為止並回錯,之後的行照常處理 | `axiom` lib `tests::serve_lines_are_bounded_and_an_overlong_line_does_not_stop_the_stream` |
| 6.2-F1 | CONFIRMED,已修 | `audio::validate_world` 的 `unwrap_or(listeners[0]…)` 會先求值而在沒有 listener 時 panic;改 `or_else(listeners.first())` | `audio::tests::a_source_without_listener_or_spatial_system_is_a_diagnostic_not_a_panic` |
| 6.2-F2 | CONFIRMED,已修 | `ops::add_camera_follow` 在 bound 非有限值時不呼叫 `clamp`(保留預設距離),由後面的 `validate` 回 `invalid_value`;ACL 與編輯器共用此 op | `tests/ops.rs` camera follow 測試新增 `min=NaN`/`max=NaN`/`min=inf` 與直接呼叫 op |
| 6.2-F3 | CONFIRMED,已修 | 根因:serde 會把 JSON 陣列解成 struct,驗證因此接受 `audio.library = []` 等陣列形式。`Registry::core` 的 `typed` 驗證器現在拒絕「物件型別的陣列形式」;另外對已存在的舊 revision,`ops`(audio clip 兩處、互動按鍵)、`ops_ui::document`、`ops_tuning::resource` 把 `.expect`/索引寫入改成 `invalid_value` 診斷 | `tests/ops.rs::a_resource_in_array_form_is_refused_and_never_panics_an_op` |
| 6.2-F4 | CONFIRMED,已修 | `Store::edit` 在 preview(`commit=false`)用 `Deferred` 交易:WAL 下只是一致的讀取快照,不再占寫鎖;提交仍是 `Immediate` + 交易內 CAS | `tests/protocol_contract.rs::preview_reads_a_snapshot_while_another_connection_holds_the_write_lock`(另一連線 `BEGIN IMMEDIATE` 時 preview 仍在 2 s 內成功;修前會等 3 s busy timeout 後失敗) |
| 6.2-F5 | DEFERRED | 確認:`current_shared` 快取未命中時同步讀取+解碼整份世界,編輯器的輪詢與 Verify 在主執行緒呼叫。修正需要把編輯器的 revision 採用與 Verify 改成背景 job(帶 rev 的結果回送、取消、watchdog),屬 R2/R8 的 runtime/編輯器重設計,不在 R1 範圍;也沒有量測數字 | — |
| 6.2-F6 / 6.1-F4 | CONFIRMED,已修 | (1)展開前檢查 `matches × 序列化值大小 ≤ MAX_SET_WHERE_BYTES`(16 MiB),超過回 `budget_exceeded`;(2)`Target::edit` 拆出 `edit_doc`,`set_where` 對整個選取只做一次 World→JSON→World(修前每個命中一次)。同一選取的目標是不同物件,`edit_doc` 只讀該實體自己的 prefab 預設,因此結果與逐一轉換相同(既有 prefab 覆寫測試仍過)。未做 A/B 量測,不宣稱效能數字 | `tests/selectors.rs::set_where_refuses_an_expansion_over_its_byte_budget_before_copying`;既有 `set_where_updates_every_match_in_one_revision_with_per_object_diff` |
| 6.2-F7 | CONFIRMED,已修 | `manifest.json` 先 `canonicalize` 並檢查仍在套件目錄內(與世界檔相同),再有界讀取且必須是一般檔案 | `tests/packages.rs::package_files_are_read_bounded_and_the_manifest_stays_inside_the_package`(符號連結那段在沒有建立 symlink 權限的 Windows 上會跳過並印出原因) |
| 6.2-F8 | CONFIRMED,已修 | 新增共用 `axiom_core::read_bounded`/`read_bounded_string`(`File::open` + `take(limit+1)` + 一般檔案檢查);用在 manifest(64 KiB)、套件世界(4 MiB)、宣告資產(32 MiB)與 CLI 的 `rpc`/`apply`/`preview`/`select`/`dispatch`(1 MiB)、`acl FILE`(64 KiB)、`scenario`/benchmark suite(1 MiB)、`import`(新上限 64 MiB) | 同上 packages 測試(超大 manifest/world 回 `exceeds`) |
| 6.2-F9 | CONFIRMED,已修 | 新增 `Store::verify_world(rev, &w)`;本機 `verify --ticks` 先取一次快照,驗證與模擬都用它。`--measure` 會重讀資料庫(量測本身的定義),量完若 rev 與驗證的 rev 不同就回錯要求重跑 | 靠閱讀確認(兩次讀取之間插入提交需要多程序時序) |
| 6.2-F10 | CONFIRMED,已修 | `resolve_proposal rebase` 在過期且含「非純 ID 清單」選取的 `set_where` 時回 `stale_selector`,指向 `conflicts N` + `keep`(在 base 上求值後三方合併)或重新提案。純 ID 清單(ACL `set ID …` 都是這種)在新 revision 只會命中相同或更少物件,維持可 rebase | `tests/proposals.rs::a_stale_proposal_whose_selector_could_match_new_objects_is_not_rebased`;既有 `stale_proposals_conflict_until_rebased_and_rejections_never_write` 仍過 |
| 6.2-F11 | DEFERRED | 確認:Inspector 多選貼上走 `ops::set_fields`(每物件每欄位一筆 `SetField`,受 256 筆上限),ACL `set` 是一筆 `SetWhere`(最多 4096 命中)。統一需要新的「穩定 ID 清單批次欄位」edit 與編輯器改走它,跨 core 協定與 R8 編輯器,另開一輪 | — |
| 6.2-F12 | CONFIRMED,已修 | `Propose` 的待審數查詢與 INSERT 包在同一個短 `Immediate` 交易 | 靠閱讀確認(需要兩連線精確交錯);既有佇列測試仍過 |
| 6.2-F13 | CONFIRMED,已修 | `observe_delta` 明確指定 `fields` 時,這次觀察沒有的指標列在 `missing_fields`(最多 32 筆,與投影上限相同);預設投影不列。`schema observe_delta` 說明已更新。未區分「語法無效的指標」(`pointer()` 已正規化,無效跳脫與不存在在 `serde_json` 都是 `None`) | `observe::tests::explicit_fields_that_are_absent_are_listed_not_silently_dropped` |
| 6.2-F14 / 6.1-F9 | CONFIRMED,已修 | `schema edits` 的 kinds 改用 `protocol::EDIT_KINDS`(補上 `set_where`);`discover local` 分成讀取 `selection`(limit ≤ `MAX_SELECT_LIMIT`)與 `edit_selection`(無 cursor/fields,limit ≤ `MAX_SET_WHERE_MATCHES`),上限與實作共用 `local` 常數;narrative schema 補 `GameAction::Call`、`RuntimeCommand::Batch`/`Slide`。契約測試從 serde 自己的 unknown-variant 錯誤取出實際接受的變體,與 schema 比對 | `tests/protocol_contract.rs::discovery_lists_every_edit_action_and_command_variant`、`read_and_edit_selections_declare_the_limits_the_store_enforces` |
| 6.2-F15 | CONFIRMED,已修 | ACL `--propose` 的自動備註依 UTF-8 位元組(共用 `store::MAX_PROPOSAL_NOTE_BYTES`)在字元邊界截斷 | `tests/proposals.rs::an_acl_proposal_note_is_cut_to_the_byte_limit_on_a_character_boundary`(繁中註解) |
| 6.1-F8 | CONFIRMED,已修 | `Store::schema_topic`:`schema`/`discover` 的 `typed:ID` 用目前世界(找得到 `component_schemas`/`resource_schemas`,樣本值也取自目前世界) | `tests/protocol_contract.rs::typed_discovery_finds_the_current_worlds_own_schemas` |
| 6.1-F10 | CONFIRMED,已修 | 新增 `Store::open_lazy`(初始世界用 closure,只在空資料庫時呼叫);CLI 改用它,既有資料庫上的 `schema`/`query` 不再載入 `content/fps` | `tests/protocol_contract.rs::an_existing_database_never_builds_the_initial_world` |
| 6.1-F11 | CONFIRMED,已修 | diff 的順序標記改成 `{path}#order`(與 merge 的 `/entities#order` 一致),不再與 ID 為 `order` 的物件撞名。編輯器 `diff_view::change_line` 兩種都認(資料庫裡舊提案的 preview 是 `/@order`) | `tests/coediting.rs::diff_order_marker_does_not_collide_with_an_object_called_order`;`diff_view` 測試加舊格式 |

範圍外的觸碰:`axiom-runtime/src/editor/diff_view.rs`、`followup_tests.rs`(6.1-F11 的消費端)、`scripts/m1-smoke-test.ps1`(6.1-F3)。行為變更:陣列形式的物件型元件/資源現在被驗證拒絕(舊 revision 若含此形式,下一次編輯會收到診斷,把資源改寫成物件即可);`schema edits` 的 kinds 順序變了(hash 隨之改變)。文件:`docs/user-guide.zh-TW.md`(serve/檔案上限、import `--expected-rev`、export、scenario `--save-dir`、verify 快照)。

Gate(R1 worktree,debug):`cargo fmt --all` 乾淨;`cargo clippy --workspace --all-targets -D warnings` 通過;`cargo test --workspace --no-fail-fast` 919 passed / 0 failed / 6 ignored;`editor-smoke-test` PASS;`editor-world-parity` identical(rev 13);`street-editor-parity` identical(rev 16);`m0-smoke-test` PASS(在 m1 內);`m1-smoke-test` 第一次在即時指令 `adv-help` 回 `runtime command response timed out`(機器同時有其他 agent 在編譯;此路徑本次沒有改),單獨重跑 `-SkipM0` PASS——scenario `--save-dir` 寫入的 slot 被後續 `play --load-slot` 正確讀到。手動(隔離目錄):沒有 `--save-dir` 的 scenario 不建立 `saves/`、加上後寫出 3 個 slot;`import --expected-rev` 過期回 `revision_conflict` 且 rev 不變;`export` 在錯誤回應時既有檔案保持原內容。未驗證:`export` 的遠端 `ok:false` 分支、`verify --ticks` 與 Propose 的併發時序(靠閱讀);manifest 符號連結逃逸在本機若沒有 symlink 權限會跳過。

### R2 執行期主機與生命週期

15 項 finding:CONFIRMED 15(修 14、F7 延後)。每項都先讀過被引用的 code 再判定。

| ID | 判定 | 改動 / 理由 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED(只在 `rebase`/`keep` 核准時可觸發:非 rebase 核准本來就以提案 base 做 CAS) | `approval_rev()`:編輯器送出的核准若未帶 `expected_rev`,背景 writer 補 `chain_rev`、同步路徑補 `editor.rev`;拒絕不補(不寫 revision)。Undo 記錄的版本因此等於核准實際依據的版本 | `app::tests::approving_a_proposal_cannot_let_undo_revert_a_foreign_commit` |
| F2 | CONFIRMED | `handle_command` 不再呼叫會 sleep 輪詢的 `finish_activation()`;啟用建置中時請求回到既有 deferred 機制,下一幀優先重試(保持順序),入口檢查 deadline;逾時回 `runtime_timeout` 診斷附 `requested_rev`/`live_rev`,且不執行。`reload_assets` 的取消/watch 不等建置 | `app::tests::runtime_requests_wait_for_activation_on_later_frames_never_past_their_deadline` |
| F3 | CONFIRMED | `bridge::read_request_line`:整筆請求 2 s 絕對期限(不是每次 read)、每 50 ms 檢查停止旗標、1 MiB 上限;`Drop` 限時 join 2 s,逾時 detach 並記錄 | `bridge::tests::a_trickling_unterminated_request_cannot_hold_the_bridge` |
| F4 | CONFIRMED | `editor_view` 先驗證 capture 名稱並建立目錄(以及 select 數量),才開 Play / 執行編輯命令;`pending_capture` 最後才設定 | `app::tests::invalid_editor_view_capture_leaves_no_queued_edit_behind` |
| F5 | CONFIRMED | `WorldRuntime::adopt`:只有 `world.id` 相同才呼叫新抽出的 `transfer_state`(tick、rng、事件、handle、即時值、columns、native `transfer`、combat、events、contexts);不同世界全新狀態,handle generation 維持單調遞增,舊 handle 不會解析到新世界 | `generic_runtime::replacing_the_world_with_another_world_id_starts_fresh_simulation_state` |
| F6 | CONFIRMED | (a) `Contexts::check_activate` 讓 batch 預檢與正式 `activate` 共用 context ID 驗證;(b) 部分套用改為 typed 錯誤:`Diagnostic` code `batch_partial` + context `world::PartialBatch { applied }`,回覆帶 `applied_count`/`failed_index`;`execute`(app 與 benchmark harness)把這個結果存成 receipt,同 `request_id` 重送不再重做前綴。`docs/choreography.zh-TW.md` 同步 | `m4::batch_preflight_checks_input_context_ids_like_the_real_activation`、`benchmark::retry_tests::a_partly_applied_batch_gets_a_receipt_with_its_applied_count` |
| F7 | CONFIRMED → **DEFERRED** | 修法需要「延遲回覆」的 runtime 請求(像 screenshot)加上 execute receipt 在回覆時才寫入、讀檔的替代 runtime 在 worker 建置後於主執行緒確認 session/rev 再接手。設計:存檔 = 主執行緒 `checkpoint()`(一致擷取)→ `Job` 做序列化 + 寫檔 + `sync_all` + rename,完成時回覆;讀檔 = `Job` 讀檔 + 解析 + `compile` + `WorldRuntime::new`(需把 `restore_checkpoint` 拆成「純建置」與「主執行緒套 ids/session/epoch」兩段),回來時若 `runtime_session`/`rev` 已變則丟棄並回診斷;GUI 快存/快讀走同一路徑,status bar 與 observe 顯示 job。牽涉 protocol 回覆時序、recorder 與 receipts,超出本輪範圍 | — |
| F8 | CONFIRMED | `watch` 的目錄掃描改成 `Job`(`asset_watch`,10 s watchdog,資料夾間可取消、progress = 資料夾數);同一資料夾只列一次;主執行緒只在間隔到時起 job、之後輪詢比對結果。observe `assets_reload` 新增 `watch_scan`、`watch_scan_ms_max`。`set_watch(false)` detach 掃描,不等待 | `asset_reload::tests::watch_scan_lists_each_folder_once_and_matches_the_per_model_stamp`(並改寫既有 reload 測試的 watch 段落為非同步) |
| F9 | CONFIRMED | writer watchdog 觸發後立即在背景開新的 writer(`enable_writer`,沿用 timeout/grace 與計數),`writes.stalled` 保留未確認提示並出現在 observe;關閉中不重開(維持 shutdown 回報停滯)。開不起來才退回主執行緒 | `app::tests::stalled_writer_is_detached_reported_and_editing_continues`(改寫:下一筆編輯排入新 writer、不在主執行緒提交) |
| F10 | CONFIRMED | `Reload::poll` 只接受 `stop: None` 的結果;取消/逾時轉成 `Stopped` 錯誤,`decide` 回報狀態 `cancelled` / `timed_out`,不採用、不接受 hash 基準 | `asset_reload::tests::a_reload_cancelled_after_its_build_finished_is_not_adopted` |
| F11 | CONFIRMED | `ReloadAssets` 在 Play 檢查之前處理;GUI 的 `AssetReloadAsk` 改走同一個 `App::reload_assets` | `app::tests::reload_assets_is_a_host_request_in_edit_mode_too` |
| F12 | CONFIRMED | `App::retry_activation`:`reload_assets`(協定與 GUI `asset.reload`)在「requested_rev != rev、沒有建置中、非 Play、沒有排隊寫入」時重建最新 revision,回覆 `retry:"activation"`、`rev`、`live_rev`;例行輪詢維持不重試 | `app::tests::reload_assets_retries_a_failed_activation_of_the_newest_revision` |
| F13 | CONFIRMED | `RuntimeSystem::receive_events` 改為 `&self`(唯一實作 `BehaviorSystem` 本來就只改 state),派送改 `iter()`;trait 文件寫明交易只涵蓋 state。外掛不可能再在可回滾範圍外改私有狀態 | 型別系統保證(移除的能力無法寫成 runtime 測試);既有事件派送測試全過 |
| F14 | CONFIRMED | `Assertion::Field` 的 `components` 疊上該實體的 typed column 值(重用 `checkpoint::column_value`) | `scenario::tests::field_assertions_read_live_custom_columns_not_authored_values` |
| F15 | CONFIRMED | 每個 task 最多 8 個 replay plan;整套 transcript 共用 16 MiB 位元組預算,超出後仍計入 cost、不保留內容,每筆結果帶 `transcript_status {truncated, dropped_exchanges}` | `benchmark::tests::transcripts_share_one_byte_budget_and_say_when_truncated`、`a_task_with_too_many_replay_plans_is_refused` |

未修但順帶發現(未改):`editor_view` 的 `play` 仍在 `remote_view`/`open` 驗證之前執行,後者失敗時 Play 已開始;coalesce 後多個 action 中途失敗時前面的已排入。皆非本報告 finding。

既有測試的刻意調整:`m1_gameplay::newly_activated_gameplay_rebases_initial_timers_and_node_end_saves` 原本從 `World::empty("empty")` reconcile 到 adv 世界(不同 `world.id`)並期待 tick 延續;依 F5 新語意改成從同 ID 的空世界開始,測試意圖(同一世界新啟用 gameplay 時計時器以目前 tick 重新定基)不變。`app::tests::stalled_writer_is_detached_reported_and_editing_continues` 依 F9 改為驗證替代 writer。

Gate(worktree r2,debug):`cargo fmt --all -- --check` 通過;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 第一次 71 個 target 通過、2 個失敗:`m1_gameplay`(上述 F5 語意,已改)與已知 timing flake `app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`(其他 agent 並行編譯造成負載;單獨重跑通過)。修正後重跑:`m1_gameplay` 23/23;axiom-runtime lib 360 個中 354 通過 + 5 ignored,唯一失敗的 flake 單獨重跑通過(早先一次完整 lib run 為 355 通過、0 失敗)。`editor-smoke-test` PASS(241 s)、`editor-world-parity` PASS(identical, rev 13)、`street-editor-parity` PASS(identical, rev 16)、`m0-smoke-test` PASS(含內容切換、即時 bridge apply/undo)。無渲染改動,不需截圖 A/B。

未驗證:新測試「修正前會失敗」是依程式碼推理確認(每個測試都直接重現報告描述的序列),沒有逐一 revert 實跑;F2 / F3 / F9 沒有額外做主執行緒時間的 A/B 量測(改動是移除等待,不宣稱效能數字)。

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

### R9 封裝與 player

範圍:`crates/axiom-runtime/src/packaging.rs`、`crates/axiom-runtime/src/editor/build.rs`、`crates/axiom-player`、`crates/axiom-cli`(只加 `--expected-pack-digest`)、`scripts/package-smoke-test.ps1`、`docs/user-guide.zh-TW.md` 的封裝段落。15 項 finding 全部 CONFIRMED;14 項完整修正,F10 部分修正(完整排除清單的選項沒有做)。axiom-runtime 的 `windows` 依賴多開一個 feature `Win32_Storage_FileSystem`(同一個 crate 0.58,沒有新 crate)。

刪除邏輯只變得更嚴:失敗的建置只刪自己用 `create_dir` 建立的 `<out>.partial-<pid>-<n>`;`--force` 只處理不是連結 / junction、內含 `game.json` 一般檔案的目錄,刪除前用 `symlink_metadata` 加上 reparse-point 屬性再確認一次。所有刪除測試都在 `%TEMP%` 底下新建的目錄裡做(env.ps1 讓 TEMP 指向 worktree 的 `.tools\tmp`)。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED | 暫存目錄改成每次建置專用的 `<out>.partial-<pid>-<n>`,用 `create_dir` 原子建立,已存在就換下一個名字;不再在開始時刪掉任何舊的 `.partial`。`Staging` 在 drop 時只刪自己建立的目錄(`remove_own_dir` 拒絕連結)。代價:程序被強制結束時暫存目錄會留下,要手動刪 | `packaging::tests::staging_never_removes_a_pack_or_foreign_partial_folder`(pack 叫 `Game.partial`、輸出是 `Game`;另一個 `Other.partial` 不被動到) |
| F2 | CONFIRMED | 發佈用 `rename_noreplace`(Windows `MoveFileExW` 不帶 `REPLACE_EXISTING`;實測 `std::fs::rename` 會取代空目錄與檔案)。發佈時重新檢查目的地:沒有 `--force` 時,建置期間出現的目錄不會被覆蓋;有 `--force` 時重新確認是 game folder,舊版先改名到 `<out>.old-*`,新版就位失敗就移回,成功後才刪備份。同一輸出用 OS 檔案鎖(`%TEMP%\axiom-build-<hash>.lock`,`File::try_lock`)序列化 CLI 與編輯器,程序結束時 OS 會自動釋放 | `publish_refuses_new_destinations_and_restores_on_failure`、`force_refuses_a_junction_output` |
| F3 | CONFIRMED | `copy_hashed` 用 64 KiB 共用緩衝區串流複製並算雜湊,寫入前先扣剩餘預算;內容、player、`game.json` 共用同一個 4 GiB 上限;目的檔用 `create_new` | `byte_budget_is_checked_before_writing`(超出預算時不建檔;2 byte 緩衝區得到與整檔相同的雜湊) |
| F4 | CONFIRMED | player 對遊戲目錄與 `pack` 做 canonicalize,要求 pack 真的在遊戲目錄內(junction 指到外面就拒絕);載入後呼叫與 packager 共用的 `packaging::ensure_files_inside`(拒絕 `external` 動畫素材,以及解析後落在 pack 外的檔案) | `axiom-player` `pack_junction_outside_the_game_folder_is_refused`、`packaging::tests::launch_check_refuses_external_animation_assets` |
| F5 | CONFIRMED | 走訪每個目錄、每個 64 KiB chunk、載入驗證前後、相依檢查每個素材都會檢查取消;`publish` 開頭是明確的截止點,在任何改名之前。編輯器的 watchdog 與 shutdown 都先設旗標,所以已被標成 abandoned 的 worker 之後恢復也不會發佈。`build.rs` 的註解與 `cancel_too_late` 的意思改成「在截止點之後才取消」 | `cancel_before_publish_keeps_the_previous_output`;既有 cancel 測試改查 `.partial-*` |
| F6 | CONFIRMED | staging 驗證新增 `check_asset_dependencies`:glTF / GLB 解析 JSON(GLB 只讀 JSON chunk)取出非 `data:` 的 buffer 與 image URI,`.axrig` 取 `textures`,用 importer 的 `confined` 規則解析;PMX 的貼圖表在幾何資料之後,所以整個匯入一次。錯誤帶素材 ID 與缺少的檔案。這些依賴也算進 `referenced` | `gltf_buffers_must_ship_with_the_gltf`(排除 `mesh data.bin` → 失敗、沒有輸出;保留時不列為 unreferenced) |
| F7 | CONFIRMED | 新增共用型別 `packaging::GameManifest`(`validate` / `to_bytes` / `parse` / `read`,64 KiB 上限、ASCII id 規則、title、pack、exe)。packager 在建立任何目錄前序列化並驗證;player 改用 `GameManifest::read`(透過 R1 的 `axiom_core::read_bounded`) | `game_json_contract_and_hash`(輸出由 `GameManifest::read` 讀回;Unicode id 在寫入前就被拒絕) |
| F8 | CONFIRMED | 報告新增 `pack_digest`(出貨 pack 檔案的「相對路徑 + SHA-256」清單雜湊)。CLI `--expected-pack-digest` 與編輯器 `expected_pack_digest` 不符時不發佈。編輯器的 `build.last` 新增 `pack_digest`、`authoring_rev_at_start` 與 `authoring_rev_note`(標明僅供參考,建置讀的是磁碟上的 pack) | `pack_digest_is_reported_and_can_be_pinned`、`editor::build::tests::editor_job_builds_the_same_files_as_a_direct_package_call`(新增斷言) |
| F9 | CONFIRMED | `file.build_game` 的 JSON 改用 `BuildSpec`(`deny_unknown_fields`、型別錯誤直接報錯),在啟動 worker 之前就拒絕 | `editor::build::tests::build_json_rejects_unknown_fields_and_wrong_types` |
| F10 | CONFIRMED(部分) | `collect` 改成迭代走訪:最多讀 65 536 個目錄項目(含被排除的)、深度 32、保留 4096 個檔案。報告的 `skipped` 只列前 32 項,另加 `skipped_total`。沒有做「明確選項取得完整清單」;需要時可以加 CLI flag,但目前沒有使用者 | `walk_bounds_depth_and_excluded_listing` |
| F11 | CONFIRMED | smoke 腳本的每個子程序(package、player、query)都經過 `Invoke-Bounded`:`WaitForExit(上限)`,逾時就 Kill、限時回收,錯誤帶階段名稱與 stdout / stderr 日誌路徑(`-PackageTimeoutSeconds` / `-PlayerTimeoutSeconds`,預設 300) | 執行腳本(見 gate) |
| F12 | CONFIRMED | 不再掃描所有 JSON 字串。`resolved_files` 依欄位列出 package import 會改寫的檔案(`ui.document/images/*`、`animation.library/assets/*/path`、`audio.library/clips/*/file/path`;shader 已 inline),回傳 JSON pointer。來源檢查、staging 檢查、`referenced`、player 都用這份清單,錯誤會指出欄位。代價:之後新增的 package resolver 要把它的欄位加進 `resolved_files`(已寫在註解) | `absolute_path_text_is_not_a_file_reference`(world title 含 `C:\...` 文字仍可封裝;越界錯誤帶 pointer) |
| F13 | CONFIRMED | `game.json` 用同一份序列化位元組算長度與 SHA-256,列進 `files`,也計入 `bytes` 與預算 | `game_json_contract_and_hash`(改 title → 雜湊清單不同;`bytes` = 各檔加總) |
| F14 | CONFIRMED | smoke 腳本:複製後把原始輸出移走再啟動;啟動前後比對「相對路徑 + SHA-256」;用 `axiom --db <player db> query world` 讀出解析後的素材路徑,每個都必須在搬移後的 `content` 底下 | 執行腳本 |
| F15 | CONFIRMED | 新增 `locations()`:有 `--db` 時不讀 `APPDATA`,crash log 寫在資料庫旁;只有用預設資料庫時才需要 `APPDATA` | `axiom-player` `explicit_db_does_not_need_appdata` |

文件:`docs/user-guide.zh-TW.md` 的封裝段落已同步(partial 命名、`--force` 備份與還原、`pack_digest`、`skipped_total`、`--db` 時的 crash log 位置)。

Gate(debug,worktree `r9`):`cargo fmt --all -- --check` 通過;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 963 passed / 0 failed / 6 ignored(76 個 test target),其中封裝與建置相關的 22 個單獨跑也全部通過。`package-smoke-test` PASS:plaza、adv、fps、action 都完成封裝 → 複製 → 移走原始輸出 → 用隔離的 APPDATA 執行 player → 截圖;`query world` 查到的已解析素材路徑都在搬移後的 `content` 內(plaza 2、adv 1、fps 0、action 2);外部素材與 `--force` 的拒絕案例通過。plaza 與 action 的截圖已目視確認。`editor-smoke-test` PASS(port 4952)、`editor-world-parity` identical(rev 13,port 4966)、`street-editor-parity` identical(rev 16)、`m0-smoke-test` PASS(含 QA live bridge)。沒有改渲染器,所以沒做截圖 A/B。smoke 腳本第一次執行時,我改的腳本本身有兩個問題:子程序的工作目錄不在 repo,pack 相對路徑失效;Windows PowerShell 沒有讀 `$proc.Handle` 時取不到 ExitCode。兩者修正後才通過。

未驗證:新測試「修正前會失敗」只靠推理判斷,沒有逐一 revert 實跑;依據是每個測試都直接重現報告描述的情境(例如 F1 的 `Game.partial` pack 在舊程式會被 `remove_dir_all`,F12 的 title 文字在舊程式會被當成外部路徑而拒絕)。非 Windows 的 `rename_noreplace` 是先檢查再改名的 best effort。新增的取消檢查與串流複製沒有做效能 A/B,不宣稱效能或記憶體改善。
