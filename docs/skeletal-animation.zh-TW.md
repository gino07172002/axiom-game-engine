# 骨架動畫與跨工具匯入

`axiom-animation` 負責骨架、蒙皮、動畫、映射與約束。`animation.evaluate` 用 120 Hz 固定步進取樣，依序執行參考姿勢 retargeting、姿勢混合、骨骼覆寫、twist 校正與 IK。渲染取得已變形的頂點和不可變網格資料，支援索引三角形、base color、貼圖、alpha mask 與基本混合。CPU 蒙皮預設使用線性混合，對 PMX SDEF／QDEF 頂點依來源模式處理，也可選擇 dual quaternion。非剛體蒙皮矩陣會回退到線性混合。

雙擊 `Play Animation.cmd` 開啟展示。有本機指定模型測試包時會使用該包，初次執行也可載入內建 Rigify 範例。`Imported clip` 播放匯入動畫，`Wave` 播放測試動作，`Blend` 切換短混合，`Pose 0.5s` 暫停在指定姿勢。`Twist ON/OFF` 切換前臂校正，`IK ON/OFF` 切換左手目標。內建範例依骨架的可用約束顯示按鈕。

```powershell
. .\scripts\env.ps1
cargo build --workspace
$model = 'D:\path\to\model.pmx'
.\target\debug\axiom.exe animation-inspect $model --bone 左ひじ --bone 左手首 --profile-output data/humanoid.json
.\target\debug\axiom.exe animation-demo $model --source content/animation/rigify.glb --output data/animation-reference/pack
.\target\debug\axiom.exe --db data/animation-preview.sqlite --pack data/animation-reference/pack play --start
```

## Rigify 與 Unity Humanoid

在 Blender 選取 Rigify armature 和蒙皮網格，將動作透過 glTF 匯出為 GLB。啟用動畫取樣與 deformation bones，確認 IK／FK、控制器與動畫 Actions 在 Blender 中已產生預期姿勢。Axiom 讀取烘焙後的 TRS、inverse bind matrices、蒙皮權重和必要的父節點。`.blend` 內的控制器、drivers、B-Bone 行為與約束由 Blender 求值。參考 [Rigify 骨骼類別](https://developer.blender.org/docs/features/animation/rigify/rig_class/) 與 [glTF 2.0 規格](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)。

Unity 將 `tools/unity/Editor/AxiomHumanoidExport.cs` 放進專案 `Assets/Editor`，由 **Tools → Axiom → Export Humanoid** 選擇具有效 Humanoid Avatar 的 Animator 和選用的 AnimationClip。`.axrig` v1 保存已求值的骨骼 TRS、mesh、base color、骨位映射和貼圖相對路徑。Axiom 直接使用 Unity 的 `HumanBodyBones` 對應，包括手指等選用角色。Unity Animator 負責 Avatar 與 muscle 動畫的求值。若場景單位不是公尺，匯出時調整 `Meters per scene unit`。參考 [Unity Humanoid Avatar 設定](https://docs.unity3d.com/6000.0/Documentation/Manual/ConfiguringtheAvatar.html)。

`animation-inspect` 可列出映射、缺失／歧義和骨骼分頁。Rigify `DEF-`、Unity Humanoid、常見 Mixamo 和 MMD 骨名可自動建議映射，結果仍可透過 `profile.mapping` 明確覆寫。映射需通過唯一性與肢體階層驗證。`profile.reference` 可為個別骨骼指定同語意的參考姿勢，用來校準 T／A pose 差異。Retarget 保留目標骨長，使用雙肩方向對齊模型朝向，再傳遞相對參考姿勢的旋轉。可選 `root_motion` 會依腿長比例傳遞 Hips 平移。PMX 平行腿骨符合階層及參考位置時，自動映射至對應的變形鏈。

```powershell
.\target\debug\axiom.exe animation-retarget content/animation/rigify.glb $model --clip rigify_wave --fps 30 --output data/retargeted.clip.json
```

烘焙輸出可加入 `animation.library.clips`。輸出省略與目標 rest pose 相同的軌道，常數軌道只保存一個 key。可選 `--source-profile`／`--target-profile` 讀取明確的映射與參考姿勢。執行期也能在 `animation.player.retarget` 指定來源資產與來源／目標 profile，免除先烘焙目標 clip 的步驟。

## 座標與約束

模型空間統一為右手、Y 向上、公尺。`coordinates.basis` 三列代表來源 X、Y、Z 在 Axiom 的軸向量，可指定正交旋轉或反射。`meters_per_unit` 控制比例。位置、骨架矩陣、動畫平移、四元數、法線、inverse binds 與反射下的三角形繞序會一致轉換。一般 GLB 使用 glTF 原生座標，Blender 匯出到 glTF 時已轉換。下表適用於輸入仍使用來源座標的資料。

| 來源 | basis | meters_per_unit |
|---|---|---:|
| Axiom／glTF | `[[1,0,0],[0,1,0],[0,0,1]]` | 1 |
| Blender Z-up | `[[1,0,0],[0,0,-1],[0,1,0]]` | 1 |
| Unity／MMD | `[[1,0,0],[0,1,0],[0,0,-1]]` | Unity 1，PMX 預設 0.08 |
| Unreal | `[[0,0,-1],[1,0,0],[0,1,0]]` | 0.01 |

Twist 使用 driver 參考姿勢下的本地單位軸，將 swing 與 twist 分解。`driver_fraction` 和每根 helper 的 `fraction` 介於 0 到 1，`min_angle`／`max_angle` 以弧度限制。helper 的比例相對原始角度計算，父子 helper 不累加。`preserve` 保留手掌等指定骨骼的模型空間朝向。`animation.player.twists` 以穩定 ID 命名，適合透過 `set_field` 修改。

IK 使用 `root`／`middle`／`end`、模型空間 `target`、控制彎曲平面的 `pole`、`weight` 及可選的 `end_rotation`。超出長度時會縮至可達範圍，回報 `reachable` 和以公尺表示的 `residual`。雙骨解算要求參與骨骼使用正的均勻縮放。允許肢體鏈中間有 twist 骨。

```powershell
.\target\debug\axiom.exe discover animation
.\target\debug\axiom.exe schema animation.player
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /animation/players/character
```

在 loopback 或 scenario 中使用 `RuntimeCommand::Call`。`inspect` 按穩定骨 ID 回傳局部 TRS 和模型空間位置。`play`、`seek`、`twist`、`ik` 和 `bone` 支援同一命令合約，寫入可用現有 session receipt 重試。

```json
{"op":"command","command":{"kind":"call","capability":"animation","action":"ik","args":{"entity":"character","id":"left_hand","target":[0.2634,1.1043,0.1479],"pole":[0.077,0.5745,0.9907],"weight":1.0}}}
```

## 實測與範圍

指定 PMX 有 459 根骨、19,857 個頂點、29,711 個三角形、18 個材質與 8 張 base color 貼圖。以 Blender 4.5.3 LTS 生成並烘焙的 Rigify 波動作測試。21 個關節位置樣本的最大誤差約 0.00053 mm。重定向後頂點最大位移 0.290 m，前臂 twist 開關的頂點最大差異 0.0385 m。左手 IK 在測試姿勢的殘差低於 0.001 mm。原始 PMX 與貼圖由本機路徑引用，測試內容包不內含該模型。

目前 PMX 支援加權骨架與基本材質。PMX 表情 morph、布料／頭髮物理、toon／sphere shader、PMX 控制器以及 append constraints 由原工具處理或烘焙。超過四個權重、需要額外擴充的壓縮幾何、以及含 shear 的局部骨變換會拒絕匯入或需要事先烘焙。原生動畫播放狀態(片段、時間、速度與漸變、淡入、骨骼覆寫、IK 目標、twist 開關、控制器狀態)存進 checkpoint 的 `systems`。此項目完成 M3 的骨架動畫底座，能力時序與 Action 原型另屬後續工作。

```powershell
. .\scripts\env.ps1
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --examples
cargo build --workspace
.\scripts\animation-smoke-test.ps1 -Model $model
```

`scripts/blender-rigify-fixture.py` 可重新產生內建 Rigify 測試資產與 Blender 比對資料。`scripts/unity-humanoid-smoke.ps1` 接受 Unity Editor 與含 Humanoid `Walking` clip 的 FBX，使用臨時專案驗證匯出。附 `-Model` 可接著跑到 PMX 的重定向、腳部蒙皮和原生渲染。Unity 6000.6.0f1 的 Walking 已驗證實際 Avatar 求值、150 條來源軌道和 PMX 腳部位移。兩條驗證路徑分別記錄在 `artifacts/animation-validation.json` 與 `artifacts/animation-unity-validation.json`。

## 動畫控制器(animation.controllers)

狀態機式的動畫控制器,讓遊戲只設定參數,由控制器決定播哪個片段。控制器宣告在 `animation.controllers` 資源(可多個實體共用),實體用 `animation.controller {"controller":ID}` 掛上(同一實體必須有 `animation.player`;控制器存在時 `animation.play` 會被拒絕,FSM / 動作的片段 cue 會被計數在 `ignored_cues`,不會默默丟掉)。

- **參數**:`float`、`bool`、`trigger`(第一個用到它的轉移觸發時自動清除)。`float` 可設 `"source":"speed"`:每個固定步從實體 transform 的平面位移量測速度(m/s),所以角色控制器、FSM 移動、path.follow、tween 都能直接驅動,不需要寫程式。
- **狀態**:`motion` 是單一片段(`{"kind":"clip","clip":ID,"speed"?,"looping"?}`)、1D 混合樹(`{"kind":"blend1d","parameter":P,"points":[{"at":V,"clip":ID}…]}`,參數兩側的兩個點線性混合,片段依正規化時間同步,例如 walk / run 步伐對齊),或省略 = rest pose。
- **轉移**:依列表順序檢查,第一個通過的觸發(`from` 可為 `*`)。`conditions` 全部成立(`gt/lt/eq/ne` 比 float、`true/false` 比 bool、`trigger`),可加 `exit_time`(來源狀態正規化時間,1 = 一個循環),`duration` 是交叉淡入秒數。淡入期間不打斷(下一個轉移等淡入結束)。
- **執行期**:`animation.evaluate` 每個 120 Hz 固定步先更新綁定參數,再推進目前狀態(與淡出中的前一狀態)並檢查轉移,最後把最多 4 個加權片段取樣混成姿勢。控制器狀態是固定大小的 `Copy` 值、姿勢緩衝區重複使用,步進不配置記憶體(`tests/animator.rs::steady_controller_steps_do_not_allocate` 用計數配置器量到 240 步 0 次配置;retarget、twist、IK、蒙皮輸出仍沿用既有路徑,會配置)。
- **命令**:`animation.param {entity,name,value}`(UI 按鈕、`runtime.bindings`、scenario、loopback 都走同一個 `RuntimeCommand::Call`);`animation.inspect` 與 `observe` 的 `metrics.animation.players.ID.controller` 回報目前狀態、淡出狀態、淡入比例、參數與觸發次數。
- **確定性與存檔**:控制器狀態在 `state_hash` 內(observe metrics),同一 scenario 重播雜湊一致。Checkpoint 的 `systems.animation.evaluate` 保存每個播放器的片段 / 時間 / 速度與漸變 / 淡入起始姿勢 / 骨骼覆寫 / IK 目標 / twist 開關、控制器的目前與淡出狀態(以名稱)、參數、觸發次數與片段完成集合;讀檔後重新求值,與未存檔的執行逐步相同(`checkpoints_round_trip_controller_and_clip_playback`)。舊格式存檔缺少這段時明確拒絕讀取,狀態或參數名稱對不上內容也會拒絕。
- **編輯中熱重載**:修改控制器時,同名狀態與參數保留(例如 Play 中在 Graphs ▸ Animator 改轉移),狀態被刪掉時回到初始狀態。

```powershell
.\target\debug\axiom.exe --db data/animator-lab.sqlite --pack content/animator-lab play --start
.\target\debug\axiom.exe --db data/animator-lab.sqlite --pack content/animator-lab call animation.param '{"entity":"walker","name":"wave","value":true}' --ticks 120
```

ACL:`animctl C create|remove`、`animctl C state add|remove|rename|set ..`、`animctl C initial S`、`animctl C transition add|remove|move|set ..`、`animctl C param set NAME JSON | param remove NAME`、`animctl C node S x,y`、`animctl SEL attach C`。示範內容包 `content/animator-lab` 只用 mascot 已提交的第一方片段(idle / walk / run / wave),角色沿 path 行走,UI 按鈕改 path 速度,控制器自動在 idle、walk↔run 混合與 wave 之間切換。

## 片段事件、播放速度與音效標記

- **片段標記**(`animation.library.markers`):`{"CLIP":[{"at":秒,"event"?:E,"cue"?:C}]}`。播放(或控制器目前狀態的主要片段:單一片段、或 1D 混合中權重最大的點)越過標記時,`event` 送進 `behavior.program`(事件若宣告文字欄位 `entity` / `clip` 會自動填入),`cue` 走既有的 `audio.events` 播放 `audio.library` 的音效。循環片段每一圈觸發一次(每步最多 8 圈),一次性片段停在結尾。未宣告的事件、不存在的音效、缺少 `behavior.execute` / `audio.events` 系統都在建置時明確報錯。ACL:`marker CLIP add SECONDS [event=E] [cue=C] | marker CLIP remove I | marker CLIP move I SECONDS`;編輯器 Animation 分頁的 `anim.marker add [SECONDS] [event=E] [cue=C]`(省略秒數 = 播放頭)。
- **片段結束**:`animation.player.on_end` 事件在片段完成時送出(一次性片段到尾、循環片段每圈)。FSM 新增條件 `{"kind":"clip_finished"}`:agent 的一次性片段(或控制器的一次性狀態)播完才成立,所以 `clip_loop:false` 的狀態可以自動接下一段;控制器內則用 `exit_time` 轉移。
- **播放速度**:`animation.speed {entity, speed, ramp_seconds?}` 改變播放速率但不重置時間,`ramp_seconds` 內線性漸變;控制器狀態可設 `speed_parameter`(float 參數乘上播放速率,0 = 暫停但保留時間)。
- 標記與結束事件只在觸發時配置記憶體;穩定步進仍是 0 次配置。`observe` 的 `metrics.animation.markers_fired`(有觸發才出現)與 `emit_fault` 回報送出狀況。

## 表情(morph target)、部位顯示與動畫圖層

**glTF morph target。** 匯入時讀取每個 mesh 的 morph target(位置與法線位移;名稱取 `extras.targetNames`,沒有就用索引;預設權重取 node / mesh 的 `weights`)。同名 target 在第二個使用它的節點加上節點 ID 前綴。位移以稀疏、以頂點為主的表格儲存(`offsets` + `deltas`),CPU 蒙皮在變形每個頂點前加上權重 × 位移:不需要另一份緩衝,權重全為 0 時完全不做。上限:每個 mesh 64 個、每個資產 128 個 target、1,000,000 個非零位移;超過時匯入失敗並回傳型別化錯誤 `axiom_animation::morph::MorphBudget {what, limit, requested, at}`。PMX 的表情 morph 仍不支援。

- **權重動畫軌**:glTF `weights` 通道變成片段的 `morphs`(每個 target 一條軌,linear / step / cubic spline 都保留);`animation.library.clips` 的片段也可以寫 `"morphs":[{"target":T,"times":[..],"values":[..]}]`。片段播放、控制器的混合與淡入、圖層都會混合權重。片段動到資產沒有的 target 時明確報錯(不默默丟掉);函式庫片段只套用到 bone 與 target 都存在的播放器。retarget 時來源片段的 morph 軌以 target 名稱綁到畫出的資產,對方沒有的軌不套用並在 `inspect` 的 `unbound_morph_tracks` 回報數量;`animation-retarget` 只烘焙骨骼軌。
- **執行期命令** `animation.morph {entity, target | index, weight, ramp_seconds?}` 覆寫一個 target 的權重(-2..2,通常 0..1),可線性漸變,不改變播放時間;`{"clear":true}` 立即交回片段或預設值(不能與漸變同用)。`inspect` 與 `observe` 的 `players.ID.morphs` 回報目前權重(只有有 target 的播放器才有這個欄位,其他世界的 `state_hash` 不變)。
- **存檔**:覆寫值與漸變(以 target 名稱)、淡入起點的權重都存在 checkpoint;讀檔後逐步相同。

**部位顯示(換裝)。** 每個 primitive 可以用材質 ID(`node/mesh/primitive`)或 glTF 的 mesh、材質、節點名稱選取(一個名稱可選多個 primitive)。`animation.player.hidden: [PART..]` 是編寫的預設(最多 64 個),`animation.visibility {entity, group, visible}` 在執行期切換;不存在的名稱在建置 / 命令時報錯並列出可用的部位。隱藏的 primitive 不畫也不投影(`MeshFrame.hidden` 位元遮罩,材質上限 256)。`animation.inspect` 回傳 `parts:[{id,groups,visible}]`。執行期狀態與編寫值不同時存進 checkpoint。放在動畫系統(不是 render 命令)的原因:primitive 屬於播放器的資產、狀態要進 checkpoint 與 `state_hash`,render 端沒有模擬狀態。編輯:ACL `meshpart SEL hide|show PART`、編輯器 `object.mesh_part hide|show PART`(同一個 `ops::mesh_part`)。

**動畫圖層(A5)。** 控制器可以有最多 4 個 `layers`,依序套在狀態機的姿勢之上:

```json
{"name":"wave_arm","clip":"wave","mode":"additive","weight":1,"weight_parameter":"wave_weight","mask":{"subtrees":["upper.R"]},"speed":1,"looping":true}
```

- `override`:遮罩內的骨骼依權重混向圖層片段;`additive`:加上片段相對於第 0 秒的差(旋轉 `ref⁻¹·sample` 乘在原旋轉之後、位移差、縮放比),乘以權重。
- 權重 = `weight` ×(`weight_parameter` 的 float 參數,夾在 0..1),所以 `animation.param` 就能淡入淡出圖層。
- `mask.bones` 只選列出的骨骼、`mask.subtrees` 選整個子樹,兩者都空 = 全身;骨骼名稱在 runtime 對片段取樣的骨架解析(retarget 時是來源骨架),不存在時建置失敗。
- 每層有自己的時間(固定步、確定性,`State.layers`);熱重載以名稱保留,checkpoint 以名稱存讀,名稱或數量不符時拒絕讀檔。只有片段,沒有圖層內的混合樹或狀態機。
- 編輯:ACL `animctl C layer set NAME JSON | layer remove NAME`、編輯器 `graph.layer NAME JSON|remove` 與 Graphs ▸ Animator 的 LAYERS 表(片段、模式、權重、遮罩摘要、新增 / 移除)。

示範內容包 `content/character-lab`:引擎產生的 `morph-head.glb`(`crates/axiom-animation/tests/morph.rs` 的產生器輸出,測試確認檔案與產生器一致)兩顆頭(一顆播放權重片段 `pulse`、一顆預設隱藏帽子並由按鈕漸變 `puff`)以及 mascot 的 idle 加上右臂的 additive wave 圖層(按鈕調 `wave_weight`、隱藏頭髮)。

```powershell
.\target\debug\axiom.exe --db data/character-lab.sqlite --pack content/character-lab play --start
```

## 次級運動:彈簧骨、IK socket、look-at(W2b)

三者都在 `animation.evaluate` 的 120 Hz 固定步內、IK 之後、蒙皮之前執行,順序:片段 / 控制器 → retarget → 淡入 → 骨骼覆寫 → twist → 雙骨 IK → **look-at** → **彈簧骨** → 蒙皮。純計算在 `axiom-animation/src/rig.rs`,runtime 整合在 `axiom-runtime/src/animation/rig.rs`。

- **彈簧骨 `animation.spring`**(元件,需同一實體有 `animation.player`):`chains:{ID:{root, tip?, stiffness, drag, gravity, radius, colliders?, enabled}}`、`colliders:{ID:{bone, offset, tail?, radius}}`(沒有 `tail` = 球,有 = 膠囊,都在骨骼局部座標)、`reset_distance`、`debug`。鏈從 `root` 沿唯一子骨往下(遇到葉骨或分岔停止),或到 `tip`;每根骨(除了尾端)被轉向,讓它的子骨跟著一個模擬點。模擬點在世界座標(實體移動、轉身會甩動),每步:慣性 `(現在 − 上一步) × (1 − drag)`、往動畫方向拉回 `stiffness × dt`、`gravity × dt`(VRM 式單位:每秒位移,不是加速度),再約束回骨長並推出碰撞體(半徑 = 碰撞體 + 鏈的 `radius`)。碰撞體讀「彈簧之前」的姿勢。上限:每實體 8 條鏈、每鏈 32 根骨、合計 64 段、16 個碰撞體;鏈不能共用或巢狀(一條鏈的骨是另一條鏈 root 的祖先)— 建置時明確錯誤並指出 `/entities/@ID/components/animation.spring`。
- **重置規則**:第一次求值從實體目前位置的動畫姿勢開始(世界座標,沒有速度);模擬點離動畫位置超過 `reset_distance`(預設 1 m,傳送 / 場景切換)時,該點回到動畫位置、速度歸零;熱重載(編輯中修改世界)時 `animation.spring` / `animation.look_at` 內容沒變(只切換 `debug` 也算沒變)就延續模擬,改了就從動畫姿勢重新開始;只有旋轉、移動距離小於 `reset_distance` 的瞬間轉身不會重置(會甩動);暫停播放(`playing:false`)時彈簧照常模擬。
- **IK socket**:`animation.player.ik.ID.socket = {entity, bone?, offset?}`,目標改成那個實體的骨骼(沒寫 `bone` = 實體原點)加上骨骼局部 `offset`;`target` 只是第一次解析前的備用值。`animation.ik` 命令可給 `target`(固定目標,同時解除 socket)或 `socket`(綁定;實體或骨骼不存在時拒絕)。目標實體停用時手停在上一次的位置。
- **look-at `animation.look_at`**:`chains:{ID:{bones:[{bone, weight}], target, forward, up, max_yaw, max_pitch, weight, smoothing, enabled}}`。`bones` 由父到子(例如 neck 0.35、head 0.65,權重合計 ≤ 1,每根必須是前一根的子孫);以最後一根的 `forward`(靜止姿勢的模型座標,預設 +Z)量 yaw(繞它的 up)與 pitch(繞它的 right),相對動畫姿勢的朝向夾住在 `max_yaw` / `max_pitch`(度)內,乘上 `weight`,再以 `smoothing` 秒的指數平滑逼近;整個轉動(先繞 right 抬頭、再繞 up 轉頭,不產生 roll)依累積權重以 slerp 分給每根骨,權重合計 1 時整條鏈的總轉動精確等於目標。ACL `bones=neck:0.4,head:0.6`;全部不寫權重 = 平均分配;只有 `:` 後面是數字才算權重(`mixamorig:Head` 是骨名)。`target`:`{kind:camera}`(第一個啟用的 active 相機)、`{kind:entity, entity, bone?, offset?}`、`{kind:point, position}`(世界座標)。目標消失(實體停用)時平滑回到動畫姿勢。眼睛這種兄弟骨請各自做一條鏈(每實體最多 4 條、每條 4 根骨)。yaw / pitch 從最後一根骨量一次再分給各骨,脖子轉動造成的頭部位移不另外修正(近似);目標跨過正後方時 yaw 會從 +max_yaw 跳到 −max_yaw(平滑後頭會繞遠路轉回)。實體縮放為 0 等退化 transform 時目標視為不存在。
- **跨實體順序(確定性)**:每個固定步開始時,先對**所有**播放器解析目標(IK socket、look-at 的實體 / 骨骼 / 相機),再依序推進。目標實體的 transform 用這一步的值,骨骼姿勢用**上一步結束時**的姿勢,所以骨骼目標固定落後一步(1/120 s),結果與實體求值順序無關(`tests/rig.rs::the_hand_follows_the_other_puppets_hand_one_step_late` 精確驗證)。
- **確定性與存檔**:彈簧鏈尾端點、look-at 角度與 socket 解析點在 `metrics.animation.players.ID.rig`(只有用到時才出現,所以既有世界的 `state_hash` 不變),同一 scenario 重播雜湊一致。Checkpoint 的 `systems.animation.evaluate.players.ID.rig` 保存每段模擬點(現在 / 上一步)、look-at 角度與目標、socket 解析點與實體矩陣,讀檔後逐步相同;內容有 rig 但存檔沒有(舊存檔)、或鏈不一致時明確拒絕。
- **配置**:彈簧與 look-at 用重複使用的緩衝區,穩定步進 0 次配置(`tests/rig.rs::springs_and_look_at_do_not_allocate_while_stepping` 用計數配置器量 240 步);IK 解算本身仍沿用既有會配置的路徑。
- **Debug 繪製**:元件 `debug:true` 時場景畫出彈簧關節(青)、碰撞體(洋紅,球 / 膠囊兩端各一個方塊)、look-at 的面向射線與目標(黃);Play 與遊戲畫面都看得到。
- **編寫**:ACL `rig SEL spring|collider|look ID key=value..`、`rig SEL spring|collider|look ID remove`、`rig SEL ik ID socket=ENTITY[:BONE] [offset=x,y,z] | socket=none`、`rig SEL debug on|off`(`ops::rig_edit`);編輯器 `rig.edit` 取同樣的參數作用在選取上;Inspector 的 `animation.spring` / `animation.look_at` 表單由登錄的 schema 產生。

```powershell
.\target\debug\axiom.exe --db data/rig-lab.sqlite --pack content/rig-lab play --start
.\target\debug\axiom.exe --db data/rig-lab.sqlite acl -e "rig puppet/a spring ponytail stiffness=3 drag=0.2"
```

示範內容包 `content/rig-lab`:`scripts/build-rig-lab.py`(只用 Python 標準函式庫,輸出固定)產生的程序化人偶 `puppet.glb`(馬尾 6 根骨、雙骨手臂、脖子與頭)。A 左右擺動,馬尾甩動並撞到頭與背的碰撞體,頭看著相機;B 的右手 IK 綁在 A 的左手(牽手),頭看著 A 的頭。
