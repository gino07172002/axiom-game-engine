# AI authoring cost: legacy JSON vs selective JSON vs ACL

Suite: [`benchmarks/ai-tasks.json`](../benchmarks/ai-tasks.json) (5 tasks). Run it with:

```powershell
.\target\debug\axiom.exe benchmark benchmarks/ai-tasks.json --output artifacts/ai-tasks.json
```

Each task is solved three ways with the **same four steps**: read what is needed, preview, apply (CAS-checked), verify. Every run must pass the same acceptance checks. These are field checks on the stored world, and for the gameplay tasks also headless scenarios (all three ADV endings, skirmish victory or defeat).

- **legacy:** whole-scope `query` plus JSON edits by array index or whole component.
- **local:** projected `select` plus stable-ID `set_field`/`set_where` JSON.
- **acl:** the line-based ACL front end (`{"op":"acl","text":"…"}`).

## Result (2026-11-08, engine 0.4.0, protocol replay)

All 15 runs succeeded (success rate 1.0 in every mode, 0 failed requests, 4 tool calls per task).

| Task | legacy | local | ACL | ACL / legacy | ACL / local |
|---|---:|---:|---:|---:|---:|
| fps-health: set one entity's health | 106,858 B | 1,851 B | 1,131 B | 1.1% | 61.1% |
| adv-door: change a narrative choice condition | 8,944 B | 3,342 B | 2,280 B | 25.5% | 68.2% |
| spawn-ten: 10 prefab instances in a row | 10,582 B | 8,393 B | 3,102 B | 29.3% | 37.0% |
| batch-hp: health of every grunt in one revision | 6,986 B | 3,398 B | 1,766 B | 25.3% | 52.0% |
| tune-defeat: change attack damage so the hero loses | 3,193 B | 1,812 B | 1,206 B | 37.8% | 66.6% |
| **Total** (request + response) | **136,563 B** | **18,796 B** | **9,485 B** | **6.9%** | **50.5%** |

Totals by direction: legacy 5,870 B requests and 130,693 B responses; local 6,174 B and 12,622 B; ACL 2,539 B and 6,946 B.

## How to read this

- **Bytes, not tokens.** These numbers are transport sizes of what an agent sends and reads. Token counts depend on the model's tokenizer and are not claimed here. JSON punctuation and long pointer paths usually tokenize worse than ACL's short lines, but that is unmeasured.
- **Scripted, not agent-driven.** The plans are fixed. A real agent also reads discovery output (`discover acl` is ~1 KB, `discover local` is larger) and may retry. The benchmark already counts failed attempts. Running the same suite through an agent adapter (`--agent-executable`) records those effects.
- **Where the savings come from.** Reads are smaller because of selection and the lines format; the biggest single case is fps-health, where legacy reads the whole world. Writes are smaller because of selectors and `spawn` (spawn-ten sends one line instead of ten entity objects twice). Previews and receipts are smaller because ACL summarizes the diff in at most 12 lines.
