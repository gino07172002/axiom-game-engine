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

目前 PMX 支援加權骨架與基本材質。表情 morph、布料／頭髮物理、toon／sphere shader、PMX 控制器以及 append constraints 由原工具處理或烘焙。glTF morph、超過四個權重、需要額外擴充的壓縮幾何、以及含 shear 的局部骨變換會拒絕匯入或需要事先烘焙。原生動畫播放狀態暫不提供 checkpoint。此項目完成 M3 的骨架動畫底座，能力時序與 Action 原型另屬後續工作。

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
