# 3D 聲源與聽者(`audio.source` / `audio.listener`)

聲源掛在物件上,依「聽者」的位置與朝向計算距離衰減與左右聲道;聽者預設是啟用中的攝影機。用途:街道 demo 的車聲(車子經過時由小變大再變小、從左聲道移到右聲道)、噴水池的循環環境音、門被補間打開時的吱嘎聲(單次播放)。

執行期系統是 `audio.spatial`(capability `audio`,phase 45:所有移動、補間、相機與觸發區都寫完這一步的姿態之後)。所有計算結果都在 `observe` 的 `audio` 段,**無頭執行(CI、AI)不需要音效裝置就能驗證聽者會聽到什麼**。

## 音訊資源:程序合成,不用音檔

`audio.library` 有兩種內容:

- `cues`:既有的短音提示(`audio.events` 播放,例如動作遊戲的命中音),格式不變。
- `clips`:**程序合成的片段**,給 `audio.source` 用。repo 裡沒有任何第三方音檔;片段是合成參數,不是檔案。

```json
"audio.library":{"clips":{
  "engine":{"wave":"saw","frequency":55,"seconds":2,"gain":0.6,"lowpass_hz":420,"tremolo_hz":6,"tremolo_depth":0.35,"attack":0,"release":0},
  "creak":{"wave":"saw","frequency":170,"frequency_end":95,"seconds":0.9,"jitter":0.35,"lowpass_hz":1400,"tremolo_hz":23,"tremolo_depth":0.6,"attack":0.03,"release":0.25,"seed":3}
}}
```

| 欄位 | 說明 |
|---|---|
| `wave` | `sine`(預設)/ `square` / `saw` / `triangle` / `noise`(白噪音,忽略頻率) |
| `frequency` / `frequency_end` | 20..8000 Hz,預設 220;`frequency_end` 是線性掃頻的終點(選填) |
| `seconds` | 0.01..20,預設 1 |
| `gain` | 0..1,預設 0.5:合成後**依峰值正規化**到這個振幅 |
| `harmonics` | 0..8:加上第 2..k+1 倍正弦泛音(振幅 1/k) |
| `lowpass_hz` | 選填,20..11025:二階(兩個一階串接)低通 |
| `tremolo_hz` / `tremolo_depth` | 音量顫動 0..40 Hz、深度 0..1 |
| `jitter` | 0..1:每 10 ms 換一次的隨機音高抖動(±50% × jitter),吱嘎聲用 |
| `attack` / `release` | 秒,預設 0.005;線性淡入 / 淡出 |
| `seed` | 亂數種子(噪音、抖動),同參數永遠產生同一段 PCM |

預設片段(`audio clip ID PRESET` / 編輯器 New clip):`tone`(440 Hz 正弦 1 秒)、`engine`(引擎低鳴,可循環)、`fountain`(低通噪音 + 慢顫動,可循環)、`creak`(門的吱嘎聲,單次)、`footstep`(短促的低通噪音,單次)。可循環的預設片段都是整數個振盪 / 顫動週期,而且沒有淡入淡出,接縫不會爆音。

**解碼 / 合成不在播放時做**:建立執行期時(背景執行緒上的 revision 啟用流程)把**有被聲源用到的**片段合成成 22,050 Hz 單聲道 16-bit PCM,用定義內容的 hash(FNV-1a,含全文比對)放進全程序共用的快取;熱重載與其他執行期共用同一份 PCM。`audio.play` 只換播放狀態,不做任何解碼。

預算(超過都是明確錯誤,不會默默丟掉):

| 項目 | 上限 | 超過時 |
|---|---|---|
| 片段數 | 64 / library | 資源驗證錯誤 `audio clip budget exceeded` |
| 單一片段 | 20 秒 | `seconds must be 0.01..20` |
| 一個 library 的 PCM 總量 | 8,388,608 樣本(16 MiB) | `audio clip memory budget exceeded (N > M samples …)` |
| 全程序片段快取 | 32 MiB | 先淘汰沒有執行期在用的片段,仍放不下就讓執行期建立失敗:`audio clip cache budget exceeded` |
| 聲源 | 1024 / world | `audio_budget` |
| 同時發聲 voice | 32 | 不是錯誤:多出來的變成 virtual(見下) |

## `audio.source`

```json
"audio.source":{"clip":"engine","loop":true,"autoplay":true,"min_distance":2,"max_distance":25}
```

| 欄位 | 說明 |
|---|---|
| `clip` | 必填,`audio.library.clips` 的 ID。找不到 = 編寫錯誤 `missing_clip`(列出現有片段與加片段的指令) |
| `volume` | 0..4,預設 1 |
| `pitch` | 播放速率 0.25..4,預設 1(同時改變音高) |
| `loop` | 預設 false = 單次播放,播到片段結尾自己停 |
| `autoplay` | 預設 false;true = 第一個固定步開始播放 |
| `spatial` | 預設 true;false = 2D(增益 = volume,置中,不需要 `core.transform`) |
| `min_distance` | 0.01..1000 公尺,預設 1:這個距離以內是全音量 |
| `max_distance` | > min,≤ 10000,預設 30:這個距離(含)以外完全聽不到 |
| `rolloff` | `inverse`(預設)/ `linear`,見下 |
| `priority` | -1000..1000,預設 0:voice 不夠時大的優先 |
| `enabled` | 預設 true;false 的聲源不會播放,`audio.play` 回錯誤 |

世界層級檢查(`World::compile`):有聲源或聽者就必須排入 `audio.spatial`(`invalid_audio`);`spatial` 聲源與聽者必須有 `core.transform`(`invalid_audio`);片段必須存在(`missing_clip`);同時啟用的聽者最多一個(`audio_listener_conflict`)。

## `audio.listener`

```json
"audio.listener":{"forward":"+z"}
```

| 欄位 | 說明 |
|---|---|
| `active` | 預設 true。**同時只能有一個 active 的聽者**,兩個以上是編寫錯誤 `audio_listener_conflict`(不做隱性挑選);`audio ID listener` / Make Audio Listener 會在同一個 revision 把其他聽者設為 false |
| `volume` | 0..4,預設 1:主音量 |
| `forward` | `"-z"`(預設,攝影機的慣例)/ `"+z"`(角色模型的慣例):聽者面向的軸,右耳在它的右手邊 |

**沒有編寫聽者時**,聽者是「編寫順序第一個啟用中(場景啟用)、`render.camera.active` 為 true、而且目前的輸入情境允許的攝影機」(和畫面用的攝影機同一條規則),朝 -Z。聽者實體所在場景停用時也改用攝影機。兩者都沒有時,`spatial` 聲源一律無聲(2D 聲源照常),`observe` 的 `audio.listener` 是 `null` 並附 `listener_note` 說明原因。

## 空間化(每個固定步,在模擬端計算)

距離 d = 聽者世界位置到聲源世界樞紐點(含 parent)的 3D 距離。

- `linear`:d ≤ min 為 1,之後直線降到 max 為 0。
- `inverse`:d ≤ min 為 1,之後 `min / d`;在 `min..max` 的最後 10% 內再線性乘上淡出到 0,所以在 max 剛好結束、不會突然斷掉。d ≥ max 為 0(和 Godot 的 max_distance 一樣是可聽範圍的邊界)。
- 聲像(pan):把相對位置投影到聽者的右方向 x 與前方向 f(忽略高度),pan = x / √(x² + f²)(水平方位角的正弦,正後方 = 置中),在 `min_distance` 以內再依水平距離 / min 往中間收,聲源穿過聽者時不會左右跳。
- 等功率聲像:θ = (pan + 1)·π/4,左 = cos θ、右 = sin θ(左² + 右² = 1,置中時兩邊都是 0.7071)。2D 聲源也走同一條公式(置中 0.7071)。
- 最終:左右增益 = volume × 聽者 volume × 衰減 × 聲像增益。增益 ≤ 1e-4(-80 dB)的聲源算「聽不到」(culled),不佔 voice。

時間軸:`audio.play` / autoplay 開始後的第一個固定步從 0 送出(片段開頭一定會送到音訊執行緒),之後每步前進 `1/120 × pitch` 秒;單次播放到片段長度就停(`finished` +1);循環則取模。實體所在場景停用時暫停計時、不發聲。

## Voice 預算與搶占(確定性)

每步把所有聽得到的聲源排序:`priority` 大者優先 → 距離近者優先 → 編寫順序。前 32 個拿到 voice,其餘是 virtual(繼續計時,之後輪到時從**目前時間**開始發聲)。已經有 voice 而且仍在前 32 名的聲源保留**同一個 voice 槽**(音訊執行緒不必重新開始);掉出前 32 名而仍聽得到的算一次搶占(`steals`)。排序用 `sort_unstable_by` 在預先配置好的陣列上做,**每步不配置記憶體**。

結果是一個固定大小的 `[VoiceParams; 32]`(片段 hash、播放 token、起始時間、pitch、左右增益、是否循環)。

## 音訊輸出(原生視窗)

沿用既有的 WinMM 後端與工作執行緒,沒有新增 crate:原本 `PlaySoundW` 一次播一個 cue,現在同一個工作執行緒(`axiom-audio`)改成 `waveOut` 串流,混音器同時混 32 個空間 voice 與 8 個 cue voice(**行為變更:cue 現在會重疊混音,不再排隊依序播放**;cue 滿了取代最舊的)。格式 22,050 Hz 立體聲 16-bit,4 個 512 frame 的緩衝(約 93 ms 延遲)。

- **主執行緒 → 音訊執行緒**:voice 參數用序列鎖(sequence lock)寫進原子變數,寫入端不等待;讀取端最多重試兩次,失敗就沿用上一份。片段表與 cue 走容量有限的 channel(`try_send`,滿了算 `dropped`,不阻塞)。
- **音訊執行緒不配置、不釋放、不等鎖**:混音器的狀態在建立時配置好;被替換的片段表與播完的 cue 緩衝會送回主執行緒釋放(那個 channel 滿了才在音訊執行緒釋放,計入 `late_frees`)。唯一的等待是裝置的「緩衝播完」事件(最多 50 ms,以便發現關閉要求)。驗證方式:`tests/audio.rs` 用只計算本執行緒的配置計數器,確認 200 次 `Mixer::render`(32 voice 全滿)配置次數為 0;`perf_bench --generated-audio` 用全域計數器量同一件事。channel 的 `try_recv` 是 std 有界 channel(無鎖陣列實作)的非阻塞輪詢。
- 增益每個緩衝內線性內插,換 token 的 voice 先淡出舊聲音一個緩衝再從新的起點開始;停止的 voice 淡出一個緩衝。
- 編輯模式、暫停、主控台開啟時送出全靜音參數(會淡出)。
- 關閉時設停止旗標,最多等 2 秒 join,逾時就放手(不卡住關閉)。
- **沒有裝置**(`waveOutOpen` 失敗)時執行緒結束並在 `observe audio.output.device` 回報 `unavailable (error N)`,模擬照常。無頭指令(`axiom call`、`verify`、scenario、測試)根本不開裝置。

`observe` 的 `audio.output`(只有原生視窗有):`backend`、`device`(`starting` / `open` / `disabled` / `unavailable (error N)`)、`rendered`(混好的緩衝數)、`underruns`(醒來時所有緩衝都已播完)、`mix_us_max`、`voices_mixed`、`clipped_samples`、`played`(開始播放的 cue)、`late_frees`、`write_errors`(`waveOutWrite` 失敗次數,錯誤碼在 `device` 旁的 error)、`dropped`(佇列滿或沒有裝置而沒送出的 cue)。片段表送不出去時下一幀重送;切到沒有 `audio.spatial` 的世界時送一個空表,釋放舊 PCM 讓快取可以回收。

## 動作與觀測(不需要 GPU 或音效裝置)

| 動作 | 參數 | 結果 |
|---|---|---|
| `audio.play` | `id`、`restart`(選填) | 開始播放(單次:播完自停);已在播放時不變,除非 `restart`。停用的聲源回錯誤 |
| `audio.stop` | `id` | 停止並倒回 0;voice 在下一步釋放 |
| `audio.inspect` | `id` | clip、playing、time、length、loop、gain、left、right、pan、distance、voice(null = 沒有 voice)、priority |
| `audio.summary` | — | 同 `observe` 的 `audio` |

可以從 `runtime.bindings` 呼叫,所以 `behavior.program` 規則、互動事件、補間的 `on_finish`、觸發區事件都能播聲音而不用寫 Rust:

```json
"runtime.bindings":{"events":[
  {"event":"open_door","calls":[
    {"kind":"call","capability":"tween","action":"play","args":{"id":"door","clip":"swing"}},
    {"kind":"call","capability":"audio","action":"play","args":{"id":"door"}}]},
  {"event":"door_moved","calls":[{"kind":"call","capability":"audio","action":"play","args":{"id":"latch"}}]}
]}
```

`observe` 的 `audio`:

```json
{"sources":3,"playing":2,"audible":2,"voiced":2,"virtual":0,"culled":0,"max_voices":32,
 "steals":0,"finished":0,"starts":1,
 "listener":{"id":"ear","from":"audio.listener","position":[0,1,0]},
 "voices":[{"voice":0,"id":"car","clip":"engine","gain":0.214,"left":0.04,"right":0.21,"pan":0.69,"distance":4.67}],
 "clips":{"count":2,"bytes":176400,"cache_bytes":176400},"halted":null}
```

有 `audio.events` 時 `emitted` / `recent` 也合併在同一段。

## 熱重載、生成、存檔

- **熱重載**:依實體 ID 對應;片段內容(hash)、`loop`、`enabled` 都沒變時保留播放狀態、時間、voice token 與 **voice 槽**(音訊執行緒繼續播同一個聲音,不重新開始);改了其中之一就照編寫值重新開始(`autoplay` 從 0 播放,否則停止)。改 volume、距離、rolloff、priority 立即生效而不中斷。
- **執行期生成**帶有 `audio.source` / `audio.listener` 的實體:系統停止(`halted` 說明原因),所有 voice 靜音,`audio.play` 回錯誤。請改用編寫的實體。
- **存檔(checkpoint)**:不支援,明確失敗(`system audio.spatial does not implement checkpoints`)。播放狀態屬於執行期,不寫進 revision。

## 編輯器與無頭等價

| 編輯器 | 無頭(ACL) |
|---|---|
| Object ▸ Add Audio Source(`object.audio_source`):選取的物件都加上預設聲源(library 的第一個片段;沒有片段時同一個 revision 建立 `tone`),補 capability 與 `audio.spatial` | `audio SEL source [clip=] [volume=] [loop=on\|off] [autoplay=on\|off] [spatial=on\|off] [min=] [max=] [rolloff=inverse\|linear]` |
| Object ▸ Make Audio Listener(`object.audio_listener`,主選取) | `audio ID listener`(其他聽者在同一個 revision 設為 inactive) |
| Inspector ▸ Audio source ▸ Clip 下拉選單 | `set ID audio.source/clip "CLIP"`(片段不存在 = `missing_clip`) |
| Inspector ▸ Audio source ▸ New clip ▸ 預設片段(名稱 = 預設名,重複時 `engine2`…) | `audio clip ID tone\|engine\|fountain\|creak\|footstep` |
| Inspector ▸ Components ▸ `audio.source` / `audio.listener` 表單 | `set ID audio.source/FIELD V` |

注意:Add Audio Source / Make Audio Listener 會**取代**物件上既有的同名元件(和 Make Interactable 相同),沒給的欄位回到預設值;只想改片段或欄位請用 Clip 下拉選單或表單(`set`)。

Gizmo(受 Gizmos 開關與隱藏旗標控制):`spatial` 聲源畫 `min_distance` 球(亮綠)與 `max_distance` 球(暗綠),各三個世界軸向的圓,標 `SND`(2D 聲源只標 `SND 2D`);聽者畫小圓圈與面向線,標 `EAR`。Hierarchy 標籤 `SND` / `EAR`。Inspector 沒有試聽按鈕(沒做;試聽請進 Play)。

## 已知限制

- 沒有都卜勒、HRTF、殘響、遮蔽;聲像只用水平方位角(上下不分)。
- 只有程序合成片段,不讀 WAV / OGG 檔。
- voice 參數每個畫面送一次:比一個畫面還短的單次聲音(例如 10 ms 片段、或畫面卡頓時)可能在兩次送出之間開始又結束而聽不到(`observe` 照樣記錄 `finished`)。
- 模擬端的播放時間與音訊執行緒的實際位置各自前進,長時間播放會有小幅偏差;只在 voice 重新開始時對齊。
- `waveOutWrite` 失敗的緩衝會在下一次醒來重新混音(聲音會跳一小段),計入 `write_errors`。
- 試聽按鈕沒做。

## 量測

見 `docs/perf-baseline.zh-TW.md` 的「3D 音訊」段與 WORKLOG。
