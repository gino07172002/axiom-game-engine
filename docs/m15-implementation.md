# M1.5：AI 操作介面與成本驗證

M1.5 保留 World IR v2、交易式 authoring 與固定步進 runtime。新操作可透過 `axiom rpc FILE`、JSONL serve 或 loopback 使用；原有 FPS／ADV 命令持續支援。

## 局部查詢與穩定 ID

`select` 的 query.from 支援 entities、resources、prefabs、scenes、nodes、choices、rules、widgets、variables；ids 精確選取，entities 另支援 tags／components 的 AND 篩選，choices 可用 parent 指定 node ID。

```json
{"op":"select","query":{"from":"choices","parent":"workshop","ids":["open"],"fields":["id","when","next"],"max_bytes":2048}}
```

預設只回傳物件摘要，不展開 mesh／整份資源。明確 fields 才回傳選定欄位；路徑支援相對 JSON pointer，空字串明確要求整個物件。missing_ids／missing_fields 顯式指出未取得的資料。預設 16 筆／8192 bytes；最大 128 筆／65536 bytes。單一物件超額會拒絕，沒有靜默截斷欄位。next_cursor 綁定 revision、篩選與投影；世界更動後需重新查詢。

`set_field`、`put_object`、`remove_object` 的 target 支援 entity／resource／prefab／scene／node／choice／rule／widget／variable。choice 指定 node 與 id；編輯不依賴陣列位置。原有 put_entity／set_component 等操作保留。修改仍需使用 apply／preview 的 expected_rev；批次最終驗證與原子性不變。

## Diff、診斷及探索

Diff 除原有 entity 統計外，包含 total_changes、resources、changes 與 truncated；具 ID 陣列按 ID 比對，重排獨立列出。路徑使用 `@ID` 定位，ID 內的斜線按 JSON pointer 轉義。大型 before／after 只顯示 bytes 與 fingerprint，需用 select 明確取值。預設最多 64 筆差異；提交後可用 `diff FROM_REV TO_REV --offset N --limit N` 分頁查完整差異。

共用行為的變數、事件、動作與敘事引用診斷會指出 stable-ID 路徑，例如 `/resources/narrative.graph/nodes/@workshop/choices/@open/next`，而非只回傳 `/resources`。behavior 查詢涵蓋 behavior.program 和 narrative.graph。

`discover TOPIC --if-hash HASH` 支援 schema 快取，未改變時只回傳 hash／not_modified。schema local 提供 JSON Schema；schema narrative.graph 提供 graph、choice、condition、operand、action、command 的 JSON Schema 定義。

## 增量 observe

`observe-delta`／`op:observe_delta` 支援 cursor、fields、trace_limit、max_bytes。首次回覆 snapshot；後續回覆以 JSON pointer 表達的 set／remove changes，陣列整體替換。未改變時沿用 cursor 並回傳 unchanged。預設不傳 CPU／GPU 時間、FPS、UI 全文和 trace。

歷史最多 64 個投影快照、4 MiB 序列化狀態。過期／外來游標、投影改變、runtime 重開或 checkpoint 恢復，都以 reset 和 reason 明確通知。Trace 有單調序號，最多取 128 筆，缺漏以 trace_gap 通知。這是狀態差異介面；沒有宣稱永久事件日誌或所有瞬間狀態都能回放。

## 冪等重試

```json
{"op":"execute","request_id":"agent-edit-001","request":{"op":"apply","expected_rev":1,"edits":[{"kind":"set_field","target":{"kind":"choice","node":"workshop","id":"open"},"field":"next","value":"rescue"}]}}
```

apply／undo 的 receipt 與修改在同一 SQLite 交易提交；重啟後仍回覆原結果，同 ID 的不同 payload 被拒絕。失敗請求不佔用 ID。Receipt 有明確容量限制，不靜默逐出舊 ID。

command／save／load 必須提供 observe 中的 runtime_session；成功結果在 host session 快取，重複請求回覆原結果。舊 session 在新程序中被拒絕，不宣稱跨程序持久化 gameplay commands。CLI 亦可用 `--request-id ID --runtime-session SESSION` 包裝單次修改。

## 成本 benchmark

```powershell
.\target\debug\axiom.exe benchmark benchmarks/m15.json --output artifacts/m15-benchmark.json
.\target\debug\axiom.exe benchmark benchmarks/m15.json --agent-executable PATH --agent-arg ARG --output artifacts/m15-agent.json
```

沒有 adapter 時，使用固定 legacy／local 計畫，量測協定 request／response bytes、工具次數、失敗次數、時間與獨立驗收成功率。這是協定回放，不是真實 agent token benchmark，model tokens 永遠為 null。

有 adapter 時，每個任務在新程序、新記憶體資料庫與 runtime 執行。stdin 每行提供 protocol、task_id、instruction、turn、上次 feedback；stdout 每行回覆一個 `{request,usage?}` 或 `{done:true,usage?}`。任務 acceptance 與參考計畫不提供給 adapter。最多 64 輪，每輪有逾時與輸出上限；錯誤請求會回饋給 agent 以便修正。只有 done 且獨立 field／scenario 驗收通過才算成功。

usage 格式：provider、model、input_tokens、output_tokens、可選 cached_input_tokens、source。source 為 provider_reported 且所有互動回報完整時，才計算總 tokens；任何缺漏、fixture 或中斷都標示未知。數字是 adapter 回報，核心不宣稱可向供應商獨立核驗。總成本包含失敗嘗試；每個成功交付的平均成本以全部任務成本除以成功數，並並列完成率。

測試用 deterministic adapter 不是真實模型資料。沒有設定實際 agent adapter 前，不宣稱真實 token 節省百分比。重現比較應固定模型、工具、初始內容與 acceptance，並分開報告冷／暖 schema 快取。

v0.4 的固定兩任務協定回放結果（完整資料：`artifacts/m15-benchmark.json`）：

| 計畫 | 驗收成功 | 工具呼叫 | request bytes | response bytes | 真實模型 tokens |
|---|---:|---:|---:|---:|---|
| legacy | 2/2 | 8 | 1090 | 114712 | 未量測 |
| local | 2/2 | 8 | 1402 | 3707 | 未量測 |

這些數字支持「局部操作減少此任務集的傳輸資料量」，尚不能推論真實模型的 token 成本、完成速度或首輪成功率。
