# 燈光系統(`render.light` 與 `render.environment`)

參考 Blender / Unity 的燈光類型。燈光是一般元件,和其他內容一樣走交易協定(`apply` / ACL / 編輯器),不需要改 Rust。
範例內容包:`content/light-lab`(夜間廣場,`Play Light Lab.cmd`)。

## `render.light`(元件,capability `render`)

燈光沿實體的**本地 -Z 軸**照射,跟 `render.camera` 的方向慣例相同。位置、方向都來自 `core.transform`(含 parent)。

```json
{"kind":"spot","color":[0.85,0.92,1],"intensity":5,"range":9,
 "spot":{"inner_degrees":28,"outer_degrees":44},"source_radius":0.05,"shadows":false,"enabled":true}
```

| 欄位 | 說明 |
|---|---|
| `kind` | `directional` / `point` / `spot` / `area`(必填) |
| `color` | RGB 0..1,預設 `[1,1,1]` |
| `intensity` | 0..100,預設 1 |
| `range` | 公尺 0.01..500。point / spot / area **必填**;directional **不可填** |
| `spot` | 只限 `spot`(且 spot 必填):`{inner_degrees, outer_degrees}`,**整個錐角**(Unity spotAngle、Blender spot_size 的定義),`0 ≤ inner ≤ outer`、`outer` 1..179。內外角之間用 smoothstep 漸弱 |
| `area` | 只限 `area`(且 area 必填):`{shape: rect\|disk, size:[w,h]}`,公尺 0.01..100;rect 沿本地 X / Y,disk 必須是 `[直徑, 直徑]`。大小不受 transform scale 影響 |
| `source_radius` | 選填,只限 point / spot,0..range:讓明暗交界變柔和(近似球形光源) |
| `shadows` | 預設 false。見下方「陰影」 |
| `enabled` | 預設 true |

驗證是嚴格的:未知欄位、種類不符的欄位(例如 point 帶 `spot`)、超出範圍的數值都會讓交易失敗,並回傳原因。

### 預算

- 每個世界最多 **4 盞 directional**、**32 盞 point / spot / area**(所有帶 `render.light` 的實體都算,不論啟用與否或場景狀態,結果因此是確定的)。超過時交易失敗(`directional light budget exceeded` / `point/spot/area light budget exceeded`)。
- GPU 的局部光陣列有 32 格,由編寫的燈光和 VFX 粒子光源(`vfx.library` renderer `light`)共用:**編寫的燈光先放(依實體編寫順序),VFX 光源接在後面(依 emitter 順序)**,放不下的丟棄。32 格沒有改:每格從 32 bytes 增加到 80 bytes,每個 view 的 uniform 從 1280 bytes 增加到 3072 bytes(8 個相機共 24 KB);加入天空後再增加到 3328 bytes(8 個相機共 26 KB)。

### 平行光(directional)

- 使用**第一盞啟用中的** directional(依實體編寫順序)當主光,其餘(最多到第 4 盞)也會照明,但**只有第一盞能投射陰影**,而且要它的 `shadows: true`。第一盞 `shadows: false` 時,陰影 pass 直接跳過。
- 陰影矩陣沿著該平行光方向,範圍沿用 `render.settings.shadow_extent`,中心固定在**世界原點**(跟舊版一樣;不跟著相機移動,所以大場景遠處沒有陰影)。
- 世界裡**沒有任何啟用的 directional** 時,沿用舊版寫死的太陽(方向 `(-0.45,0.85,0.35)`、顏色 `(1.12,1.0,0.82)`、舊的陰影矩陣)。所以停用唯一一盞 directional 會讓預設太陽回來;要「沒有太陽」請把 intensity 設成 0。

### 點光源、聚光燈

- 衰減:`(1 - 距離/range)²`,超過 range 為 0(和舊的 VFX 光源同一個公式)。
- 聚光燈再乘上 `smoothstep(cos(outer/2), cos(inner/2), dot(光軸, 光→表面方向))`。inner = outer 時視為硬邊。
- `source_radius`:把 N·L 改成 wrap 形式 `(N·L + k)/(1 + k)`,`k = source_radius / 距離`(上限 1)。`source_radius = 0` 時和原本完全相同。這是**近似**,不是真正的球形面光源積分。

### 面光源(area)— **近似**

面光源**不是** LTC(Linearly Transformed Cosines)那種準確的面光源積分。實作方式:

1. 取矩形 / 圓盤上**離被照點最近的一點**當代表點;
2. 對該點套用點光源的 range 衰減與 N·L;
3. 再乘上單面發光的餘弦 `max(dot(光軸, 代表點→表面), 0)`,所以只朝 -Z 那面發光。

面積越大、照亮的區域越大(代表點會跟著移動),但亮度不會按面積積分,也沒有正確的鏡面反射形狀。schema 說明與編輯器選單都標了「approximate」。

### 陰影

| 種類 | 狀態 |
|---|---|
| directional | 第一盞啟用的 directional 可投射陰影(2048² PCF,和舊版相同) |
| spot | **尚未支援**,`shadows: true` 會被拒絕:`spot light shadows not supported yet` |
| point | **尚未支援**,`shadows: true` 會被拒絕:`point light shadows not supported yet` |
| area | **尚未支援**,`shadows: true` 會被拒絕:`area light shadows not supported yet` |

聚光燈陰影的成本量測見下方「量測」。點光源與面光源也不會被物體遮擋(光會穿過牆)。

## `render.environment`(資源,capability `render`)

```json
{"ambient":{"mode":"gradient","sky":[0.05,0.06,0.12],"equator":[0.025,0.03,0.05],"ground":[0.012,0.01,0.015],"intensity":1},
 "fog":{"color":[0.02,0.03,0.06],"density":0.025}}
```

- `ambient`(選填):
  - `{"mode":"color","color":[r,g,b],"intensity":k}`:單色環境光;
  - `{"mode":"gradient","sky":…,"equator":…,"ground":…,"intensity":k}`:Unity Trilight。法線朝上用 sky,水平用 equator,朝下用 ground,之間線性插值。
  - 省略時沿用舊版:ground `(0.11,0.16,0.19)` 到 sky `(0.37,0.48,0.53)` 的半球漸層,再乘 `0.85 + accent × 0.15`(`render.settings.accent`)。編寫了 ambient 就**不再**乘 accent。
- `fog`(選填):`{color, density}`,指數平方霧 `1 - exp(-(density × 距離)²)`(Unity ExponentialSquared 的定義)。省略時沿用舊版(`k = 0.00012`,約等於 density 0.011)。
- `sky`(選填,見下節「天空」)。省略時背景沿用舊版固定的螢幕空間漸層。

### 天空(`render.environment.sky`)與天空光

```json
{"sky":{"mode":"procedural","sun_size_degrees":1.5,"atmosphere":1,"ground_color":[0.22,0.2,0.18],"exposure":1},
 "ambient":{"mode":"sky","intensity":1}}
```

| 模式 | 欄位 | 說明 |
|---|---|---|
| `gradient` | `zenith_color`、`horizon_color`、`ground_color`(rgb 0..1,必填)、`exponent` 0.1..8(預設 1)、`intensity` 0..8(預設 1) | 依**視線方向**:地平線以上 `mix(horizon, zenith, pow(up, exponent))`,地平線以下約 10° 內漸變到 ground |
| `procedural` | `sun_size_degrees` 0..10(預設 1.5,0 = 不畫日盤)、`atmosphere` 0..4(預設 1,越大地平線霧帶越高、日落越紅;0 = 整片天頂色)、`ground_color`、`exposure` 0..8(預設 1) | 解析式日光天空,跟著**第一盞啟用的 directional**(沒有時是預設太陽)的方向與顏色:太陽高 = 藍天,接近地平線 = 暖色地平線,低於地平線 = 夜色;太陽周圍有光暈與日盤。**近似**,不是物理散射解算 |

- 背景天空在每個 view 用 inverse view-projection 算出像素的世界方向,所以會跟著相機轉;多相機各自正確(NDC 由像素位置和該 view 的 viewport 矩形算出:把 NDC 當 vertex→fragment varying 傳的版本在 MX150 Vulkan 驅動 32.0.15.6614 建立 pipeline 時直接當掉)。**正交相機**的每個像素方向相同,整片是同一個顏色。
- 背景天空**不套**色調曲線與霧(跟舊版背景一樣直接輸出,只截到 1,所以 `gradient` 寫的顏色就是畫面上的顏色);物體上的反射與天空光則和其他光照一樣經過 `x/(x+0.7)` 與霧。所以完美鏡面裡的天空會比背後的天空暗一些,遠方霧化的地面也不會和地平線完全融合。
- `ambient: {"mode":"sky","intensity":k}` = **天空光**:CPU 每幀用固定 16×16 方向格點(等立體角)積分天空(不含日盤),得到法線朝上 / 水平(四個方位平均)/ 朝下的 irradiance/π,再乘 `k` 當作 trilight。必須同時有 `sky`,否則交易失敗(`ambient mode sky requires a sky`)。
- 只寫 `sky` 不寫 ambient:背景換成天空,環境光仍是舊版預設。
- 編輯器:World 視窗 ▸ **SKY** 列的 Gradient / Procedural / None 按鈕,或 World 選單 ▸ Sky…(`world.sky gradient|procedural|none`);ACL `sky gradient|procedural|none`。兩者同一個 `ops::set_sky`:沒有 `render.environment` 時建立它(並補 `render` capability),放入預設天空,沒有編寫 ambient 時加上 `ambient: sky`;同一模式再按一次不寫入(保留調整過的數值);`none` 移除天空與 sky 模式的 ambient(其他 ambient、fog 保留)。細部數值在 Inspector 開 `render.environment` 調整(`*_color` 欄位是色票),或 `set resources[id=render.environment] sky/exposure 1.4`。
- 編寫的天空在不透明幾何**之後**畫(depth LessEqual 對清空的遠平面),只有看得到天空的像素才算方向與公式;舊版背景仍然最先畫(depth Always),路徑不變。
- 量測見下方「量測(天空、反射、網格陰影)」。

### 表面反射(`render.surface`,元件)

```json
{"metallic":1,"smoothness":0.9}
```

- `metallic` 0..1(預設 0):0 = 絕緣體(正入射 4% 反射),1 = 金屬(反射染上 base color,沒有漫射)。`smoothness` 0..1(預設 0.5):1 = 鏡面般清楚的天空反射,0 = 模糊成 trilight。
- 作用在該物件**所有** `render.mesh` 方塊部件,以及它的匯入 / 程序網格(`animation.player`、`render.geometry`;網格只在世界有編寫燈光時生效,跟網格的光照模型同一個條件)。自訂材質(`render.material`)的物件由材質 shader 決定外觀,不套用。
- 反射內容:有 `sky` 時是天空(smoothness 越低越往 trilight 模糊,smoothness > 0.95 時看得到日盤),沒有 `sky` 時是 trilight。Fresnel 用 Schlick + 依 smoothness 的掠射項。**不會反射其他物體**(沒有反射探針、沒有 SSR,理由見 `docs/plan-engine-next.zh-TW.md` 的 R ADR);陰影裡的表面仍反射天空(沒有反射遮蔽)。
- 沒有 `render.surface` 的物件畫面完全不變。
- 編輯器:Inspector ▸ Add component ▸ `render.surface`(滑桿範圍來自 schema);ACL `add SEL render.surface [JSON]`、`set SEL render.surface/smoothness 0.9`。

## 和舊版的相容性

- 沒有 `render.light` directional、也沒有 `render.environment` 的世界,畫面跟改動前一樣:axiom-town、fps、shader-lab、action、effects-tour 在同一個視窗大小下前後截圖逐像素比對,576,000 像素中最多 2 個像素差 1/255(浮點捨入)。
- 天空 / 反射 / 網格陰影這一版(工作線 R):沒有編寫燈光的內容包逐像素比對不變(axiom-town、fps、shader-lab、light-lab、effects-tour、action、two-cameras、mascot、rts,1440×900 smoke 截圖:同一 exe 連跑兩次的雜訊只有 fps HUD 右上角 16×13 的數字區;改動前後其餘差異最多 2 個像素差 1/255)。**已經編寫燈光**的內容包裡的網格會開始接收 / 投射陰影,這是刻意的改變(light-lab 沒有網格,所以不變)。
- 匯入的三角網格(角色、glTF、`render.geometry`):沒有編寫燈光時仍用原本的平光公式(不接收也不投射陰影,舊畫面不變);世界一旦編寫了 directional、`render.environment`(ambient / fog / sky 任一項),網格改用和方塊相同的模型(環境光 + 平行光 + 局部光 + 霧 + 色調曲線),並且:
  - **接收** sun 0 的陰影(同一張 2048² 陰影貼圖、同樣的 3×3 PCF;網格的 camera bind group 多了陰影貼圖與比較取樣器,自訂材質的 `SurfaceInput.shadow` 也是實際值);
  - **投射**陰影:不透明的 primitive(材質 alpha opaque,或自訂材質 alpha opaque)在 sun depth pass 畫進陰影貼圖(不剔除背面,深度偏移和方塊相同);mask / blend 不投射。自訂材質的 `vertex_offset` 不影響投影(陰影用未位移的幾何)。
- 自訂材質(`render.shaders`):`SurfaceInput.diffuse` / `shadow` 改成第一盞 directional(或預設太陽)的值;`axiom_local_light(p, n)` 現在包含編寫的 point / spot / area 光和 VFX 光源。

## 編輯器與無頭操作

| 編輯器 | 無頭(ACL) |
|---|---|
| Object 選單 / 指令面板:`object.light_directional`、`object.light_point`、`object.light_spot`、`object.light_area` | `create light directional\|point\|spot\|area [id=] [at=x,y,z] [parent=] [scene=]` |
| Hierarchy「+ Object」▸ Directional / Point / Spot / Area light | 同上;也可寫 `create light.point` |

預設值:directional 和預設太陽同方向、`shadows: true`;point range 6;spot range 10、內外角 30° / 45°、朝正下方;area 2×1 m 矩形、range 8、朝正下方。預設 ID 是 preset 名稱(`light.point`、`light.point/2`…)。

Gizmo(場景工具列 Gizmos 開關,隱藏的物件不畫):directional = 小圓 + 箭頭(標 SUN);point = 三個軸向的 range 圓(有 source_radius 再加一個小圓);spot = 外錐(邊緣在 range 球面上)+ 較暗的內錐圓 + 光軸;area = 矩形 / 圓盤外框 + 法線。停用的燈光畫暗色,選取中的畫白色。Hierarchy 標籤 `LIGHT`。

## 量測(聚光燈陰影要不要做)

先量現有平行光陰影的成本,當作「多一張陰影貼圖」的代價估計(聚光燈陰影同樣是一個深度 pass 加上 PCF 取樣)。

- 環境:**debug build**(`target/debug/axiom.exe`)、NVIDIA GeForce MX150 / Vulkan、960×600 隱藏視窗(`--smoke-frames`)。
- 量測時**暫時**把 present mode 改成 `AutoNoVsync`(量完已改回 `AutoVsync`),否則每幀都被 vsync 卡在 16.7 ms,看不出差異。
- 方法:每次都開新的 process,每幀成本 = (600 幀的牆鐘時間 − 60 幀的牆鐘時間) / 540。A = 第一盞 directional `shadows: true`(陰影 pass + 9 次 PCF 取樣),B = `shadows: false`(跳過陰影 pass,shader 也不取樣)。A / B 交錯(ABBA…),取中位數。

| 場景 | 回合 | A(有陰影)中位數 | B(無陰影)中位數 | 差 |
|---|---|---|---|---|
| `content/light-lab`(27 個方塊部件) | 5 | 2.48 ms | 2.60 ms | 測不出來(落在雜訊內,B 甚至比較慢) |
| `content/rts-bench` + `create light directional id=sun`(64 個單位) | 7 | 4.73 ms | 3.81 ms | 約 +0.9 ms / 幀(7 回合中有 6 回合 A 比較慢) |

結論:在 MX150 上,一張 2048² 陰影貼圖在方塊多的場景大約多 1 ms / 幀(debug build,數字會隨負載變動 2–3 倍,只能當量級參考)。每多一盞投影的聚光燈就要多一個 pass,而且還需要額外的貼圖綁定與每盞燈的矩陣。這一版**不實作**聚光燈陰影,`shadows: true` 會被明確拒絕。點光源要 6 面 cube map,成本約是聚光燈的 6 倍,同樣不做。沒有量 GPU 時間戳(wgpu 沒開 `TIMESTAMP_QUERY`),上面的數字是整個 process 的牆鐘時間差。
## 量測(天空、反射、網格陰影)

- 環境:**release build**(`cargo build --release -p axiom -j1`)、NVIDIA GeForce MX150 / Vulkan、1440×900 隱藏視窗(`--smoke-frames`)、`content/render-lab`。量測用的 build **暫時**把 present mode 改成 `AutoNoVsync`,並加一個只存在於量測 build 的環境變數略過網格投影 pass(量完都已還原,沒有提交)。
- 方法:每次開新 process,每幀成本 = (600 幀牆鐘 − 60 幀牆鐘) / 540;A / B 交錯(ABBA…)7 回合,取中位數。**同時有其他代理在編譯**,同一設定的回合之間差到 2–3 倍,下表只能當量級參考。

| 比較 | A 中位數 | B 中位數 | 差 | A 較慢的回合 |
|---|---|---|---|---|
| 天空(程序天空 + 天空光,先畫的版本)vs `sky none` | 6.44 ms | 4.53 ms | +1.9 ms | 6/7 |
| 天空(後畫的版本)vs `sky none` | 7.37 ms | 5.41 ms | +2.0 ms | 4/7 |
| 後畫 vs 先畫(同一份內容,兩個 build) | 8.04 ms | 8.12 ms | 測不出來 | 2/7 |
| `render.surface`(地面、金屬方塊、鉻球)vs 移除 | 5.60 ms | 6.22 ms | 測不出來(A 還比較快) | 1/7 |
| 網格投影 pass(1 個 2466 頂點的角色)vs 略過 | 6.41 ms | 6.44 ms | 測不出來 | 3/7 |

結論:反射與網格投影在這個場景落在雜訊內;天空背景大約多 2 ms / 幀的量級(第二輪只有 4/7 回合較慢,不是穩定的結果)。把天空改成在不透明幾何之後畫沒有量到改善,保留它是因為只算看得到的像素、不改變畫面。沒有量 GPU 時間戳(這是整個 process 的牆鐘時間差);CPU 端天空光積分每幀 256 個方向,沒有單獨量。