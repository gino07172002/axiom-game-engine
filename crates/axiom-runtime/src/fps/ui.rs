use super::simulation::Simulation;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use glam::{Mat4, Vec3};

const MINT: Color32 = Color32::from_rgb(102, 245, 205);
const WHITE: Color32 = Color32::from_rgb(231, 241, 240);
const MUTED: Color32 = Color32::from_rgb(135, 158, 163);
const AMBER: Color32 = Color32::from_rgb(255, 178, 91);

use crate::ui::{Action, UiState};
pub fn draw(ctx: &egui::Context, sim: &Simulation, state: &mut UiState) -> Vec<Action> {
    let mut actions = vec![];
    let rect = ctx.screen_rect();
    let w = rect.width();
    let h = rect.height();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let text = |p: Pos2, align: Align2, s: &str, size: f32, color: Color32| {
        painter.text(p, align, s, FontId::proportional(size), color);
    };
    if !state.playing {
        let mut scrim = egui::Mesh::default();
        let shade = Color32::from_rgba_unmultiplied(6, 14, 19, 235);
        for (pos, color) in [
            (rect.left_top(), shade),
            (rect.right_top(), Color32::TRANSPARENT),
            (rect.right_bottom(), Color32::TRANSPARENT),
            (rect.left_bottom(), shade),
        ] {
            scrim.vertices.push(egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color,
            });
        }
        scrim.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(scrim));
        painter.line_segment(
            [Pos2::new(42.0, 90.0), Pos2::new(w - 42.0, 90.0)],
            Stroke::new(1.0_f32, Color32::from_white_alpha(25)),
        );
        text(
            Pos2::new(44.0, 42.0),
            Align2::LEFT_TOP,
            "A X I O M",
            26.0,
            WHITE,
        );
        text(
            Pos2::new(46.0, 82.0),
            Align2::LEFT_BOTTOM,
            concat!("AI-NATIVE GAME ENGINE  /  ", env!("CARGO_PKG_VERSION")),
            10.0,
            MUTED,
        );
        text(
            Pos2::new(w - 44.0, 47.0),
            Align2::RIGHT_TOP,
            "RUST  /  WGPU  /  NO BEVY",
            11.0,
            MINT,
        );
        let menu_scale = (h / 900.0).clamp(0.65, 1.0);
        egui::Area::new("menu".into())
            .fixed_pos(Pos2::new(48.0, h * 0.20))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0 * menu_scale);
                ui.set_width((450.0 * menu_scale).min(w - 80.0));
                ui.label(
                    RichText::new("EXPERIMENT 001    /    FPS SANDBOX")
                        .size(11.0)
                        .color(AMBER),
                );
                ui.add_space(16.0 * menu_scale);
                ui.label(
                    RichText::new("PROVING")
                        .size(68.0 * menu_scale)
                        .strong()
                        .color(WHITE),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new("GROUND")
                            .size(68.0 * menu_scale)
                            .strong()
                            .color(MINT),
                    )
                    .selectable(false),
                );
                ui.add_space(12.0 * menu_scale);
                ui.label(
                    RichText::new("One world. Endless iterations.")
                        .size(20.0 * menu_scale)
                        .color(WHITE),
                );
                ui.label(
                    RichText::new("Clear the sentinels. Rewrite the rules live.")
                        .size((14.0 * menu_scale).max(11.0))
                        .color(MUTED),
                );
                ui.add_space(26.0 * menu_scale);
                let button = egui::Button::new(
                    RichText::new(if sim.tick > 0 {
                        "RESUME SIMULATION  >"
                    } else {
                        "ENTER SIMULATION  >"
                    })
                    .size(14.0)
                    .strong()
                    .color(Color32::from_rgb(10, 28, 24)),
                )
                .fill(MINT)
                .min_size(Vec2::new(318.0 * menu_scale.max(0.85), 52.0 * menu_scale));
                if ui.add(button).clicked() {
                    actions.push(if sim.tick > 0 {
                        Action::Resume
                    } else {
                        Action::Start
                    });
                }
                ui.add_space(10.0 * menu_scale);
                ui.horizontal(|ui| {
                    if ui.button("RESTART").clicked() {
                        actions.push(Action::Restart);
                    }
                    if ui.button("EXIT").clicked() {
                        actions.push(Action::Quit);
                    }
                });
                ui.add_space(20.0 * menu_scale);
                ui.label(
                    RichText::new("W A S D   MOVE     /     MOUSE   AIM")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new("LMB   FIRE     R   RELOAD     SPACE   JUMP")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(if state.player {
                        "SHIFT   SPRINT"
                    } else {
                        "SHIFT   SPRINT     TAB   WORLD CONSOLE"
                    })
                    .size(11.0)
                    .color(MUTED),
                );
            });
        let p = Pos2::new(w - 290.0, h - 200.0);
        painter.rect_filled(
            Rect::from_min_size(p, Vec2::new(246.0, 122.0)),
            3.0,
            Color32::from_black_alpha(125),
        );
        painter.line_segment([p, p + Vec2::new(0.0, 122.0)], Stroke::new(2.0_f32, MINT));
        text(
            p + Vec2::new(18.0, 16.0),
            Align2::LEFT_TOP,
            "WORLD ONLINE",
            11.0,
            MINT,
        );
        text(
            p + Vec2::new(18.0, 43.0),
            Align2::LEFT_TOP,
            &format!(
                "{:02} SENTINELS    /    REV {:03}",
                sim.world.enemies.count, sim.rev
            ),
            15.0,
            WHITE,
        );
        text(
            p + Vec2::new(18.0, 72.0),
            Align2::LEFT_TOP,
            "TRANSACTIONAL. DECLARATIVE. LIVE.",
            9.0,
            MUTED,
        );
        text(
            Pos2::new(44.0, h - 32.0),
            Align2::LEFT_BOTTOM,
            "WORLD IR  /  RUST RUNTIME  /  OBSERVATION",
            10.0,
            MUTED,
        );
        text(
            Pos2::new(w - 44.0, h - 32.0),
            Align2::RIGHT_BOTTOM,
            &format!("{:.0} FPS   •   FIXED 120 HZ", state.fps),
            10.0,
            MINT,
        );
    } else {
        text(
            Pos2::new(30.0, 25.0),
            Align2::LEFT_TOP,
            "A X I O M",
            20.0,
            WHITE,
        );
        text(
            Pos2::new(31.0, 54.0),
            Align2::LEFT_TOP,
            &format!("{}  /  ELIMINATE ALL SENTINELS", sim.world.title),
            10.0,
            MUTED,
        );
        text(
            Pos2::new(w * 0.5, 29.0),
            Align2::CENTER_TOP,
            &format!(
                "{:02}  /  {:02}  CLEARED",
                sim.world.enemies.count.saturating_sub(sim.alive()),
                sim.world.enemies.count
            ),
            14.0,
            WHITE,
        );
        let progress = if sim.world.enemies.count > 0 {
            sim.world.enemies.count.saturating_sub(sim.alive()) as f32
                / sim.world.enemies.count as f32
        } else {
            0.0
        };
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(w * 0.5 - 90.0, 56.0), Vec2::new(180.0, 2.0)),
            0.0,
            Color32::from_white_alpha(35),
        );
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(w * 0.5 - 90.0, 56.0),
                Vec2::new(180.0 * progress.min(1.0), 2.0),
            ),
            0.0,
            MINT,
        );
        // Revision / FPS and the console hint are developer chrome; a shipped game has no console.
        if state.player {
            text(
                Pos2::new(w - 30.0, 28.0),
                Align2::RIGHT_TOP,
                "ESC  PAUSE",
                10.0,
                MUTED,
            );
        } else {
            text(
                Pos2::new(w - 30.0, 28.0),
                Align2::RIGHT_TOP,
                &format!("LIVE   REV {:03}   /   {:.0} FPS", sim.rev, state.fps),
                11.0,
                MINT,
            );
            text(
                Pos2::new(w - 30.0, 50.0),
                Align2::RIGHT_TOP,
                "TAB  WORLD CONSOLE    ESC  PAUSE",
                10.0,
                MUTED,
            );
        }
        let c = rect.center();
        let hit = sim.hit_flash > 0.0;
        let color = if hit {
            AMBER
        } else {
            Color32::from_white_alpha(210)
        };
        for (a, b) in [
            (Vec2::new(-12.0, 0.0), Vec2::new(-5.0, 0.0)),
            (Vec2::new(5.0, 0.0), Vec2::new(12.0, 0.0)),
            (Vec2::new(0.0, -12.0), Vec2::new(0.0, -5.0)),
            (Vec2::new(0.0, 5.0), Vec2::new(0.0, 12.0)),
        ] {
            painter.line_segment([c + a, c + b], Stroke::new(1.4_f32, color));
        }
        painter.circle_filled(c, 1.2, color);
        if hit {
            for s in [-1.0, 1.0] {
                for t in [-1.0, 1.0] {
                    painter.line_segment(
                        [
                            c + Vec2::new(s * 15.0, t * 15.0),
                            c + Vec2::new(s * 21.0, t * 21.0),
                        ],
                        Stroke::new(2.0_f32, AMBER),
                    );
                }
            }
        }
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(24.0, h - 100.0), Vec2::new(250.0, 76.0)),
            3.0,
            Color32::from_black_alpha(155),
        );
        text(
            Pos2::new(42.0, h - 88.0),
            Align2::LEFT_TOP,
            "VITALS",
            10.0,
            MUTED,
        );
        text(
            Pos2::new(42.0, h - 68.0),
            Align2::LEFT_TOP,
            &format!("+  {:03}", sim.health.ceil() as u32),
            30.0,
            if sim.health < 30.0 { AMBER } else { WHITE },
        );
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(152.0, h - 46.0), Vec2::new(100.0, 4.0)),
            0.0,
            Color32::from_white_alpha(35),
        );
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(152.0, h - 46.0),
                Vec2::new(100.0 * (sim.health / sim.world.player.health), 4.0),
            ),
            0.0,
            MINT,
        );
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(w - 220.0, h - 100.0), Vec2::new(196.0, 76.0)),
            3.0,
            Color32::from_black_alpha(155),
        );
        text(
            Pos2::new(w - 202.0, h - 87.0),
            Align2::LEFT_TOP,
            "AR-01  /  PULSE RIFLE",
            10.0,
            MUTED,
        );
        text(
            Pos2::new(w - 200.0, h - 68.0),
            Align2::LEFT_TOP,
            &format!("{:02}", sim.ammo),
            34.0,
            if sim.ammo < 6 { AMBER } else { WHITE },
        );
        text(
            Pos2::new(w - 140.0, h - 48.0),
            Align2::LEFT_CENTER,
            &format!("/ {:02}   INF", sim.world.weapon.magazine),
            17.0,
            MUTED,
        );
        if sim.reload_left > 0.0 {
            text(
                Pos2::new(w * 0.5, h * 0.62),
                Align2::CENTER_TOP,
                "RELOADING",
                12.0,
                MINT,
            );
            let p = 1.0 - sim.reload_left / sim.world.weapon.reload_seconds;
            painter.line_segment(
                [
                    Pos2::new(w * 0.5 - 50.0, h * 0.62 + 22.0),
                    Pos2::new(w * 0.5 - 50.0 + 100.0 * p, h * 0.62 + 22.0),
                ],
                Stroke::new(2.0_f32, MINT),
            );
        }
        let vp = sim.camera_projection(w / h)
            * Mat4::look_at_rh(sim.eye(), sim.eye() + sim.forward(), Vec3::Y);
        for i in 0..sim.actors.positions.len() {
            if sim.actors.health[i] <= 0.0 {
                continue;
            }
            let p = sim.actors.positions[i];
            if p.distance(sim.position) > 32.0 || !sim.actor_visible(i) {
                continue;
            }
            let clip = vp * (p + Vec3::Y * 1.15).extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.x.abs() > 0.95 || ndc.y.abs() > 0.9 {
                continue;
            }
            let screen = Pos2::new((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h);
            painter.rect_filled(
                Rect::from_min_size(screen - Vec2::new(20.0, 0.0), Vec2::new(40.0, 2.0)),
                0.0,
                Color32::from_black_alpha(180),
            );
            painter.rect_filled(
                Rect::from_min_size(
                    screen - Vec2::new(20.0, 0.0),
                    Vec2::new(
                        40.0 * (sim.actors.health[i] / sim.max_health(i)).clamp(0.0, 1.0),
                        2.0,
                    ),
                ),
                0.0,
                AMBER,
            );
        }
        if sim.damage_flash > 0.0 {
            painter.rect_stroke(
                rect.shrink(4.0),
                0.0,
                Stroke::new(
                    10.0_f32,
                    Color32::from_rgba_unmultiplied(
                        255,
                        65,
                        50,
                        (sim.damage_flash * 400.0).min(160.0) as u8,
                    ),
                ),
                egui::StrokeKind::Inside,
            );
        }
        if sim.victory() || sim.health <= 0.0 {
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            egui::Area::new("result".into())
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(if sim.victory() {
                                "SIMULATION COMPLETE"
                            } else {
                                "SIGNAL LOST"
                            })
                            .size(34.0)
                            .strong()
                            .color(if sim.victory() {
                                MINT
                            } else {
                                AMBER
                            }),
                        );
                        ui.label(format!(
                            "{} eliminations  /  {} shots  /  {:.0}% accuracy",
                            sim.kills,
                            sim.shots,
                            sim.hits as f32 / sim.shots.max(1) as f32 * 100.0
                        ));
                        ui.add_space(15.0);
                        if ui.button("RESTART SIMULATION  >").clicked() {
                            actions.push(Action::Restart);
                        }
                    });
                });
        }
    }
    if state.console {
        egui::Window::new("WORLD CONSOLE  /  AI PROTOCOL").default_pos(Pos2::new((w-480.0).max(10.0),100.0)).default_width(435.0).resizable(true).show(ctx,|ui| {
            ui.label(RichText::new(format!("REV {:03}  •  127.0.0.1:{}  •  SIMULATION PAUSED",sim.rev,state.port)).size(10.0).color(MINT));
            ui.label(RichText::new("Typed edits > validation > atomic commit > live reload").size(11.0).color(MUTED));
            ui.horizontal(|ui| {for (label,path,value) in [("12 SENTINELS","enemies.count","12"),("SLOW AI","enemies.speed","0.8"),("POWER RIFLE","weapon.damage","100")] {
                if ui.small_button(label).clicked(){actions.push(Action::Apply(format!(r#"{{"op":"apply","expected_rev":{},"edits":[{{"kind":"set","path":"{}","value":{}}}]}}"#,sim.rev,path,value)));}
            }});
            ui.add(egui::TextEdit::multiline(&mut state.command).font(egui::TextStyle::Monospace).desired_rows(8).desired_width(f32::INFINITY));
            if ui.add(egui::Button::new("EXECUTE TRANSACTION  >").fill(Color32::from_rgb(29,82,68))).clicked(){actions.push(Action::Apply(state.command.clone()));}
            ui.separator();egui::ScrollArea::vertical().max_height(160.0).stick_to_bottom(true).show(ui,|ui| {for line in &state.output {ui.label(RichText::new(line).monospace().size(11.0).color(if line.contains("ERROR"){AMBER}else{MINT}));}});
            ui.separator();ui.label(RichText::new(&state.gpu).size(10.0).color(MUTED));
            ui.label(RichText::new("TAB to close  /  No Rust rebuild required").size(10.0).color(MUTED));
        });
    }
    actions
}
