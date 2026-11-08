# Effects Atlas / 材質與 VFX 展示館

雙擊 `Play Effects Atlas.cmd`。使用獨立的 `data/effects-atlas.sqlite`，既有 FPS、ADV、RTS、Action 世界及存檔不會被覆蓋。

清點依據是 Flax 官方 FlaxSamples 的固定版本 `5f762a1a0862268c595ba2b8d6281b2379c79767`（2026-10-21）。直接讀取 [MaterialsFeaturesTour/Main.scene](https://github.com/FlaxEngine/FlaxSamples/blob/5f762a1a0862268c595ba2b8d6281b2379c79767/MaterialsFeaturesTour/Content/Scenes/Main.scene) 與 [ParticlesFeaturesTour/Main.scene](https://github.com/FlaxEngine/FlaxSamples/blob/5f762a1a0862268c595ba2b8d6281b2379c79767/ParticlesFeaturesTour/Content/Scenes/Main.scene) 的展台標題，得到 **24 個材質展台、12 個粒子展台**。排除未使用的「1. Title」模板與空白「13.」。原始 GUID、場景 blob、SHA-256、子項名稱及 Axiom 對應 ID 保存在 [inventory.json](../content/effects-tour/inventory.json)。

這是依展示主題重新實作的 Axiom 版本。場地、程序材質、粒子與 UI 為本專案內容；沒有匯入 Flax 的二進位材質圖或搬用原專案模型。骨骼拖尾使用既有 Action 原型的本專案角色。

## 操作

左側選擇展台，或在總覽點選展示座。左右方向鍵切換、O 返回總覽；W/S 調整俯仰、A/D 環繞、Q/E 縮放。可自動環繞、暫停與重播效果。底部滑桿透過 expected_rev + set_field 提交真正的 authoring revision；重新啟動會沿用已儲存的內容。導覽、視口與效果播放時間屬於 runtime，不會逐幀產生內容修訂。

聚焦模式依展台的 `postfx` ID 清單只啟用該展台的後處理，讓鄰台的熱扭曲或色差不干擾單項對照；總覽保留完整效果。後處理以不透明深度判斷體積遮擋，透明前後順序仍有限制。

## 清點與對應

「原生等效展示」表示 Axiom 已實作對應技術，不表示與 Flax 的完整渲染管線、資產或品質相同。特別有差異的項目在介面與下表直接標示。

### 24 個材質展示

| ID | Flax 展台 | Axiom 版本 |
|---|---|---|
| mat/01-pbr | 1. Physically Based Rendering | GGX 微表面反射與 Fresnel；環境使用程序化天空與棚燈，未使用 Flax 的反射探針。 |
| mat/02-color | 2. Material Color | 程序木紋與純色材質對照，使用相同表面介面。 |
| mat/03-metalness | 3. Metalness | 同色金屬與絕緣體：F0、漫反射與鏡面能量依金屬度改變。 |
| mat/04-roughness | 4. Roughness | 粗糙度 0.1 / 0.4 / 0.6，觀察反射峰值與高光寬度。 |
| mat/05-specular | 5. Specular | 相同漫反射色，分別關閉與開啟絕緣體鏡面反射。 |
| mat/06-emissive | 6. Emissive | 顏色與程序遮罩自發光，可獨立控制強度及脈動速度；不宣稱 HDR bloom。 |
| mat/07-animated | 7. Animated | UV 移動與色彩動畫，時間來自固定步進模擬。 |
| mat/08-transparent | 8. Transparent | 透明混合與 Fresnel 反射外殼；反射為程序環境，不是即時場景反射或折射。 |
| mat/09-opacity | 9. Opacity | Alpha 0.1 / 0.4 / 0.6，背景模型顯示實際透明混合。 |
| mat/10-alpha | 10. Alpha | 真正的 Alpha Test 丟棄與 Alpha Blend 對照，皆使用程序圓孔遮罩。 |
| mat/11-vertex | 11. Vertex Offset | 頂點階段實際改變位置：平面波浪與沿法線膨脹。 |
| mat/12-normal | 12. Normal Mapping | 程序高度場轉為切線空間法線，與平面法線比較；不需要外部貼圖資產。 |
| mat/13-parallax | 13. Parallax Occlusion | 16 步視差遮蔽搜尋程序高度場；A/D 改變視角可看出 UV 深度位移，輪廓仍是平面。 |
| mat/14-displacement | 14. Displacement | 程序高度場在 GPU 頂點階段實際改變細分網格的表面與輪廓。 |
| mat/15-tessellation | 15. Tessellation | 以真實三角形展示 6 / 12 / 24 / 48 段細分。對應 Flax 的細分主題；沒有宣稱 GPU adaptive、Phong 或 PN triangle 實作。 |
| mat/16-two-sided | 16. Two Sided | 自動環繞觀察單面／雙面剔除；線框使用三角形重心座標，不依賴硬體線框功能。 |
| mat/17-instances | 17. Material Instances | 三個獨立材質實例共用同一份 PBR WGSL；修改參數會保留 shader pipeline。 |
| mat/18-layered | 18. Layered Materials | 木紋、青銅與程序遮罩分層，展示基材、混合與覆蓋材。 |
| mat/19-forward | 19. Forward Pass | AO 與 roughness 的材質輸入診斷；Axiom 目前沒有 Flax deferred G-buffer，這不是場景 G-buffer 採樣。 |
| mat/20-postfx | 20. PostFx Material | 真正讀取已渲染的 Scene Color，執行 RGB 通道偏移與著色；UI 在後處理後繪製。 |
| mat/21-decals | 21. Decals | 由 Scene Depth 重建世界位置，將標記投影到地面及方塊。這是軸對齊 box decal，不是漂浮的透明平面。 |
| mat/22-subsurface | 22. Subsurface | 使用 wrap diffuse 與背光透射近似蠟材質；不是 Flax 的屏幕空間 SSS 或體積散射解算。 |
| mat/23-blend | 23. Blend Modes | 真正的 opaque、alpha、additive 與 multiply GPU blend state，依序疊在相同背景色上。 |
| mat/24-cel | 24. Cel Shading | 2 / 3 / 6 階光照與 Fresnel 邊緣亮度。 |

### 12 個粒子展示

| ID | Flax 展台 | Axiom 版本 |
|---|---|---|
| fx/01-cpu | 1. CPU Particles | 固定步進 CPU 評估的帶種子噴泉；生命週期、速度、顏色與大小皆由資料控制。 |
| fx/02-gpu | 2. GPU Particles | 真正的 compute shader 寫入粒子 storage buffer，再以 instanced sprite 繪製。CPU/GPU 共用解析式軌跡契約，另有讀回比對。 |
| fx/03-lights | 3. Light Particles | 16 個移動點光源，實際加入場景的逐像素光照，同時繪製可見光點。 |
| fx/04-model | 4. Model Particles | 真正的 3D 立方體模型粒子，具有立體面與旋轉；使用 Axiom 原生幾何而非 Flax 的酪梨資產。 |
| fx/05-ribbon | 5. Ribbon Particles | 保存最近 96 個固定步進位置，生成面向相機的連續三角形帶狀拖尾；不是散點連線。 |
| fx/06-attached | 6. Attached Ribbons | 兩條拖尾附在實際動畫骨架的 fore.R / fore.L，跟隨最終骨骼世界矩陣；可用於劍光與武器軌跡。 |
| fx/07-smoke | 7. Smoke | 帶旋轉、膨脹、色彩與 Alpha 漸變的 soft sprite 煙霧；讀取場景深度淡化交界。 |
| fx/08-heat | 8. Heat Distortion | 火焰精靈前方使用真正 Scene Color UV 扭曲，背景格線隨熱流折射移動。 |
| fx/09-snow | 9. Snow | GPU compute 評估的循環飄雪，使用同一份種子契約、生命週期及深度淡化。 |
| fx/10-collision | 10. Collisions | CPU 解析式重力與平面碰撞：解出落地時刻，依回彈係數反射速度，最多追蹤八次碰撞。 |
| fx/11-sampling | 11. Sampling Textures | 從內容包中的二值影像採樣發射位置，粒子組成字形。此版本支援 CPU mask 取樣，未宣稱 GPU 任意貼圖採樣。 |
| fx/12-fog | 12. Volumetric Fog | 以 12 步射線步進積分程序密度，依不透明場景深度終止，呈現霧內模型與遮擋；不是全場景體積光系統。 |



## 可重用的引擎能力

- `render.geometry`：球、平面、環面、真實三角形重心 UV 線框、可選程序位圖。網格在啟用時準備一次。
- `render.shaders`：沿用表面 WGSL，新增可選 `vertex_offset`，及 additive / multiply 原生 blend state。`axiom_local_light(position,normal)` 可取移動點光源貢獻。
- `vfx.library` / `vfx.simulate`：帶種子的 CPU 或 GPU 解析式粒子，顏色與大小漸變、sprite / mesh / light / ribbon、CPU 平面碰撞、二值位圖發射、實體及骨骼掛點。GPU 以 compute pass 寫 storage buffer，直接作為繪製輸入。
- `postfx.settings` / `postfx.prepare`：場景顏色／深度後處理、熱扭曲、色差、深度重建投影貼花，以及有深度遮擋的體積密度積分。UI 在後處理之後繪製。
- `showcase.tour` / `showcase.navigate`：資料驅動的展台目錄、鏡頭、播放與共用編輯介面。

支援最多 64 個 emitters、8192 顆粒子、單 emitter 2048 顆、32 個局部點光源、96 點拖尾及 8 個後處理體積。GPU backend 目前為解析式 sprite / mesh 軌跡；碰撞、light / ribbon 與位圖發射走 CPU，無效組合會拒絕啟用。更換材質參數保留未修改 emitter 的時間與骨骼拖尾。

## AI 操作

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe discover showcase
.\target\debug\axiom.exe discover vfx
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /showcase --field /vfx/emitters/fx~102-gpu
```

選擇展台的共用 command：

```json
{"kind":"call","capability":"showcase","action":"focus","args":{"station":"fx/02-gpu"}}
```

用 `vfx.inspect {id}` 讀取最多 8 個 CPU 參考樣本；GPU backend 的回覆明確標示為參考資料，實際 GPU 運算另由 storage readback 驗證。材質局部修改以 resource `render.shaders`、欄位 `/materials/metal/parameters/metalness` 為例，仍使用 select、preview、apply/execute、observe_delta 和 undo。

## 驗證與再生

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p axiom
cargo run -p axiom-runtime --example verify_vfx_gpu
.\scripts\effects-smoke-test.ps1
```

原生 QA 逐一擷取 36 個展台、測試局部調參、冪等重試、增量觀察、emitter 時間保留，以及頂點階段 GPU 管線失敗後保留可用版本並 undo 修復。報告為 `artifacts/effects-validation.json`。GPU compute 驗證比較 3 種場 × 3 個時間 × 64 顆粒子；讀回誤差是正確性量測，不是 GPU 時間或模型 token 量測。

如需重新清點及產生內容，執行 `scripts/update-effects-tour.ps1`。它下載上述固定版本的兩份公開場景，然後執行 `scripts/build-effects-tour.cjs`；一般遊玩與匯入不需要網路。

## 明確差異

PBR 的環境反射使用程序天空／棚燈，不是反射探針、SSR 或即時光追。細分展台是 CPU 均勻網格細分，不是 GPU adaptive tessellation、Phong 或 PN triangles。Forward 展台呈現材質屬性，不提供 deferred G-buffer 採樣。次表面展台是 wrap diffuse／薄表面透射近似，不是屏幕空間 SSS。體積霧採限定體積的密度步進，不是完整全域體積光或多重散射。CPU 位圖發射為二值遮罩，不宣稱 GPU 任意貼圖取樣。

透明粒子與材質仍受繪製順序影響；目前沒有 OIT。程序三角網格沿用既有網格渲染的陰影限制。VFX 與導覽私有狀態 checkpoint 目前會明確回報未支援；authoring 修訂仍會保存。
