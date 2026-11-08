//! Bounded reachability over authored choices/buttons and waiting for scheduled events.
use crate::gameplay::Gameplay;
use anyhow::{ensure, Result};
use axiom_core::{
    gameplay::{BoundWidget, RuntimeCommand},
    World,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
pub fn analyze(world: &World, budget: usize) -> Result<Value> {
    ensure!(
        (1..=100_000).contains(&budget),
        "reachability budget must be 1..100000"
    );
    let initial = Gameplay::new(world)?;
    ensure!(initial.node.is_some(), "world has no narrative graph");
    let mut pending = VecDeque::from([(initial, 0u64, Vec::<Value>::new())]);
    let mut visited = BTreeSet::new();
    let mut endings = std::collections::BTreeMap::new();
    let mut explored = 0;
    let mut complete = true;
    let mut state_bytes = 0usize;
    let mut queued_bytes = serde_json::to_vec(&pending[0].0.progress())?.len();
    while let Some((g, tick, path)) = pending.pop_front() {
        queued_bytes = queued_bytes.saturating_sub(
            serde_json::to_vec(&g.progress())?.len() + serde_json::to_vec(&path)?.len(),
        );
        let mut p = g.progress();
        for e in &mut p.pending {
            e.due = e.due.saturating_sub(tick);
        }
        let key = serde_json::to_string(&p)?;
        state_bytes = state_bytes.saturating_add(key.len());
        if state_bytes > 16_777_216 {
            complete = false;
            break;
        }
        if !visited.insert(key) {
            continue;
        }
        if explored == budget {
            complete = false;
            break;
        }
        explored += 1;
        if let Some(e) = &g.ending {
            endings.entry(e.clone()).or_insert(path);
            continue;
        }
        let mut commands = g
            .choices()
            .iter()
            .map(|c| RuntimeCommand::Choose {
                node: g.compiled.nodes[g.node.unwrap()].id.clone(),
                choice: c.id.clone(),
            })
            .collect::<Vec<_>>();
        for w in &g.compiled.widgets {
            if let BoundWidget::Button(_, _, command, condition) = w {
                if g.test(condition) && !matches!(command, RuntimeCommand::Locale { .. }) {
                    commands.push(command.clone());
                }
            }
        }
        for command in commands {
            if pending.len() >= budget {
                complete = false;
                break;
            }
            let mut next = g.clone();
            next.command(&command, tick)?;
            let mut path = path.clone();
            path.push(json!(command));
            let bytes =
                serde_json::to_vec(&next.progress())?.len() + serde_json::to_vec(&path)?.len();
            if state_bytes
                .saturating_add(queued_bytes)
                .saturating_add(bytes)
                > 16_777_216
            {
                complete = false;
                continue;
            }
            queued_bytes += bytes;
            pending.push_back((next, tick, path));
        }
        if let Some(due) = g.pending.iter().map(|e| e.due).min() {
            if pending.len() >= budget {
                complete = false;
                continue;
            }
            let mut next = g.clone();
            next.step(&crate::ActionMap::default(), due);
            ensure!(
                next.fault.is_none(),
                "reachability encountered gameplay fault"
            );
            let mut path = path.clone();
            path.push(json!({"wait_until":due}));
            let bytes =
                serde_json::to_vec(&next.progress())?.len() + serde_json::to_vec(&path)?.len();
            if state_bytes
                .saturating_add(queued_bytes)
                .saturating_add(bytes)
                > 16_777_216
            {
                complete = false;
                continue;
            }
            queued_bytes += bytes;
            pending.push_back((next, due, path));
        }
    }
    Ok(
        json!({"ok":true,"complete":complete,"explored_states":explored,"budget":budget,"reachable_endings":endings,"scope":"authored choices, UI button commands and delayed events; external event sources excluded"}),
    )
}
