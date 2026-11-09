# 最終驗收:Axiom Street(黃昏街口)

> 建立:2026-11-09。plan-engine-components 第 9 項。內容包 `content/street`,啟動器 `Play Street.cmd`。
> 驗收標準(使用者原意):
> 1. 人在編輯器做得到的事,AI 不開編輯器也做得到(對等)。
> 2. 只用編輯器已有的功能做一個 demo:有車、有行人走動的街道;玩家可以和人互動;玩家可以走進一個房間。過程中缺什麼功能,如實記錄。

## 這個 demo 有什麼

- **街道**:100 m 長的東西向車道、雙向車道線、兩側人行道與路緣、斑馬線、停止線;5 棟建築(`nav.blocker` + 網格)、公車站、3 棵樹(prefab);8 盞路燈(prefab 燈柱 + 子物件 spot 燈);黃昏的低角度夕陽(directional,有陰影)、三段漸層環境光與霧。
- **紅綠燈**:`signal` 根物件帶 `tween.clips`(green / yellow / red 三個 once 片段,各 8 條軌:兩支燈柱的 6 個燈泡自發光 + 2 盞 point 光的顏色)。片段的 `on_finish` 事件接 `behavior.program` 規則的 `delay`,形成 6 s 綠 → 2 s 黃 → 6 s 紅的循環。
- **車**:3 輛車(prefab,各自有車頭 spot 燈、引擎聲 `audio.source` 循環)沿封閉路徑 `road/loop` 以 8 m/s 繞行(東行走南車道、迴轉、西行走北車道)。每個方向的停止線前有每輛車各一個觸發區;紅燈時進入觸發區 → 規則 → `path.pause`,轉綠燈時 `path.resume`。
- **行人**:5 位行人(prefab)沿兩條人行道迴圈路徑走(不同速度、側向 offset);1 位行人是 `nav.agent`,每次轉紅燈時走斑馬線到對面。
- **互動**:等公車的阿明(`interact.target`,E 交談,三層對話)、咖啡廳裡的咖啡師小雨(點一杯拿鐵,HUD 顯示「手上:一杯熱拿鐵」)。
- **房間**:咖啡廳(四面牆 `nav.blocker`、門楣與屋頂用 `physics.collider` 擋相機、吧台與兩張桌子、兩盞暖色 point 燈)。門:鉸鏈物件帶 tween 片段 `open`(yaw 0 → 95°,1.2 s,ease_out)、`interact.target`(E 開門 / 關門,切換)與吱呀聲 `audio.source`。門關著時走進門口會被觸發區送回門外並顯示「門關著。先按 E 開門。」;門內的觸發區 `cafe/inside`:HUD 改成「你進入了咖啡廳」、開始播咖啡廳背景音、相機拉近成 5 m / 30°(被屋頂擋住時會再拉近)。
- **玩家**:左鍵點地面走路的 `nav.agent`(`nav.grid.pointer_agent`),`camera.follow` 跟隨相機(Z / C 環繞、R / F 縮放,撞牆 / 屋頂立即拉近)。聽者在玩家身上。
- **UI**:`ui.document` 精簡外框(`chrome: compact`):左上標題、位置、手上物品;上方互動提示(`prompt` 綁定)與門關著的警示;下方對話(講者、台詞、選項)。

內容包共 85 個實體、5 個 prefab(tree、lamp-post、car、person、cafe-table);世界 JSON 沒有手寫,全部由下節的腳本產生。

## 第一部分:對等(AI 不開編輯器也做得到)

### 建置方式:整個世界由編寫協定建出來

`content/street/world.json` 不是手寫的。它是下列腳本經由公開協定重播後匯出的結果:

| 檔案 | 協定 | 內容 |
|---|---|---|
| `content/street/build/00-seed.apply.json` | 型別化 `apply` 請求(**非編輯器路徑,見 GAP-3**) | 世界 id / title、capability(physics、narrative、ui、nav、behavior)、`behavior.execute` 系統,以及**空殼**資源 `behavior.program`、`render.environment`、`nav.grid`、`narrative.graph`、`ui.document`(內容由後面的 ACL 填) |
| `build/10-street.acl` | ACL(1 個 revision) | 地面、車道、人行道、斑馬線、夕陽與環境光、5 棟建築、公園與公車站、8 盞路燈 |
| `build/20-traffic.acl` | ACL | 車 prefab ×3、車道路徑與跟隨、車頭燈、引擎聲、紅綠燈與三個片段、6 個停止線觸發區 |
| `build/30-people.acl` | ACL | 行人 prefab ×5、兩條人行道路徑、過馬路的導航代理、阿明 |
| `build/40-cafe.acl` | ACL | 咖啡廳的牆、門楣、屋頂、地板、吧台、桌子、燈、門(tween / 互動 / 聲音)、門口與室內觸發區、背景音、咖啡師 |
| `build/50-player.acl` | ACL | 玩家、點擊移動、輸入軸、聽者、跟隨相機 |
| `build/60-logic.acl` | ACL | 規則、`runtime.bindings`、對話、HUD |

`scripts/build-street.ps1` 從 `content/empty` 建一個全新的資料庫(在 `target/street-build/`,不碰使用者的世界),依序送出上表,`verify`,再 `export` 成 `content/street/world.json` 並產生 `manifest.json`。ACL 共 293 個敘述(set 141、create 65、rename 38、instantiate 20、audio 7、tween 7、prefab 5、interact 3、follow 3、camera 2、nav 2),每個檔案恰好一個 revision(rev 3..8)。

重現性測試 `crates/axiom-runtime/tests/street.rs::street_pack_is_reproducible_from_its_build_script`:在程序內用同一套協定(`Store::execute` 的 `apply` / `acl` 請求)重播腳本,檢查每段恰好 +1 revision、最後 `verify` 通過、結果和 `content/street/world.json` 逐欄相同(不同時列出前 12 個 JSON pointer),並且內容包能載入、編譯。

### 每個用到的 ACL 動詞 ↔ 編輯器操作

| ACL(腳本裡的用法) | 編輯器裡同一件事 | 備註 |
|---|---|---|
| `create cube\|empty\|camera\|obstacle\|path P` | Object 選單 / Hierarchy「+ Object」(`object.cube` 等) | 同一個 `ops::create_object` |
| `create light directional\|spot\|point P` | `object.light_*` | |
| `create trigger box P` | `object.trigger_box` | |
| `create … parent=X at=…` | 右鍵 ▸ Create Child(之後在 Inspector 調位置) | |
| `rename ID "中文"` | F2 / 右鍵 ▸ Rename | |
| `set ID render.mesh/parts [...]`、`set ID nav.blocker/half […]`、`set ID render.light/…`、`set ID trigger.zone/…`、`set ID path.follow/…`、`set ID path.spline/points […]` | Inspector 元件表單或 JSON 模式(欄位層級 `set_field`,兩邊同一種 edit) | 多個物件同一欄位(`set entities[id=a,id=b] …`、`set lamp/*/light …`)在編輯器只能逐一設(GAP-10) |
| `set ID core.transform/rotation [x,y,z,w]` | Inspector Transform(Rotation°)或旋轉 gizmo | ACL 只收四元數(GAP-11) |
| `set ID physics.collider {…}` | Inspector ▸ Add component… ▸ physics.collider | 世界要先有 physics capability(GAP-3) |
| `set resources[id=R] FIELD V` | Resource inspector(JSON pointer + Apply) | 資源必須已存在(GAP-3);陣列只能整個替換(GAP-8) |
| `set ID /tags [...]` | **沒有**(GAP-1) | 4 行,腳本裡標了 `# GAP-1` |
| `prefab create P from ID` | 右鍵 ▸ Create Prefab From Selection | |
| `instantiate P id= at= [parent=]` | Assets ▸ 點 prefab(之後改 ID、位置) | |
| `set ID render.mesh/parts/1/color …`(prefab 實例覆寫) | Inspector 改實例的欄位 | |
| `follow SEL PATH speed=` | Object ▸ Follow Path(多個跟隨者平均分散) | |
| `nav SEL agent speed= radius=` | Object ▸ Add Nav Agent + Inspector | |
| `interact SEL event= prompt= range=` | Object ▸ Make Interactable + Inspector | 編輯器沒有選項對話框,事件名稱改了不會自動宣告(GAP-9、GAP-14) |
| `audio clip ID PRESET` | Inspector ▸ Audio source ▸ New clip ▸ 預設 | 編輯器的片段 ID = 預設名稱(`fountain`),不能取 `cafe_hum`(GAP-14) |
| `audio SEL source clip= loop= autoplay= min= max= volume=` | Object ▸ Add Audio Source + Inspector | |
| `audio ID listener` | Object ▸ Make Audio Listener | |
| `tween SEL clip NAME property= target= duration= autoplay= on_finish= play_on=` | Object ▸ Add Tween Clip + 時間軸(Add track、模式、Autoplay)+ Inspector(on_finish) | `play_on` 在編輯器沒有對應 UI(GAP-14) |
| `tween ID key CLIP T move\|remove\|add` | 時間軸拖曳 key / Delete key / Record key | |
| `camera main ID` | Object ▸ Set as Main Camera | |
| `camera CAM follow TARGET distance= …` | Object ▸ Camera Follow Selected + Inspector | |

### 用真的 GPU 編輯器做同樣的步驟:結果逐位元相同

`scripts/street-editor-parity.ps1`:用 `build-street.ps1` 建兩份相同的街道資料庫(rev 8),一份開**真正的 GPU 編輯器**(`axiom editor`,bridge 連線),用 `editor_view {select, command}` 執行選單指令(和人點選單是同一個指令註冊表),另一份不開編輯器、用 ACL 做同樣的 8 步,最後兩邊都 `export` 比對。

| # | 編輯器(選取 → 指令) | ACL |
|---|---|---|
| 1 | — → `object.cube` | `create cube` |
| 2 | `cube` → `object.interactable` | `interact cube` |
| 3 | `cube` → `object.audio_source` | `audio cube source` |
| 4 | `cube` → `object.tween_clip` | `tween cube clip clip` |
| 5 | — → `object.obstacle` | `create obstacle` |
| 6 | — → `object.trigger_box` | `create trigger box` |
| 7 | `camera`, `ped/n1` → `object.camera_follow` | `camera camera follow ped/n1` |
| 8 | `ped/s1` → `edit.duplicate` | `duplicate ped/s1` |

結果(debug 建置,2026-11-09):兩邊都是 rev 8 → 16,匯出的世界 167,045 bytes **逐位元相同**(`identical: true`;同日稍早改內容前也跑過一次,同樣相同)。編輯器每步從送出到新 revision 上線 352–1042 ms(含本機每幀 200–475 ms 的 present 阻塞)。過程中也驗到一致的拒絕:對 `obstacle`(有 `nav.blocker`)做 Add Tween Clip,編輯器回 `tween_conflict`,和 ACL 相同。

其餘所有會改內容的編輯器功能已有單元層級的對等測試(`crates/axiom-runtime/src/editor/parity.rs`,見 `docs/editor-parity.zh-TW.md`)。

## 第二部分:缺口報告

嚴重度:**阻斷**(照規則做不到,只能繞過規則)/ **權宜**(做得到,但要繞路)/ **打磨**(不順手、容易錯)。負責里程碑對照 `docs/plan-editor-human-tuning.md` 的 M3–M10,或引擎待辦(`docs/plan-engine-components.zh-TW.md` 次優先)。

### 對等方向的結論

- **AI → 編輯器做得到的,AI 都做得到**:這次用到的每一個編輯器操作都有 ACL 對應,實測 8 步逐位元相同(上節)。沒有發現「只有編輯器做得到」的內容操作。
- **反方向不成立**:有幾件事 AI(ACL / 型別化 `apply`)做得到、人在編輯器裡**做不到**(GAP-1、GAP-3、GAP-14 的 `play_on`)。對使用者的第 1 條標準這不算違反,但代表「人只用編輯器從零做出這個 demo」目前不可能 —— 至少要從含有這些資源的範本開始,而且玩家的 `player` tag 沒辦法在編輯器加上。

### 缺口清單

| # | 缺口 | 嚴重度 | 這次怎麼繞 / 證據 | 建議歸屬 |
|---|---|---|---|---|
| GAP-1 | **編輯器不能編輯實體的 tags**。Inspector 只有 label、transform、元件、parent / 場景;沒有 tags 欄位。可是 `interact.target`、`trigger.zone` 的預設 filter 都是 tag `player`,`camera_blocker` 也是 tag | **阻斷**(對人):在編輯器裡無法讓互動 / 觸發區認得玩家 | ACL 用原始 pointer `set hero /tags ["player"]`、`set car/a /tags [...]`(腳本 4 行,標 `# GAP-1`)。相機要擋的門楣 / 屋頂改用 `physics.collider` 而不是 tag | 編輯器 M3(Inspector 2.0) |
| GAP-2 | **鍵盤操控的角色(`action.actor`)和防穿牆相機不能同時用**:`action.session` 擁有自己的相機(必填、每步寫 transform),加 `camera.follow` 是 `camera_conflict`;action 相機沒有碰撞 | 權宜 | 玩家改成「左鍵點地面」的 `nav.agent`(`nav.grid.pointer_agent`)+ `camera.follow`。代價:沒有 WASD、沒有跑步 / 閃避動畫;axiom-town 的英雄與 glb 沒有沿用 | 引擎待辦:action 控制器改讀 `camera.follow` 的 yaw、session 相機改選填(見 camera-follow 文件末節) |
| GAP-3 | **編輯器與 ACL 都不能「新建」資源、開 capability / 系統,也不能改世界 id / title**。Resource inspector 只能編輯已存在的資源;Add component 只列出世界已有 capability 的元件(所以 `physics.collider` 在沒有 physics 的世界加不了);ACL `set` 要選擇器先比對到物件。`narrative.graph`、`ui.document`、`nav.grid`、`render.environment` 沒有任何 op 會建立 | **阻斷**(對人從零開始)/ 權宜(AI) | `00-seed.apply.json`:型別化 `apply`(`set /capabilities`、`set title`、`set_systems`、5 個 `set_resource` 空殼),其餘內容都由 ACL `set resources[id=…]` 填。`narrative.graph` 空殼還必須有 `behavior.program` + `behavior.execute` 與 catalog 鍵,否則 `apply` 失敗(`gameplay resources require behavior.program…`、`missing localization key`) | 編輯器 M5(Assets:建立 UI 文件、輸入表等資產)+ 新的「World settings」面板;ACL 新動詞 `resource create ID` / `capability add` |
| GAP-4 | **`runtime.bindings` 的參數是固定值**,不能引用事件 payload(例如觸發區的 `entity`) | 權宜 | 一個方向一個觸發區不夠:每輛車每個方向各一個觸發區(6 個,filter 用每台車獨有的 tag)、3 個事件、3 條規則、3 個 `pause_car_*` 綁定。車數 × 方向成長 | 引擎待辦:binding 參數模板(例如 `{"$event":"entity"}`) |
| GAP-5 | **沒有計時器**;`loop` 補間片段不發事件 | 權宜 | 紅綠燈:三個 `once` 片段(0.25 s)的 `on_finish` → 規則 `delay 720/240/720 ticks` → 下一個片段。規則裡的時間要換算成 tick(120 Hz) | 引擎待辦「計時器」(次優先清單) |
| GAP-6 | **會動的門不能擋路,也不擋相機**:被 tween 的子樹不能有 `nav.blocker` / `physics.collider`(`tween_conflict`);導航格子在建立時固定 | 權宜 | 門洞永遠可走。門口放觸發區 `cafe/doorway`:門關著時規則發 `door_blocked` → 綁定 `nav.go hero` 送回門外、HUD 顯示「門關著」(scenario 驗證:被送回 z ≥ −8.1、location 仍是街道)。門扇不擋相機,相機可能看穿關著的門 | 引擎待辦:導航動態障礙(增量格子)+ 相機的移動阻擋物 |
| GAP-7 | **prefab 不能含子物件**,`duplicate` 也不複製子物件 | 權宜 | 8 盞路燈的 spot 燈、3 輛車的車頭燈都是逐一 `create light … parent=` 再用萬用字元選擇器設定 | 編輯器 M5 / 引擎:巢狀 prefab、子樹複製 |
| GAP-8 | **ACL `set` 與 Resource inspector 不能在陣列尾端新增元素**(`field index out of range`),只能整個替換 | 打磨(容易出錯) | `runtime.bindings.events` 整個重寫(13 個綁定、一行約 2.6 KB),**蓋掉了** `tween … play_on=door_opening` 自動加的那一筆,必須自己記得放回去;規則表同樣整個寫 | ACL `append` / `bind EVENT …` 動詞;編輯器 M6(規則表格編輯) |
| GAP-9 | **在 Inspector / `set` 改事件名稱不會宣告事件**(只有 ops 會:interact、tween on_finish、play_on) | 打磨 | 腳本裡 16 行 `set resources[id=behavior.program] events/NAME {"fields":{}}`;漏一行會讓整個 revision 失敗(錯誤訊息有指出事件) | 編輯器 M3 / M6:事件欄位用下拉 + 「新事件」 |
| GAP-10 | **Inspector 不能多物件同時編輯** | 打磨 | ACL `set entities[id=a,id=b] …`、`set lamp/*/light …` 一行改多個;在編輯器要逐一改(腳本裡有 32 行是多目標 `set`) | 編輯器 M3(多物件編輯) |
| GAP-11 | **ACL 的旋轉只收四元數**,Inspector 用歐拉角(度) | 打磨 | 夕陽方向、車頭燈俯角、北側路燈的 180° 都手算四元數寫進腳本 | ACL:`set ID core.transform/rotation_deg [x,y,z]` 或 `rotate` 動詞(`ops::rotation_from_degrees` 已存在) |
| GAP-12 | **不同移動系統之間沒有碰撞 / 避讓**:路徑跟隨者(車、行人)不避開導航代理(玩家、過馬路的人),導航代理只避開導航代理;車不會為行人停 | 權宜(視覺) | 車只在紅燈停;行人只在紅燈時過馬路(規則)。行人會穿過站在人行道上的玩家 | 引擎待辦:碰撞層與遮罩、路徑跟隨的前方偵測 |
| GAP-13 | **基本形狀角色沒有走路動畫**,`nav.agent` / `path.follow` 也不會切換 `animation.player` 的片段 | 打磨 | 人物是方塊組合,平移滑行 | 引擎待辦:動畫狀態機 / 混合樹;或 tween 子物件擺動(需要巢狀 prefab,GAP-7) |
| GAP-14 | **編輯器的建立指令沒有選項**:Make Interactable(event / prompt / range)、Add Tween Clip(property / target / duration / on_finish / **play_on**)、Add Audio Source(clip / loop)、New clip(ID = 預設名稱) | 打磨;`play_on` 在編輯器完全沒有 UI | ACL 一行帶選項;編輯器要先建預設值再到 Inspector / 時間軸 / Resource inspector 改。編輯器建的音效片段 ID 只能是 `fountain` 之類 | 編輯器 M3 / M6 |
| GAP-15 | **`ContentPackage::load` 會補型別預設值**(`ui.document` 的 widget `when`、`images`),所以 import 後 export 和原檔逐字不同 | 打磨 | 重現性測試直接比對匯出的檔案(不經 `load` 正規化),另外檢查內容包能載入、編譯 | 核心:匯出 / 載入的正規化一致 |
| GAP-16 | **scenario 沒有「等到條件成立」**,只能寫固定 tick | 打磨 | 斷言時間要試出來(過馬路的行人在 1500 tick 還沒到,改成 1700);路線一改就要重調 | scenario 新增 `wait_until` 或區間斷言;編輯器 M8(錄製) |
| GAP-17 | **存檔不支援**:trigger / path / nav / tween / audio / camera 系統的 checkpoint 都會明確失敗 | 已知限制 | 街道不提供存檔;`ui.document.session_controls: false` 隱藏 SAVE / LOAD 按鈕 | 引擎待辦:各系統的 checkpoint 區段 |
| GAP-18 | **精簡外框(`chrome: compact`)與敘事的 session 按鈕重疊**:`session_controls` 預設 true 時,右上角的 EN / 中文 / SAVE / LOAD 列被 PAUSE / RESTART / CONSOLE / EXIT 蓋住,兩組字疊在一起 | 打磨(畫面 bug) | 第一次 GPU 截圖看到後,改成 `session_controls: false`(GAP-17 本來就不能存檔) | `ui.rs` `draw_compact`:把 session 按鈕併進同一個 pill |
| GAP-19 | **這台筆電上互動遊玩是慢動作**:每幀 swapchain 取得阻塞約 260 ms(MX150 / Optimus,見 perf-baseline),主迴圈每幀最多推進 0.1 s 模擬時間,所以實際遊玩約 0.4 倍速 | 效能(環境) | smoke / 截圖用 `--smoke-frames`(每幀固定 1/60 s,確定性);主執行緒 CPU 工作在預算內(見下方效能) | 既有待辦(perf-baseline 剩餘熱點 2:present mode / frame latency / 後端) |
| GAP-20 | **聚光燈沒有陰影**(spot / point 陰影未支援) | 打磨 | 路燈只照亮、不投影;只有夕陽有陰影 | 引擎待辦(lighting 文件) |
| GAP-21 | **互動距離以樞紐點計算** | 打磨 | 門的互動放在鉸鏈(不動的樞紐點),不放在會轉的門扇 | 文件已說明;可考慮包圍盒距離 |
| GAP-22 | **每步模擬仍有記憶體配置**:street 穩定狀態每步 18 次、最多 158 次(release `perf_bench`) | 效能(打磨) | 時間預算內(p99 < 0.03 ms),但違反「熱路徑不配置」的目標;來源未拆解 | 引擎待辦:事件 / 規則 / 綁定路徑零配置 |
| GAP-23 | **跟隨相機會被身後的小物件拉到很近**:公車站、樹在玩家身後時相機貼著它們(第一張截圖 `hit: park/tree-1`,actual 4.85 / desired 7) | 打磨(畫面) | 截圖腳本對話時用 `camera.follow.set yaw=200`(等於玩家按 Z / C)轉到街道那一側 | 引擎待辦:遮擋物淡出 / 只擋大型阻擋物的選項(camera-follow 限制已列) |

沒有發現需要改引擎程式才能完成的 bug(GAP-18 以內容設定繞過,未改 `ui.rs`)。

## 第三部分:怎麼執行

### 玩

雙擊 **`Play Street.cmd`**(debug 建置;第一次會先 `cargo build -p axiom`)。它用自己的資料庫 `data\street.sqlite`(第一次從 `content\street` 建立),不碰預設世界與存檔。內容包更新後要看到新版,刪掉 `data\street.sqlite` 再開。

| 操作 | 效果 |
|---|---|
| 滑鼠左鍵點地面 | 玩家走過去(導航會繞過建築、路燈、桌子;過馬路請點斑馬線) |
| Z / C | 相機左右環繞 |
| R / F | 相機拉近 / 拉遠(0.5–12 m) |
| E | 和聚焦的人說話 / 開門、關門(畫面上方會顯示「[E] …」提示) |
| 點對話選項 | 選擇回答 |

建議路線:先找公車站旁的阿明(出生點左後方)聊天 → 點斑馬線走到對面 → 走到粉紅招牌的咖啡廳門口按 E 開門 → 走進去(HUD 變成「你進入了咖啡廳」,背景音開始,相機拉近)→ 在吧台前按 E 和小雨點一杯拿鐵。紅燈時注意車會停在白線前,穿白衣的行人會走斑馬線。

### 驗證(都會建立自己的資料庫,不碰 `data\`)

```powershell
. .\scripts\env.ps1
# 重建內容包(腳本 → 協定 → 匯出),再跑測試
powershell -NoProfile -File scripts\build-street.ps1
cargo test -p axiom-runtime --test street
# 無頭 scenario(CLI)
target\debug\axiom.exe --db $env:TEMP\street.sqlite --pack content\street scenario examples\street.scenario.json
# 真 GPU 編輯器 ↔ ACL 對等 + 編輯器 gizmo 截圖
powershell -NoProfile -File scripts\street-editor-parity.ps1
# 真遊戲視窗(release)四張截圖:street / talk / door / room
cargo build --release -p axiom
powershell -NoProfile -File scripts\street-smoke-test.ps1
```

### 效能(量測,不是估計)

**無頭,release**,`scripts/perf-bench.ps1 -Packs content/street,content/axiom-town -Repeats 3`(交錯 3 次,1200 步,中位數 [範圍]),i5-8265U 筆電:

| 項目 | street(85 實體 / 181 實例) | axiom-town(對照) |
|---|---|---|
| step p50 / p95 / p99 / max | 0.009 / 0.010 / 0.016 [0.012–0.027] / 0.046 [0.043–0.051] ms | 0.018 / 0.019 / 0.024 / 0.056 ms |
| step 配置 p50 / max | **18 / 158** | 69 / 82 |
| `render_frame`(CPU 端擷取)p50 / p95 | 0.008 / 0.009 ms,29 次配置、56.9 KiB | 1.30 / 1.54 ms(骨架網格) |
| observation p50 | 0.042 ms,583 次配置 | 0.035 ms |
| 編輯器主執行緒每次 commit p50 / p95 | 1.26 / 1.88 ms(背景寫入器) | 1.30 / 1.69 ms |
| 編輯 → 上線 p50 / p95 | 144 / 180 ms(SQLite commit 137 ms;不在主執行緒) | 137 / 245 ms |
| 背景建置執行期 p50 | 6.8 ms(其中導航格子 385×193 建置約 4–6 ms) | 9.6 ms |
| working set | 16.6 MiB | 19.3 MiB |

- 每步模擬遠低於 16 ms 預算(p99 < 0.03 ms);主執行緒上的編輯器工作每次 ≤ 2.2 ms。
- **每步仍有配置**:穩定狀態每步 18 次(town 69 次),最多 158 次(推測是發事件的那幾步;可能的來源:`dispatch_bindings` 對每個事件複製呼叫清單、gameplay 規則與 trace —— **未拆解,未證實**)。列為 GAP-22(引擎待辦:事件 / 綁定路徑零配置)。

**真 GPU 遊戲(release,MX150 / Vulkan,1805×950)**:四個獨立程序(`scripts/street-smoke-test.ps1`,`play --smoke-frames N --screenshot`;smoke 模式每幀固定推進 1/60 s,所以結果確定),2026-11-09,報告 `artifacts/street-20261109-125548/report.json`:

| 截圖 | 幀數 / 模擬時間 | 牆上時間 | 慢幀(> 16 ms) | 最後一個慢幀的分項 |
|---|---|---|---|---|
| street | 150 / 2.5 s | 30.9 s | 108 | render_present 508 ms;simulate、observation、UI 合計 < 0.5 ms |
| talk | 300 / 5 s | 59.9 s | 236 | render_present 484 ms |
| door | 820 / 13.7 s | 154.3 s | 628 | render_present 262 ms;simulate 0.06 ms |
| room | 1150 / 19.2 s | 235.4 s | 907 | render_present 253 ms;simulate 0.05 ms、observation 0.33 ms |

- **主執行緒上的 CPU 工作每幀都在 16 ms 預算內**(simulate ≤ 0.08 ms、UI ≤ 0.14 ms、observation ≤ 0.33 ms、bridge ≤ 0.02 ms);慢幀全部來自 `render_present`(swapchain 取得),這台 MX150 / Optimus 筆電在每個建置都是每幀 200–500 ms(perf-baseline「剩餘熱點 2」)。互動遊玩因此是慢動作(GAP-19)。GPU 計時與 VRAM 在 Vulkan 上取不到(`gpu_memory.available: false`),沒有宣稱。
- 程序記憶體(release,整個程序含 GPU 驅動與 egui):street 截圖結束時 working set 369 MiB、peak 376 MiB、private 485 MiB;room 361 / 369 / 486 MiB。無頭模擬本身 16.6 MiB(上表),差距是 GPU / 視窗端,未拆解。

### 截圖(實際看過)

`artifacts/street-20261109-125548/`(release,1805×950):

- `street.png`:黃昏的街道。左右是公寓、書店、辦公大樓,遠處是掛粉紅招牌的咖啡廳(窗戶發光、門關著)。車道上一輛黃色計程車開著車頭燈、左邊露出紅色轎車的車尾;斑馬線、停止線、中央黃線;紅綠燈只有綠燈亮(紅、黃是暗的)。人行道上有 3 位行人,中央是橘衣黃帽的玩家;左下角很近的是背對鏡頭的阿明和公車站的藍色廣告板 —— 相機被身後的樹擋住而拉近(`hit: park/tree-1`,4.85 / 7 m,GAP-23)。左上 HUD「AXIOM STREET · 黃昏街口」「位置:黃昏街口」,下方說明列,右上精簡外框只有 PAUSE / RESTART / CONSOLE / EXIT。
- `talk.png`:相機轉到街道那一側(`yaw 200`),玩家站在公車站前的阿明(綠衣)旁邊,背景是公車站、樹與老屋。上方提示「[E] 和阿明說話」,下方對話框「阿明 · 等公車的人 傍晚好!這條街的紅綠燈剛換新的,車子都會乖乖停在白線前。」與三個選項「附近有咖啡廳嗎?」「車子真的會等紅燈?」「再見」。畫面有透視造成的傾斜感(FOV 75°、相機低)。
- `door.png`:玩家站在咖啡廳門口,門已打開(門扇往內轉 95°,貼在左側門框邊,不太看得出來),門洞裡看得到室內的暖色地板與牆。上方提示「[E] 開門 / 關門」。門楣上的粉紅招牌、左右兩扇發光的窗。
- `room.png`:相機在室內、屋頂下(被屋頂擋住,actual 3.7 / desired 5 m,`hit: cafe/roof`),看得到兩側牆、後牆、吧台、吧台後的咖啡師、前景兩張桌子的桌面;沒有牆在相機前面。HUD「你進入了咖啡廳」,提示「[E] 和咖啡師小雨說話」。室內顏色偏亮、偏黃(兩盞暖色 point 光加上環境光),玩家的橘衣看起來接近黃色。

編輯器(`scripts/street-editor-parity.ps1` 擷取,debug):

- `artifacts/captures/street-editor-crossing.png`:選取「紅綠燈」聚焦斑馬線。可見停止線觸發區的青色線框(TRIG)、車道路徑(PATH)與跟隨者到起點的線(FOLLOW)、對等步驟建立的方塊上的聲源球(SND)與互動圈(USE)、移動 gizmo;Inspector 的 Tween timeline 顯示 green / red / yellow 三個片段、once、Autoplay、8 條軌;Hierarchy 有中文標籤與 MESH / FOLLOW / SND / PFB / LIGHT / PATH / TWEEN 標記;標題列「Axiom Street / 黃昏街口」,REV 16。導航代理(NAV)與跟隨相機(RIG)的 gizmo 不在這個視角內。
- `artifacts/captures/street-editor-gizmos.png`:選取「咖啡廳大門」聚焦門口,可見門的 USE 圈、吱呀聲的 SND 球、門口 / 室內觸發區線框、Tween timeline(`open`,yaw 軌兩個 key)。
