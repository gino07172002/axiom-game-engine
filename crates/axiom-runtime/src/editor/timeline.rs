//! Inspector ▸ Tween timeline: one strip per clip with a row of key markers per track.
//! Scrubbing moves the playhead and previews the clip in the scene view for the current frame
//! only (edit mode, nothing is written); key edits and options go through `axiom_core::ops`
//! like their ACL verbs (`tween ID key ...`, `tween ID clip NAME set ...`).
use axiom_core::{
    ops::TweenClipOptions,
    tween::{TweenClips, TweenMode, PROPERTIES},
};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui};

const ROW: f32 = 18.0;
const RULER: f32 = 16.0;
const LABELS: f32 = 74.0;
const KEY: Color32 = Color32::from_rgb(105, 222, 195);
const SELECTED: Color32 = Color32::from_rgb(255, 178, 111);
const PLAYHEAD: Color32 = Color32::from_rgb(240, 72, 72);
const GRID: Color32 = Color32::from_rgb(52, 62, 78);
const TEXT: Color32 = Color32::from_rgb(155, 174, 194);

/// Editor-only timeline state (never in revisions).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TweenUi {
    /// Clip shown on the timeline (default: the first).
    pub clip: Option<String>,
    /// Playhead (seconds).
    pub time: f32,
    /// Track that Record key writes to.
    pub track: usize,
    /// Selected key of `track`.
    pub key: Option<usize>,
    /// The scene view shows the clip at the playhead (edit mode).
    pub preview: bool,
    /// Key being dragged: (track, index, time).
    pub drag: Option<(usize, usize, f32)>,
    /// Runtime playback (clip, time) of the selected object during Play preview.
    pub live: Option<(String, f32)>,
    /// Screen rect of the strip drawn last frame (tests, remote capture).
    pub strip: Option<Rect>,
}

/// What the timeline asks the editor to do after drawing.
pub(super) enum TweenRequest {
    Command(&'static str),
    MoveKey {
        clip: String,
        track: usize,
        index: usize,
        time: f32,
    },
    Options {
        clip: String,
        options: TweenClipOptions,
    },
    AddTrack {
        clip: String,
        property: String,
    },
    RemoveClip(String),
}

fn track_label(target: &str, property: &str) -> String {
    let short = property.rsplit('/').next().unwrap_or(property);
    let short = if property.starts_with("render.light/") {
        format!("light {short}")
    } else if property.starts_with("render.mesh/") {
        format!("mesh {short}")
    } else if property.starts_with("behavior.program/") {
        format!("var {short}")
    } else {
        short.to_owned()
    };
    match target.rsplit('/').next() {
        Some(t) if !target.is_empty() => format!("{t}·{short}"),
        _ => short,
    }
}

/// Ruler step (seconds) that keeps at most about 8 labels.
fn ruler_step(length: f32) -> f32 {
    [
        0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0,
    ]
    .into_iter()
    .find(|s| length / s <= 8.0)
    .unwrap_or(1200.0)
}

/// Draws the timeline of `clips` (authored) into `ui`. `live` is the runtime playback
/// (clip, time) during Play preview, which then drives the playhead and disables edits.
pub(super) fn timeline(
    ui: &mut Ui,
    clips: &TweenClips,
    state: &mut TweenUi,
    live: Option<(&str, f32)>,
    editable: bool,
) -> Option<TweenRequest> {
    let mut request = None;
    let names: Vec<&String> = clips.clips.keys().collect();
    if state
        .clip
        .as_ref()
        .is_none_or(|c| !clips.clips.contains_key(c))
    {
        state.clip = names.first().map(|n| (*n).clone());
        state.key = None;
    }
    let name = state.clip.clone()?;
    let clip = &clips.clips[&name];
    ui.horizontal_wrapped(|ui| {
        for n in &names {
            if ui
                .selectable_label(state.clip.as_ref() == Some(*n), n.as_str())
                .clicked()
            {
                state.clip = Some((*n).clone());
                state.key = None;
                state.track = 0;
            }
        }
    });
    let length = clip.length();
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal(|ui| {
            let mut mode = clip.mode;
            egui::ComboBox::from_id_salt(("tween_mode", &name))
                .width(86.0)
                .selected_text(match mode {
                    TweenMode::Once => "once",
                    TweenMode::Loop => "loop",
                    TweenMode::PingPong => "ping_pong",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, TweenMode::Once, "once");
                    ui.selectable_value(&mut mode, TweenMode::Loop, "loop");
                    ui.selectable_value(&mut mode, TweenMode::PingPong, "ping_pong");
                });
            if mode != clip.mode {
                request = Some(TweenRequest::Options {
                    clip: name.clone(),
                    options: TweenClipOptions {
                        mode: Some(mode),
                        // loop / ping_pong never finish: drop the event instead of failing.
                        on_finish: (mode != TweenMode::Once && !clip.on_finish.is_empty())
                            .then(String::new),
                        ..Default::default()
                    },
                });
            }
            let mut autoplay = clip.autoplay;
            if ui.checkbox(&mut autoplay, "Autoplay").changed() {
                request = Some(TweenRequest::Options {
                    clip: name.clone(),
                    options: TweenClipOptions {
                        autoplay: Some(autoplay),
                        ..Default::default()
                    },
                });
            }
            ui.label(
                RichText::new(format!("{length:.2} s · ×{}", clip.speed))
                    .size(11.0)
                    .color(TEXT),
            );
        });
    });
    let rows = clip.tracks.len();
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, RULER + rows as f32 * ROW + 4.0),
        Sense::click_and_drag(),
    );
    state.strip = Some(rect);
    let lane = Rect::from_min_max(pos2(rect.left() + LABELS, rect.top()), rect.right_bottom());
    let span = (lane.width() - 8.0).max(1.0);
    let x_of = |t: f32| lane.left() + 4.0 + t / length * span;
    let t_of = |x: f32| ((x - lane.left() - 4.0) / span * length).clamp(0.0, length);
    let row_y = |r: usize| rect.top() + RULER + r as f32 * ROW + ROW * 0.5;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(18, 22, 30));
    let step = ruler_step(length);
    let mut t = 0.0;
    while t <= length + 1e-4 {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, rect.top() + RULER - 4.0), pos2(x, rect.bottom())],
            Stroke::new(1.0f32, GRID),
        );
        // Labels near the right end hang left of their tick so they stay inside the strip.
        let (at, align) = if x + 28.0 > rect.right() {
            (pos2(x - 2.0, rect.top() + 1.0), egui::Align2::RIGHT_TOP)
        } else {
            (pos2(x + 2.0, rect.top() + 1.0), egui::Align2::LEFT_TOP)
        };
        painter.text(
            at,
            align,
            format!("{t:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.'),
            egui::FontId::monospace(9.0),
            TEXT,
        );
        t += step;
    }
    let pointer = response.interact_pointer_pos();
    // Press: pick a key marker (drag) or a row / the ruler (scrub). A drag starts only after
    // the pointer moved, so hit-test where the button went down.
    if response.drag_started() || response.clicked() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        if let Some(p) = origin {
            let row = ((p.y - rect.top() - RULER) / ROW).floor();
            let row = (row >= 0.0 && (row as usize) < rows).then_some(row as usize);
            let hit = row.and_then(|r| {
                clip.tracks[r]
                    .keys
                    .iter()
                    .position(|k| (x_of(k.t) - p.x).abs() <= 6.0)
                    .map(|i| (r, i))
            });
            if let Some(r) = row {
                state.track = r;
            }
            match hit {
                Some((r, i)) => {
                    state.key = Some(i);
                    if editable && response.drag_started() {
                        state.drag = Some((r, i, clip.tracks[r].keys[i].t));
                    }
                }
                None if p.x >= lane.left() => {
                    state.key = None;
                    if live.is_none() {
                        state.time = t_of(p.x);
                        state.preview = true;
                    }
                }
                None => state.key = None,
            }
        }
    }
    if response.dragged() {
        if let Some(p) = pointer {
            match &mut state.drag {
                Some((_, _, t)) => *t = (t_of(p.x) * 100.0).round() / 100.0,
                None if live.is_none() && p.x >= lane.left() => {
                    state.time = t_of(p.x);
                    state.preview = true;
                }
                None => {}
            }
        }
    }
    if response.drag_stopped() {
        if let Some((track, index, time)) = state.drag.take() {
            if (clip.tracks[track].keys[index].t - time).abs() > 1e-4 {
                request = Some(TweenRequest::MoveKey {
                    clip: name.clone(),
                    track,
                    index,
                    time,
                });
            }
        }
    }
    for (r, track) in clip.tracks.iter().enumerate() {
        let y = row_y(r);
        if r == state.track {
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(rect.left(), y - ROW * 0.5),
                    pos2(rect.right(), y + ROW * 0.5),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(105, 222, 195, 18),
            );
        }
        painter.text(
            pos2(rect.left() + 4.0, y),
            egui::Align2::LEFT_CENTER,
            track_label(&track.target, &track.property),
            egui::FontId::proportional(10.0),
            if r == state.track { KEY } else { TEXT },
        );
        painter.line_segment(
            [pos2(lane.left(), y), pos2(lane.right(), y)],
            Stroke::new(1.0f32, GRID),
        );
        for (i, key) in track.keys.iter().enumerate() {
            let t = match state.drag {
                Some((dr, di, dt)) if dr == r && di == i => dt,
                _ => key.t,
            };
            let selected = r == state.track && state.key == Some(i);
            let c = pos2(x_of(t), y);
            let s = if selected { 5.5 } else { 4.5 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x, c.y - s),
                    pos2(c.x + s, c.y),
                    pos2(c.x, c.y + s),
                    pos2(c.x - s, c.y),
                ],
                if selected { SELECTED } else { KEY },
                Stroke::new(1.0f32, Color32::from_rgb(18, 22, 30)),
            ));
        }
    }
    let playhead = match live {
        Some((c, t)) if c == name => Some(t),
        Some(_) => None,
        None => Some(state.time.min(length)),
    };
    if let Some(t) = playhead {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5f32, PLAYHEAD),
        );
    }
    let track = clip.tracks.get(state.track.min(rows - 1));
    ui.label(
        RichText::new(match (live, playhead) {
            (Some(_), Some(t)) => format!("live {t:.2} s (Play)"),
            (Some((c, _)), None) => format!("live: {c} is playing"),
            _ => format!(
                "playhead {:.2} s · {}{}",
                state.time,
                track.map_or(String::new(), |t| track_label(&t.target, &t.property)),
                if state.preview { " · previewing" } else { "" }
            ),
        })
        .size(11.0)
        .color(TEXT),
    );
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Record key")
                .on_hover_text("Store the track's current value at the playhead")
                .clicked()
            {
                request = Some(TweenRequest::Command("tween.record_key"));
            }
            if ui
                .add_enabled(state.key.is_some(), egui::Button::new("Delete key"))
                .clicked()
            {
                request = Some(TweenRequest::Command("tween.remove_key"));
            }
            ui.menu_button("Add track", |ui| {
                for property in PROPERTIES {
                    if ui.button(property).clicked() {
                        request = Some(TweenRequest::AddTrack {
                            clip: name.clone(),
                            property: property.to_owned(),
                        });
                        ui.close_menu();
                    }
                }
            });
            if state.preview && ui.button("Stop preview").clicked() {
                state.preview = false;
            }
            if ui.small_button("Remove clip").clicked() {
                request = Some(TweenRequest::RemoveClip(name.clone()));
            }
        });
    });
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2};

    fn clips() -> TweenClips {
        serde_json::from_value(serde_json::json!({"clips":{"open":{"tracks":[
            {"property":"core.transform/yaw","keys":[{"t":0,"v":0},{"t":1,"v":90}]}]}}}))
        .unwrap()
    }

    /// One frame with `events`; returns the timeline's request.
    fn frame(ctx: &egui::Context, state: &mut TweenUi, events: Vec<Event>) -> Option<TweenRequest> {
        let clips = clips();
        let mut request = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    request = timeline(ui, &clips, state, None, true);
                });
            },
        );
        request
    }

    fn press(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn click_scrubs_and_dragging_a_key_requests_a_move() {
        let ctx = egui::Context::default();
        let mut state = TweenUi::default();
        frame(&ctx, &mut state, vec![]);
        let strip = state.strip.unwrap();
        let x = |t: f32| strip.left() + LABELS + 4.0 + t * (strip.width() - LABELS - 8.0);
        let row = strip.top() + RULER + ROW * 0.5;
        // Click between the keys: the playhead moves there and the scene previews it.
        let at = pos2(x(0.5), row);
        frame(
            &ctx,
            &mut state,
            vec![Event::PointerMoved(at), press(at, true)],
        );
        frame(&ctx, &mut state, vec![press(at, false)]);
        assert!((state.time - 0.5).abs() < 0.01, "{}", state.time);
        assert!(state.preview && state.key.is_none());
        // Drag the key at 1 s to 0.75 s: one move request on release (snapped to 0.01 s).
        let from = pos2(x(1.0), row);
        let to = pos2(x(0.75), row);
        let mut requests = vec![];
        for events in [
            vec![Event::PointerMoved(from), press(from, true)],
            vec![Event::PointerMoved(pos2(x(0.9), row))],
            vec![Event::PointerMoved(to)],
            vec![press(to, false)],
            vec![],
        ] {
            requests.extend(frame(&ctx, &mut state, events));
        }
        assert_eq!(requests.len(), 1);
        match &requests[0] {
            TweenRequest::MoveKey {
                clip,
                track,
                index,
                time,
            } => assert_eq!((clip.as_str(), *track, *index, *time), ("open", 0, 1, 0.75)),
            _ => panic!("expected a key move"),
        }
        assert_eq!(state.key, Some(1));
        assert!(state.drag.is_none());
    }
}
