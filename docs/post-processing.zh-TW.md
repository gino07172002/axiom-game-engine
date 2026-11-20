# 後處理:FXAA、HDR、bloom、tone mapping(`render.environment.post`)

工作線 W3c。全部**預設關閉**:沒有 `post`(或全部關閉)的世界不建立任何目標、不跑任何 pass,畫面與改動前逐像素相同(證據見 `docs/plan-engine-wave3.zh-TW.md` 的「W3c 執行補記」)。設定是世界層級,所有相機共用。

```json
{"post":{"antialiasing":"fxaa",
         "hdr":{"exposure":1.0,"tonemap":"aces",
                "bloom":{"threshold":1.0,"intensity":0.4,"radius":0.7}}}}
```

| 欄位 | 說明 |
|---|---|
| `antialiasing` | `none`(預設)/ `fxaa`。**FXAA 是後處理邊緣濾波**(仿 Lottes FXAA 3.11 quality:亮度邊緣偵測、沿邊 12 步搜尋、次像素混合),作用在最後的低動態範圍顏色上;**不是 MSAA**,也會讓細小貼圖細節稍微變軟。MSAA / TAA 沒有實作,`msaa` 會被拒絕 |
| `hdr` | 選填;有寫 = 場景畫進 16-bit 浮點(`Rgba16Float`)目標,再做曝光 + tone mapping 輸出到 sRGB |
| `hdr.exposure` | 0.05..16,預設 1:tone mapping 之前的線性倍率 |
| `hdr.tonemap` | 只有 `aces`(預設):Stephen Hill 對 ACES RRT + sRGB ODT 的擬合(BakingLab `ACESFitted`)。AgX 沒有實作 |
| `hdr.bloom` | 選填:`threshold` 0..16(預設 1,線性亮度,曝光之前;下方有一半門檻寬的 soft knee)、`intensity` 0..4(預設 0.4)、`radius` 0..1(預設 0.7:每一層較粗 mip 的權重,0 = 只用最細一層的窄光暈,1 = 各層等權最寬) |

驗證是嚴格的:未知欄位、未知列舉值、超出範圍都會讓交易失敗。

## HDR 時畫面會怎麼變

- 內建的光照路徑(方塊 `fs_main`、編寫燈光時的網格與 toon、編寫的天空)原本在 shader 裡套 `x/(x+0.7)` 色調曲線、天空截到 1;HDR 開啟時改成直接輸出線性輻射(`Globals.ambient_ground.w = 1`),由後處理統一 tone map。
- 直接輸出顯示值的路徑(自訂材質 `render.shaders`、沒有編寫燈光時網格的平光、VFX、舊版背景漸層)照原樣輸出,但不再被 8-bit 目標截在 1,而且一樣經過 ACES。所以 ACES 會讓中間調比舊曲線暗一些(0.18 → 約 0.11 線性)、亮部保留更多層次;需要時調 `exposure`。
- 自發光(`render.mesh` 的 `emission`、自訂材質輸出 > 1、加法混合的粒子)超過門檻就會 bloom。Effects Atlas 的 `mat/06-emissive` 預設沒有開 `post`,所以內容包本身仍「不宣稱 HDR bloom」;開了 `post bloom on` 才會發光。
- 編輯器 Unlit 顯示模式在 HDR 下也經過 tone mapper。

## Pass 順序

每個相機 view:世界(天空、方塊、網格、透明)→ VFX → **bloom**(prefilter:13-tap 降採樣 + Karis 平均 + soft-knee 門檻;降採樣鏈;tent 3×3 加法升採樣鏈)→ **tone map**(加上 bloom × intensity ÷ 各層權重和、乘曝光、ACES)→ 有 `postfx` 體積時照舊跑 postfx(讀這個 view 的深度)。所有 view 之後 → **FXAA**(整個畫面)→ 遊戲介面 / 編輯器介面(不經過 FXAA 或 tone map)。

- bloom 與 tone map 以 view 的矩形為 viewport,取樣夾在 view 內,不會光暈到別的相機畫面。
- bloom 層數:從半解析度開始,較短邊 ≥ 8 px 時再減半,最多 6 層(1920×1080 = 6 層)。

## 預算與資源

- 本模組 5 條 pipeline(prefilter、downsample、upsample、tone map、FXAA)。
- HDR 開啟時內建世界 pipeline 需要 `Rgba16Float` 版本:方塊 + 兩種天空 3 條、網格 4 + 描邊 1、VFX 6 條,共 14 條;自訂材質的程式以(HDR 與否, WGSL)為快取鍵,每個程式仍是 16 條,總數仍受 `shader_gpu` 的 512 條上限(LRU)限制。
- **這些 pipeline 都在背景執行緒建立(`hdr_build.rs`),不在主執行緒**:設定第一次需要它們時開一個 job(`jobs::Job`:一次只有一個;每建好一組 pipeline 檢查取消;進度 done/total;30 秒 watchdog;關閉程式時限時 join)。建好之前畫面照舊走沒有後處理的輸出(不會出現黑畫面或只做一半的畫面),全部建好的那一幀一次切換。建置中設定又改變 → 目前的 job 在下一組 pipeline 後停止(已建好的保留),再用最新的設定開下一個 job(只取最新,不排隊);把後處理關掉也會取消。逾時或失敗 → `render_post.build.state = failed` 與 `error`、編輯器狀態列紅字,畫面維持舊輸出,直到設定改變才重試。
- 沒有改的部分:HDR 已經開著時新增或修改材質,啟用 revision 的流程本來就在主執行緒同步編譯材質 pipeline(為了在啟用前抓 WGSL / pipeline 錯誤),這時編的是 HDR 版本,成本與原本編 surface 版本相同。若有材質在 HDR 版本還沒建好時被用到,畫面暫時走沒有後處理的輸出,背景 job 補建。
- 無頭 `--smoke-frames N --screenshot` 只有在**截圖那一幀**會等建置完成(最多 watchdog 時間),所以截圖一定是要求的輸出;互動視窗的幀從不等待。
- 目標在第一次需要時建立,只有在畫面大小或需要的組合改變時才重建(`target_builds` 計數),全部關閉時釋放。1920×1080:HDR 目標 15.8 MiB、bloom 6 層約 5.3 MiB、FXAA 輸入 7.9 MiB。
- 每幀不配置記憶體:每個 view 的 uniform 寫進預先配置的緩衝。

## 編輯與觀察

| 編輯器 | 無頭(ACL) |
|---|---|
| World 視窗 ▸ **POST** 列:Anti-aliasing None / FXAA、HDR + ACES、Bloom 勾選、Exposure 與 Bloom threshold / intensity / radius 滑桿(放開時寫入一次);指令 `world.post SETTING VALUE` | `post antialiasing none\|fxaa`、`post hdr on\|off`、`post bloom on\|off`(on 會一起開 HDR)、`post exposure V`(需要 HDR)、`post threshold\|intensity\|radius V`(需要 bloom)、`post none`;細調也可 `set resources[id=render.environment] post/hdr/exposure V` |

兩邊都是 `ops::set_post`(沒有 `render.environment` 時建立並補 `render` capability;全部關掉時移除 `post`)。

`observe` 在有設定開啟時多一個 `render_post`:`antialiasing`、`hdr`(含 bloom 數值)、`target`、`target_bytes`、`bloom_mips`、`target_builds`、`pipeline_ms`(背景建置花的毫秒)、`gpu_frames`、`gpu_ms {bloom, tonemap, fxaa}`(量到的幀的平均 GPU 毫秒),以及 `build {state: ready|building|failed, error, job, builds, worker_ms, request_frame_ms, install_frame_ms, before_frame_ms, fallback_frames}`(`*_frame_ms` = `Renderer::render` 在主執行緒的毫秒,不含等 swapchain 的時間)。建置中或失敗時,即使還沒有生效的設定也會回報 `render_post.build`。沒有編輯器 Profiler 時,只有在後處理設定開啟時才寫 GPU timestamp;編輯器 Profiler 多了 bloom / tonemap / fxaa 三欄。

## 量測

見 `docs/plan-engine-wave3.zh-TW.md`「W3c 執行補記」。

## 後續(沒有做)

- **MSAA 4×**:每條顏色 pipeline 要 `sample_count = 4` 的版本(內建 14 條 + 每個自訂材質程式 16 條)、多重取樣顏色與深度目標與 resolve;VFX 軟粒子用 `textureLoad(scene_depth)` 要改成 multisampled 深度;postfx 讀深度需要自己寫深度 resolve pass(wgpu 不能 resolve 深度)。改動面積遠大於 FXAA,所以先做 FXAA。
- TAA、AgX、LUT、景深、SSAO、每個相機各自的設定、HDR 螢幕輸出。
