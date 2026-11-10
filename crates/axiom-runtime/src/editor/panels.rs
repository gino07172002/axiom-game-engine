//! Panel headers over the dock's tab strips: an icon before every tab title (titles start
//! with [`ICON_PAD`] so the icon has room) and a "…" menu at the right end of each strip
//! (close the active tab, reset the layout). Close buttons are not drawn on the tabs.
use super::icons::Icon;
use super::{theme, Dock, Editor};
use egui::{pos2, vec2, LayerId, Rect};
use egui_dock::{DockState, Node};

/// Leading spaces of every tab title: the tab icon is painted over them.
pub(super) const ICON_PAD: &str = "       ";

/// Icon of a dock tab.
pub(super) fn tab_icon(tab: Dock) -> Icon {
    match tab {
        Dock::Hierarchy => Icon::Hierarchy,
        Dock::Scene => Icon::Scene,
        Dock::Game => Icon::Game,
        Dock::Inspector => Icon::Inspector,
        Dock::World => Icon::World,
        Dock::Changes => Icon::Layers,
        Dock::Proposals => Icon::Proposals,
        Dock::History => Icon::History,
        Dock::Problems => Icon::Problems,
        Dock::Scenarios => Icon::Scenarios,
        Dock::Graphs => Icon::Graphs,
        Dock::Assets => Icon::Assets,
        Dock::Agent => Icon::Agent,
        Dock::UiEditor => Icon::UiDocument,
        Dock::Console => Icon::Console,
        Dock::Profiler => Icon::Chip,
        Dock::Trace => Icon::Event,
        Dock::InputMap => Icon::Input,
        Dock::Localization => Icon::Localization,
        Dock::Animation => Icon::Animation,
        Dock::Vfx => Icon::Vfx,
    }
}

impl Editor {
    /// Tab title as the dock draws it: icon room, name, and a count on Problems / Proposals
    /// (Console: kept errors).
    pub(super) fn tab_label(&self, tab: Dock) -> String {
        let count = match tab {
            Dock::Problems => self.problems.len(),
            Dock::Proposals => self.proposals.len(),
            Dock::Console => self.debug.console.count(super::debug::Level::Error),
            _ => 0,
        };
        if count == 0 {
            format!("{ICON_PAD}{}", tab.title())
        } else {
            format!("{ICON_PAD}{} ({count})", tab.title())
        }
    }

    /// Icons over the main surface's tab strips (leaf rectangles are the ones the dock stored
    /// this frame). Painting only: no widgets (an always-present Area here would sit above the
    /// dock and take its clicks).
    pub(super) fn panel_icons(&self, ctx: &egui::Context, dock: &DockState<Dock>) {
        let icons = self.icons;
        // The dock's own layer: after its tab strips, under every window.
        let painter = ctx.layer_painter(LayerId::background());
        let font = egui::TextStyle::Button.resolve(&ctx.style());
        let pad = ctx.fonts(|f| {
            f.layout_no_wrap(ICON_PAD.into(), font.clone(), theme::TEXT)
                .size()
                .x
        });
        for node in dock.main_surface().iter() {
            let Node::Leaf {
                rect,
                viewport,
                tabs,
                active,
                scroll,
                collapsed,
            } = node
            else {
                continue;
            };
            if *collapsed || !rect.is_positive() || viewport.min.y <= rect.min.y {
                continue;
            }
            let bar = Rect::from_min_max(rect.min, pos2(rect.max.x, viewport.min.y));
            let menu_left = bar.max.x - 30.0;
            // Same widths as the dock's tabs: text + 8 px on each side, no spacing between.
            let mut x = bar.min.x + scroll;
            for (k, tab) in tabs.iter().enumerate() {
                let width = ctx.fonts(|f| {
                    f.layout_no_wrap(self.tab_label(*tab), font.clone(), theme::TEXT)
                        .size()
                        .x
                }) + 16.0;
                let at = Rect::from_center_size(
                    pos2(x + 8.0 + pad / 2.0, bar.center().y),
                    vec2(15.0, 15.0),
                );
                if at.min.x >= bar.min.x && at.max.x < menu_left {
                    let tint = if k == active.0 {
                        theme::ACCENT
                    } else {
                        theme::MUTED
                    };
                    icons.paint(&painter, at, tab_icon(*tab), tint);
                }
                x += width;
            }
        }
    }

    /// The "…" panel menu at the right end of a leaf's tab strip (`bar`, last frame's), drawn
    /// from the active tab's body in the dock's own layer: close the tab, reset the layout.
    pub(super) fn panel_menu(
        &mut self,
        ui: &mut egui::Ui,
        tab: Dock,
        bar: Option<Rect>,
    ) -> Option<(&'static str, Option<String>)> {
        // The child is created every frame, strip known or not: a child ui takes one of the
        // parent's automatic IDs, and the tab's widgets must keep theirs between frames.
        let at = bar.map_or(Rect::NOTHING, |bar| {
            Rect::from_min_size(
                pos2(bar.max.x - 30.0, bar.center().y - 11.0),
                vec2(22.0, 22.0),
            )
        });
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(if bar.is_some() { at } else { ui.max_rect() })
                .id_salt(("panel_menu", tab.name())),
        );
        bar?;
        child.set_clip_rect(at);
        let more = theme::icon_button_sized(
            &mut child,
            self.icons,
            Icon::More,
            "Panel menu (close the tab, reset the layout)",
            false,
            22.0,
        );
        self.rects
            .insert(format!("panel:more:{}", tab.name()), more.rect);
        let id = child.make_persistent_id("menu");
        let mut run = None;
        theme::dropdown(&mut child, id, &more, |ui| {
            let r = ui.button(format!("Close {}", tab.title()));
            self.rects
                .insert(format!("panel:close:{}", tab.name()), r.rect);
            if r.clicked() {
                run = Some(("window.close", Some(tab.name().to_owned())));
                theme::close_dropdown(ui, id);
            }
            if ui.button("Reset Layout").clicked() {
                run = Some(("layout.reset", None));
                theme::close_dropdown(ui, id);
            }
        });
        run
    }
}
