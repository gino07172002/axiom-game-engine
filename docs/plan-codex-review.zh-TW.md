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

#### R1 第二輪(複審)

輸入:`R1.recheck.md` A 節 7 列「有問題」與 C 節 N1–N4。每條都重讀程式碼後判定;新測試都確認過在舊程式碼上會失敗(F3、F6 實際把修正拿掉重跑過,其餘靠閱讀:舊程式碼的行為正是測試斷言的反面)。

| ID | 判定 | 修改 | 測試 |
|---|---|---|---|
| A-6.2-F3(第一輪造成的回歸) | CONFIRMED,已修 | 實測:用舊程式碼開啟最新 revision 含 `audio.library: []` 的資料庫,`Store::open` 直接失敗(`prefabs/crate`)。修法:讀取時正規化、寫入時仍嚴格。`TypeDescriptor` 新增 `normalize`;`Registry::core` 裡 29 個用 `typed` 驗證的內建 struct 型別改由 `typed!` 巨集註冊(`typed_component`/`typed_resource`),同時登記 `object_form::<T>`(陣列 → 反序列化成 T → 序列化回物件)。`Registry::decode`(所有讀取 revision、package、proposal base、undo 的共同入口)之後呼叫 `Registry::normalize`,改寫 entity 元件、prefab 元件與資源中的陣列形式;只看陣列值,其他值只多一次型別判斷。已存的舊 revision 本身不改寫(歷史不變);下一次提交寫入物件形式,diff 不會把正規化算成變更。新寫入的陣列形式仍被拒絕。其他第一輪新增的拒絕(`ops_tuning`/`ops_ui` 的非物件資源、`item_id`、manifest 邊界)只在執行 op/選取時回診斷,修前是 panic 或越界,不影響開啟資料庫,不是同類回歸 | 新檔 `tests/legacy_forms.rs`:`a_database_holding_legacy_array_forms_opens_edits_and_undoes`(暫存目錄建舊格式資料庫 → open、verify、preview、apply op、diff、undo 回舊 revision、再編輯、重開)、`new_writes_of_the_array_form_are_still_refused`、`decoding_rewrites_only_registered_object_types`;拿掉修正後 3 條全失敗 |
| A-6.2-F6 / 6.1-F4 | CONFIRMED,已修 | `Edit::apply_budgeted(w, r, &mut budget)`;`Edit::apply_all` 讓整批共用一份 `MAX_SET_WHERE_BYTES`,在複製前扣除。`Store::edit_in`、`ops::apply_edits`(proposal/merge)、runtime `writer::stage` 都改走 `apply_all`;ACL 腳本(一次一批)在 scratch 上也共用一份預算。單筆 `apply` 行為不變 | `tests/selectors.rs::set_where_edits_of_one_batch_share_one_expansion_budget`(34 筆約 481 KB 可過,36 筆在第 35 筆回 `budget_exceeded` 且不提交;改回每筆獨立預算時此測試失敗) |
| A-6.2-F10 | CONFIRMED,已修 | 第一輪的「純 ID 清單可 rebase」前提是錯的(不存在的 ID 會被略過,之後出現就會被選到;choice ID 可跨節點重複)。改成 `Store::selectors_grow`:在 base revision 與目前 revision 上依序重播整批 edits,比對每個 `set_where` 解析出的完整目標(`Target::path`,含 kind 與 choice 的 node);目前版多出任何目標就回 `stale_selector`。目標相同(例如 tag 選取但無新物件)可照常 rebase。刪除不再使用的 `Selection::is_id_list` | `tests/proposals.rs::an_explicit_id_that_only_exists_after_the_proposal_is_not_rebased`、`a_choice_id_shared_by_a_new_node_is_a_new_target`、`a_selector_that_resolves_to_the_reviewed_targets_still_rebases`;第一輪的 tag 測試仍過 |
| A-6.2-F8 | CONFIRMED,已修 | `read_bounded` 先 `fs::metadata`(跟隨連結)確認是一般檔案才 `open`,開啟後再檢查 handle。剩餘風險:metadata 與 open 之間路徑被換成 FIFO 的競態(要用 `O_NONBLOCK` 開啟,需要 libc 或平台常數,本輪不做) | `tests/packages.rs::bounded_reads_refuse_a_fifo_without_blocking`(`cfg(unix)`,用 `mkfifo`,本機 Windows 不會跑)、`bounded_reads_refuse_a_directory_before_opening_it` |
| A-6.2-F14 / 6.1-F9 | CONFIRMED,已修 | narrative schema 的 operand 補 `animation {entity, field: clip…param, param?}`;`batch.commands` 改指向 `$defs/batch_command`(只有 choose/emit/locale/call),`maxItems` 用 `gameplay::MAX_BATCH` | `tests/protocol_contract.rs::gameplay_schema_accepts_and_refuses_the_same_samples_as_the_runtime`:測試內的小型 JSON Schema 驗證器(只支援 discovery 用到的子集),對合法/非法完整樣本比對 schema 與 serde/`validate_command` 的判定 |
| A-6.1-F10 | CONFIRMED,已修 | CLI 在開 Store 前先用 `registry_only` 回答 `schema`/`discover` 的純 Registry 主題(`typed:ID` 仍開 Store);回應由新的 `Store::registry_response` 產生,`Store::execute` 的 Discover 也共用 `discover_response`,所以 bytes 與 hash 相同 | `axiom` lib `tests::registry_topics_are_answered_without_a_store_and_world_topics_are_not`;手動見下 |
| A-6.2-F13 | CONFIRMED,已修 | `observe_delta` 先檢查每個欄位的 RFC 6901 跳脫(`~` 後只能是 0/1),不合法回 `invalid_field`;合法但暫時缺值仍列在 `missing_fields` | `observe::tests::a_malformed_pointer_escape_is_an_error_but_an_absent_optional_field_is_not` |
| N1 | CONFIRMED,已修 | `Predicate::test` 兩個整數(i64/u64,以 i128)精確比較,其他情況才用 f64 | `tests/selectors.rs::integer_predicates_compare_exactly_beyond_two_to_the_53`(select eq/lt 與 set_where 只命中 2^53+1) |
| N2 | CONFIRMED,已修 | 新增唯讀請求 `{"op":"proposal_diff","id","offset","limit"}`:重算提案在 base revision 上的完整變更並分頁(limit 1..128,預設 32,與 `diff` 共用 `diff_page`);`discover` 的 review 範例列出 | `tests/proposals.rs::a_proposal_with_more_than_64_changes_can_be_paged_before_approval` |
| N3 | NOT A BUG | `bridge::call` 的確把 `ok:false` 當 `Ok(Value)`,但 CLI 的單次分派最後一律經過 `print`,而 `print`(自 f9302ca 起)遇到 `ok:false` 會把原始回應(含 diagnostics)印到 stderr 並 `exit(1)`;`verify` 與 `export` 也經過 `print`。`serve` 用 `println!` 逐行回應,不受影響。沒有改 bridge | 手動:loopback stub 回 `revision_conflict` 的 `ok:false`,`axiom --connect … apply` 結束碼見下 |
| N4 | CONFIRMED,已修 | 投影時跳過「祖先欄位也被要求」的子欄位(祖先的值已包含它),兩種順序結果相同;子欄位不存在時仍列入 `missing_fields` | `observe::tests::a_parent_and_child_field_project_the_same_in_either_order` |

範圍外的觸碰:`axiom-runtime/src/writer.rs`(F6,`stage` 改用 `Edit::apply_all`,一行)。協定新增 `proposal_diff`;`schema narrative.graph` 與 `discover` review 主題的 hash 會改變。

Gate(r1b worktree,debug):`cargo fmt --all` 乾淨;`cargo clippy --workspace --all-targets -D warnings` 通過;`cargo test --workspace --no-fail-fast` 962 passed / 1 failed / 6 ignored,失敗的是 `axiom-runtime` `m15::agent_timeout_is_a_failed_task_and_never_a_zero_token_success`(`started.elapsed() < 10 s` 的牆鐘斷言,機器同時有其他 agent 編譯;與本輪無關),單獨重跑 1.42 s 通過 → 963 passed;`editor-smoke-test -Port 4911` PASS;`editor-world-parity -Port 4912` identical(rev 13);`street-editor-parity -Port 4913` identical(rev 16);`m0-smoke-test -Port 4914` PASS。沒有改渲染,未做截圖 A/B。手動(隔離目錄,`content/fps/world.json` 故意損壞):`axiom --db <tmp>/w.sqlite schema edits`、`discover local` 結束碼 0 且沒有建立資料庫;`schema typed:core.health` 需要世界,仍會開 Store(照預期因損壞的預設內容包失敗)。N3:loopback stub 回 `ok:false` 的 `revision_conflict`,`axiom --connect 127.0.0.1:47391 apply FILE` 結束碼 1,stderr 為原始 diagnostics。未驗證:FIFO 測試只在 Unix 執行(本機 Windows 跳過);`read_bounded` 在 metadata 與 open 之間換成 FIFO 的競態沒有處理。

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

#### R2 第二輪(複審)

輸入:`R2.recheck.md`(A 段 9 項「有問題」、C 段 N1/N2)。每項先讀程式碼再判定;全部 CONFIRMED 並修正。worktree `r2b`(基於 `integ`)。

| ID | 判定 | 改動 | 測試 |
|---|---|---|---|
| A-F2 | CONFIRMED(兩點都成立) | (1)啟用失敗後 `requested_rev != rev`:`runtime_request` 的 `Command` 若未帶 `expected_rev` 回 `activation_failed`(附 `requested_rev`/`live_rev`)且不執行;帶 `expected_rev = live_rev` 表示刻意操作執行中的版本,照常執行。(2)Bridge 等待改為 deadline + `RUNTIME_REPLY_GRACE`(1 s),讓主執行緒的 `runtime_timeout`(含兩個版本欄位)送達;主執行緒連 grace 都沒回時,Bridge 自己回同一 code,版本取自 observation。`frame` 的請求迴圈抽成 `handle_commands` 供測試共用 | `app::tests::a_bridge_command_never_runs_on_the_older_revision_after_a_failed_or_slow_activation`(真實 loopback Bridge + `bridge::call`,m4 的 `rts.train` 以 `spent` 觀察副作用;涵蓋建置失敗、逾時、之後在新版本執行) |
| A-F5 | CONFIRMED | `WorldRuntime::new_replacing(.., first_generation)`:handle generation 在 factory 執行前就從替換起點開始。同步 `reconcile` 用舊 runtime 的 `next_generation`;背景啟用用 `next_generation + 2^32`(建置期間舊 runtime 仍可 spawn)。`adopt` 跨 `world.id` 時只有在 generation 會撞到舊值(用 `new` 建的,或 spawn 超過 headroom)才重新編號(保底,舊 handle 永不解析)。同一世界的 `transfer_state` 不變 | `generic_runtime::a_handle_a_system_factory_keeps_resolves_after_switching_to_another_world`(公開 `Plugin` 的 factory 保存 `state.ids[0]`)、`world::generation_tests::a_replacement_built_off_thread_keeps_its_factory_handles_across_worlds` |
| A-F9 | CONFIRMED(兩點) | (1)`writer.rs`:開庫失敗不再永久保存;每個請求前重試開庫,仍失敗回 `writer_unavailable`(暫不可寫,可重試)。(2)watchdog 換 writer 失敗時設 `writer_required`,之後的編輯先重試起 writer,失敗回 `writer_unavailable`,不再退回主執行緒同步提交。`axiom-runtime` 新增 dev-dependency `rusqlite`(與 axiom-core 相同版本與 features,不增加建置的 crate) | `writer::tests::a_writer_that_could_not_open_the_database_retries_on_the_next_request`(另一連線 `BEGIN IMMEDIATE` 持鎖超過 busy timeout)、`app::tests::an_edit_after_a_failed_writer_restart_starts_a_writer_again`;thread spawn 失敗分支靠閱讀 |
| A-F10 | CONFIRMED(兩點) | (1)`Reload::poll` 對 stop 後完成的 `Ok(Built)` 標記 `stopped`,由 `decide` 回 `Discard(Some(runtime))` 交給 `retire`,不在畫幀執行緒釋放。(2)`Job` 的 worker 連同完成時間送出結果;`poll` 領取時若完成時間超過 deadline 一律回 `TimedOut`(準時完成但晚被 poll 的不算) | `asset_reload::tests::a_reload_cancelled_after_its_build_finished_is_not_adopted`(改為帶實際 `WorldRuntime`,並加逾時後才被領取的情況)、`jobs::tests::a_result_finished_past_the_deadline_is_timed_out_even_when_already_queued` |
| A-F12 | CONFIRMED | `reload_assets {cancel}` 也取消它自己啟動的 activation 重試(`retry_rev`;取消後不採用,runtime 留在 live 版本)與等待中的編輯器 reload。GUI `asset.reload` 在 activation 進行中改為等待(`reload_waiting`)完成後再跑,與 RPC 延後執行一致,不再對即將被取代的舊版本重建 | `app::tests::reload_assets_start_and_cancel_agree_while_an_activation_is_involved` |
| A-F6 | CONFIRMED | 新增 `world::request_error_response`(`batch_partial` 帶 `applied_count`/`failed_index`),`handle_command` 與 benchmark `exchange` 共用。receipt 測試改用會累加的前綴(`rts.train` 扣 60 minerals) | `app::tests::a_partly_applied_live_execute_batch_applies_its_prefix_once`(live `execute` 經 `handle_command`);`benchmark::retry_tests::a_partly_applied_batch_gets_a_receipt_with_its_applied_count`(改為累加前綴並檢查 `exchange` 回覆) |
| A-F8 | CONFIRMED | `scan` 的資料夾上限改為 `FOLDER_FILES + 1`(清單含模型本身),每個模型扣掉自己後仍須 ≤ 512,與 `companions` 同語意。無法讀取時不再轉成 `None`:記在 `watch_error`(observe `assets_reload.watch_error`),新錯誤時寫到 console 與編輯器狀態列一次;掃描逾時也回報。`job_observation` 在沒有 reload job 時顯示執行中的 `asset_watch` 掃描(狀態列 jobs) | `asset_reload::tests::a_flat_folder_with_512_companions_is_watched_and_one_more_is_reported` |
| A-F14 | CONFIRMED | `Assertion::Field` 指向 `float`/`vec3` typed column 時,期望值先轉成 `f32` 精度再比較(數字與陣列) | `scenario::tests::float_field_assertions_compare_at_the_columns_precision`(`eq 0.1`、`le`、`vec3` 元素與整個陣列;`eq 0.11`、`gt 0.1` 仍失敗) |
| A-F11 | CONFIRMED(測試缺口) | 程式不變;補正式入口測試 | `app::tests::an_editor_asset_reload_is_cancelled_and_watched_over_the_loopback`(含模型的 edit session:GUI `asset.reload` 啟動、loopback `cancel`、loopback `watch` 開關與 observe) |
| N1 | CONFIRMED | `Editor::check_remote`(無副作用):未存草稿、`select` ID、`dock` 名稱、`open` 目標(與 `remote_open` 相同的 not_found 診斷)、未知 `command` ID,在 `editor_play` 之前檢查。gizmo 拖曳條件與指令的狀態相依可用性仍由原本的地方檢查(Play 會結束拖曳)。未動 capture 處理 | `app::tests::a_refused_editor_view_never_starts_play` |
| N2 | CONFIRMED | `App::record_command`:三個記錄點(bridge/RPC、元件 ▶、遊戲 UI)共用;`PartialBatch` 呼叫 `recorder.mark_inexact`,錄製不寫 `state_hash` 斷言 | `app::tests::a_partly_applied_batch_makes_a_play_recording_inexact`(m4 錄製,摘要 `exact:false`,檔案可重播) |

行為變更:未帶 `expected_rev` 的 runtime `command` 在最新版本啟用失敗時被拒(`activation_failed`);Bridge runtime 請求最長多等 1 s(仍在 `call` 的 5 s 讀取期限內);`Job` 在 deadline 後才完成的結果標為 `timed_out`(scenario job 也適用);背景啟用切換到另一個 `world.id` 後 handle generation 數值會大(起點 +2^32)。

範圍外觸碰:`editor/graph.rs`(`graph_world` 改 `pub(super)` 供 `check_remote` 使用)、`axiom-runtime/Cargo.toml`(dev-dependency)。

Gate(worktree r2b,debug):`cargo fmt --all` 乾淨;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 958 passed / 4 failed / 6 ignored,4 個失敗都是負載下的計時測試(其他 agent 同時在編譯):`app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`、`editor_scenario_watchdog_stops_a_long_run`、`bridge::tests::fragmented_tcp_request_and_atomic_round_trip`(連線逾時 10060)、`m15::agent_timeout_is_a_failed_task_and_never_a_zero_token_success`,單獨重跑全部通過。新增的 17 個測試(含改寫的)單獨跑全過。`editor-smoke-test -Port 4752` PASS;`editor-world-parity -Port 4766` identical(rev 13);`street-editor-parity -Port 4762` identical(rev 16);`m0-smoke-test -Port 4772` PASS。無渲染改動,不做截圖 A/B。

未驗證:新測試在修正前會失敗是依程式碼推理(每個測試重現複審描述的序列),沒有逐一 revert 實跑;F9 的 thread spawn 失敗分支無法在測試中觸發,靠閱讀;未做 A/B 量測,不宣稱效能數字。

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

#### R4 第二輪(複審)

依 `docs/reviews/codex-2026-11/R4.recheck.md`:A 節 6 項「有問題」與 C 節 N1–N3。8 項 CONFIRMED 並修改,F5 部分修改、其餘 DEFERRED。改動只在 `crates/axiom-animation` 與 `axiom-runtime/src/animation*.rs`(及其測試);沒有改 axiom-core、app.rs 或渲染檔案。測試輸入都在程式碼裡產生。

| ID | 判定 | 修改 / 理由 | 測試 |
|---|---|---|---|
| N1 | CONFIRMED | `gltf 1.4.1` 的 accessor iterator 先算 `stride * (count - 1)`、`offset + length` 才檢查 slice,並對非預期 component type `unreachable!`。`import::gltf::check_accessors` 在建立任何 reader 之前檢查:每個 buffer view 在 buffer 內(`checked_add`);每個 accessor `count >= 1`、密集大小 ≤ 256 MiB、byteStride ≥ 元素大小、範圍用 checked arithmetic 落在 view 內;sparse 的 count 在 `1..=count`,indices / values 範圍同樣檢查;POSITION/NORMAL/TEXCOORD_0/JOINTS_0/WEIGHTS_0/indices/morph target/inverseBindMatrices/animation input/output 的 dimensions 與 component type 符合 reader 的假設 | `import_limits::malformed_gltf_accessors_are_errors_not_panics`(count 0、count 溢位、超出 view、offset 近 `usize::MAX`、VEC2 POSITION、float indices、sparse count 0、sparse values 超界)、`a_gltf_view_stride_shorter_than_its_element_is_an_error`;拿掉檢查實跑兩個測試都在 gltf crate 內 panic(util.rs:376 / :384) |
| A-F2 | CONFIRMED | `push_texture` 改收編碼位元組:先用 `ImageReader::into_dimensions` 讀檔頭,以 `寬 × 高 × 4` 對剩餘的 256 MiB 預算做 checked 檢查,超出就不解碼;解碼後再以實際大小確認一次。讀檔共用 `import::read_texture(impl Read)`(最多讀 32 MiB + 1 byte) | `import_limits::an_oversize_texture_stream_is_read_only_to_the_limit`(計數 reader 提供 64 MiB,斷言實際讀取量 = 32 MiB + 1);`pmx_textures_share_the_decoded_texture_budget` 的第 5 張改為只有檔頭、截斷在 IDAT 內的 PNG,只有先檢查尺寸才會得到預算錯誤(拿掉預檢實跑:失敗) |
| A-F3 | CONFIRMED | `crossings` 不再用 `prev / duration`(舊 checkpoint 的 `time = 86400` 配 `duration = 1e-35` 會變成 `inf` 而整步跳過);改為 `prev.rem_euclid(duration)`(精確餘數)求本圈位置,再加上本步時間差 | `animation::review_tests::an_unnormalized_checkpoint_time_over_a_tiny_loop_still_fires_its_first_step` |
| A-F8 | CONFIRMED | 指令成功提交時,新 player 以 `SkinCache::take_from` 接手舊 player 的快照(指令不改 asset 與 skinning mode;快照以 pose + morph 權重為鍵),所以等值變更(可見性、同時間 seek)後的擷取重用同一份 `Arc`。骨矩陣暫存:`skin::deform_scratch` + `skin::Scratch` 由快照保存並重用(`globals` 仍每次配置)。**沒有 A/B 量測,不宣稱效能改善** | `animation_review::an_unchanged_pose_reuses_the_skinned_vertices`(新增可見性來回與同時間 seek 後 `Arc::ptr_eq`) |
| A-F12 | CONFIRMED | 球與圓柱側面的判別式改由最近點的垂直偏移計算(`r² − |q|²`),不再用會在 `f32` 消去的 `b² − (|oc|² − r²)`;側面在垂直於軸的平面求解 | `animation_parts::tests::far_rays_beside_a_tiny_part_miss_it`(100 m 外、垂距 4r 的 5 mm 球與 1 cm 膠囊必須未命中;半徑內仍命中前表面) |
| A-F14 | CONFIRMED | `part_names` 先量長度再組字串(不再先 `join` / `format!`),並為「+N more」提示保留空間,整段 ≤ 1 KiB;錯誤訊息回顯的 selector 以 `clipped` 截到 64 bytes | `animation::review_tests::part_lists_in_errors_stay_within_their_byte_budget`(超長群組名、接近填滿預算的項目、40 個 primitive、單項超過預算;經指令送出 250 字元的無效 selector,schema 已限制 ≤ 256 字元) |
| N2 | CONFIRMED | 熱重載只有在舊 `active` 仍存在**且**作者設定的 clip 未改時才延續 `time` 與 `marker_fresh`;改了 clip 就照作者設定從頭播放 | `animation_review::an_authored_clip_switch_starts_the_new_clip_from_its_head`(長片段播到 3 秒後改成 1 秒單次片段;無關編輯仍延續) |
| N3 | CONFIRMED | 新增 `Controller::playhead()`:有 controller 時回傳 controller state 的 primary clip 與 `frac × duration`,否則回傳 clip 播放欄位。`inspect`、`observe`(metrics)與 gameplay bindings 共用 | `animation_review::inspect_and_observe_report_the_controller_clip_and_time`(animator-lab 走到 `move` 後兩個入口都報 `walk` 與相同秒數) |
| A-F5 | 部分 CONFIRMED / DEFERRED | 見下 | `import_limits::an_import_fills_the_clip_index_the_validator_reads` |

F5 的處理:
- **已做(縮小同步成本)**:`import::load` 成功後把 clip ID 寫入 `clip_ids` 用的同一份索引(path + 長度 + mtime)。runtime 建置(在啟用工作中)匯入過的資產,之後的 `preview`、`apply`、編輯器 Verify 都直接命中索引,不再讀檔。測試把檔案換成同長度、同 mtime 的亂碼,`clip_ids` 仍回傳匯入時的結果;mtime 改變後會重新讀取。
- **量測(debug build,其他 agent 同時編譯造成高負載)**:索引未命中時 `clip_ids` 解析 `.gltf`:1 MiB 116 ms、16 MiB 444 ms、100 MiB 2.81 s;命中 0.6–0.9 ms。release 未量測。這是只有「本 process 尚未匯入過該資產」時的成本(新的 CLI process、或剛加入的資產)。
- **DEFERRED 1(片段可綁定性)**:runtime 只收錄骨名 / morph 名都能綁定的 library clip,驗證只比對 ID。正確做法需要各 importer(glTF、PMX、`.axrig`)在不完整匯入的情況下產出骨骼 ID 與 morph target 名(glTF 的 ID 規則含重名 `#index` 與 morph 名衝突前綴,PMX 需要跳過頂點區塊),讓驗證與 runtime 共用同一個「可綁定片段索引」。
- **DEFERRED 2(修復編輯被舊 revision 擋下)**:`Store::edit_in` 為了計算差異會 `previous.compile(registry)`,重跑所有 validator;外部資產改名後舊 revision 本身就驗證失敗,修好新 world 的編輯也被拒絕。需要在 axiom-core 讓差異計算只做結構編譯(不跑依賴外部檔案的 validator),屬於 R1 範圍。
- **DEFERRED 3(寫入交易內解析)**:索引未命中時解析仍發生在 `apply` 的 SQLite 寫入交易內。避免它需要 axiom-core 在開交易前呼叫一個 registry 的「預先載入」hook(或背景預熱索引),同樣屬於 R1 的 store 範圍;本輪沒有在動畫範圍內能避開的做法。

Gate(debug,worktree `r4b`,其他 agent 同時編譯造成高負載):`cargo fmt --all` 已套用;`cargo clippy --workspace --all-targets -- -D warnings` 通過;`cargo test --workspace --no-fail-fast` 579 passed、2 failed,兩個都是負載下的計時測試(`app::tests::editor_scenario_runs_in_background_with_progress_cancel_and_result`、`m15::agent_adapter_runs_isolated_tasks_and_fixture_usage_is_not_model_usage`,後者以 15 s timeout 啟動 PowerShell),單獨重跑都通過;`editor-smoke-test` ok(`-Port 14452`);`editor-world-parity` identical(rev 13,`-Port 14466`);`street-editor-parity` identical(rev 16,`-Port 14462`);`m0-smoke-test` PASS(M0 + QA,`-Port 14262`)。沒有改渲染檔案,所以沒做截圖 A/B。

修正前會失敗的確認:N1 兩個測試與 F2 的預算測試在暫時拿掉檢查後實跑,都失敗(N1 在 gltf crate 內 panic)。其餘依推理:F12 的舊公式在該射線上判別式捨入為 0 並回傳 100;F3 舊程式 `prev / duration = inf` 直接 return;F8 舊程式提交後快取為空,`Arc::ptr_eq` 不成立;F14 的「接近填滿預算」案例舊程式加上提示後超過 1 KiB,selector 也不會截斷;N2 舊程式延續 3 秒;N3 舊程式回報 `idle` / 0。
