use crate::*;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    SceneTree {
        #[serde(default)]
        query: crate::scene_edit::TreeQuery,
    },
    Select {
        query: Box<crate::local::Selection>,
    },
    /// Terse line-oriented script (see `discover acl`), compiled to typed requests.
    Acl {
        text: String,
        #[serde(default)]
        preview: bool,
        #[serde(default)]
        expected_rev: Option<i64>,
        /// Queue edit scripts for human review instead of applying them.
        #[serde(default)]
        propose: bool,
    },
    /// Validate an edit batch (preview) and queue it for review without writing a revision.
    Propose {
        #[serde(default)]
        expected_rev: Option<i64>,
        edits: Vec<Edit>,
        #[serde(default)]
        note: String,
    },
    Proposals {
        /// pending | approved | rejected; omitted lists all.
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Approve (apply against the proposal's base revision, or the current one with
    /// `rebase`) or reject a pending proposal. `keep` approves by a field-wise three-way
    /// merge onto the current revision instead: fields both the proposal and newer
    /// revisions changed take the proposal's value unless their path is listed in `keep`
    /// (see `acl: conflicts ID`). `expected_rev`, when given, must be the current revision
    /// (the one the conflicts were listed against), so no unseen conflict is decided.
    ResolveProposal {
        id: i64,
        approve: bool,
        #[serde(default)]
        rebase: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keep: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_rev: Option<i64>,
    },
    Discover {
        #[serde(default)]
        topic: Option<String>,
        #[serde(default)]
        if_hash: Option<String>,
    },
    Diff {
        from_rev: i64,
        to_rev: i64,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
    },
    ObserveDelta {
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        fields: Vec<String>,
        #[serde(default)]
        trace_limit: usize,
        #[serde(default)]
        max_bytes: Option<usize>,
    },
    Execute {
        request_id: String,
        #[serde(default)]
        runtime_session: Option<String>,
        request: Box<Request>,
    },
    Command {
        command: crate::gameplay::RuntimeCommand,
        #[serde(default)]
        expected_rev: Option<i64>,
    },
    Save {
        slot: String,
    },
    Load {
        slot: String,
    },
    Schema {
        #[serde(default)]
        topic: Option<String>,
    },
    Query {
        #[serde(default)]
        scope: Scope,
        #[serde(default)]
        limit: Option<usize>,
    },
    Apply {
        #[serde(default)]
        expected_rev: Option<i64>,
        edits: Vec<Edit>,
    },
    Preview {
        #[serde(default)]
        expected_rev: Option<i64>,
        edits: Vec<Edit>,
    },
    Verify {},
    Observe {},
    /// Drive a running editor: select objects, frame them, switch dock tabs, capture a PNG
    /// to `artifacts/captures/<capture>.png` on the next frame. Refused while a human draft
    /// is dirty so remote control never clobbers unsaved edits.
    EditorView {
        #[serde(default)]
        select: Vec<String>,
        #[serde(default)]
        focus: bool,
        #[serde(default)]
        dock: Option<String>,
        #[serde(default)]
        capture: Option<String>,
        /// Start (true) or stop (false) the Play-in-editor preview.
        #[serde(default)]
        play: Option<bool>,
        /// Open an asset inspector (`material:ID`, `prefab:ID`, `resource:ID`), a component of
        /// the selection in the Inspector (`component:ID`) or a create command's options
        /// dialog (`options:COMMAND`).
        #[serde(default)]
        open: Option<String>,
        /// Run a registered editor command (the same IDs as the menu bar and palette).
        #[serde(default)]
        command: Option<String>,
        /// The argument of a command that takes one (`discover editor_commands`), e.g.
        /// `world.capability_add` + `physics` or `object.tag` + `+player -npc`.
        #[serde(default)]
        arg: Option<String>,
    },
    Undo {
        rev: i64,
        #[serde(default)]
        expected_rev: Option<i64>,
    },
    History {
        #[serde(default)]
        limit: Option<usize>,
    },
}
impl Request {
    pub fn is_runtime(&self) -> bool {
        if let Self::Execute { request, .. } = self {
            return request.is_runtime();
        }
        matches!(
            self,
            Self::Command { .. } | Self::Save { .. } | Self::Load { .. } | Self::EditorView { .. }
        )
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Packages,
    #[default]
    Summary,
    World,
    Entities,
    Behavior,
    Components,
    Resources,
    Systems,
}

/// The wire envelope stays typed; unrecognized edit names are routed only to installed adapters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Edit {
    Core(CoreEdit),
    Extension(Value),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreEdit {
    Reparent {
        entity: String,
        parent: Option<String>,
        #[serde(default)]
        keep_world: bool,
    },
    DuplicateEntity {
        source: String,
        id: String,
    },
    DeleteEntity {
        id: String,
    },
    /// Moves an entity within the authored entity order (which is also the sibling order in
    /// the scene tree): directly before `before`, or to the end when `before` is null.
    MoveEntity {
        id: String,
        #[serde(default)]
        before: Option<String>,
    },
    SetSceneMembership {
        scene: String,
        entity: String,
        member: bool,
    },
    SetField {
        target: crate::local::Target,
        field: String,
        value: Value,
    },
    /// Set one field on every object matched by a selector. Matching runs once against the
    /// pre-edit world, then each target receives an ordinary `set_field` in the same batch.
    /// `select.limit` (default 4096) caps the match count; zero matches is an error.
    SetWhere {
        select: Box<crate::local::Selection>,
        field: String,
        value: Value,
    },
    PutObject {
        target: crate::local::Target,
        value: Value,
    },
    RemoveObject {
        target: crate::local::Target,
    },
    Set {
        path: String,
        value: Value,
    },
    PutEntity {
        entity: Entity,
    },
    RemoveEntity {
        id: String,
    },
    SetComponent {
        entity: String,
        component: String,
        value: Value,
    },
    RemoveComponent {
        entity: String,
        component: String,
    },
    PutPrefab {
        id: String,
        prefab: Prefab,
    },
    SetResource {
        id: String,
        value: Value,
    },
    RemoveResource {
        id: String,
    },
    DefineComponent {
        id: String,
        schema: RecordSchema,
    },
    DefineResource {
        id: String,
        schema: RecordSchema,
    },
    PutRelation {
        relation: Relation,
    },
    RemoveRelation {
        relation: Relation,
    },
    PutScene {
        scene: Scene,
    },
    SetSystems {
        systems: Vec<String>,
    },
    ReplaceWorld {
        world: Value,
    },
}
impl Edit {
    pub fn set(path: impl Into<String>, value: Value) -> Self {
        Self::Core(CoreEdit::Set {
            path: path.into(),
            value,
        })
    }
    pub fn replace(world: World) -> Self {
        Self::Core(CoreEdit::ReplaceWorld {
            world: json!(world),
        })
    }
    pub fn apply(self, w: &mut World, r: &Registry) -> Result<()> {
        match self {
            Self::Core(CoreEdit::Reparent {
                entity,
                parent,
                keep_world,
            }) => crate::scene_edit::reparent(w, &entity, parent.as_deref(), keep_world)?,
            Self::Core(CoreEdit::DuplicateEntity { source, id }) => {
                crate::scene_edit::duplicate(w, &source, id)?
            }
            Self::Core(CoreEdit::DeleteEntity { id }) => crate::scene_edit::delete(w, &id)?,
            Self::Core(CoreEdit::MoveEntity { id, before }) => {
                crate::scene_edit::move_entity(w, &id, before.as_deref())?
            }
            Self::Core(CoreEdit::SetSceneMembership {
                scene,
                entity,
                member,
            }) => crate::scene_edit::membership(w, &scene, &entity, member)?,
            Self::Core(CoreEdit::SetField {
                target,
                field,
                value,
            }) => target.edit(w, Some(&field), Some(value))?,
            Self::Core(CoreEdit::SetWhere {
                select,
                field,
                value,
            }) => {
                if select.cursor.is_some() || !select.fields.is_empty() {
                    return Err(crate::Diagnostic::error(
                        "invalid_selector",
                        "/select",
                        "set_where selectors take no cursor or fields",
                    ));
                }
                let cap = select.limit.unwrap_or(4096);
                ensure!((1..=4096).contains(&cap), "set_where limit must be 1..4096");
                let targets = crate::local::matches(w, r, &select)?;
                if targets.is_empty() {
                    return Err(crate::Diagnostic::error(
                        "no_match",
                        "/select",
                        "selector matched no objects",
                    ));
                }
                if targets.len() > cap {
                    return Err(crate::Diagnostic::error(
                        "too_many_matches",
                        "/select",
                        format!("selector matched {} objects; limit is {cap}", targets.len()),
                    ));
                }
                for (target, _) in targets {
                    target.edit(w, Some(&field), Some(value.clone()))?;
                }
            }
            Self::Core(CoreEdit::PutObject { target, value }) => {
                target.edit(w, None, Some(value))?
            }
            Self::Core(CoreEdit::RemoveObject { target }) => target.edit(w, None, None)?,
            Self::Core(CoreEdit::Set { path, value }) => {
                if path == "id" || path == "title" || path.starts_with('/') {
                    ensure!(
                        path != "/schema_version" && path != "/id/schema_version",
                        "schema version cannot be edited"
                    );
                    let mut doc = json!(&w);
                    let pointer = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("/{path}")
                    };
                    *doc.pointer_mut(&pointer)
                        .with_context(|| format!("unknown path {path}"))? = value;
                    *w = serde_json::from_value(doc)?;
                } else {
                    let args = json!({"path":path,"value":value});
                    ensure!(
                        r.edit_handlers.iter().try_fold(false, |handled, f| Ok::<
                            _,
                            anyhow::Error,
                        >(
                            handled || f(w, "set", &args)?
                        ))?,
                        "unknown or non-editable path {path}"
                    );
                }
            }
            Self::Core(CoreEdit::PutEntity { entity }) => {
                if let Some(old) = w.entity_mut(&entity.id) {
                    *old = entity;
                } else {
                    w.entities.push(entity);
                }
            }
            Self::Core(CoreEdit::RemoveEntity { id }) => {
                let before = w.entities.len();
                w.entities.retain(|e| e.id != id);
                ensure!(before != w.entities.len(), "entity not found {id}");
            }
            Self::Core(CoreEdit::SetComponent {
                entity,
                component,
                value,
            }) => {
                w.entity_mut(&entity)
                    .context("entity not found")?
                    .components
                    .insert(component, value);
            }
            Self::Core(CoreEdit::RemoveComponent { entity, component }) => {
                ensure!(
                    w.entity_mut(&entity)
                        .context("entity not found")?
                        .components
                        .remove(&component)
                        .is_some(),
                    "component not found"
                );
            }
            Self::Core(CoreEdit::PutPrefab { id, prefab }) => {
                w.prefabs.insert(id, prefab);
            }
            Self::Core(CoreEdit::SetResource { id, value }) => {
                w.resources.insert(id, value);
            }
            Self::Core(CoreEdit::RemoveResource { id }) => {
                ensure!(w.resources.remove(&id).is_some(), "resource not found");
            }
            Self::Core(CoreEdit::DefineComponent { id, schema }) => {
                w.component_schemas.insert(id, schema);
            }
            Self::Core(CoreEdit::DefineResource { id, schema }) => {
                w.resource_schemas.insert(id, schema);
            }
            Self::Core(CoreEdit::PutRelation { relation }) => {
                if !w.relations.contains(&relation) {
                    w.relations.push(relation);
                }
            }
            Self::Core(CoreEdit::RemoveRelation { relation }) => {
                let n = w.relations.len();
                w.relations.retain(|r| r != &relation);
                ensure!(w.relations.len() != n, "relation not found");
            }
            Self::Core(CoreEdit::PutScene { scene }) => {
                if let Some(old) = w.scenes.iter_mut().find(|s| s.id == scene.id) {
                    *old = scene;
                } else {
                    w.scenes.push(scene);
                }
            }
            Self::Core(CoreEdit::SetSystems { systems }) => w.systems = systems,
            Self::Core(CoreEdit::ReplaceWorld { world }) => *w = r.decode(world)?,
            Self::Extension(mut v) => {
                let name = v
                    .get("kind")
                    .and_then(Value::as_str)
                    .context("edit requires a kind")?
                    .to_owned();
                ensure!(
                    ![
                        "set",
                        "set_field",
                        "set_where",
                        "put_object",
                        "remove_object",
                        "put_entity",
                        "remove_entity",
                        "reparent",
                        "duplicate_entity",
                        "delete_entity",
                        "move_entity",
                        "set_scene_membership",
                        "set_component",
                        "remove_component",
                        "put_prefab",
                        "set_resource",
                        "remove_resource",
                        "define_component",
                        "define_resource",
                        "put_relation",
                        "remove_relation",
                        "put_scene",
                        "set_systems",
                        "replace_world"
                    ]
                    .contains(&name.as_str()),
                    "invalid fields or types for edit {name}"
                );
                v.as_object_mut()
                    .context("edit must be object")?
                    .remove("kind");
                let mut handled = false;
                for f in &r.edit_handlers {
                    if f(w, &name, &v)? {
                        handled = true;
                        break;
                    }
                }
                ensure!(handled, "unregistered edit {name}");
            }
        }
        Ok(())
    }
}
pub fn parse_request(text: &str) -> Result<Request> {
    if text.len() > 1_048_576 {
        bail!("request exceeds 1 MiB");
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|e| Diagnostic::error("invalid_json", "/", e.to_string()))?;
    if let Some(request) = value.get("request") {
        ensure!(
            request.get("op").and_then(Value::as_str) != Some("execute"),
            "nested execute envelope is not allowed"
        );
    }
    serde_json::from_value(value)
        .map_err(|e| Diagnostic::error("invalid_request", "/", e.to_string()))
}
