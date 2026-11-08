# M0 實作與驗收

M0 交付 World IR v2、能力／組件／系統註冊、通用固定步進、typed columns、generation IDs、內容包、v1 遷移、呈現快照、多相機、preview、啟用診斷與第一個情境 runner。

## 可執行驗收

| 驗收點 | 證據 |
|---|---|
| 核心不要求 FPS | content/empty；World::empty；core-only runtime 測試 |
| 同一引擎支援非 FPS 呈現 | content/two-cameras；透視／正交 viewport；motion.integrate |
| FPS 成為可載入內容包 | content/fps/manifest.json 與 world.json；原生能力獨立於核心 |
| 原 FPS 可玩 | 原碰撞、射擊遮擋、裝填、死亡、狀態機與重現性測試；ActionMap 輸入 scenario |
| 舊資料保留 | v1 SQLite fixture 遷移測試，原 rev 1 原文保留，undo 輸出 v2 |
| AI 可安全修改 | typed edits、CAS、preview、結構化 diagnostics；即時 bridge apply／observe／undo |
| 更新不破壞當前執行 | prepare／transfer／swap；失敗保留前一版；requested_rev 與 rev 分開 |
| 狀態與引用 | 邏輯 ID 遷移、runtime generation、typed column 保存與 reference relink 測試 |
| 可重複驗收 | scripts/m0-smoke-test.ps1；artifacts/m0-validation.json；同執行檔 SHA256 比較 |

執行 README 中的 fmt、tests、clippy、build 及 M0 smoke。Smoke 使用獨立測試資料庫，不清除或重設使用者的預設世界；所有 GPU 測試視窗都隱藏並自動結束。

## 資料的責任

World IR v2 的 entities／components 是通用世界來源；FPS 的玩法設定位於命名空間組件及資源，不能成為核心必填欄位。fps.defaults 提供新增敵人的數值與舊配方 metadata，實際敵人數量由實體集合決定。

FPS 靜態幾何是 render.mesh parts；角色與武器模型是 fps.visuals 資料。`axiom-fps/examples/export_pack.rs` 是基準配方匯出工具，會重建 content/fps/world.json；它只用於維護基準資料，正常遊戲編輯不需執行，也不應覆寫使用者修改過的包。

Renderer 只消費 RenderFrame，沒有對 FPS Simulation 的依賴。通用 UI 顯示世界狀態與控制台；FPS 的選單、HUD、姿勢與特定控制保留在註冊模組，尚未泛化為通用 UI／動畫 IR。

## 後續接點

M1 將加入共用事件／條件／動作、敘事圖、UI 資料綁定及存檔。M0 的自訂 schema、typed columns、系統 factories、ActionMap、scenes、diagnostics 與 scenario 提供基礎，不宣稱現階段已能只靠資料製作完整 RTS、ADV 或 Action。

新增能力需要註冊版本、schema／驗證、系統 metadata、runtime factory 與測試；新增原生演算法仍需 Rust 建置。現有內容包的配置、物件、外觀與已支援玩法數值可透過資料更新。
