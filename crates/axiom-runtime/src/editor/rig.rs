//! `rig.edit`: spring chains, colliders, look-at chains, IK sockets and their debug draw on
//! the selected animated characters. The argument is the ACL `rig SEL ..` statement after
//! the selection, parsed by the same function and written by the same op
//! (`axiom_core::ops::rig_edit`), so a person, a shortcut and an agent write the same.
use super::{Action, Dock, Editor};

impl Editor {
    pub(super) fn rig_command(&mut self, arg: Option<&str>) -> anyhow::Result<Vec<Action>> {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            // From the menu or palette: the components are edited in the Inspector.
            self.show_dock(Dock::Inspector);
            self.notice(
                "Edit animation.spring / animation.look_at in the Inspector, or run rig.edit with e.g. `spring hair root=hair.0`",
            );
            return Ok(vec![]);
        };
        let tokens: Vec<&str> = arg.split_whitespace().collect();
        let edit = axiom_core::ops::parse_rig_edit(&tokens)?;
        let result =
            axiom_core::ops::rig_edit(&self.world, &self.registry, &self.selection(), &edit);
        Ok(self.tuning_write(result).into_iter().collect())
    }
}
