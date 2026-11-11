# 風格化材質與美術管線(`render.toon`、`render.outline`、`render.decal`、模型熱重載)

> 工作線 W2d(2026-11-12),對應 `docs/gaps-paladin-night.zh-TW.md` 的 B5、E1、E2、E4、E5。
> 示範內容包:`content/style-lab`(第一方吉祥物 glb + 程序球體 + 兩張自繪 PNG)。

三個元件都掛在**有網格的實體**上(`animation.player` 匯入的 glTF / PMX,或 `render.geometry` 程序網格),
作用在內建網格 shader。沒有這些元件的物件畫面不變(所有新 shader 參數為 0 時走原本的分支)。

## `render.toon`(元件,capability `render`)

```json
{"bands":2,"softness":0.08,"threshold":0.05,"shadow_color":[0.56,0.5,0.72],
 "rim_color":[1,0.95,0.9],"rim_strength":0.35,"rim_power":4,
 "skin":{"wrap":0.45,"tint":[1,0.38,0.3],"materials":["skin","blush"]},
 "face":{"bone":"head","forward":[0,0,1],"blend":0.75,"materials":["skin","eye"]}}
```

| 欄位 | 範圍(預設) | 意義 |
|---|---|---|
| `bands` | 1..8(2) | 陰影色到全亮之間的亮度階數;1 = 兩色 |
| `softness` | 0..1(0.1) | 每一階邊緣的寬度,0 = 硬邊 |
| `threshold` | -0.9..0.9(0) | 陰影側結束的位置(光線餘弦,wrap 之後) |
| `shadow_color` | RGB 0..1 | 陰影側乘上的顏色(帶色相的陰影) |
| `rim_color` / `rim_strength` / `rim_power` | RGB、0..1(0.3)、0.5..16(4) | 邊緣光(Fresnel 次方,越大越細) |
| `materials` | 材質名稱(glTF `material.name`,PMX 材質名)或 primitive ID(`node/mesh/primitive`)清單(空 = 全部) | 這個外觀套在哪些 primitive(**逐 primitive / 材質群組**) |
| `skin` | `{wrap 0..1 (0.5), tint, materials}` | **皮膚透光的近似**:wrap lighting(光照延伸過明暗交界)+ 交界線附近一條染色帶。**不是次表面散射(SSS)**,沒有擴散、沒有厚度 |
| `face` | `{bone?, forward (0,0,1), blend 0..1 (1), materials}` | **臉部陰影控制(近似)**:臉部 primitive 的著色法線往 `forward`(在 `bone` 的空間,沒有就用實體空間)彎,臉上的明暗由頭的朝向決定,不會出現鼻子 / 眼窩的雜亂陰影。不是 SDF 臉部陰影貼圖 |

- 光照:每個太陽(directional)的 `N·L` 先經 wrap,再分成 `bands` 階;sun 0 的陰影貼圖也量化成硬邊;陰影側 = 環境光 × `shadow_color`。局部光(point / spot / area)、霧、色調曲線與一般網格相同。
- 世界沒有編寫燈光時(舊版平光),toon 以舊版固定主光分階。
- 編輯器 Unlit 顯示模式:toon 不作用(平塗底色)。
- 有 `render.material`(自訂 surface shader)的實體:由自訂 shader 決定光照,`render.toon` 不作用;描邊與貼花仍作用。
- 有 toon 的 primitive 不套 `render.surface` 反射。

## `render.outline`(元件)

```json
{"width":2.5,"units":"pixels","color":[0.08,0.05,0.12],"materials":[]}
```

- **反向外殼(inverted hull)**:把 primitive 再畫一次,頂點沿法線往外推,剔除正面,只留下輪廓外的一圈。
- `units: pixels`(預設)寬度 0..16 個實體像素,螢幕上固定粗細;`meters` 寬度 0..0.2 m,隨距離變細。
- 只畫**不透明** primitive(套用自訂材質的 alpha 之後;規則與投射陰影相同);mask / blend(頭髮卡片、透明)沒有外殼。
- 法線分裂的硬邊(方塊角)外殼會有缺口 — 反向外殼的已知限制。
- 雙面材質:glTF `doubleSided` 已由匯入器讀入、網格 pipeline 依它關閉背面剔除(`import/gltf.rs` `double_sided`、`mesh_render.rs` pipeline 選擇;本工作線只查證、未改)。**雙面 primitive 不畫外殼**:薄片沒有「內側」,繞向背對鏡頭的薄片,它的外殼會蓋住薄片本身(示範包第一次截圖時吉祥物的白色上衣與頭髮因此變暗,`outlined()` 規則與測試)。要描邊雙面的薄片,請在建模時做成有厚度的封閉體,或讓材質不是 doubleSided。
- 成本:每個有外殼的 primitive 多一次 draw(同一份頂點、同一個 bind group,一條新 pipeline `mesh outline pipeline`)。

## `render.decal`(元件;第二層貼圖)

```json
{"image":"emblem","blend":"alpha","opacity":1,
 "projector":{"bone":"chest","position":[0,0.07,0.12],"size":[0.11,0.11,0.2]}}
```

- `image`:`ui.document` 的圖片資產 ID(PNG,≤ 4 MiB、≤ 2048²;與 UI 圖片同一個解碼與預算)。世界驗證時檢查圖片存在(`unknown_image` 指向 `…/components/render.decal/image`);Assets 的 Find Usages / Rename 會找到這個欄位。
- 兩種貼法:
  - **UV**(沒有 `projector`):`rect: [u0,v0,u1,v1]` = 圖片蓋住網格 UV 的哪個矩形(glTF UV,v 向下)。需要網格有 UV。
  - **投影**(`projector`):在 `bone`(沒有就實體)空間裡的一個盒子(`position`、`rotation` 四元數、`size`),沿盒子 -Z 投影;圖片的左右 = 盒子 X,上下 = 盒子 Y。背向 +Z 的面不畫。跟著骨骼動(胸口徽章跟著身體轉)。吉祥物 glb 沒有 UV,所以示範用投影。
- `blend`:`alpha`(依圖片 alpha 蓋上)、`multiply`(乘,畫暗部細節)、`add`(加,發光)。`opacity` 0..1。`materials` 限定 primitive(空 = 全部)。
- 一個實體一層貼花(臉部的眼 / 眉 / 嘴 / 腮紅可以畫在同一張圖)。自訂材質(`render.material`)也套用(在 surface hook 之前疊到 base color)。
- 圖片解碼在 runtime 建置時(背景啟用 job),以檔案內容雜湊快取(最多 32 張);GPU 貼圖每個實體 + 模型一份,圖片換了才重建 bind group。

## 模型熱重載(缺口 E4)

| 入口 | 行為 |
|---|---|
| 協定 `{"op":"reload_assets"}` | 開始一次重載,立刻回覆 `{ok, started, job}`;結果在 `observe` → `assets_reload.last` |
| `{"op":"reload_assets","watch":true\|false}` | 開 / 關檔案監看:每秒 `stat` 一次每個模型檔(大小 + 修改時間),有變就自動開始重載 |
| `{"op":"reload_assets","cancel":true}` | 取消執行中的重載 |
| 編輯器 Assets ▸ Reload Changed Models(`asset.reload`)、Watch Model Files(`asset.watch on\|off`) | 同上(`editor_view {command}` 也可) |

- 背景 job(`asset_reload`,60 秒 watchdog、可取消、進度 = 已雜湊的檔案數 / 檔案數 + 1、出現在編輯器狀態列的 jobs 與 `observe`):
  1. 對世界的每個模型檔算內容雜湊(檔案位元組 + 匯入選項,FNV-1a),檔案之間檢查取消。
  2. 有任何雜湊和上次接受的不同(第一次一律視為不同)→ 在 worker 上建一個新的 runtime(匯入快取只對內容變了的檔案失效)。
- 主執行緒(下一幀)用每個動畫實體的資產**內容鍵**比較:
  - 都一樣 → `unchanged`,什麼都不換。
  - 有變、骨架相同(骨骼 ID 與父子關係依序一致)→ `reloaded`,像一次 revision 熱重載一樣 adopt:片段時間、控制器狀態、物理等照常延續。
  - 骨架變了 → `refused`(訊息指出實體與第一根不同的骨骼),原本的 runtime 不動。
  - job 期間有新的 revision 在啟用 → `superseded`(新的啟用本來就會載入新檔)。
- 記錄:`assets_reload.last = {state, changed, files_changed, hash_ms, build_ms, adopt_ms, elapsed_ms}`,同一行也寫到 Console(`Assets reloaded (…) in N ms`)與編輯器狀態列。
- 限制:參考圖疊圖檢視(缺口 E4 的後半)沒有做;監看只看模型檔,不看貼花 PNG(PNG 改了要 `asset.reload` 之外的 revision 編輯或重開;貼花快取以內容雜湊判斷,下一次 runtime 建置就會讀到)。

## 量測

(由下方「量測紀錄」填入)
