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

### R6 音效與輸入

報告 `R6.md`(F1–F13)。逐條讀過引用的程式碼才下結論:13 條全部確認;F1 已由 R1 修好,本輪修 10 條(F6 為部分修正),延後 2 條(F2、F3)。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| F1 | CONFIRMED,R1 已修 | `audio::validate_world` 已是 `or_else(listeners.first())`(44d3a81);本輪只複查 | `axiom-core` `audio::tests::a_source_without_listener_or_spatial_system_is_a_diagnostic_not_a_panic` |
| F2 | DEFERRED | 確認:`ClipFile::validate` 只檢查路徑形狀,`preview`/`apply`/`verify` 不讀檔,缺檔要到 runtime 建置才失敗(`tests/audio_files.rs` 已記錄此行為)。正解是交易外的有界背景檢查(讀檔+解碼在 runtime、要回報進度與取消),並讓提交沿用已驗證的摘要與 `expected_rev`;跨 core 協定、store 與編輯器背景 job,不在本輪範圍。同步在驗證器裡讀檔會違反 16 ms 與「不隱式觸發重工作」規則,所以沒有做半套 | — |
| F3 | DEFERRED | 確認:authoring 只存路徑,`undo` 無法還原被覆寫的檔案內容。修正要在 `ClipFile` 加內容摘要欄位、匯入 op/ACL/編輯器 parity、pack resolver 比對、`asset_changed` 診斷與舊資料遷移,屬 authoring 格式變更 | — |
| F4 | CONFIRMED,已修 | `AudioSpatialSystem` 在短 one-shot(片段長度/pitch ≤ `SHOT_SECONDS` 0.25 s)持有 voice 而自然結束時,記錄一筆 shot(有界 32 筆、序號 + stream ID,超量計數 `shots_dropped`);`Output::update` 依游標把新 shot 以 `Message::Shot` 送到音訊執行緒;`Mixer::play_shot` 以固定增益從起點播到 PCM 結尾。兩個 64 格 token 環避免重複:voice block 已起播的 token 不再當 shot 播,已當 shot 播的 token 之後出現在 voice block 也不再起播。`stop` 不會產生 shot(只有自然結束才記錄),保留取消語意。observe 新增 `audio.shots`/`shots_dropped` 與 output `shots_played`。無配置:佇列容量預留,mixer 為固定陣列 | `tests/audio.rs::a_one_shot_that_ends_within_one_frame_still_reaches_the_mixer`(一次輸出前跑兩個固定步進,再檢查混音樣本非零)、`audio_mix::tests::a_one_shot_sounds_once_through_either_path` |
| F5 | CONFIRMED,已修 | `Events` 加 `stream`(`audio.events` factory 指派新的程序唯一 ID,同世界熱重載沿用);`Output` 在 session 或 stream 改變時把游標歸零。涵蓋換世界與移除後重新排入 `audio.events` | `audio::tests::a_new_cue_stream_on_the_same_session_is_not_skipped` |
| F6 | CONFIRMED,部分修正 | 快取命中不再讀檔+SHA-256:`cached_clip` 先以 (path, size, mtime, gain) 查有界(256)的 stamp→cache text 表,命中且 PCM 仍在快取就只花一次 `metadata`。檔案大小或 mtime 改變就重讀重雜湊(stamp 在讀檔前取,讀檔中被換掉的檔下次會重讀)。限制:同大小且 mtime 不變的覆寫不會被發現(NTFS mtime 每次寫入都會更新)。**未修的部分**:首次解碼與整個 Play runtime 建置仍在主執行緒同步執行;改成背景 runtime 建置屬 R2/R8 的編輯器/host 重設計。A/B(debug,4 個各 11 MB、已快取的檔案片段,`WorldRuntime::new` 8 次):修前 median 704.7 / 401.9 ms,修後 18.5 / 12.5 ms(兩輪,順序對調;機器同時有其他 agent 編譯) | `tests/audio_files.rs::an_unchanged_decoded_file_is_not_read_again_by_the_next_build`(Windows:以 share_mode 0 鎖住檔案後再建置仍成功) |
| F7 | CONFIRMED,已修 | `resolve_files` 在開第一個檔案前檢查 `clips.len() ≤ MAX_LIBRARY_CLIPS`,在 `/resources/audio.library/clips` 回 `audio_budget`;合成後的預算檢查保留 | `tests/audio_files.rs::an_over_budget_clip_count_fails_before_any_file_is_read` |
| F8 | CONFIRMED,已修 | 新增 `InputBindings::validate_keys`(`input.bindings` 驗證器呼叫):同名 button/axis 回 `action_kind`,路徑指向 `buttons/ACTION`,訊息附 `axes/ACTION`。`edit_input` 的 `Remove` 不再短路,同一筆編輯清掉兩邊 | `tests/tuning_ops.rs::duplicate_actions_and_unmatchable_keys_fail_validation_with_their_path` |
| F9 | CONFIRMED,已修 | `Contexts::transfer`:舊 `active` ID 仍存在就保留(路由、相機、指標捕捉取新定義),被移除才回 `initial`;`held` 一律沿用(實體按鍵狀態) | `input::tests::a_label_edit_keeps_the_players_context_and_a_removed_one_falls_back` |
| F10 | CONFIRMED,已修 | mixer 循環越界改 `pos %= len`(抽出共用 `mix_voice`) | `audio_mix::tests::a_one_sample_loop_at_high_pitch_keeps_playing` |
| F11 | CONFIRMED,已修(降頻分散) | `Gamepads::poll` 每次最多探測一個到期的空槽(round robin),已連接槽每幀照讀;observe 新增 `probes`。沒有新增執行緒或 crate。實測(本機無手把,debug):單一空槽 `XInputGetState` 41–176 µs,修前首幀/每秒一幀四槽合計約 250–390 µs,修後每幀最多一次(首次 poll 41–86 µs);兩者都遠低於 16 ms,此修正是把尖峰攤平 | `gamepad::tests::empty_slots_are_probed_one_per_poll_and_connected_ones_every_poll`(注入假 reader,驗證每次 poll 叫了哪些槽) |
| F12 | CONFIRMED,已修 | `ops::KEY_NAMES`:winit 0.30 全部具名 `KeyCode` 的 Debug 名稱 + `MouseLeft`/`WheelUp`/`WheelDown`;`valid_key_name` 與 `check_key_name` 用它,資源驗證器(`validate_keys`)與 `edit_input` 共用,診斷帶欄位路徑,`PadA` 這類值提示 `input pad` | `editor::input_map::tests::core_key_names_match_winit_and_captured_keys`(以 winit 本身逐一核對清單,並確認 Input Map 能擷取的鍵都在清單內)、上面的 `duplicate_actions_…` |
| F13 | CONFIRMED,已修 | 移除程序全域 `spatial_audio::file_info(path)`;Inspector 改從目前 runtime 的 `AudioSpatialSystem::clip_file(clip)` 讀,並比對路徑、標示 runtime revision,尚未載入顯示 not loaded。`docs/audio.zh-TW.md` 同步 | `tests/audio_files.rs::changed_file_content_restarts_and_old_paths_still_work_after_reload` 新增 `clip_file` 斷言(同路徑換檔前後 1.0 → 2.0 s) |

範圍外的觸碰:`axiom-core/src/ir.rs`(`InputBindings` 驗證)、`registry.rs` 一行、`editor/ui.rs` 一行、`docs/audio.zh-TW.md`。行為變更:`input.bindings` 內同名 button/axis、或 host 產生不了的鍵名,現在驗證失敗(既有 content/examples 都通過)。每個新測試都在暫時還原對應修正時確認會失敗(8/8 FAILED),之後已還原修正。

未驗證:實際聽感(無法聆聽;以 mixer 樣本與 observe 計數驗證);實體手把熱插拔(本機無手把,只量到空槽探測);WinMM 裝置下 `Message::Shot` 的端到端路徑只靠單元測試 + 閱讀(無裝置時 `send` 直接計入 `dropped`)。

Gate(R6 worktree,debug):`cargo fmt --all` 乾淨;`cargo clippy --workspace --all-targets -D warnings` 通過;`cargo test --workspace --no-fail-fast` 929 passed / 0 failed / 6 ignored;`editor-smoke-test` PASS(port 4652);`editor-world-parity` identical(rev 13,port 4666);`street-editor-parity` identical(rev 16,port 4662);`m0-smoke-test` PASS(port 4672)。沒有改渲染,未做截圖 A/B。
