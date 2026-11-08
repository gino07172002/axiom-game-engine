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
| Light 預設 | **不存在**:引擎沒有 light 元件,`create light` 回傳 `unknown_preset` | — |
| Assets ▸ 點 prefab 實例化 | `instantiate PREFAB [id=] [at=] [parent=] [scene=]` | ✅ `instantiate_prefab_matches_acl` |
| 大量生成(帶 tag、label、step 間距) | `spawn PREFAB count=N …`(只有 ACL 有;編輯器只能一次一個) | — |
| `edit.duplicate`(Ctrl+D)、Inspector 的 Duplicate | `duplicate SEL`(ID 依序為 `SRC/copy`、`/copy/2`;`entities[id=a,id=b]` 照書寫順序處理) | ✅ `duplicate_and_paste_match_acl` |
| `edit.paste`(Ctrl+V,複製剪貼簿裡的 ID) | `duplicate SEL` | ✅ 同上 |
| `edit.delete`(Delete)、Inspector 的 Delete | `delete SEL` | ✅ `delete_matches_acl` |
| Hierarchy 拖曳改 parent(保留世界姿態) | `reparent ID PARENT\|root`(預設 `keep=world`) | ✅ `reparent_drag_matches_acl` |
| Inspector 的 Parent 欄位 + Keep world 勾選 | `reparent ID PARENT\|root keep=world\|local` | ✅(ops 相同)`acl_verbs_compile_to_the_shared_ops` |
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
| `edit.select_none`、`edit.frame`、選取、聚焦 | 只是畫面;無頭查詢用 `select` / `inspect` |
| `edit.copy`、`file.copy_context`、`help.copy_discovery` | 用 `inspect ID` / `discover` |
| `file.verify` | `axiom verify` |
| `play.start`(Play 預覽,不寫入 revision) | `axiom play` / `axiom scenario FILE` |
| History 的 diff 檢視 | `diff REV REV` |
| Scenarios 分頁 | `axiom scenario FILE` |
| Problems、Graphs、AI/Protocol 分頁 | 只讀;用 `verify` 和 `discover` |

## 刻意改變的行為

- 從 Assets 實例化 prefab 時,預設 ID 原本是 `instance`、`instance/2`,現在改成 **prefab ID**(例如 `cargo-crate`、`cargo-crate/2`),跟 `spawn` 的命名一致。
- ACL 腳本裡後面的敘述可以看到前面敘述的結果(`create cube; create cube` 會產生 `cube`、`cube/2`;`create empty id=t; delete t` 也合法)。整份腳本仍然只寫一個 revision。
