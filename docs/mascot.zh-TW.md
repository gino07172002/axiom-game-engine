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

動畫片段為 `idle`、`sway`、`turn`、`reach`、`finale`，30 fps、25 格，姿勢表與 duet 生成器相同（mirror = 1），透過 Actions 模式匯出並強制取樣。`animation.player` 直接以片段名稱播放。

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
| short | 6,760 | 519,036 B | 11 |
| long | 7,696 | 599,656 B | 12 |

上限是每個檔案 8,000 個三角形、1.5 MB，含描邊外殼（約佔三角形的一半）。執行期展開後的頂點數為 short 9,120、long 10,800。要加細節時，先減少 `tube` 的段數或描邊範圍，不要拿掉描邊。
