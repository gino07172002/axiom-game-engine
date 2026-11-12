//! `asset.reload` / `asset.watch`: model hot reload by content hash. The host (app) owns the
//! background job (it needs the running runtime and renderer); the editor only asks.
use super::Editor;

/// What the editor asked the host to do on its next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetReloadAsk {
    Now,
    Watch(bool),
}

impl Editor {
    /// `asset.reload` (no argument) and `asset.watch on|off` (none = toggle).
    pub(super) fn reload_command(&mut self, id: &str, arg: Option<&str>) -> anyhow::Result<()> {
        let arg = arg.map(str::trim).filter(|a| !a.is_empty());
        match id {
            "asset.reload" => {
                anyhow::ensure!(arg.is_none(), "asset.reload takes no argument");
                self.reload_ask = Some(AssetReloadAsk::Now);
                self.notice("Reloading changed model files in the background…");
            }
            "asset.watch" => {
                let on = match arg.map(str::to_ascii_lowercase).as_deref() {
                    None => !self.reload_watching,
                    Some("on") => true,
                    Some("off") => false,
                    Some(_) => anyhow::bail!("asset.watch takes on|off"),
                };
                self.reload_watching = on;
                self.reload_ask = Some(AssetReloadAsk::Watch(on));
                self.notice(if on {
                    "Watching model files: changed GLB / glTF / PMX reload automatically"
                } else {
                    "Stopped watching model files"
                });
            }
            _ => anyhow::bail!("unknown asset command {id}"),
        }
        Ok(())
    }

    /// The host takes the pending request once per frame.
    pub(crate) fn take_asset_reload(&mut self) -> Option<AssetReloadAsk> {
        self.reload_ask.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_commands_queue_one_request_for_the_host() {
        let mut e = Editor::new(axiom_core::World::empty("w"), 1, None);
        assert_eq!(e.take_asset_reload(), None);
        e.run_command("asset.reload").unwrap();
        assert_eq!(e.take_asset_reload(), Some(AssetReloadAsk::Now));
        assert_eq!(e.take_asset_reload(), None);
        e.run_command("asset.watch").unwrap();
        assert_eq!(e.take_asset_reload(), Some(AssetReloadAsk::Watch(true)));
        e.run_command_with("asset.watch", Some("off")).unwrap();
        assert_eq!(e.take_asset_reload(), Some(AssetReloadAsk::Watch(false)));
        assert!(e.run_command_with("asset.watch", Some("maybe")).is_err());
        assert!(e.run_command_with("asset.reload", Some("x")).is_err());
    }
}
