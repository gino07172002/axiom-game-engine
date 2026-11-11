//! Checkpoints contain simulation state, independent of immutable authoring history.
use crate::{gameplay::Progress, world::WorldRuntime};
use anyhow::{ensure, Context, Result};
use axiom_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};
pub const MAX_SAVE_BYTES: u64 = 8_388_608;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityState {
    pub transform: Option<Transform>,
    pub health: Option<Health>,
    pub components: Components,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub save_version: u32,
    pub engine_version: String,
    pub authoring_rev: i64,
    pub content: World,
    pub tick: u64,
    pub entities: BTreeMap<String, EntityState>,
    pub gameplay: Option<Progress>,
    /// Random stream position. Absent in pre-0.4.1 saves, which restore the content seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng: Option<u64>,
    /// Native system state by system ID (`RuntimeSystem::save_state`, e.g. animation
    /// playback). Absent in older saves; a system that needs it then refuses the load.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub systems: BTreeMap<String, Value>,
}
fn column_value(column: &Column, row: usize, state: &crate::world::RuntimeState) -> Option<Value> {
    match column {
        Column::Bool(v) => v[row].map(|v| json!(v)),
        Column::Int(v) => v[row].map(|v| json!(v)),
        Column::Float(v) => v[row].map(|v| json!(v)),
        Column::String(v) => v[row].as_ref().map(|v| json!(v)),
        Column::Vec3(v) => v[row].map(|v| json!(v)),
        Column::Entity(v) => v[row].map(|i| json!(state.compiled.entities[i].id)),
    }
}
impl WorldRuntime {
    pub fn checkpoint(&self) -> Result<Checkpoint> {
        self.checkpoint_supported()?;
        self.checkpoint_ready()?;
        self.assert_valid()?;
        let mut entities = self
            .state
            .compiled
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| {
                (
                    e.id.clone(),
                    EntityState {
                        transform: self.state.transforms[i].clone(),
                        health: self.state.health[i].clone(),
                        components: BTreeMap::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (name, table) in &self.state.columns {
            for (row, slot) in table.entities.iter().enumerate() {
                let fields = table
                    .fields
                    .iter()
                    .filter_map(|(field, col)| {
                        column_value(col, row, &self.state).map(|v| (field.clone(), v))
                    })
                    .collect::<serde_json::Map<_, _>>();
                entities
                    .get_mut(&self.state.compiled.entities[*slot].id)
                    .unwrap()
                    .components
                    .insert(name.clone(), Value::Object(fields));
            }
        }
        Ok(Checkpoint {
            save_version: 1,
            engine_version: env!("CARGO_PKG_VERSION").into(),
            authoring_rev: self.rev,
            content: self.state.compiled.source.clone(),
            tick: self.state.tick,
            entities,
            gameplay: self.state.gameplay.as_ref().map(|g| g.progress()),
            rng: Some(self.state.rng),
            systems: self.save_system_states()?,
        })
    }
    pub fn restore_checkpoint(&mut self, c: &Checkpoint) -> Result<()> {
        self.checkpoint_supported()?;
        ensure!(c.save_version == 1, "unsupported checkpoint version");
        ensure!(
            ["0.3.0", "0.3.1", env!("CARGO_PKG_VERSION")].contains(&c.engine_version.as_str()),
            "checkpoint engine version mismatch"
        );
        ensure!(
            c.content == self.state.compiled.source,
            "checkpoint content mismatch; load the matching content revision or migrate explicitly"
        );
        ensure!(
            c.tick < 1_000_000_000_000 && c.entities.len() == self.state.compiled.len(),
            "invalid checkpoint tick/entity budget"
        );
        let mut snapshot = c.content.clone();
        for e in &mut snapshot.entities {
            let saved = c.entities.get(&e.id).context("checkpoint entity missing")?;
            let i = self.state.compiled.indices[&e.id];
            let original = &self.state.compiled.entities[i];
            ensure!(
                saved.transform.is_some() == original.transform.is_some()
                    && saved.health.is_some() == original.health.is_some(),
                "checkpoint component layout mismatch"
            );
            if let Some(t) = &saved.transform {
                t.validate()?;
                e.set("core.transform", t)?;
            }
            if let Some(h) = &saved.health {
                h.validate()?;
                e.set("core.health", h)?;
            }
            let expected = original
                .components
                .keys()
                .filter(|id| snapshot.component_schemas.contains_key(*id))
                .collect::<Vec<_>>();
            ensure!(
                saved.components.len() == expected.len()
                    && saved.components.keys().all(|id| expected.contains(&id)),
                "checkpoint custom component layout mismatch"
            );
            for (id, value) in &saved.components {
                e.components.insert(id.clone(), value.clone());
            }
        }
        let storage = snapshot.compile(&self.registry_clone().authoring)?;
        let mut next = Self::new(c.content.clone(), self.rev, self.registry_clone())?;
        for (i, e) in storage.entities.iter().enumerate() {
            next.state.transforms[i] = e.transform.clone();
            next.state.health[i] = e.health.clone();
        }
        next.state.columns = storage.columns;
        next.state.tick = c.tick;
        if let Some(rng) = c.rng {
            next.state.rng = rng;
        }
        ensure!(
            next.state.gameplay.is_some() == c.gameplay.is_some(),
            "checkpoint gameplay state mismatch"
        );
        if let (Some(g), Some(p)) = (&mut next.state.gameplay, &c.gameplay) {
            g.restore(p, c.tick)?;
        }
        // Keep entity handles valid when loading state into the same content.
        next.state.ids = self.state.ids.clone();
        next.runtime_session = self.runtime_session.clone();
        next.runtime_epoch = self.runtime_epoch + 1;
        next.state.next_generation = self.state.next_generation;
        next.state.sync_scenes();
        next.load_system_states(&c.systems)?;
        next.restored();
        next.assert_valid()?;
        *self = next;
        Ok(())
    }
    pub fn save_slot(&self, root: &Path, slot: &str) -> Result<Value> {
        let path = slot_path(root, &self.state.compiled.source.id, slot)?;
        let raw = serde_json::to_vec_pretty(&self.checkpoint()?)?;
        ensure!(
            raw.len() as u64 <= MAX_SAVE_BYTES,
            "checkpoint exceeds size budget"
        );
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&temp)?;
            f.write_all(&raw)?;
            f.sync_all()?;
        }
        // rename cannot replace an existing file on Windows; use a backup to keep the old save recoverable.
        let backup = path.with_extension("json.bak");
        if path.exists() {
            if backup.exists() {
                std::fs::remove_file(&backup)?;
            }
            std::fs::rename(&path, &backup)?;
        }
        if let Err(e) = std::fs::rename(&temp, &path) {
            if backup.exists() {
                let _ = std::fs::rename(&backup, &path);
            }
            return Err(e.into());
        }
        Ok(json!({"ok":true,"slot":slot,"save_version":1,"rev":self.rev,"tick":self.state.tick}))
    }
    pub fn load_slot(&mut self, root: &Path, slot: &str) -> Result<Value> {
        let path = slot_path(root, &self.state.compiled.source.id, slot)?;
        let path = if path.exists() {
            path
        } else {
            path.with_extension("json.bak")
        };
        ensure!(
            std::fs::metadata(&path)?.len() <= MAX_SAVE_BYTES,
            "checkpoint exceeds size budget"
        );
        let checkpoint: Checkpoint = serde_json::from_slice(&std::fs::read(path)?)?;
        self.restore_checkpoint(&checkpoint)?;
        Ok(json!({"ok":true,"slot":slot,"save_version":1,"rev":self.rev,"tick":self.state.tick}))
    }
}
fn slot_path(root: &Path, world: &str, slot: &str) -> Result<std::path::PathBuf> {
    // Flat file names, including reserved Windows names, never accept a path from a protocol request.
    ensure!(
        !slot.is_empty()
            && slot.len() <= 48
            && slot
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid save slot"
    );
    let world = world
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut path = root.to_path_buf();
    for chunk in world.as_bytes().chunks(96) {
        path.push(std::str::from_utf8(chunk)?);
    }
    Ok(path.join(format!("slot-{slot}.json")))
}
