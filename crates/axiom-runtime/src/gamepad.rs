//! Gamepads: XInput state of up to four controllers, polled once per frame on the main thread
//! by the native host, and the 0..1 levels of the named inputs (`axiom_core::PAD_INPUTS`) that
//! `input.bindings.gamepad` binds to actions ([`crate::ActionMap::from_input`]).
//!
//! `XInputGetState` on an empty slot is slow (it searches for the device), so only connected
//! slots are read every frame; empty ones are probed once per [`PROBE_INTERVAL`]. A controller
//! that disappears reads as disconnected on the next poll (its levels drop to 0). Headless
//! runs, tests and the editor's automated hosts never poll: their pad state is empty unless a
//! scenario injects one.
use axiom_core::{PAD_INPUTS, PAD_TRIGGER_THRESHOLD};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const PADS: usize = 4;
/// How often an empty slot is probed for a newly plugged controller.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// Number of named inputs ([`PAD_INPUTS`]).
pub const INPUTS: usize = PAD_INPUTS.len();

/// XInput button bits of `PAD_INPUTS[0..14]` (same order).
const BUTTON_BITS: [u16; 14] = [
    0x1000, 0x2000, 0x4000, 0x8000, // A B X Y
    0x0100, 0x0200, // LB RB
    0x0020, 0x0010, // Back Start
    0x0040, 0x0080, // LS RS (stick clicks)
    0x0001, 0x0002, 0x0004, 0x0008, // D-pad up down left right
];

/// One controller as XInput reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    /// Thumb sticks, x then y (y up), -32768..32767.
    pub left: [i16; 2],
    pub right: [i16; 2],
}

/// Radial deadzone: inside `deadzone` (0..1 of full deflection) the stick is centred; outside
/// it the magnitude is rescaled to 0..1 so the response has no step at the edge.
fn stick(raw: [i16; 2], deadzone: f32) -> [f32; 2] {
    let x = (f32::from(raw[0]) / 32767.0).clamp(-1.0, 1.0);
    let y = (f32::from(raw[1]) / 32767.0).clamp(-1.0, 1.0);
    let m = (x * x + y * y).sqrt();
    if m <= deadzone || m <= 0.0 {
        return [0.0, 0.0];
    }
    let scaled = ((m - deadzone) / (1.0 - deadzone)).min(1.0);
    [x / m * scaled, y / m * scaled]
}

fn trigger(raw: u8) -> f32 {
    if raw <= PAD_TRIGGER_THRESHOLD {
        0.0
    } else {
        f32::from(raw - PAD_TRIGGER_THRESHOLD) / f32::from(255 - PAD_TRIGGER_THRESHOLD)
    }
}

impl PadState {
    /// Level 0..1 of every named input, in [`PAD_INPUTS`] order.
    pub fn levels(&self, deadzone: f32) -> [f32; INPUTS] {
        let mut out = [0.0; INPUTS];
        for (o, bit) in out.iter_mut().zip(BUTTON_BITS) {
            *o = f32::from(u8::from(self.buttons & bit != 0));
        }
        out[14] = trigger(self.left_trigger);
        out[15] = trigger(self.right_trigger);
        for (base, raw) in [(16, self.left), (20, self.right)] {
            let [x, y] = stick(raw, deadzone);
            out[base] = y.max(0.0);
            out[base + 1] = (-y).max(0.0);
            out[base + 2] = (-x).max(0.0);
            out[base + 3] = x.max(0.0);
        }
        out
    }
}

/// The controllers of one frame (`None` = slot empty).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pads(pub [Option<PadState>; PADS]);

impl Pads {
    pub fn connected(&self) -> usize {
        self.0.iter().flatten().count()
    }
    /// Levels of the named inputs over every connected controller (the strongest wins), so
    /// any controller drives a single-player game.
    pub fn levels(&self, deadzone: f32) -> [f32; INPUTS] {
        let mut out = [0.0f32; INPUTS];
        for pad in self.0.iter().flatten() {
            for (o, l) in out.iter_mut().zip(pad.levels(deadzone)) {
                *o = o.max(l);
            }
        }
        out
    }
}

/// A controller state written by hand: scenarios and tests inject pads with it (values as a
/// game would see them before the deadzone: sticks -1..1, y up; triggers 0..1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PadInput {
    /// Held buttons by name (`PadA`, `PadLB`, `PadDpadUp`, `PadStart`, ...).
    #[serde(default)]
    pub buttons: Vec<String>,
    #[serde(default)]
    pub left_stick: [f32; 2],
    #[serde(default)]
    pub right_stick: [f32; 2],
    #[serde(default)]
    pub left_trigger: f32,
    #[serde(default)]
    pub right_trigger: f32,
}

impl PadInput {
    pub fn state(&self) -> anyhow::Result<PadState> {
        let unit = |v: f32| v.is_finite() && (-1.0..=1.0).contains(&v);
        anyhow::ensure!(
            self.left_stick
                .iter()
                .chain(&self.right_stick)
                .all(|v| unit(*v))
                && [self.left_trigger, self.right_trigger]
                    .iter()
                    .all(|v| unit(*v) && *v >= 0.0),
            "pad sticks must be -1..1 and triggers 0..1"
        );
        let mut buttons = 0;
        for name in &self.buttons {
            let bit = PAD_INPUTS[..BUTTON_BITS.len()]
                .iter()
                .position(|n| n == name)
                .map(|i| BUTTON_BITS[i])
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown pad button {name} (use {})",
                        PAD_INPUTS[..BUTTON_BITS.len()].join(", ")
                    )
                })?;
            buttons |= bit;
        }
        let axis = |v: f32| (v * 32767.0).round() as i16;
        let pull = |v: f32| (v * 255.0).round() as u8;
        Ok(PadState {
            buttons,
            left_trigger: pull(self.left_trigger),
            right_trigger: pull(self.right_trigger),
            left: [axis(self.left_stick[0]), axis(self.left_stick[1])],
            right: [axis(self.right_stick[0]), axis(self.right_stick[1])],
        })
    }
}

/// The native host's controllers.
pub struct Gamepads {
    enabled: bool,
    pads: Pads,
    /// When each empty slot is probed next.
    probe: [Option<Instant>; PADS],
    polls: u64,
    poll_us_max: u64,
    poll_us_last: u64,
    plugged: u64,
    unplugged: u64,
}

impl Gamepads {
    /// `enabled: false` never calls XInput (tests, headless hosts).
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: enabled && cfg!(windows),
            pads: Pads::default(),
            probe: [None; PADS],
            polls: 0,
            poll_us_max: 0,
            poll_us_last: 0,
            plugged: 0,
            unplugged: 0,
        }
    }

    /// Reads the controllers (once per frame; never blocks beyond the XInput calls).
    pub fn poll(&mut self) -> &Pads {
        if !self.enabled {
            return &self.pads;
        }
        let started = Instant::now();
        for slot in 0..PADS {
            let connected = self.pads.0[slot].is_some();
            if !connected && self.probe[slot].is_some_and(|at| started < at) {
                continue;
            }
            let read = read_slot(slot as u32);
            match (connected, read.is_some()) {
                (false, true) => self.plugged += 1,
                (true, false) => self.unplugged += 1,
                _ => {}
            }
            self.pads.0[slot] = read;
            self.probe[slot] = read.is_none().then(|| started + PROBE_INTERVAL);
        }
        let us = started.elapsed().as_micros() as u64;
        self.polls += 1;
        self.poll_us_last = us;
        self.poll_us_max = self.poll_us_max.max(us);
        &self.pads
    }

    /// Whether this host reads hardware (the native window on Windows).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn observation(&self) -> Value {
        json!({"backend": if self.enabled {"xinput"} else {"disabled"},
            "connected": self.pads.0.iter().map(Option::is_some).collect::<Vec<_>>(),
            "polls": self.polls, "poll_us_last": self.poll_us_last, "poll_us_max": self.poll_us_max,
            "plugged": self.plugged, "unplugged": self.unplugged,
            "probe_interval_ms": PROBE_INTERVAL.as_millis() as u64})
    }
}

#[cfg(windows)]
fn read_slot(slot: u32) -> Option<PadState> {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    let mut state = XINPUT_STATE::default();
    // SAFETY: `state` is a valid, writable XINPUT_STATE for the duration of the call.
    let code = unsafe { XInputGetState(slot, &mut state) };
    (code == 0).then(|| {
        let g = state.Gamepad;
        PadState {
            buttons: g.wButtons.0,
            left_trigger: g.bLeftTrigger,
            right_trigger: g.bRightTrigger,
            left: [g.sThumbLX, g.sThumbLY],
            right: [g.sThumbRX, g.sThumbRY],
        }
    })
}
#[cfg(not(windows))]
fn read_slot(_: u32) -> Option<PadState> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_apply_the_deadzone_and_rescale() {
        let mut p = PadState {
            buttons: 0x1000 | 0x0001,
            left_trigger: 30,
            right_trigger: 255,
            left: [0, 32767],
            right: [-32768, 0],
        };
        let l = p.levels(0.24);
        let at = |name: &str| l[PAD_INPUTS.iter().position(|n| *n == name).unwrap()];
        assert_eq!((at("PadA"), at("PadB"), at("PadDpadUp")), (1.0, 0.0, 1.0));
        assert_eq!((at("PadLT"), at("PadRT")), (0.0, 1.0));
        assert_eq!((at("PadLStickUp"), at("PadLStickDown")), (1.0, 0.0));
        assert_eq!((at("PadRStickLeft"), at("PadRStickRight")), (1.0, 0.0));
        // Inside the deadzone: centred. Just outside: small, continuous.
        p.left = [(0.2 * 32767.0) as i16, 0];
        assert_eq!(p.levels(0.24)[19], 0.0);
        p.left = [(0.25 * 32767.0) as i16, 0];
        let edge = p.levels(0.24)[19];
        assert!(edge > 0.0 && edge < 0.02, "{edge}");
        // Diagonal full deflection keeps unit length.
        p.left = [23170, 23170];
        let l = p.levels(0.24);
        assert!((l[16].hypot(l[19]) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn pads_merge_and_injected_state_round_trips() {
        let input: PadInput = serde_json::from_value(
            json!({"buttons":["PadA","PadRB"],"left_stick":[0.0,1.0],"right_trigger":0.5}),
        )
        .unwrap();
        let state = input.state().unwrap();
        assert_eq!(state.buttons, 0x1000 | 0x0200);
        assert_eq!(state.left, [0, 32767]);
        assert_eq!(state.right_trigger, 128);
        let mut pads = Pads::default();
        assert_eq!(pads.levels(0.24), [0.0; INPUTS]);
        pads.0[2] = Some(state);
        pads.0[0] = Some(PadState {
            left: [0, -32767],
            ..Default::default()
        });
        let l = pads.levels(0.24);
        assert_eq!((l[16], l[17], l[0], l[5]), (1.0, 1.0, 1.0, 1.0));
        assert_eq!(pads.connected(), 2);
        let bad = |v: Value| {
            serde_json::from_value::<PadInput>(v)
                .map_err(anyhow::Error::from)
                .and_then(|p| p.state())
                .unwrap_err()
                .to_string()
        };
        assert!(bad(json!({"buttons":["PadZ"]})).contains("unknown pad button"));
        assert!(bad(json!({"buttons":["PadLStickUp"]})).contains("unknown pad button"));
        assert!(bad(json!({"left_stick":[2.0,0.0]})).contains("-1..1"));
        assert!(bad(json!({"left_trigger":-0.5})).contains("0..1"));
        assert!(bad(json!({"rumble":1})).contains("unknown field"));
    }

    #[test]
    fn a_disabled_host_never_polls() {
        let mut g = Gamepads::new(false);
        assert_eq!(g.poll().connected(), 0);
        assert_eq!(g.observation()["backend"], "disabled");
        assert_eq!(g.observation()["polls"], 0);
    }
}
