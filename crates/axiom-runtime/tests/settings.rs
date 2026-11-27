//! Player settings in the runtime (docs/plan-player-settings.zh-TW.md S1): the typed
//! `setting` command, author defaults, the town's settings page slider, and the rule that
//! settings are not simulation state (not in checkpoints or state_hash, never batched).
use axiom_core::gameplay::RuntimeCommand;
use axiom_core::{ContentPackage, World};
use axiom_runtime::{installed_registry, ui::Action, ui::UiState, ActionMap, WorldRuntime};
use egui::{Pos2, Rect, Vec2};
use serde_json::json;

fn town() -> World {
    ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/axiom-town"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world
}

fn setting(key: &str, value: serde_json::Value) -> RuntimeCommand {
    RuntimeCommand::Setting {
        key: key.into(),
        value,
    }
}

#[test]
fn the_setting_command_changes_player_settings_and_observe_reports_them() {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    // The town's author default.
    assert_eq!(r.player_settings().0.master, 0.8);
    assert_eq!(
        r.observation().metrics["settings"]["audio.master"],
        json!(0.8)
    );
    let before = axiom_runtime::scenario::state_hash(&r).unwrap();
    let rev = r.player_settings().1;
    let reply = r.command(&setting("audio.master", json!(0.3))).unwrap();
    assert_eq!(reply["settings"]["audio.master"], json!(0.3));
    assert_eq!(r.player_settings().1, rev + 1);
    assert_eq!(
        axiom_runtime::scenario::state_hash(&r).unwrap(),
        before,
        "settings are not simulation state"
    );
    // The same value again is not a change.
    r.command(&setting("audio.master", json!(0.3))).unwrap();
    assert_eq!(r.player_settings().1, rev + 1);
    for (command, needle) in [
        (setting("audio.master", json!(3)), "0..1"),
        (setting("audio.master", json!(null)), "needs a value"),
        (setting("nope", json!(1)), "unknown setting"),
        (
            RuntimeCommand::Batch {
                commands: vec![setting("audio.master", json!(0.5))],
            },
            "batch",
        ),
    ] {
        let e = r.command(&command).expect_err(needle);
        assert!(format!("{e:#}").contains(needle), "{e:#}");
    }
    assert_eq!(r.player_settings().0.master, 0.3);
}

#[test]
fn checkpoints_do_not_carry_player_settings() {
    // ADV enables saves. A load keeps the player's current settings.
    let adv = ContentPackage::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/adv"),
        &installed_registry().authoring,
    )
    .unwrap()
    .world;
    let mut r = WorldRuntime::new(adv, 1, installed_registry()).unwrap();
    r.command(&setting("audio.master", json!(0.4))).unwrap();
    let saved = r.checkpoint().unwrap();
    r.command(&setting("audio.master", json!(0.9))).unwrap();
    r.restore_checkpoint(&saved).unwrap();
    assert_eq!(r.player_settings().0.master, 0.9);
}

#[test]
fn the_settings_page_slider_sends_a_setting_and_shows_its_value() {
    let mut r = WorldRuntime::new(town(), 1, installed_registry()).unwrap();
    r.step(&ActionMap::default());
    for page in ["pause", "settings"] {
        r.command(&RuntimeCommand::Open { page: page.into() })
            .unwrap();
    }
    // The slider's command is a setting: a slide changes the player setting.
    let reply = r
        .command(&RuntimeCommand::Slide {
            widget: "settings_master".into(),
            value: 0.52,
        })
        .unwrap();
    assert_eq!(reply["value"], json!(0.5), "snapped to the 0.05 step");
    assert_eq!(r.player_settings().0.master, 0.5);
    // Drawn: the slider shows the setting's value.
    let ctx = egui::Context::default();
    let mut ui = UiState {
        playing: true,
        ..Default::default()
    };
    let mut texts = vec![];
    for _ in 0..4 {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
            ..Default::default()
        };
        let out = ctx.run(raw, |ctx| {
            let actions: Vec<Action> = r.draw_ui(ctx, &mut ui);
            assert!(actions.is_empty());
        });
        texts = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
    }
    assert!(
        ui.targets.contains_key("slider/settings_master"),
        "{:?}",
        ui.targets.keys()
    );
    assert!(texts.iter().any(|t| t == "總音量"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.starts_with("0.5")),
        "the value box: {texts:?}"
    );
    // The pause menu under the settings page is not drawn while settings is on top.
    assert!(!texts.iter().any(|t| t == "繼續遊戲"), "{texts:?}");
    assert!(!ui.targets.contains_key("button/pause_resume"));
}
