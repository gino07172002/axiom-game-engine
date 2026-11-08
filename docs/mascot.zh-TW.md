# Axiom 吉祥物模型

`content/mascot` 收錄 Axiom 吉祥物的兩個變體：`axiom-short.glb`（鮑伯短髮，髮尾到下顎）與 `axiom-long.glb`（多層長髮，到大腿）。兩者共用同一套服裝、骨架與動畫，只有頭髮不同。模型完全以程式產生，不含外部貼圖或第三方素材。

## 重新產生

```powershell
.\.tools\blender\blender-4.5.3-windows-x64\blender.exe --background --factory-startup --python scripts/blender-mascot.py -- content/mascot
```

腳本每個變體輸出一行 `MASCOT <variant> triangles=<n> bytes=<n>`。使用 Blender 4.5.3 LTS 與現有 GLB 相同的 glTF 匯出設定（Khronos glTF Blender I/O v4.5.48）。Blender 5.2 也能執行，但提交的檔案以 4.5.3 產生。預覽內容包：

```powershell
.\target\debug\axiom.exe --db $env:TEMP\mascot.sqlite --pack content/mascot play --start
```

## 骨架與動畫合約

骨名與階層沿用 `scripts/blender-duet.py`：`root → hip → chest → head`，`chest → upper.L/R → fore.L/R`，`hip → thigh.L/R → shin.L/R → foot.L/R`。`.L` 是角色左側（Blender +X），角色面向 Blender −Y，也就是 glTF/Axiom 的 +Z。

動畫以 Actions 模式匯出、30 fps 逐格強制取樣，每個 Action 匯出自己的格數範圍（`export_frame_range=False`），一個片段對應一個 glTF animation，`animation.player` 直接以片段名稱播放。前五個片段 `idle`、`sway`、`turn`、`reach`、`finale` 沿用 duet 生成器的姿勢表（mirror = 1），第 1–25 格（時間 0.033–0.833 s），與先前輸出的取樣值逐值相同。其餘是給第三人稱城鎮示範用的日常動作，從第 0 格取樣到片段長度。

| 片段 | 長度 | 類型 | 內容 |
|---|---:|---|---|
| idle / sway / turn / reach / finale | 0.833 s | 循環 | duet 原有片段 |
| `walk` | 1.0 s | 循環 | 原地走路，腿與手臂交替擺動，骨盆／胸口小幅扭轉 |
| `run` | 0.6 s | 循環 | 原地跑步，前傾、手肘彎曲、大幅擺臂 |
| `wave` | 1.6 s | 單次 | 右手舉到頭側揮三下，再回到 idle |
| `bow` | 1.4 s | 單次 | 從髖部前彎約 36°（hip 18°＋chest 12°＋head 6°），雙手垂在大腿前，再回到 idle |
| `sit` | 3.0 s | 循環 | 坐在約 0.32 m 高的長椅上：hip 下降 0.32 m，大腿近水平、小腿直立、雙手放膝上，呼吸起伏 |
| `dance` | 2.0 s | 循環 | 四拍 step-touch，左右輪流點腳、手臂左右擺、每拍小跳（hip 上升最多 0.03 m） |
| `cheer` | 1.2 s | 單次 | 雙拳舉高成 V 字並小跳（hip 上升最多 0.08 m），再回到 idle |
| `think` | 3.0 s | 循環 | 右手托下巴、左手托右肘，頭部緩慢歪斜 |
| `pose` | 2.0 s | 循環 | 拍照姿勢：左手在臉頰旁比 V、右手叉腰、身體微傾並輕輕晃動。手套沒有手指，V 字只靠手的位置表達 |
| `hop` | 0.6 s | 單次 | 原地小跳（hip 上升最多 0.09 m），落地回到原姿勢 |
| `talk` | 2.5 s | 循環 | 說話手勢，左右手輪流比劃、小幅點頭 |

城鎮片段的規則：
- 只有旋轉鍵；`hip` 的平移只用在 `sit`、`hop`、`dance`、`cheer`，而且只有垂直方向。`root` 永遠不動，移動角色由引擎負責實體位置，`walk`/`run` 都是原地動作。
- 單次片段的第一格與最後一格等於 `idle` 第一格的姿勢（`REST`），方便混合；循環片段的最後一格等於第一格。
- 非移動類片段雙腳不離地；只有 `hop`、`cheer`、`dance` 會跳。逐格檢查時，蒙皮網格（含描邊外殼）最低點在 −0.015 m 到 0 m 之間，與原本 `idle`（−0.013 m）相同等級；`run` 支撐腳最低 −0.012 m。
- `sit` 的坐姿相對於實體原點：骨盆維持在原點正上方，腳往前約 0.33 m。放長椅時，椅面前緣約在原點前方 0–0.15 m、椅面高 0.32 m，大腿會略微陷入椅面約 2 cm。

蒙皮規則：
- 身體各部件剛性綁定到單一骨骼；短裙、短褲、腰帶綁 `hip`，軀幹上半與高領綁 `chest`。
- 長髮與側髮依高度在 `head` 與 `chest` 間線性混合（z 1.10 m 以上全 `head`，0.90 m 以下全 `chest`），身體動作時頭髮跟隨。
- 每個頂點最多兩個影響骨，權重已正規化，僅輸出 `JOINTS_0`/`WEIGHTS_0`，沒有 morph target。

## 外觀與材質

角色總高 1.34 m（含耳機鰭片），頭頂約 1.31 m。比例依擁有者的三視圖以正交輪廓疊圖調整。設定圖的側面與背面鏤空一律改為完整布料，短裙下加深色短褲，不提供泳裝變體。

| 材質 | 顏色 | 用途 |
|---|---|---|
| white | #FFFFFF | 白色面板、袖套、手套、耳機外殼與鰭片、膝甲 |
| dark | #2B2B36 | 衣身結構、頭帶、高領、腰帶、腿帶 |
| red | #FF3B47 | 胸前三角標誌、耳機「^」、高領指示燈、袖口條紋；另輸出 emissiveFactor |
| metal | #D8D8E0 | 方形扣具 |
| skin | #FFE2DD | 皮膚 |
| hair | #F4F4F6 | 銀白頭髮 |
| hairshade | #D6D6E2 | 長髮的髮束陰影帶 |
| boot | #16161D | 亮黑過膝靴、百褶裙外層、短褲 |
| eye / pupil / blush | #E0283A / #5A0E1C / #FFB3B8 | 紅色虹膜、瞳孔、腮紅 |
| outline | #1C1C24 | 反向外殼描邊 |

顏色以 baseColorFactor 輸出，沒有貼圖。目前渲染器只用 base color 和漫射光，emissive 在執行期不會發光。描邊使用 inverted hull：在主要部件外推 2–4.5 mm 並翻轉面向，以單面材質繪製。頭髮、袖套與短裙是開放曲面，用雙面材質。

## 預算

| 變體 | 三角形 | GLB 大小 | 材質／primitive |
|---|---:|---:|---:|
| short | 6,760 | 656,816 B | 11 |
| long | 7,696 | 737,488 B | 12 |

上限是每個檔案 8,000 個三角形、1.5 MB，含描邊外殼（約佔三角形的一半）。執行期展開後的頂點數為 short 9,120、long 10,800。要加細節時，先減少 `tube` 的段數或描邊範圍，不要拿掉描邊。
