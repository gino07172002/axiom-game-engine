# Make Love, No War

The Velvet Hour 的兩位成年舞者 Valentina 與 Ren 會在同意邀舞後一起跳舞。舞台使用原創的低多邊形晚裝角色、紅絲絨布幕及 Art Deco 霓虹線條。兩個 GLB 的模型、骨架與動作可用 `scripts/blender-duet.py` 重新生成。

雙擊 `Play Love No War.cmd`。按 **E** 邀舞，對方接受後再按 **E** 開始。倒數後，白點進入金色判定窗時按提示的 **J**（擺步）、**K**（旋轉）或 **L**（伸手）。八拍中踩中六拍就會進入雙人收尾。**Backspace** 或 **Step away** 可以離場，**Esc** 暫停。這版以畫面提示節拍。

```powershell
. .\scripts\env.ps1
cargo build -p axiom
.\target\debug\axiom.exe --db data/duet-edit.sqlite --pack content/love-no-war editor
.\target\debug\axiom.exe --db data/duet-qa.sqlite --pack content/love-no-war scenario examples/love-no-war.scenario.json
```

`rhythm.sequence` 的拍速、判定寬度、通過條件、順序及每一步的動畫都可以改。引擎以 120 Hz 固定步進計分，舞台、相機和動畫由內容包提供。一次按住只認一個輸入。節奏資料改動會重新開始一輪，缺失動畫會在啟用時回報穩定路徑並保留上個可用的修訂。

```powershell
.\target\debug\axiom.exe --connect 127.0.0.1:4242 discover rhythm
.\target\debug\axiom.exe --connect 127.0.0.1:4242 observe-delta --field /rhythm --field /rev
```

先用 `select` 投影 `rhythm.sequence` 的 `window_ticks` 與 `moves`。再用 `preview`、`apply` 與 `expected_rev` 修改欄位，例如 `/window_ticks` 或 `/moves/moon/clips/0`。原生按鍵、HUD 按鈕、loopback 和 scenario 共用邀請、開始、踩拍與離場規則。
