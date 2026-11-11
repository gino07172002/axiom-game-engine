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

### R 繪圖品質(worktree `wr`,branch `work-wr`)

**ADR R-1:反射走「天空環境反射(解析式,不用 cubemap)」,不做反射探針與 SSR(這一輪)**

| 方案 | MX150 成本 | 品質 | 編輯 / AI | 確定性 | 是否需要重工作 |
|---|---|---|---|---|---|
| A. 反射探針 cubemap(指令觸發擷取) | 每個探針擷取 = 6 次場景 pass(一次性);執行期每像素 1 次 cube 取樣 + box projection;每個探針 6×N² 貼圖記憶體 + mip | 看得到其他物體(靜態時) | 需要探針元件、擷取指令、快取(依內容雜湊、放在 revision 外)、失效規則、gizmo | 擷取的是「那一刻」的執行期畫面(動態物件、粒子、動畫時間會進去),同一 revision 可能擷取出不同結果,要另外定義「擷取哪一個狀態」 | 擷取必須是明確指令或背景工作;不能隱性重擷取 |
| B. SSR | 每幀全螢幕 ray march(MX150 1080p 估計數 ms);forward renderer 沒有法線 G-buffer,要加 MRT 或從深度重建 | 只反射畫面內的東西,邊緣 / 被遮住處破洞,要 fallback | 幾乎不用編寫 | 依視角變化,不影響內容 | 每幀成本,多相機 viewport 要各算一次 |
| C. **天空環境反射(採用)** | 每個 opted-in 像素約數十個 ALU(估計;實測見下)(解析天空求值 + Schlick),零貼圖、零擷取 | 只反射天空(smoothness 低時模糊到 trilight),不反射物體 | 一個元件 `render.surface {metallic, smoothness}`,Inspector 直接加 | 純函式,完全確定 | 無 |

決定:C。理由:(1)計畫允許的降級方案就是 cubemap 環境反射,C 的畫面等同「只有天空的 cubemap」但省掉貼圖與擷取;(2)A 的擷取確定性與快取失效設計是一整條工作線的量,這一輪的時間要留給驗證;(3)B 在 MX150 上成本最高,而且 forward 管線沒有法線緩衝。A 列為下一步(`render.surface` 與 `axiom_environment()` 是它的接點:之後探針只要替換 `axiom_environment` 的來源)。成本實測(release A/B)見 `docs/lighting.zh-TW.md`「量測(天空、反射、網格陰影)」:反射與網格投影在雜訊內,天空背景約 +2 ms / 幀(MX150、機器同時有其他編譯,不穩定)。

**補記(原計畫 → 變成什麼 → 為什麼)**

- 「反射探針或 SSR」→ 天空環境反射(ADR R-1)。
- 「匯入網格接收陰影」→ 接收 **並且投射**(只在編寫了燈光的世界):只接收不投射時角色腳下沒有影子,看起來更假;投射的條件和接收相同,舊內容包(沒有編寫燈光)的陰影貼圖不變。
- 天空光不是新的獨立元件,而是 `ambient` 的第三種模式 `sky`(CPU 積分天空成 trilight),沿用既有的 trilight 管線與 shader。
- 背景天空原本計畫用 vertex→fragment 傳 NDC;在 MX150 Vulkan 驅動(32.0.15.6614)上「vs 用陣列索引角點 + 輸出 varying」這個組合讓 `nvoglv64.dll` 在建立 pipeline 時 access violation(每個內容包啟動即崩潰,事件記錄可查)。改成 fragment 用 `@builtin(position)` 與新的 `sky_viewport` global 自己算 NDC,舊版背景分支一字不改。
- 第一輪 A/B 顯示天空背景約 +1.9 ms(6/7),所以把編寫的天空改到不透明幾何之後畫(只算看得到的像素);第二輪 A/B 量不出這個改動的差別(見 lighting 文件),因為不改變畫面而保留,不宣稱有改善。
- 事故紀錄:二分搜尋驅動崩潰時,一個 PowerShell `[IO.File]::WriteAllText` 用了相對路徑,寫到了**主工作樹**的 `crates/axiom-runtime/src/scene.wgsl`(.NET 的 cwd 是 D:\rust_learn,不是 worktree)。發現後立刻用 `git checkout` 還原該檔(diff 確認內容全是我的實驗碼,主工作樹之後 `git status` 乾淨)。期間若主工作樹的代理剛好在編譯,可能看過一次失敗的 shader。
- 新增第一方示範包 `content/render-lab`(程序天空 + 天空光、拋光地面、金屬方塊、鉻球 `render.geometry`、無表面的對照方塊、遮棚、`fighter.glb`——和 `content/action` 同一個本專案角色檔)。

**Milestones(每個都 commit 在 `work-wr`)**

| Milestone | 內容 |
|---|---|
| M1 walking skeleton | `render.environment.sky`(gradient / procedural)型別 + 驗證 + schema;`SkyParams`(CPU)與 `axiom_sky`(WGSL)同一公式;`ambient: sky`;Globals 加 `inv_view_proj` / 天空 4 個 vec4 / `sky_viewport`(stride 3072 → 3328,8 相機 26 KB);`ops::set_sky` + ACL `sky` + `world.sky` + World 視窗 SKY 列;parity 測試 |
| M2 網格陰影 | `shadow.wgsl` 共用;網格 camera group 加陰影貼圖 + 比較取樣器(接收);`vs_mesh_shadow` + 獨立 uniform-only group 的投影 pass(避免同一 pass 既取樣又寫入陰影貼圖);只在編寫燈光的世界生效 |
| M3 反射 | `render.surface` 元件(ADR R-1);方塊 `material.zw`、網格 `Draw.params.zw` 編碼 (metallic, smoothness+1),0 = 沒有元件 = 舊路徑 |
| M4 驗證 | 示範包、截圖、舊包逐像素比對、smoke、release A/B、fresh-context 審查與修正 |

**驗證紀錄**

- `cargo fmt`、`cargo clippy --workspace --all-targets -D warnings` 乾淨;`cargo test --workspace` 701 通過 / 0 失敗(基準 692,新增 9)。
- 舊內容包逐像素(1440×900 smoke,改動前 exe 是 base commit 重編的 debug 版):axiom-town / shader-lab / effects-tour / action / mascot / street 0 差異;light-lab 2、two-cameras 4、rts 1 個像素差 1/255;fps 132 個像素全在 HUD 右上 16×13 的數字區(同一個舊 exe 連跑兩次也有 51 個,是 HUD 數字不是渲染)。
- `editor-smoke-test` ok:true;`editor-world-parity` identical:true;`street-editor-parity` identical:true;`build-street`:content/street 不變;`m0-smoke-test` PASS;`effects-smoke-test` PASS(36 站全擷取,機器同時有其他代理在編譯,仍在時限內)。
- 截圖(實際看過):`artifacts/wr/wr-{procedural,gradient}-{1080,768}.png`、`wr-sunset-1080.png`;編輯器 `artifacts/captures/wr-editor-{1080,768}.png`(World 視窗 SKY 列 + Inspector `render.surface`)。網格接收陰影另外用「把網格輸出改成陰影值」的暫時 shader 確認(角色在柱子陰影線上 = 全藍),之後把示範場景調成只有軀幹在遮棚陰影裡。
- Fresh-context 審查:無嚴重錯誤;確認並修正 — 日盤大小在 3° 以下幾乎不變(邊緣寬度改成跟日盤成比例,加測試)、`set_sky` 經 f32 重寫整個資源(改成只改 `/sky`、`/ambient` 的 JSON)、World 按鈕與 ACL 同模式行為不一致(同模式一律空批次)、缺 Rust/WGSL Globals 大小測試(加 naga 測試)、網格投影 alpha 規則沒測試(抽成 `casts_shadow` + 測試)、文件提到不存在的量測段落(補上 A/B)、背景天空不套色調曲線(文件說明)。
