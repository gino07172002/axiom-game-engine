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
| 設為主攝影機 | `camera main ID`(在同一批次把 ID 設為 active,其他攝影機設為 inactive) | ⚠️ 只有 op 和 ACL;**編輯器 UI 還沒做這個按鈕**(`camera_main_switches_the_active_camera_in_one_revision`) |
| Inspector:label、transform、元件表單、JSON | `set ID label "…"`、`set ID core.transform/position [x,y,z]`、`set ID comp/field V` | 欄位層級的 `set_field`,兩邊是同一種 edit |
| 視窗內 gizmo 移動/旋轉/縮放 | `set ID core.transform/position\|rotation\|scale V` | 同上 |
| 新增元件(元件挑選器 / Stage component) | `set ID comp.name {…}` | 同上 |
| 移除元件 | `axiom apply FILE`,內容是 `{"kind":"remove_component",…}` | ⚠️ **ACL 沒有這個動詞**,只能用協定 JSON |
| Prefab inspector | `set prefabs[id=P] comp/field V` | 欄位層級 `set_field` |
| Material inspector | `set resources[id=render.shaders] materials/ID/param V` | 同上 |
| Resource inspector | `set resources[id=R] field V` | 同上 |
| Undo / Redo(Ctrl+Z / Ctrl+Y)、History 的 Restore | `undo REV`(新增一筆還原 revision) | 機制相同 |
| Proposals 核准 / 退回 / rebase | `axiom resolve ID --approve\|--reject [--rebase]` | 同一個 `resolve_proposal` 請求 |

## 唯讀或只影響畫面的功能(不改內容,不需要無頭等價)

| 編輯器功能 | 說明 / 無頭替代 |
|---|---|
| `tool.select/move/rotate/scale`、`view.space`、`view.snap`、`view.grid` | 工具與偏好設定 |
| `view.palette`、`help.shortcuts`、`edit.preferences`、`file.exit` | 只是 UI |
| `window.*`(dock 分頁) | 只是 UI;需要的話用 `editor_view {dock}` 遠端切換 |
| `edit.select_none`、`edit.frame`(右鍵 ▸ Focus)、選取、聚焦 | 只是畫面;無頭查詢用 `select` / `inspect` |
| `edit.select_children`(右鍵 ▸ Select Children) | 把所有子孫加進選取;無頭用 `select entities[parent=ID]` 逐層查 |
| `edit.toggle_hidden`(H)、`edit.toggle_locked`(L)、Hierarchy 每列的眼睛 / 鎖頭 | **只影響編輯器視圖**:隱藏 = 場景視口不畫、不能點選;鎖定 = 照畫,但視口不能點選、gizmo 不能拖(Hierarchy 仍可選)。子孫一起生效。存在 `*.editor-prefs.json` 的 `hidden` / `locked`(以 world ID 為 key),不進 revision,Play 照常顯示。`editor_view` 的觀察結果有 `hidden` / `locked` 欄位 |
| `view.perspective` / `view.top` / `view.front` / `view.right` / `view.bottom` / `view.back` / `view.left`(Alt+1..7、場景工具列 View 選單、右下角座標軸點 X / Y / Z,再點一次切到反面,點模式文字回 Perspective) | 只影響編輯器相機:正交軸向視圖,picking / gizmo 拖曳 / 框選都用同一個投影矩陣。在軸向視圖中按右鍵旋轉會回到 Perspective。`editor_view` 觀察結果的 `view` 欄位回報目前模式 |
| 飛行相機:按住右鍵 + WASD(Q / E 下降 / 上升),Shift 3 倍速;按住右鍵時滾輪調整飛行速度(畫面下方短暫顯示) | 只是相機;速度存在 `*.editor-prefs.json` 的 `fly_speed`。按住右鍵期間單鍵快捷鍵(W = Move 等)暫停。沒按移動鍵的右鍵拖曳仍是環繞 |
| 框選:左鍵在空白處(或非 gizmo 握把處)拖曳畫框,放開時選取「投影中心點落在框內」的物件(網格 / 骨架網格用包圍盒中心,其餘用 pivot);Shift 加選、Ctrl 反選(主選取保留)、空框清除選取 | 只是選取;隱藏 / 鎖定物件不會被框到。無頭用 `select` |
| `view.gizmos`(場景工具列 Gizmos):相機視錐(啟用中的相機黃色、其餘灰藍,標 CAM)、`render.light`(directional 箭頭 SUN、point 三軸 range 圓、spot 內外錐、area 矩形 / 圓盤 + 法線;停用的燈畫暗色)、`nav.blocker` 矩形(XZ,位置 + offset,和 runtime 一樣用編寫位置)、`physics.collider` 方塊 / 膠囊、`fsm.agent` 身體半徑圈(標 FSM) | 只在編輯器視圖用 egui painter 畫,不進渲染、Play 看不到;設定存在偏好檔 `gizmos` |
| `view.bookmark.save.N`(Ctrl+F6..F9)/ `view.bookmark.recall.N`(F6..F9),N = 1..4;場景工具列 View 選單也有 | 只是相機;依 world ID 存在偏好檔 `bookmarks`。空的槽位不可用(指令回報原因) |
| Hierarchy 型別標籤 `CAM` `LIGHT` `MESH` `GEO` `SKIN` `FSM` `PFB` | 只是顯示(`render.camera`、`render.light`、`render.mesh`、`render.geometry`、`animation.player`、`fsm.agent`、prefab 實例) |
| `edit.copy`、`file.copy_context`、`help.copy_discovery` | 用 `inspect ID` / `discover` |
| `file.verify` | `axiom verify` |
| `play.start`(Play 預覽,不寫入 revision) | `axiom play` / `axiom scenario FILE` |
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
- `duplicate` 的複本以前一律加在 `world.entities` 最後(Hierarchy 裡跑到同層最下面),現在緊接在來源之後;批次裡每個 `duplicate_entity` 後面多一筆 `move_entity`(來源已經是最後一個就不用)。ACL 與編輯器共用 `ops::duplicate`,結果一致。
- `ground SEL` / Drop to Ground 的規則:最低點 = 物件與其子孫所有 `render.mesh` 部件的世界包圍盒底部(沒有部件就用 pivot);支撐面 = 其他物件的 `render.mesh` 部件頂面,XZ 範圍要重疊、而且不高於最低點(容差 1 mm),取最高者,沒有就用 y=0。被選取的物件和其子孫不當支撐面(同一批不會疊起來);匯入的 glTF / 骨架模型與 `render.geometry` 沒有編寫時可算的包圍盒,不當支撐面,也不用來算最低點。只改本地 `core.transform/position`(世界位移經 parent 反矩陣換算);祖先也在選取中的物件只跟著祖先移動。已經著地就不寫 revision(ACL 回 `unchanged`,編輯器顯示 Already on the ground)。
- 編輯器隱藏旗標現在也作用在附著於該物件的 VFX(`vfx.library` emitter 的 `attachment`:粒子批次、緞帶、VFX 光源)。程序化幾何(`render.geometry`)與骨架網格本來就跳過停用的 slot,現在有測試涵蓋。沒有 `attachment` 的 VFX 是世界座標,無法用物件隱藏。
- 編輯器隱藏旗標也作用在 `render.light`:隱藏的燈光(含隱藏物件底下的子燈光)在場景視口不照明、不畫 gizmo;Play 照常。