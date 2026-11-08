# Shader 與材質

雙擊 `Play Shader Lab.cmd`。畫面從左至右展示 Lit、Toon、WGSL 掃描線、Alpha Mask 格柵和半透明外殼。展示資料位於 `content/shader-lab`。

```powershell
. .\scripts\env.ps1
cargo build --workspace
.\target\debug\axiom.exe --db data/shader-preview.sqlite --pack content/shader-lab play --start
```

內容包在 manifest 與 world 同時宣告 `render: 1` 和 `shader: 1`。`render.shaders` 保存可重用的 shader 程式和材質實例，物件或 prefab 的 `render.material` 以 `{"id":"scanner"}` 指定材質。方塊材質會套到物件的全部 parts。匯入蒙皮網格的材質會套到該物件全部 primitive，並保留各 primitive 的 base color 和貼圖。原本內容可以繼續使用預設渲染。

| Shader | 行為 |
|---|---|
| `lit` | 使用漫射光與環境光。方塊另可接收場景陰影 |
| `unlit` | 直接輸出乘上 `tint` 的顏色與來源 alpha |
| `toon` | 分段光照與邊緣亮度，使用 `bands` 和 `rim` |
| 自訂 ID | 執行內容包內的 WGSL `surface` 函式 |

材質可指定線性 RGBA `tint`、依名稱覆寫的 `parameters`、`alpha`、`alpha_cutoff` 和 `double_sided`。`alpha: inherit` 對方塊使用不透明，對匯入網格沿用原材質。`opaque` 和 `mask` 寫深度，`blend` 在它們之後繪製並停用深度寫入。混合使用繪製順序，交疊透明物件可能需要調整順序。方塊的不透明材質投射陰影，透明與遮罩方塊跳過投影。匯入三角網格目前使用直接光照。

## 自訂 WGSL

Shader 定義接受內嵌 `wgsl` 或包內相對路徑 `file`。載入包時會把 `.wgsl` 內容存入世界資料，讓每個 revision 可重現。專案檔可以獨立編輯，遊戲中的修改透過交易套用。

```wgsl
fn surface(s: SurfaceInput) -> vec4<f32> {
    let pulse = 0.5 + 0.5 * sin(s.time * s.params.x);
    return vec4<f32>(s.base_color.rgb * pulse, s.base_color.a);
}
```

`SurfaceInput` 提供 `base_color`、`world_position`、`normal`、`uv`、`view_direction`、`time`、`diffuse`、`shadow` 和 `params`。`time` 以模擬秒為單位。方塊 UV 是依面法線選取的世界座標投影，網格使用來源 UV。`diffuse` 與 `shadow` 來自第一盞啟用的 `render.light` directional(沒有時是預設太陽,見 [lighting.zh-TW.md](lighting.zh-TW.md))。`params` 是四個 float slots，可用 `{slot,default,min,max}` 定義參數名稱和範圍。WGSL hook 可用輔助函式、常數及條件分支。Axiom 管理 entry point、binding、深度與 alpha 處理。

```powershell
.\target\debug\axiom.exe discover shader
.\target\debug\axiom.exe schema render.shaders
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /shader --field /activation
```

以下範例將檔案內容透過現有協定熱更新。先取得 revision，再 preview。將 `op` 改為 `apply` 後送出同一份編輯即可提交。

```powershell
$axiom = '.\target\debug\axiom.exe'
$connection = @('--connect', '127.0.0.1:4242')
$current = & $axiom @connection query | ConvertFrom-Json
$request = @{
    op = 'preview'
    expected_rev = $current.rev
    edits = @(@{
        kind = 'set_field'
        target = @{kind='resource'; id='render.shaders'}
        field = '/shaders/scan/wgsl'
        value = Get-Content content/shader-lab/scan.wgsl -Raw
    })
}
$request | ConvertTo-Json -Depth 12 -Compress | & $axiom @connection rpc
```

局部調參可以把 `field` 改成 `/materials/scanner/parameters/frequency`，將 `value` 設成數值。`preview` 檢查 WGSL、引用及參數範圍，錯誤回覆包含穩定路徑和可取得的行列。`apply` 成功後，原生視窗會準備 GPU pipeline 再啟用新 revision。相同 WGSL 的材質變更重用 pipeline。管線建立失敗時，`observe` 的 `requested_rev` 顯示新提交，`rev` 仍指向上一版可用狀態，並回報 `activation_failed`。可用 `undo` 追加修復 revision。

## 驗證與範圍

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
.\scripts\shader-smoke-test.ps1
```

原生驗證以 PowerShell 7 執行。測試包含內容包載入、WGSL 診斷、運行中 shader 替換、故意觸發 GPU 管線錯誤後保留舊畫面，以及 GLB 蒙皮網格套用同一材質介面。會對截圖取樣像素驗證顏色，並重跑 FPS、ADV、RTS 與多相機場景。報告與截圖位於 `artifacts/shader-validation.json` 所列的目錄。

新增的 [Effects Atlas](effects-atlas.zh-TW.md) 提供程序化三角網格、可選 `vertex_offset` 函式、additive／multiply 混合、GPU 粒子，以及場景顏色／深度後處理。`vertex_offset(p:vec3<f32>, n:vec3<f32>, uv:vec2<f32>, time:f32, params:vec4<f32>) -> vec3<f32>` 回傳本地頂點位置；未提供時保持原有幾何。`axiom_local_light(position, normal)` 可取得局部光源的貢獻(編寫的 point / spot / area 燈光與 VFX 光源)。

GPU 粒子使用引擎內建 compute 管線；自訂材質介面仍不開放任意 compute 或資源 binding。材質節點圖與外部引擎 shader 原始碼匯入尚未支援。自訂程式仍有 16 KiB、16 個使用者函式與 2048 個運算式的上限，且禁止迴圈。透明與網格陰影的上述限制仍適用。
