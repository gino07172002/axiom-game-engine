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

### R7 玩法套件與遊戲 UI

報告 `R7.md`(F1–F15)。逐條讀過引用的程式碼:14 條確認並修正,F10 的「資產根目錄檢查」部分延後(有界讀取已修)。範圍:`axiom-fps`、`axiom-rts`、`axiom-action`、runtime 的 `rts.rs`/`action.rs`/`fps/`/`showcase.rs`/`gameplay.rs`/`narrative.rs`/`bound_ui.rs`/`ui.rs`;沒有碰 `axiom-combat` 與 runtime `combat.rs`。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED,已修 | `editing.rs` 的相容 `set` 不再用 `value[field] = …` 索引寫入(同批次先把元件設成非物件會 panic);改用 `component_object`(找不到實體或元件不是物件時回帶實體/元件路徑的錯誤),`player.*`/`weapon.*`/`player.spawn`/`behavior.*` 一併改,`entity_mut(..).unwrap()` 改成錯誤 | `axiom-fps/tests/migration.rs::mixed_generic_and_group_edits_fail_recoverably_instead_of_panicking`(apply 與 preview 都回錯、revision 不變) |
| F2 | CONFIRMED,已修 | `Simulation::set_combat` 用每格一個遮罩合併作者阻擋格與共用障礙,再依序輸出清單(最多 width×height ≤ 16384,重複座標只占一格;也去掉了原本 O(格數×清單) 的 `contains`)。`expect` 改成寫入 `last_error` 並保留舊 grid | `axiom-rts/tests/rts.rs::shared_obstacles_merge_with_duplicate_authored_cells_without_exceeding_the_cell_budget` |
| F3 | CONFIRMED,已修 | `validate_world` 以 u64 檢查每個陣營 `initial_balance + 全部礦點存量 ≤ u32::MAX`(任一陣營都可能採完所有礦點),錯誤指向 `/resources/rts.session/initial_balance/N`。交付改 `checked_add`:溢位時寫 `balance_overflow`、工人停下並保留載量(守恆不變) | `rts.rs::starting_balance_plus_world_minerals_must_fit_and_delivery_never_wraps`(每步 `validate()` 檢查守恆) |
| F4 | CONFIRMED,已修 | `axiom-action` `victory()` 的共用戰鬥分支也要求至少一個敵方 body(與獨立分支相同),只改 action crate | `axiom-action/tests/timeline.rs::shared_combat_without_opponents_is_not_a_victory` |
| F5 | CONFIRMED,已修 | `UiState::numeric_drafts` 存 `(草稿值, 起始 revision)`;閒置幀同步成目前值與 revision,拖曳中不更新,提交用起始 revision 當 `expected_rev`,期間若有其他提交就回 `revision_conflict`,滑桿回到最新值。未做「衝突後保留草稿」:衝突回覆走 app 的 console,保留需要 app 把結果回送 UI,另列 | `tests/effects.rs::a_slider_drag_commits_against_the_revision_it_started_from`(拖曳中把 revision 改成 5,放開時仍送 1) |
| F6 | CONFIRMED,已修 | `Gameplay` 記住建置時的作者 `scenes[].active`(`Arc<[bool]>`);熱載入時作者旗標沒變的場景沿用執行期狀態,變了的採用新值(undo 也是新 revision,同樣生效) | `tests/m1_gameplay.rs::committed_scene_flags_win_over_runtime_state_on_reload_and_undo`(observe 與實際 extraction 的 enabled 都檢查) |
| F7 | CONFIRMED,已修 | FPS `transfer` 只在前後 `compiled.source.id` 相同時 `restore_from`(與 RTS/gameplay 相同規則),仍會 `sync` | `tests/generic_runtime.rs::switching_to_another_fps_world_starts_a_fresh_round` |
| F8 | CONFIRMED,已修 | `restore` 依新增礦點的存量、刪除礦點的剩餘量,以及被刪除作者單位身上的載量調整 `initial_economy`,最後檢查守恆(抽出 `check_conservation`,不跑整個 `validate()` 以免改變戰鬥場景下的熱載入接受條件) | `rts.rs::hot_reload_adjusts_the_conservation_baseline_for_added_and_removed_nodes`(新增一個、刪除一個,之後每步 `validate()`) |
| F9 | CONFIRMED,已修 | `Simulation::delivered_tick`(每步歸零,在實際交付處累加);runtime 用它發布 `rts.delivered`,不再用餘額淨增量 | `tests/rts_frontier.rs::delivery_events_report_gross_minerals_when_the_opponent_spends_in_the_same_tick`(對手同一 tick 生產扣款;frontier 世界沒有匯出此事件,所以檢查事件來源值與餘額差;事件發布路徑由既有 m4 測試覆蓋) |
| F10 | 部分修,根目錄檢查 DEFERRED | 已修:`prepare_assets` 用 R1 的 `axiom_core::read_bounded`(開檔後最多讀 4 MiB+1),錯誤帶 `ui.document image <id>`。延後:套件載入時已把圖片路徑 canonicalize 並檢查在套件目錄內,但 runtime 拿到的 authoring 世界沒有記錄套件根目錄,之後的編輯可以填任意絕對路徑。要修需要在 `PackageLock` 記錄資產根目錄(或統一的資產解析器)並讓 UI 圖片、decal(`mesh_style.rs` 同樣模式)、動畫來源共用,屬資產信任模型的設計 | `tests/m1_gameplay.rs::ui_images_are_read_with_one_bounded_read_and_name_the_image` |
| F11 | CONFIRMED,已修 | `narrative::analyze` 遇到 `call`(或含 `call` 的 `batch`)按鈕時不展開,記入 `skipped_native_buttons`(widget 與 `capability.action`,最多 64 筆)並回 `complete:false`,其他分支照常探索。使用手冊已更新 | `tests/m1_gameplay.rs::reachability_skips_native_buttons_and_keeps_proven_endings` |
| F12 | CONFIRMED,已修 | `Gameplay::step` 沒有到期事件且按住的輸入不變時直接返回(不 clone、不重建 `held`);慢路徑不再複製 `compiled.inputs`。`step` 回傳是否切換場景,`BehaviorSystem::step` 只在切換時 `sync_scenes`(指令、事件、熱載入、checkpoint 路徑本來就自己 sync)。未量測,不宣稱效能數字 | `tests/m1_gameplay.rs::idle_steps_change_nothing_and_rule_scene_switches_still_sync`(閒置 30 步觀察與可見性不變;延遲規則在固定步中切場景時可見性跟上) |
| F13 | CONFIRMED,已修 | 無變數滑桿的已提交值存在 `Gameplay::slider_values`(`slide` 成功後才寫入;呼叫被拒時不變),納入 `bindings` 觀察、存檔 `Progress.sliders`(選填,舊存檔可讀)與熱載入。GUI 只保留拖曳草稿,`UiState::slider_values` 移除 | `tests/choreography.rs::a_slider_without_a_variable_keeps_its_committed_value_in_the_runtime` |
| F14 | CONFIRMED,已修 | `enemies.count` 減少與 `remove_obstacle` 都改呼叫共用的 v2 刪除(`axiom_core::scene_edit::delete` 改為 `pub`,`delete_entity` 也用它),一併清理 scene membership 與 relations;`remove_obstacle` 以 prefab 解析後的 `fps.cover` 找實體 | `migration.rs::compatibility_deletes_share_the_v2_reference_cleanup_and_prefab_resolution` |
| F15 | CONFIRMED,已修 | 沒有 GUI 時自動環繞由固定步進以 0.25 rad/s 推進;有 GUI 時仍由每幀 `view` 推進(避免 `plan-ai-authoring-gaps` 記錄過的固定步進頓挫),GUI 的 `view` 帶 `auto:true` 表示由它負責。`effects-atlas` 文件已更新 | `tests/effects.rs::auto_orbit_advances_in_fixed_steps_without_a_gui` |

行為變更:`observe.gameplay.bindings` 現在包含可見的無變數滑桿(出貨內容沒有這種滑桿,state hash 不變);`narrative-check` 輸出多了 `skipped_native_buttons`;`showcase.view` 多一個選填 `auto`;`Gameplay::step`/`slider_value` 簽名改變(只有 runtime 內部呼叫);`scene_edit::delete` 變成公開。

Gate(R7 worktree,debug):`cargo fmt --all` 乾淨;`cargo clippy --workspace --all-targets -D warnings` 通過;`cargo test --workspace --no-fail-fast` 932 passed / 2 failed / 6 ignored,失敗的是 `m15.rs` 的 `agent_timeout_is_a_failed_task_and_never_a_zero_token_success`(`elapsed < 10 s`)與 `agent_adapter_runs_isolated_tasks_and_fixture_usage_is_not_model_usage`,都是外部程序計時(同時有其他 agent 在編譯,本次沒有改這條路徑),單獨重跑 2 passed;`editor-smoke-test` ok;`editor-world-parity` identical(rev 13);`street-editor-parity` identical(rev 16);`m0-smoke-test` PASS;另跑 `combat-smoke-test` PASS(F4 動到 action 勝利判定)與 `m1-smoke-test -SkipM0` PASS(F11/F12/F13 在 gameplay)。全部用非預設 port。修前驗證:把 `editing.rs` 換回修前版本時,F1 測試在 `serde_json` 索引處 panic、F14 測試失敗;其他新測試修前會失敗的理由見上表(panic/斷言值),沒有逐一回退實測。未驗證:F5 衝突後 GUI 的實際畫面(只驗證送出的 `expected_rev`);F15 在 GUI 下的平滑度沒有重新量測(GUI 路徑與修前相同);F12 沒有效能量測。
