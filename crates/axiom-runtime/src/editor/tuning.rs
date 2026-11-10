//! Tuning editors: Input Map, Localization, Animation (ability timeline events) and VFX
//! (size curve / colour gradient) tabs, plus the material inspector's shader source. Each
//! content edit is one `axiom_core::ops` call (= the ACL `input` / `loc` / `ability` / `vfx` /
//! `shader` verb); the `*.edit` registry commands take that verb's arguments and compile
//! through `acl::tuning_command_edits`, so a person, a shortcut and an agent write the same.
use super::{Action, Dock, Editor};
use serde_json::{json, Value};

/// Registry commands handled here (the `*.edit` ones write; the others are view state).
pub(super) const TUNING_PREFIXES: [&str; 6] =
    ["input.", "loc.", "ability.", "anim.", "vfx.", "shader."];

pub(super) fn is_tuning(id: &str) -> bool {
    TUNING_PREFIXES.iter().any(|p| id.starts_with(p))
}

/// Editor-only state of the tuning tabs (never in revisions).
#[derive(Default)]
pub(crate) struct Tuning {
    pub input: super::input_map::InputUi,
    pub loc: super::localization::LocUi,
    pub anim: super::anim_timeline::AnimUi,
    pub vfx: super::vfx_editor::VfxUi,
    pub shader: super::shader_editor::ShaderUi,
}

/// Test-only widget rectangles (`Editor::rects`); nothing is built in other builds.
macro_rules! mark {
    ($rects:expr, $key:expr, $rect:expr) => {
        #[cfg(test)]
        $rects.insert($key, $rect);
        #[cfg(not(test))]
        let _ = (&$rects, &$rect);
    };
}
pub(super) use mark;

impl Tuning {
    /// Start of an editor frame: tabs mark themselves visible again when drawn.
    pub fn begin_frame(&mut self) {
        self.anim.visible = false;
    }
}

impl Editor {
    /// `input.*`, `loc.*`, `ability.*`, `anim.*`, `vfx.*`, `shader.*` registry commands.
    pub(super) fn tuning_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        let (verb, dock) = match id {
            "input.edit" => ("input", Dock::InputMap),
            "loc.edit" => ("loc", Dock::Localization),
            "ability.edit" => ("ability", Dock::Animation),
            "vfx.edit" => ("vfx", Dock::Vfx),
            "shader.edit" => ("shader", Dock::Inspector),
            "input.capture" => return self.input_capture_command(arg).map(|_| vec![]),
            "loc.filter" => {
                self.tuning.loc.filter = arg.unwrap_or_default().to_owned();
                self.show_dock(Dock::Localization);
                return Ok(vec![]);
            }
            "anim.target" => return self.anim_target_command(arg).map(|_| vec![]),
            "anim.scrub" => return self.anim_scrub_command(arg).map(|_| vec![]),
            "vfx.select" => return self.vfx_select_command(arg).map(|_| vec![]),
            "shader.check" => return self.shader_check_command().map(|_| vec![]),
            "shader.draft" => return self.shader_draft_command(arg).map(|_| vec![]),
            "shader.cancel_check" => {
                self.cancel_shader_check();
                return Ok(vec![]);
            }
            other => anyhow::bail!("{other} is not a tuning editor command"),
        };
        let Some(arg) = arg else {
            // From the menu or palette: open the tab where the person picks the arguments.
            self.show_dock(dock);
            return Ok(vec![]);
        };
        let result = axiom_core::acl::tuning_command_edits(&self.world, &format!("{verb} {arg}"));
        Ok(self.tuning_write(result).into_iter().collect())
    }

    /// The write of an op result; an edit that changes nothing writes nothing (the store
    /// refuses empty batches) and a failed op becomes a problem.
    pub(super) fn tuning_write(
        &mut self,
        result: anyhow::Result<Vec<axiom_core::Edit>>,
    ) -> Option<Action> {
        match result {
            Ok(edits) if edits.is_empty() => None,
            other => self.op_write(other, None),
        }
    }

    /// Why a tuning command cannot run right now, if it cannot (`None` = not one of ours or
    /// runnable).
    pub(super) fn tuning_blocked(&self, id: &str) -> Option<&'static str> {
        match id {
            "shader.cancel_check" if self.tuning.shader.job.is_none() => {
                Some("no shader check is running")
            }
            "shader.check" if self.tuning.shader.job.is_some() => {
                Some("a shader check is already running")
            }
            "shader.check" | "shader.draft" if self.shader_under_edit().is_none() => {
                Some("open a material with a custom shader (Assets ▸ Open)")
            }
            _ => None,
        }
    }

    /// While a key is being captured for the Input Map, the editor's shortcuts stand down
    /// (the key goes to the binding instead).
    pub(super) fn keys_captured(&self, ctx: &egui::Context) -> bool {
        self.tuning.input.capturing(ctx)
    }

    /// `observe` → `editor.tuning`.
    pub(crate) fn tuning_observation(&self) -> Value {
        json!({
            "input":self.tuning.input.observation(),
            "localization":self.tuning.loc.observation(),
            "animation":self.tuning.anim.observation(),
            "vfx":self.tuning.vfx.observation(),
            "shader":self.tuning.shader.observation(),
        })
    }
}
