# 編輯器 ↔ 無頭(headless)對等表

驗收標準:**人在編輯器做得到的事,AI 不開編輯器也做得到,而且結果一樣。**

會改內容的編輯器功能都實作在 `axiom-core/src/ops.rs`(純函式 `fn(&World, 參數) -> Result<Vec<Edit>>`)。
編輯器 UI 跟 ACL 動詞呼叫的是同一份程式碼,寫入時都走同一套交易協定(CAS、收據、diff、undo)。
無頭用法:`axiom --db DB acl -e "..."`;要用到引號時改成 `axiom --db DB acl FILE`。可以加 `--preview` 或 `--propose`。

對等測試:
- `crates/axiom-runtime/src/editor/parity.rs`:用編輯器命令或按鈕實際呼叫的方法做一次,再用 ACL 腳本跑一次,檢查兩邊得到的 `World` 完全相同(ID 也要一樣)。
- `crates/axiom-core/tests/ops.rs`:檢查 ACL 動詞編譯出的 edit 批次跟 ops 的結果相同,錯誤也有結構化代碼。

## 會改內容的功能

| 編輯器功能 | 無頭等價 | 對等測試 |
|---|---|---|
| `object.empty` / `object.cube` / `object.camera`(+ Object 選單、Ctrl+Shift+N) | `create empty\|cube\|camera [id=] [at=x,y,z] [parent=] [scene=]` | ✅ `create_presets_match_acl`、`repeated_creates_and_scene_filter_match_acl` |
| 篩選某個場景時新增物件,物件自動加入該場景 | `create … scene=ID` | ✅ 同上 |
| `object.light_directional` / `object.light_point` / `object.light_spot` / `object.light_area`(Object 選單、指令面板)、Hierarchy「+ Object」▸ Directional / Point / Spot / Area light | `create light directional\|point\|spot\|area P`(也可寫 `create light.KIND P`;只寫 `create light` 會回 `acl_syntax`,要求種類)。建立 `render.light`,沿本地 -Z 照射,見 `docs/lighting.zh-TW.md` | ✅ `light_presets_match_acl`(兩種編輯器入口 × 四種燈、空世界補 render capability、場景篩選);`light_presets_create_valid_lights_aimed_along_minus_z` |
| `object.trigger_box` / `object.trigger_sphere`(Object 選單、指令面板)、Hierarchy「+ Object」▸ Box trigger / Sphere trigger | `create trigger box\|sphere P`(也可寫 `create trigger.box`;只寫 `create trigger` 回 `acl_syntax`)。建立 `trigger.zone`(filter tag `player`、沒有事件),世界缺少時補 `trigger` capability 與 `trigger.detect` 系統,見 `docs/triggers.zh-TW.md` | ✅ `trigger_presets_and_field_edits_match_acl`(兩種編輯器入口 × 兩種形狀、空世界補 capability / 系統、Inspector 表單改欄位 = ACL `set`) |
| `object.path`(Object 選單、指令面板)、Hierarchy「+ Object」▸ Path | `create path P`。建立 `path.spline`(8×6 m 封閉矩形),世界缺少時補 `path` capability 與 `path.move` 系統,見 `docs/paths.zh-TW.md` | ✅ `path_presets_follow_and_point_edits_match_acl`(兩種入口、空世界補 capability / 系統) |
| `object.follow_path`(Object ▸ Follow Path):選取的物件跟隨選取中的路徑(沒選路徑時用唯一的路徑);多個跟隨者起點平均分散 | `follow SEL PATH [speed=] [end=loop\|ping_pong\|stop] [offset=]` | ✅ 同上(兩個跟隨者的 `start_fraction`、沒選路徑時的唯一路徑;兩條路徑都沒選 = problem 不寫入);`path_verbs_match_the_ops_and_report_structured_errors` |
| `path.add_point` / `path.remove_point`(Object 選單、Inspector ▸ Path points 按鈕),作用在選取的點 | `path ID add [x,y,z]`、`path ID insert I [x,y,z]`、`path ID remove I` | ✅ 同上(無選點 add、選點 1 後 insert 2、remove 0;低於最少點數 = problem) |
| 場景視口拖曳路徑點把手(水平面、吸附) | `path ID move I x,y,z` | ✅ `path_point_handles_drag_one_point_through_the_shared_op`(egui 事件實際拖曳,結果與 ACL 相同)+ parity 測試的 `edit_path_point` |
| `object.obstacle`(Object 選單、指令面板)、Hierarchy「+ Object」▸ Obstacle | `create obstacle P`。1 m 方塊 + `nav.blocker {half:[0.5,0.5]}`,世界缺少時補 `render` capability,見 `docs/navigation.zh-TW.md` | ✅ `nav_agents_obstacles_and_targets_match_acl`(兩種入口、空世界) |
| `object.nav_agent`(Object ▸ Add Nav Agent):選取的物件都加上預設 `nav.agent`(取代既有的) | `nav SEL agent [speed=] [radius=]`(世界缺少時補 `nav` capability 與 `nav.steer` 系統) | ✅ 同上;`nav_verbs_match_the_ops_and_report_structured_errors` |
| `nav.target_object`(Object ▸ Nav Target: Selected Object):選取中的代理走向選取中唯一不是代理的物件(沒有或兩個以上 = problem 不寫入) | `nav AGENTS target ENTITY` | ✅ 同上 |
| `nav.target_point`(Object ▸ Nav Target: Click Ground…):下一次在場景視口點地面(通過主選取代理高度的水平面),Esc 取消 | `nav AGENTS target x,y,z` | ✅ `nav_ground_pick_sets_the_target_through_the_shared_op`(egui 事件實際點擊,結果與 ACL 相同)+ parity 測試的 `set_nav_target` |
| `nav.clear_target`(Object ▸ Nav Target: Clear) | `nav AGENTS target none` | ✅ `nav_agents_obstacles_and_targets_match_acl` |
| `object.interactable`(Object ▸ Make Interactable,指令面板):選取的物件(選取順序)都加上預設 `interact.target`(取代既有的) | `interact SEL [event=E] [prompt=TEXT] [range=M] [action=BUTTON]`。一個 revision 內補 `interact` capability 與 `interact.focus` 系統、在 `behavior.program` 宣告事件(預設 `interacted`,文字欄位 `interactor` / `target`;沒有 program 就建立並補 `behavior.execute`)、按鈕(預設 `interact`)沒綁定時綁 `KeyE`(`KeyE` 已被占用 = `missing_binding`),見 `docs/interaction.zh-TW.md` | ✅ `make_interactable_and_field_edits_match_acl`(選取順序、空世界、Inspector 表單改 range / 中文 prompt / priority = ACL `set`);`interact_verb_matches_the_op_and_wires_events_and_bindings` |
| `object.audio_source`(Object ▸ Add Audio Source,指令面板):選取的物件(選取順序)都加上預設 `audio.source`(取代既有的;片段 = library 第一個,沒有片段時同一個 revision 建立 `tone`) | `audio SEL source [clip=] [volume=] [loop=on\|off] [autoplay=on\|off] [spatial=on\|off] [min=] [max=] [rolloff=inverse\|linear]`。世界缺少時補 `audio` capability 與 `audio.spatial` 系統;片段不存在 = `missing_clip`,見 `docs/audio.zh-TW.md` | ✅ `audio_sources_listeners_and_clips_match_acl`(選取順序、空世界、Inspector 表單改 loop / max_distance / rolloff = ACL `set`);`audio_verbs_match_the_ops_and_report_structured_errors` |
| `object.audio_listener`(Object ▸ Make Audio Listener,主選取) | `audio ID listener`(其他 active 聽者同一個 revision 設為 false;直接寫出兩個 active = `audio_listener_conflict`) | ✅ 同上 |
| Inspector ▸ Audio source ▸ Clip 下拉選單 | `set ID audio.source/clip "CLIP"`(`ops::set_audio_clip` 先檢查片段存在,產生同一個 field edit) | ✅ 同上 |
| Inspector ▸ Audio source ▸ New clip ▸ tone / engine / fountain / creak / footstep(名稱 = 預設名,已存在時 `engine2`…) | `audio clip ID PRESET`(程序合成片段,不用音檔;ID 已存在 = `id_taken`) | ✅ 同上 |
| `object.tween_clip`(Object ▸ Add Tween Clip,指令面板):選取的物件都加上新片段(名稱 `clip`、`clip2`…,第一條軌 `core.transform/position`,0 與 1 秒兩個 key 記錄目前值) | `tween SEL clip NAME [property=] [target=] [duration=] [mode=] [speed=] [autoplay=] [on_finish=] [play_on=]`。一個 revision 內補 `tween` capability 與 `tween.animate`(`on_finish` / `play_on` / 變數軌另補 `behavior.execute` 並宣告事件;`play_on=E` 在 `runtime.bindings` 加一個 `tween.play` 呼叫),見 `docs/tweens.zh-TW.md` | ✅ `tween_clips_keys_and_options_match_acl`(兩個物件、第二個片段名稱、空世界);`tween_verbs_match_the_ops_and_record_keys`、`tween_play_on_wires_runtime_bindings_and_remove_unwires` |
| Inspector ▸ Tween timeline ▸ Record key(`tween.record_key`):選取軌的編寫值存成播放頭時間的 key | `tween ID key CLIP TRACK add T [V] [ease=]`(不寫 V = 記錄目前值) | ✅ 同上(移動物件後記錄) |
| 時間軸拖曳 key(放開時寫入,吸附 0.01 秒)/ Delete key(`tween.remove_key`) | `tween ID key CLIP TRACK move I T` / `… remove I` | ✅ 同上;`timeline::tests::click_scrubs_and_dragging_a_key_requests_a_move`(egui 事件實際拖曳) |
| 時間軸的模式下拉、Autoplay 勾選 / Add track / Remove clip | `tween ID clip NAME set mode=\|autoplay=…` / `tween ID track CLIP add P [target=]` / `tween ID clip NAME remove`(連同播放它的 `runtime.bindings` 呼叫) | ✅ 同上 |
| Assets ▸ 點 prefab 實例化 | `instantiate PREFAB [id=] [at=] [parent=] [scene=]` | ✅ `instantiate_prefab_matches_acl` |
| 大量生成(帶 tag、label、step 間距) | `spawn PREFAB count=N …`(只有 ACL 有;編輯器只能一次一個) | — |
| `edit.duplicate`(Ctrl+D)、Inspector 的 Duplicate | `duplicate SEL`(ID 依序為 `SRC/copy`、`/copy/2`;`entities[id=a,id=b]` 照書寫順序處理;每個複本緊接在來源**之後**,也就是同層的下一列) | ✅ `duplicate_and_paste_match_acl`(含順序檢查) |
| `edit.paste`(Ctrl+V,複製剪貼簿裡的 ID) | `duplicate SEL` | ✅ 同上 |
| `edit.delete`(Delete)、Inspector 的 Delete | `delete SEL` | ✅ `delete_matches_acl` |
| Hierarchy 拖曳到列的中間 = 改 parent(保留世界姿態) | `reparent ID PARENT\|root`(預設 `keep=world`) | ✅ `reparent_drag_matches_acl` |
| Hierarchy 拖曳到列的上緣 / 下緣 = 同層排序(青色細線指示);來源 parent 不同時先改 parent 再排序,同一筆 revision | `move ID before OTHER\|after OTHER\|end`(必要時前面加 `reparent ID P\|root`) | ✅ `hierarchy_reorder_drop_matches_acl` |
| `edit.rename`(F2、右鍵 ▸ Rename):列內改名,Enter / 點別處提交、Esc 取消,支援中文 | `rename ID "LABEL"`(前後空白會修掉;`""` 清除 label,列改顯示 ID) | ✅ `rename_matches_acl` |
| `object.child_empty\|child_cube\|child_camera`(右鍵 ▸ Create Child) | `create empty\|cube\|camera parent=ID [scene=…]` | ✅ `create_child_matches_acl` |
| `object.create_prefab`(右鍵 ▸ Create Prefab From Selection,作用在主選取) | `prefab create [PID] from ID`(預設 PID = 物件 ID,已存在就用 `ID/2`…) | ✅ `create_prefab_from_selection_matches_acl` |
| 右鍵 ▸ Duplicate / Delete | `duplicate SEL` / `delete SEL` | ✅ 同 `edit.duplicate` / `edit.delete` |
| Inspector 的 Parent 欄位 + Keep world 勾選 | `reparent ID PARENT\|root keep=world\|local` | ✅(ops 相同)`acl_verbs_compile_to_the_shared_ops` |
| `object.drop_to_ground`(End、Object ▸ Drop to Ground):選取物件往下落到下方最高的 `render.mesh` 頂面,下方沒有東西就落到 y=0 | `ground SEL` | ✅ `drop_to_ground_matches_acl`、`acl_verbs_compile_to_the_shared_ops` |
| Scene ▸ Create scene | `scene create ID [on\|off]` | ✅ `scene_operations_match_acl` |
| Scene ▸ Activate / Deactivate | `scene activate ID on\|off` | ✅ 同上 |
| Scene ▸ Delete scene(物件保留) | `scene delete ID` | ✅ 同上 |
| Inspector 場景勾選(membership) | `scene member SCENE SEL on\|off` | ⚠️ 編輯器仍直接送出 `set_scene_membership`(單一 edit,跟 ops 輸出相同),還沒有編輯器端的對等測試 |
| `object.main_camera`(Object ▸ Set as Main Camera,主選取) | `camera main ID`(在同一批次把 ID 設為 active,其他攝影機設為 inactive) | ✅ `camera_follow_and_main_camera_match_acl`;`camera_main_switches_the_active_camera_in_one_revision` |
| `object.camera_follow`(Object ▸ Camera Follow Selected,指令面板):選取的相機都跟隨選取中唯一一個不是相機的物件(預設 `camera.follow`,取代既有的);沒有目標或兩個以上、沒有相機 = problem 不寫入 | `camera CAMERAS follow TARGET [distance=] [min=] [max=] [yaw=] [pitch=] [height=] [orbit=AXIS] [zoom=AXIS] [radius=] [collide=on\|off]`。世界缺少時補 `camera` capability 與 `camera.rig` 系統,見 `docs/camera-follow.zh-TW.md` | ✅ `camera_follow_and_main_camera_match_acl`(選取順序、兩台相機、空世界、Inspector 表單改 distance / collision.radius = ACL `set`);`camera_follow_verb_matches_the_op_and_reports_structured_errors` |
| Inspector ▸ Tags(標籤 chip:輸入後 Enter / Add、點建議、chip 旁的 x 移除;建議 = 世界已用的 tag,含 `trigger.zone` / `interact.target` 的 `filter.tags`,所以還沒有人有 `player` 時也會建議它)、`object.tag`(`editor_view {command, arg:"+player -npc"}`)。多選時作用在每個選取的物件,chip 顯示「tag 2/3」表示只有部分物件有 | `tag SEL +TAG -TAG …`(`ops::edit_tags`:移除的拿掉、新增的接在尾端、其他 tag 與順序不變;每物件 ≤ 32 個、≤ 64 bytes、不能有空白 / 逗號 / 括號 / 引號 = `invalid_tag`;都不變 = `unchanged`) | ✅ `tags_match_acl`(多選、移除保留順序、無變化不寫入);egui 實際輸入 `world_tests::tag_chips_add_suggest_and_remove_on_every_selected_object`(中文 tag、Enter、點建議、x、快取只在 revision / 選取改變時重建) |
| Window ▸ World(`window.world`,Ctrl+9;World 選單):世界 ID / Title、capabilities(含使用數,core 與使用中的不能移除)、systems(依 phase 排序,被別的系統 `after` 依賴的不能移除)、resources(點 ID 在 Inspector 開啟、Remove、從已註冊的資源型別 Create,JSON 預先填入預設值)。World 選單的 `world.*` 指令帶參數時直接執行(`editor_view {command, arg}`),不帶參數時打開這個視窗 | `world id ID` / `world title TEXT` / `capability add\|remove ID` / `system add\|remove ID` / `resource create ID [JSON]` / `resource remove ID`(`ops::set_world_id`、`set_world_title`、`add_capability`、`remove_capability`、`add_system`、`remove_system`、`create_resource`、`remove_resource`)。加系統時一併補它的 capability 與它 `after` 的系統;建資源時補它的 capability 與「同 capability 且讀它的系統」(例如 `behavior.program` → `behavior.execute`)。錯誤碼:`unknown_capability`、`capability_in_use`、`unknown_system`、`system_in_use`、`unknown_resource`、`id_taken`、`no_default`(沒有有效預設值的資源要給 JSON,例如 `narrative.graph`)、`invalid_id` | ✅ `world_panel_operations_match_acl`(空世界上每個操作 + 移除 + 使用中拒絕);egui 實際點擊 `world_tests::world_window_adds_a_capability_and_caches_rows_per_revision`(下拉選單 → Add capability;列表只在 revision 改變時重建);核心 `world_tag_component_and_rotation_verbs_compile_to_the_shared_ops`、`world_verbs_report_structured_errors` |
| Inspector ▸ Components ▸ Add component…(列出所有已安裝的元件,缺 capability / 系統的標「+ trigger, trigger.detect」)、`object.add_component`:寫入預設值並在同一個 revision 補上需要的 capability 與系統,然後在 Inspector 表單開啟。沒有有效預設值的(`path.follow`、`camera.follow` 等要指定目標)改開 JSON,Stage component + Apply 時同樣補 capability / 系統 | `add SEL COMPONENT [JSON]`(`ops::add_component`;已經有(含 prefab 繼承)= `component_exists`,沒有預設值又沒給 JSON = `no_default`) | ✅ `add_component_brings_its_capability_and_systems_like_acl`(多選 trigger.zone、physics.collider 開啟表單、camera.follow 手打 JSON) |
| Inspector 多選編輯:選了多個物件時,Transform 改的軸(位置 / 縮放逐軸、旋轉逐個歐拉角)與元件表單改的欄位套用到每個有該欄位的選取物件;沒有該欄位的跳過。label、parent、JSON 模式仍只改主選取 | `set entities[id=a,id=b] FIELD VALUE`(`ops::set_fields`) | ✅ `multi_object_inspector_edits_match_acl`(兩個方塊只改 X、兩個觸發區的 `once`)。限制:不顯示混合值(「—」),顯示的是主選取的值 |
| Inspector Rotation°(歐拉角,XYZ) | `rotate SEL x,y,z`(`ops::set_rotation`)、`create/instantiate … rot=x,y,z`(`Placement.rotation`),同一個 `ops::rotation_from_degrees` | ✅ `declare_event_and_rotation_match_acl`;`create … rot=` = `create` + `rotate` |
| Problems ▸ Declare event NAME(錯誤訊息是「… emits NAME, which behavior.program does not declare」時出現;Inspector 草稿保留,宣告後再 Apply)、`event.declare` | `event declare NAME [FIELD=text\|number\|int\|bool …]`(`ops::declare_event`;沒有 program 就建立並補 `behavior.execute`;同名同欄位 = `unchanged`,欄位不同 = `id_taken`)。ACL 送出時遇到這種錯誤,訊息尾端加上「declare it first: event declare NAME」 | ✅ `declare_event_and_rotation_match_acl`;核心 `undeclared_events_carry_the_fix` |
| Resource inspector 表單模式(物件型的投影預設用 schema 表單,只寫改過的欄位;JSON 模式照舊) | `set resources[id=R] FIELD V` | ✅ `resource_form_edits_match_acl` |
| (陣列單一元素)編輯器沒有對應按鈕,Resource inspector 只能整個替換陣列 | `append SEL F VALUE` / `insert SEL F INDEX VALUE` / `remove SEL F INDEX`(`ops::edit_array`,錯誤 `index_out_of_range` / `invalid_field`)。`set` 整個替換資源端的清單(bindings、rules、widgets…)而丟掉現有項目時,輸出 `warn line N: … dropped` 並在 JSON 的 `warnings` 列出;`set …/99` 超出範圍的錯誤會提示用 `append` | 核心 `array_element_verbs_append_insert_and_remove`、`set_replacing_a_resource_list_warns_about_dropped_items` |
| Inspector:label、transform、元件表單、JSON | `set ID label "…"`、`set ID core.transform/position [x,y,z]`、`set ID comp/field V` | 欄位層級的 `set_field`,兩邊是同一種 edit |
| 視窗內 gizmo 移動/旋轉/縮放 | `set ID core.transform/position\|rotation\|scale V` | 同上 |
| 新增元件(Component ID 欄位 Load + Stage component,手打 JSON) | 物件原本沒有這個元件:`add ID comp.name {…}`(一併補 capability / 系統);已經有:`set ID comp.name {…}`(覆寫) | 見上方 Add component 列 |
| Inspector ▸ Remove component | `remove SEL COMPONENT`(只移除物件自己編寫的元件;從 prefab 繼承的回 `not_found` 並說明要改 prefab) | ✅ `remove_component_matches_acl`;`remove_verb_matches_the_op_and_reports_inherited_components` |
| Prefab inspector | `set prefabs[id=P] comp/field V` | 欄位層級 `set_field` |
| Material inspector | `set resources[id=render.shaders] materials/ID/param V` | 同上 |
| Resource inspector | `set resources[id=R] field V` | 同上 |
| Undo / Redo(Ctrl+Z / Ctrl+Y)、History 的 Restore | `undo REV`(新增一筆還原 revision) | 機制相同 |
| Proposals 核准 / 退回 / rebase | `axiom resolve ID --approve\|--reject [--rebase]` | 同一個 `resolve_proposal` 請求 |

## 反方向:AI 做得到、編輯器還做不到(街道驗收時記錄)

見 `docs/acceptance-street.zh-TW.md` 的缺口報告。已補上(2026-11-09 晚):實體 tags(GAP-1)、新建 / 移除資源、加 / 移 capability 與系統、改世界 id / title(GAP-3)、Add component 一律補需要的 capability / 系統、多物件編輯 transform 與元件欄位(GAP-10,不顯示混合值)、未宣告事件的 Declare(GAP-9)、歐拉角(GAP-11)—— 都在上表。

目前仍然只有 AI 做得到、編輯器做不到的:

| AI(ACL) | 編輯器 | 備註 |
|---|---|---|
| `tween … play_on=E` | 沒有 UI(GAP-14) | 要先 ACL 或手改 `runtime.bindings` |
| `append` / `insert` / `remove SEL F INDEX`(陣列單一元素) | Resource inspector 只能整個替換陣列(JSON 模式),表單沒有新增 / 刪除項目 | GAP-8 的編輯器半邊,歸 M3 / M6 |
| `spawn PREFAB count=N …`(大量生成) | 一次一個 | 既有 |
| 建立指令的選項(`interact … event= prompt=`、`audio clip ID`、`tween … property=` 等) | 先建預設值再到 Inspector 改;音效片段 ID = 預設名稱 | GAP-14 |
| 多選時的 label / parent / JSON 模式 | 只改主選取 | 多物件編輯只涵蓋 transform 與表單欄位 |

真 GPU 編輯器與 ACL 的 8 步對照(`scripts/street-editor-parity.ps1`)匯出結果逐位元相同;從空世界只用編輯器指令建出 capability / 系統 / 資源、tag 玩家、做出可互動物件的對照見 `docs/acceptance-street.zh-TW.md`「從空世界用編輯器」一節。

## 唯讀或只影響畫面的功能(不改內容,不需要無頭等價)

| 編輯器功能 | 說明 / 無頭替代 |
|---|---|
| `tool.select/move/rotate/scale`、`view.space`、`view.snap`、`view.grid` | 工具與偏好設定 |
| `view.palette`、`help.shortcuts`、`edit.preferences`、`file.exit` | 只是 UI |
| `window.*`(dock 分頁:Hierarchy、Scene、Inspector、World 與原本的底部分頁)、拖曳分頁 / 分割 / 浮動視窗、`window.close`、`layout.save_as` / `layout.load` / `layout.delete` / `layout.reset`、Window ▸ Layouts(內建 Default、Tall Inspector) | **只是版面(和 hidden / locked 一樣是視圖偏好)**:存在 `*.editor-prefs.json` 的 `layout` / `layouts`(使用者層級,不分世界),不進 revision、不影響 Play 與內容。遠端:`editor_view {dock:"inspector"}` 開啟並聚焦分頁(舊的 dock 名稱就是分頁名稱)、`{command:"window.close", arg:"history"}`、`{command:"layout.load", arg:"Tall Inspector"}`;`editor_view` 回覆與 `observe` 的 `editor.tabs` 列出開著的分頁(active / focused / floating)與 `scene_visible`;`discover editor_layout` 說明全部。Scene 分頁關閉或被同一格的其他分頁蓋住時,編輯器視圖不渲染(只畫介面) |
| `edit.select_none`、`edit.frame`(右鍵 ▸ Focus)、選取、聚焦 | 只是畫面;無頭查詢用 `select` / `inspect` |
| `edit.select_children`(右鍵 ▸ Select Children) | 把所有子孫加進選取;無頭用 `select entities[parent=ID]` 逐層查 |
| `edit.toggle_hidden`(H)、`edit.toggle_locked`(L)、Hierarchy 每列的眼睛 / 鎖頭 | **只影響編輯器視圖**:隱藏 = 場景視口不畫、不能點選;鎖定 = 照畫,但視口不能點選、gizmo 不能拖(Hierarchy 仍可選)。子孫一起生效。存在 `*.editor-prefs.json` 的 `hidden` / `locked`(以 world ID 為 key),不進 revision,Play 照常顯示。`editor_view` 的觀察結果有 `hidden` / `locked` 欄位 |
| `view.perspective` / `view.top` / `view.front` / `view.right` / `view.bottom` / `view.back` / `view.left`(Alt+1..7、場景工具列 View 選單、右下角座標軸點 X / Y / Z,再點一次切到反面,點模式文字回 Perspective) | 只影響編輯器相機:正交軸向視圖,picking / gizmo 拖曳 / 框選都用同一個投影矩陣。在軸向視圖中按右鍵旋轉會回到 Perspective。`editor_view` 觀察結果的 `view` 欄位回報目前模式 |
| 飛行相機:按住右鍵 + WASD(Q / E 下降 / 上升),Shift 3 倍速;按住右鍵時滾輪調整飛行速度(畫面下方短暫顯示) | 只是相機;速度存在 `*.editor-prefs.json` 的 `fly_speed`。按住右鍵期間單鍵快捷鍵(W = Move 等)暫停。沒按移動鍵的右鍵拖曳仍是環繞 |
| 框選:左鍵在空白處(或非 gizmo 握把處)拖曳畫框,放開時選取「投影中心點落在框內」的物件(網格 / 骨架網格用包圍盒中心,其餘用 pivot);Shift 加選、Ctrl 反選(主選取保留)、空框清除選取 | 只是選取;隱藏 / 鎖定物件不會被框到。無頭用 `select` |
| `view.gizmos`(場景工具列 Gizmos):相機視錐(啟用中的相機黃色、其餘灰藍,標 CAM)、`render.light`(directional 箭頭 SUN、point 三軸 range 圓、spot 內外錐、area 矩形 / 圓盤 + 法線;停用的燈畫暗色)、`nav.blocker` 矩形(XZ,位置 + offset,和 runtime 一樣用編寫位置)、`physics.collider` 方塊 / 膠囊、`fsm.agent` 身體半徑圈(標 FSM)、`trigger.zone` 線框(box 依軸向、sphere 三個圓,`exit_margin` 加暗色外框;停用暗綠、選取白;區內狀態不在這裡畫,見下方 Play 預覽的 Behavior overlay)、`path.spline` 折線 / 曲線(控制點十字、方向箭頭、標 PATH;選取的路徑顯示可拖曳的點把手)、`path.follow` 到起點的暗線(標 FOLLOW)、`nav.agent` 半徑圈(標 NAV)+ 到編寫目標的線與十字,選取的代理另畫預覽路徑(綠色,有上限的同步搜尋,依輸入快取)、`interact.target` 的 range 圓(黃色,標 USE;有 `exit_margin` 時外加暗色放開圓;停用暗黃、選取白)、`audio.source` 的 min / max 距離球(亮綠 / 暗綠,各三個世界軸向圓,標 SND;2D 聲源只標 SND 2D;停用暗綠、選取白)、`audio.listener` 小圓圈 + 面向線(紫,標 EAR)、`camera.follow` 從注視點沿編寫的 yaw / pitch 到 max_distance 的線 + min / max 十字 + 期望距離圓圈 + 相機到注視點的暗線(橘,標 RIG;停用暗橘、選取白) | 只在編輯器視圖用 egui painter 畫,不進渲染、Play 看不到;設定存在偏好檔 `gizmos` |
| `view.bookmark.save.N`(Ctrl+F6..F9)/ `view.bookmark.recall.N`(F6..F9),N = 1..4;場景工具列 View 選單也有 | 只是相機;依 world ID 存在偏好檔 `bookmarks`。空的槽位不可用(指令回報原因) |
| Tween 時間軸的播放頭拖曳(scrub) | 只影響這一幀的場景視口繪製(編輯模式),不寫 revision、不改模擬(`tween_scrub_previews_without_writing_or_moving_the_simulation`)。無頭用 `call tween.seek` 在執行期跳到某個時間 |
| `view.nav_grid`(View ▸ Toggle Navigation Grid Overlay) | 只影響編輯器視圖:在選取物件(沒選時為相機注視點)周圍 81 × 81 格內,用紅色 × 標出選取代理(沒選代理時以 0.3 m 計)不能進入的格子,最多 2,400 個;存在偏好檔 `nav_grid`。無頭用 `observe` 的 `nav.grid` 或 `call nav.inspect` |
| Hierarchy 型別標籤 `CAM` `LIGHT` `MESH` `GEO` `SKIN` `FSM` `TRIG` `PATH` `FOLLOW` `NAV` `USE` `TWEEN` `SND` `EAR` `RIG` `PFB` | 只是顯示(`render.camera`、`render.light`、`render.mesh`、`render.geometry`、`animation.player`、`fsm.agent`、`trigger.zone`、`path.spline`、`path.follow`、`nav.agent`、`interact.target`、`tween.clips`、`audio.source`、`audio.listener`、`camera.follow`、prefab 實例) |
| 點一下路徑點把手 / Inspector ▸ Path points 的列 = 選取路徑點 | 只是編輯器狀態(`path.add_point` / `path.remove_point` 作用的點);無頭直接在 `path ID insert\|remove I` 指定索引 |
| `edit.copy`、`file.copy_context`、`help.copy_discovery` | 用 `inspect ID` / `discover` |
| `file.verify` | `axiom verify` |
| `play.start`(Play 預覽,不寫入 revision) | `axiom play` / `axiom scenario FILE` |
| 聲音的播放 / 停止 / 試聽 | 編輯器沒有試聽按鈕(沒做);進 Play 後由 `autoplay`、規則與 `runtime.bindings` 播放。無頭用 `call audio.play\|stop\|inspect\|summary` 與 `observe` 的 `audio` 段(每個 voice 的增益與聲像),不需要音效裝置 |
| Play 預覽的「Behavior overlay」:FSM 狀態標籤 + `trigger.zone` 線框(青綠 = 空、粉紅加粗 = 有實體在區內)+ 導航代理剩下的路線(綠線)+ 互動者到聚焦目標的黃線 | 只是畫面;無頭用 `observe` 的 `trigger` / `nav` / `interact` 或 `call trigger.inspect` / `nav.inspect` / `interact.inspect` |
| 執行期切換主相機、改跟隨目標 / 距離 / 角度 | 編輯器沒有對應按鈕(Play 預覽中由 `runtime.bindings` 或輸入驅動);無頭用 `call camera.activate\|follow.set\|inspect\|summary` 與 `observe` 的 `camera` 段(期望 / 實際距離、擋住的實體),不改編寫內容 |
| History 的 diff 檢視 | `diff REV REV` |
| Scenarios 分頁 | `axiom scenario FILE` |
| Problems、Graphs、AI/Protocol 分頁 | 只讀;用 `verify` 和 `discover` |

## 刻意改變的行為

- 從 Assets 實例化 prefab 時,預設 ID 原本是 `instance`、`instance/2`,現在改成 **prefab ID**(例如 `cargo-crate`、`cargo-crate/2`),跟 `spawn` 的命名一致。
- Hierarchy 的同層順序改成**物件的編寫順序**(`world.entities` 的順序,也就是 diff 裡的 `/entities/@order`),以前是依 ID 排序。這樣 `move` 才有意義。`scene_tree` 協定回傳的列仍依 ID 排序,沒有改。新協定 edit:`{"kind":"move_entity","id":…,"before":ID|null}`。
- Create Prefab From Selection:prefab 拿到物件「解析後」的全部元件(prefab 繼承值 + 覆寫),所以原本就是 prefab 實例也不會遺失覆寫。物件本身只保留 `core.transform` 作為覆寫,prefab 裡的 transform 重設為 `{}`,這樣之後 `instantiate` 出來的新實例不會全部疊在同一個位置。這跟「元件全部清空」的原始需求不同,是刻意的。
- 只會編譯出 0 個 edit 的 ACL 腳本(例如 `move X end` 而 X 已經在最後)現在回 `unchanged rev=R changes=0`,不寫 revision;以前同樣情況會變成「batch must contain 1..256 edits」錯誤。
- 修正:Hierarchy 的列以前用 egui 的 `dnd_drag_source` 包住標籤,egui 0.31 的 hit test 遇到「只感應拖曳的元件疊在可點擊元件上面」時不把點擊交給任何人,所以點列(含 Ctrl / Shift 多選、右鍵)都沒反應。現在標籤本身同時感應點擊和拖曳。
- ACL 腳本裡後面的敘述可以看到前面敘述的結果(`create cube; create cube` 會產生 `cube`、`cube/2`;`create empty id=t; delete t` 也合法)。整份腳本仍然只寫一個 revision。
- ACL 腳本開頭的 UTF-8 BOM(Windows 編輯器常見)現在會被略過;以前第一行會回 `unknown statement '\u{feff}…'`。BOM 出現在其他位置仍是語法錯誤(`acl_scripts_may_start_with_a_byte_order_mark`)。
- `duplicate` 的複本以前一律加在 `world.entities` 最後(Hierarchy 裡跑到同層最下面),現在緊接在來源之後;批次裡每個 `duplicate_entity` 後面多一筆 `move_entity`(來源已經是最後一個就不用)。ACL 與編輯器共用 `ops::duplicate`,結果一致。
- `ground SEL` / Drop to Ground 的規則:最低點 = 物件與其子孫所有 `render.mesh` 部件的世界包圍盒底部(沒有部件就用 pivot);支撐面 = 其他物件的 `render.mesh` 部件頂面,XZ 範圍要重疊、而且不高於最低點(容差 1 mm),取最高者,沒有就用 y=0。被選取的物件和其子孫不當支撐面(同一批不會疊起來);匯入的 glTF / 骨架模型與 `render.geometry` 沒有編寫時可算的包圍盒,不當支撐面,也不用來算最低點。只改本地 `core.transform/position`(世界位移經 parent 反矩陣換算);祖先也在選取中的物件只跟著祖先移動。已經著地就不寫 revision(ACL 回 `unchanged`,編輯器顯示 Already on the ground)。
- 編輯器隱藏旗標現在也作用在附著於該物件的 VFX(`vfx.library` emitter 的 `attachment`:粒子批次、緞帶、VFX 光源)。程序化幾何(`render.geometry`)與骨架網格本來就跳過停用的 slot,現在有測試涵蓋。沒有 `attachment` 的 VFX 是世界座標,無法用物件隱藏。
- 編輯器隱藏旗標也作用在 `render.light`:隱藏的燈光(含隱藏物件底下的子燈光)在場景視口不照明、不畫 gizmo;Play 照常。
- Inspector 的 Remove component 改走 `ops::remove_component`(和 ACL `remove` 同一份程式)。以前直接送出 `remove_component` edit;結果相同,但現在對「物件沒有自己編寫這個元件」會回報原因而不是送出一定失敗的交易(按鈕本來就只對自己編寫的元件啟用)。
