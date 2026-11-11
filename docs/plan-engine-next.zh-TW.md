# 計畫:引擎下一階段(繪圖品質、物理、動畫控制器、編輯器外觀)

> 2026-11-11 起。流程:`rigorous-plan`(本檔)→ 每條工作線用 `rigorous-change` 執行。
> 前一份計畫 `docs/plan-editor-human-tuning.md`(M1–M10 + 後續 FA–FF)已結案。

## Phase 0 — 需求與成功樣貌

使用者要求(2026-11-11):先做「繪圖品質、物理、動畫控制器」三項引擎能力,同時讓編輯器外觀更像設計圖 `D:\newExport\Axiom Studio Sakura Plaza Editor.png`;之後做打包發佈;跨平台與大場景效能延後。

**最終驗收 demo 腳本**(全部完成那天):

1. 開編輯器載入新的第一方示範場景(只用引擎內建幾何,不用第三方素材)。Hierarchy 是分層資料夾樹(Environment / Props / Lighting / Characters / Cameras),可展開收合;Inspector 的元件卡片有啟用勾選、圖示、Revert / ⋯、Advanced 摘要列、資源欄位顯示名稱 + 選取鈕;Assets 有資料夾樹 + 麵包屑 + 真實縮圖。整體截圖與設計圖並排,版面與質感接近。
2. 場景有可設定的天空(漸層 / 程序天空)與天空光;地面與金屬物件有反射(反射探針或螢幕空間反射);匯入的 glTF 角色會接收陰影。
3. 按 Run:箱子從高處落下、堆疊、互相碰撞反彈(3D 剛體:重力、box / sphere / capsule、靜態與動態、摩擦與彈性),角色控制器可以推動箱子;同一 scenario 重播 state_hash 一致(確定性)。
4. 角色用動畫控制器(狀態機:idle ↔ walk ↔ run,依參數轉換、交叉淡入、1D 混合樹),在編輯器的動畫控制器檢視看到狀態圖、目前狀態高亮、參數即時值;AI 透過 ACL / editor command 編輯狀態與轉換。
5. 每一項都有 ACL 動詞 + 編輯器操作 + parity 測試;GPU smoke 截圖;效能宣稱都附 release A/B。

## 約束

- Rust workspace,不加 Bevy;`axiom-core`(交易式編寫)與 `axiom-runtime`(固定步進模擬)分離;內容變更走 `ops` + ACL + parity。
- 新增第三方 crate 必須在 ADR 比較(含編譯時間、確定性、授權、維護),不得引入第三方美術 / 音效素材。
- 16 ms 主執行緒預算、背景工作有界可取消、熱路徑無每幀配置、型別化執行期儲存。
- 舊內容包與舊 DB 必須能載入(向後相容 + migration + 測試);`build-street` 不變或說明差異。
- 只在本機 Windows / MX150 驗證;GPU 功能需在不支援的 adapter 上誠實降級。

## Non-goals(本計畫不做)

- 打包發佈(下一份計畫)、跨平台、大場景(1000+ 實體 / 百萬三角形)效能。
- 全域光照(GI / lightmap bake)、光線追蹤、布料 / 頭髮模擬、軟體物理、載具物理。
- 動畫:IK 編輯器、動畫重定向 UI(已有的保持不變)、2D 混合樹(只做 1D)。
- 重做整套 UI 框架;外觀改善在既有 egui + theme / panels / chrome 上進行。

## Phase 1 — 風險

| 風險 | 若不成立 | 驗證方式 |
|---|---|---|
| 物理用自寫或 crate(rapier3d 等)— 編譯時間、確定性、與 nav / combat 碰撞共存 | 致命 | 工作線 P 的 spike + ADR |
| 反射探針 / SSR 在 MX150 的成本 | 繞得過(降級成 cubemap 環境反射) | 工作線 R release A/B |
| 匯入網格 pipeline 沒有陰影貼圖 | 繞得過 | 工作線 R |
| 動畫狀態機與現有 `animation.evaluate` 姿勢混合的整合 | 致命 | 工作線 A 讀碼 + spike |
| 4 條工作線同時改 `world.rs` / `registry` / `render.rs` / 編輯器共用檔 | 合併衝突 | 只加法修改、我逐一合併並驗證 |

## Phase 2–3 — 工作線(每條在自己的 worktree,各自寫 ADR 與 milestone)

- **R 繪圖品質**:可設定天空(漸層 / 程序天空)+ 天空光;反射(反射探針 cubemap 或 SSR,ADR 決定);匯入網格接收陰影。GPU smoke + release A/B。
- **P 物理**:3D 剛體(重力、box / sphere / capsule、靜態 / 動態 / kinematic、摩擦、彈性、睡眠)、與角色控制器互動、確定性(scenario state_hash)、ACL / 編輯器 / parity、debug 繪製碰撞形狀。
- **A 動畫控制器**:`animation.controller` 元件(狀態、轉換條件、參數、交叉淡入、1D 混合樹)、執行期求值整合、編輯器狀態圖檢視(即時高亮)、ACL / parity。
- **E 編輯器外觀**:第一方分層示範場景;Hierarchy 資料夾樹;Inspector 卡片精修;Assets 資料夾樹 + 麵包屑 + 縮圖(背景工作、依內容雜湊快取);視窗顯示模式(Lit / Unlit / Wireframe 若可行)與截圖鈕;狀態列 GPU 記憶體實際數字(不能讀時說明原因)。截圖與設計圖並排比較。

每條工作線的第一個 milestone 必須是端到端最薄的一條線(walking skeleton),並可回退。

## Phase 4 — pre-mortem

1. **合併爆炸**:四條線都碰 `registry` / `world.rs` / 編輯器共用檔 → 只加法修改;我每合併一條就跑完整驗證再合下一條。
2. **物理不確定導致 scenario 重播不一致** → 固定步進、固定迭代次數、穩定排序;測試比對 state_hash。
3. **範圍爆掉**(GI、布料、2D 混合樹最誘人)→ 已寫進 non-goals。
4. **機器負載讓計時測試不穩** → 已在 FD 修過 scenario 測試;新增的計時斷言要能抵抗負載,否則標 ignored 並另做 release 量測。

## Phase 5 — 執行協定

- 每條線:`rigorous-change`;驗證 = fmt、clippy -D warnings、`cargo test --workspace`、editor smoke、兩支 parity、build-street、GPU smoke(繪圖相關)、1080 / 768 截圖(實際看過)、fresh-context 審查。
- 每條線在本檔「執行紀錄」加一節(含補記:原計畫 → 變成什麼 → 為什麼)。
- 我(整合者)獨立重跑驗證,依序合併並推送;清掉 worktree 與增量快取。

## 執行紀錄

(各工作線完成後補上)

### E 編輯器外觀(2026-11-11,worktree `we`,分支 `work-we`,`rigorous-change`)

**先比較再動手**:改版前用主樹的 debug exe 在 Axiom Street 拍 `artifacts/captures/we-before-{1080,768}.png`,與設計圖並排列出差異(下表「之前」欄)。

| 項目 | 設計圖 | 之前(we-before) | 之後(we-after) |
|---|---|---|---|
| Hierarchy | 場景 ▸ Environment(Buildings / Props / Vegetation)、Lighting、Characters、Cameras 資料夾樹 | 街道是平的根物件列表(沒有群組資料) | Sakura Plaza 以 `group` 分層;資料夾圖示(展開時打開)、展開箭頭 |
| Inspector 元件標題 | Components 標題列右側 + Add Component… | Add Component… / Behavior… 兩個下拉在標題下方(768 時兩列) | 標題列右側 + Add Component…(Behavior 範本在同一清單最上面) |
| Transform | 第一張卡片,Revert / ⋯ | Components 之外的獨立區段,無動作 | Components 下第一張卡片,Revert(`reset SEL core.transform`)+ ⋯(Reset / Copy / Paste),X/Y/Z 紅綠藍 |
| 元件卡片 | 勾選、圖示、Revert、⋯ | 只有展開的卡片有 Reset / ⋯;收合卡片的勾選唯讀;值裡沒有 `enabled` 的元件不顯示勾選 | 每張卡片 Revert / ⋯;收合卡片可勾(一筆 revision);schema 有 `enabled` 的元件一定顯示勾選 |
| Advanced | 每張卡片「Advanced  Render Layers (2)…」摘要列 | 只有底部的「Advanced」(用 ID 載入元件) | 展開卡片內「Advanced (N): a, b…」收起仍等於預設值的欄位;底部改名 Load component by ID |
| 資源欄位 | 名稱 + 複製鈕 | 只有下拉 | 下拉(名稱)+ Copy + Show in Assets(`asset.reveal`) |
| Assets 左側 | 資料夾樹 | 平的類別清單(含 0 個的類別)+ FOLDERS 文字列 | `Assets/` ▸ 類別 ▸ 來源資料夾 / ID 前綴,箭頭、資料夾圖示、數量 |
| 麵包屑 | Assets › Characters › Android_01 | Assets › 類別 | 每段可點的完整路徑 |
| 縮圖 | 模型 / 骨架 / 動畫 / 貼圖 / 材質球 | 模型、圖片、材質;其他是 PFB / SND / UI / INPUT 文字 | 另加 prefab 縮圖(方塊或其模型);其他類別畫圖示 |
| Scene 工具列 | 格線、相機(截圖)、Snap、Lit ▾、Perspective ▾ | 沒有截圖鈕、沒有著色模式 | 截圖鈕 + Lit / Unlit 下拉 |
| 狀態列 | VRAM: 1.26 GB | GPU alloc n/a | VRAM 439 MiB(1080)/ 233 MiB(768),提示有預算與共用記憶體 |

**補記(原計畫 → 變成什麼 → 為什麼)**

- **Wireframe → 只做 Lit / Unlit**:`POLYGON_MODE_LINE` 要為每條世界 pipeline(方塊、網格 / 骨架、每個自訂材質 hook)各建一份線框版,pipeline 數加倍,且工作線 R 同時在改這些 pipeline;CPU 畫邊線則是每幀大量線段。Unlit 只改那個 view 的 uniform(均勻環境光、無太陽 / 區域光 / 霧,`render.rs::unlit_globals`),沒有新 pipeline、沒有 shader 變更。限制:顏色仍經 tone curve(平塗底色,不是原始貼圖值)。
- **元件啟用旗標**:沒有新增通用的 `enabled`(要每個 runtime 系統都尊重才有意義,不便宜)。勾選只出現在 schema 本來就有 `enabled` 的元件(讀 registry 的欄位提示 `"bool, default true"`;值裡省略時顯示預設值,沒動過不寫入)。
- **Advanced 列的「不常用」**:沒有逐元件的欄位分類(要求是通用卡片外框),改用資料定義:已提交值仍等於 `ops::default_value` 的頂層欄位;至少收兩個、外面至少留兩個才出現;篩選、多選值不同、prefab 覆寫時不收。
- **示範場景的角色**:用第一方吉祥物 glb(`content/mascot`,使用者自己的設計;與 axiom-town 相同的 blob,git 不重複存),外加一個方塊人偶。沒有天空 / 反射探針 / 天空光(工作線 R 的範圍),Lighting 資料夾是太陽 + 兩盞路燈的點光。
- **VRAM**:`GPU alloc n/a` 的原因是 wgpu 24 只有 D3D12 後端(gpu-allocator)回報 allocator report,這台走 Vulkan(gpu-alloc)。改讀 DXGI `QueryVideoMemoryInfo`(依 PCI vendor / device 對應 adapter):本程序用的專用顯示記憶體,含 swapchain 與驅動配置、不分 API。`windows` 0.58 本來就因 wgpu-hal 的 D3D12 後端在 lock 裡(同版本、同 feature),所以 Cargo.lock 只多一行相依,沒有新 crate。量測:一次讀取(兩個查詢)debug 6.6 µs(100 次平均,`vram::tests`)。
- **截圖**:沿用既有的讀回(`Renderer::grab`,同一個 COPY_SRC 讀回路徑),裁切、BGRA→RGBA 與 PNG 編碼在背景 job(20 s watchdog、可取消、關閉時 join 500 ms);10 秒內沒有畫出的幀就失敗(視窗最小化時不會卡住指令)。主執行緒不等 GPU(E3 改成非同步 map:審查發現同步讀回在 debug 一幀 434–498 ms;改後抓取幀 3–55 ms,和一般幀同一範圍,交接 0.01 ms;`--screenshot` 的 smoke 截圖路徑維持原本的同步讀回)。截圖包含疊在 Scene 視圖上的編輯器標示(FPS、Frame / Fullscreen 鈕),這是刻意的(看到的就是拍到的)。

**新指令 / 協定**:`view.display_mode lit|unlit`、`view.screenshot [NAME]`、`asset.folder PATH`、`asset.reveal KIND:ID`(都只是檢視狀態,`parity::view_commands_never_write`);`observe` 的 `editor.look`、`editor.vram`;截圖 job 出現在 `editor.jobs` 與狀態列。會改內容的新操作(收合卡片勾選、卡片 / Transform 的 Revert)都是既有 ops 與 ACL(`set SEL C/enabled B`、`reset SEL C`),有 parity 測試。

**證據**:見下方「驗證」段落(由最後一輪填入)。

**範圍追加 E3(使用者,缺口 `docs/gaps-paladin-night.zh-TW.md` E3:`--screenshot` 只在 `--smoke-frames` 結束時拍)**:新協定 op `{"op":"screenshot","path":"…png","region":"window|view"}`(`Request::Screenshot`,runtime 請求;store 無頭時回 `runtime request requires a live engine connection`),遊戲(`axiom play --port`)與編輯器都可用:路徑驗證(`.png`、無 `..`、相對工作目錄或絕對)、一次一個(第二個回錯誤)、寫完才回覆 `{ok, path, width, height, region, ms}`(bridge 對這個 op 最多等 20 s)、10 s 內沒有畫出的幀就回錯誤。共用 `crate::screenshot`(裁切 / BGRA 轉換 / PNG job,編輯器 `view.screenshot` 也用它;`view.screenshot` 另外接受路徑)。讀回改為非同步 map(`Renderer::take_grab`),主執行緒不等 GPU。端對端(真實 exe,連接埠 47393 / 47394,1600×900):play 與 editor 各 window / view / 相對路徑 / 壞副檔名 / `..` / 壞 region / 連續兩個 / 編輯器指令,全部符合;editor `view` 裁成 Scene 視圖 990×374;結果檔 `e3-results.json` 在工作區 scratchpad。限制:bridge 一次處理一條連線,所以「同時兩個」實際上被 bridge 排隊,拒絕路徑由單元測試 `screenshot::tests::one_capture_at_a_time_and_the_reply_carries_the_size` 覆蓋;遊戲模式的 `region:"view"` 等於整個視窗。
