//! Builds the pipelines `render.environment.post` needs on a worker thread, never on the main
//! thread (AGENTS.md 16 ms rule): the five post pipelines (`hdr.rs`) and, for HDR, the
//! `Rgba16Float` variants of the cube / sky, mesh and VFX pipelines and of the active material
//! programs. Until everything a setting needs is installed, the renderer keeps drawing the
//! legacy path; the results are installed together in one frame.
//!
//! One job at a time (`jobs::Job`: cooperative cancellation between pipelines, countable
//! progress, watchdog, timed join on shutdown). A request that changes while a job runs stops
//! it at its next pipeline (what it built is kept) and the current request starts once it has
//! ended: the latest request wins, nothing queues. A failed or timed
//! out build is reported (`observe` `render_post.build`, status bar) and not retried until the
//! request changes.
use crate::hdr::{Pipelines, PostInputs, Settings, HDR_FORMAT};
use crate::jobs::{Job, Outcome, Progress, Stop};
use crate::shader_gpu::{Compiler, Program};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Watchdog of one build (debug builds on a busy MX150 measured up to ~0.6 s).
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(30);

/// What a build must produce: the post pipelines and, with `hdr`, the HDR world variants and
/// the HDR programs of these material hooks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Need {
    pub post: bool,
    pub hdr: bool,
    /// `(id, hook)` of material programs without an HDR variant.
    pub programs: Vec<(String, String)>,
}
impl Need {
    pub fn is_empty(&self) -> bool {
        !self.post && !self.hdr && self.programs.is_empty()
    }
}

/// Cloned wgpu handles the worker builds from (all `Send`; wgpu 24 handles are `Clone`).
pub(crate) struct Inputs {
    pub device: wgpu::Device,
    pub post: Option<PostInputs>,
    pub scene: Option<(wgpu::PipelineLayout, wgpu::ShaderModule)>,
    pub mesh: Option<(wgpu::PipelineLayout, wgpu::ShaderModule)>,
    pub vfx: Option<(wgpu::PipelineLayout, wgpu::ShaderModule)>,
    pub compiler: Compiler,
    pub programs: Vec<(String, String)>,
}

/// Pipelines one build made (`None` = not asked for, or cancelled before it).
#[derive(Default)]
pub(crate) struct Built {
    pub post: Option<Pipelines>,
    pub scene: Option<[wgpu::RenderPipeline; 3]>,
    pub mesh: Option<(Vec<wgpu::RenderPipeline>, wgpu::RenderPipeline)>,
    pub vfx: Option<Vec<wgpu::RenderPipeline>>,
    pub programs: Vec<(String, Program)>,
    pub ms: f64,
}

/// The worker: one step per pipeline group, checking cancellation between steps.
fn build(inputs: Inputs, progress: &Progress) -> Built {
    let started = Instant::now();
    let device = &inputs.device;
    let steps = [
        inputs.post.is_some(),
        inputs.scene.is_some(),
        inputs.mesh.is_some(),
        inputs.vfx.is_some(),
    ]
    .iter()
    .filter(|s| **s)
    .count()
        + inputs.programs.len();
    progress.set_total(steps as u64);
    let mut out = Built::default();
    let mut done = 0;
    let mut step = |progress: &Progress| {
        done += 1;
        progress.set_done(done);
        !progress.cancelled()
    };
    'work: {
        if let Some(post) = &inputs.post {
            out.post = Some(post.build_pipelines(device));
            if !step(progress) {
                break 'work;
            }
        }
        if let Some((layout, shader)) = &inputs.scene {
            out.scene = Some(crate::render::scene_color_pipelines(
                device, layout, shader, HDR_FORMAT,
            ));
            if !step(progress) {
                break 'work;
            }
        }
        if let Some((layout, shader)) = &inputs.mesh {
            out.mesh = Some(crate::mesh_render::color_pipelines(
                device, layout, shader, HDR_FORMAT,
            ));
            if !step(progress) {
                break 'work;
            }
        }
        if let Some((layout, module)) = &inputs.vfx {
            out.vfx = Some(crate::vfx_gpu::draw_pipelines(
                device, layout, module, HDR_FORMAT,
            ));
            if !step(progress) {
                break 'work;
            }
        }
        for (id, hook) in &inputs.programs {
            out.programs
                .push((hook.clone(), inputs.compiler.hdr_program(device, id, hook)));
            if !step(progress) {
                break 'work;
            }
        }
    }
    out.ms = started.elapsed().as_secs_f64() * 1000.0;
    // Logged here: a stderr write on the main thread measured 150-290 ms in hidden headless
    // runs (redirected stderr), more than the whole build it reports.
    eprintln!(
        "post pipelines: {done}/{steps} step(s) built in {:.1} ms on a worker thread{}",
        out.ms,
        if progress.cancelled() {
            " (stopped early)"
        } else {
            ""
        }
    );
    out
}

/// State of the post pipelines for `observe` and the status bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    /// Nothing requested yet, or everything requested is installed.
    #[default]
    Ready,
    Building,
    Failed(String),
}

pub struct PostBuild {
    job: Option<(Need, Job<Built>)>,
    /// The need a build failed for (not retried until the need changes).
    failed: Option<(Need, String)>,
    pub state: State,
    /// Worker milliseconds of the last completed build; builds completed.
    pub last_ms: f64,
    pub builds: u64,
    /// `Renderer::render` main-thread milliseconds (without the swapchain acquire wait) of the
    /// frame that started the last build and of the frame that installed it; frames drawn on
    /// the legacy path while building (`fallback_frames`).
    pub request_frame_ms: f64,
    pub install_frame_ms: f64,
    /// Main-thread milliseconds of the frame before the request, and of the last frame.
    pub before_frame_ms: f64,
    pub last_frame_ms: f64,
    pub fallback_frames: u64,
}

impl Default for PostBuild {
    fn default() -> Self {
        Self {
            job: None,
            failed: None,
            state: State::Ready,
            last_ms: 0.0,
            builds: 0,
            request_frame_ms: 0.0,
            install_frame_ms: 0.0,
            before_frame_ms: 0.0,
            last_frame_ms: 0.0,
            fallback_frames: 0,
        }
    }
}

/// What `PostBuild::update` asks the renderer to do this frame.
pub(crate) enum Step {
    Nothing,
    /// Start a build of this need (the renderer clones the inputs).
    Start(Need),
    Install(Built),
}

impl PostBuild {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Advances the build towards `need` (empty = nothing missing). `wait` > 0 blocks at most
    /// that long for a running job (headless screenshot frames only).
    pub(crate) fn update(&mut self, need: &Need, wait: Duration) -> Step {
        if let Some((running, job)) = &mut self.job {
            if running != need {
                // Superseded (or post turned off): stop it at its next pipeline; the current
                // request starts when it has ended (never a queue).
                job.cancel();
            }
            let outcome = if wait.is_zero() {
                job.poll()
            } else {
                job.poll_within(wait)
            };
            let Some(outcome) = outcome else {
                self.state = State::Building;
                return Step::Nothing;
            };
            let (running, job) = self.job.take().unwrap();
            match outcome {
                // Finished, or stopped early: what it built is valid and is kept.
                Outcome::Done {
                    value,
                    stop: None | Some(Stop::Cancelled),
                } => {
                    self.last_ms = value.ms;
                    self.builds += 1;
                    self.state = State::Ready;
                    return Step::Install(value);
                }
                Outcome::Abandoned {
                    stop: Stop::Cancelled,
                } => {}
                Outcome::Done {
                    stop: Some(Stop::TimedOut),
                    ..
                }
                | Outcome::Abandoned {
                    stop: Stop::TimedOut,
                } => {
                    let error = format!(
                        "post pipeline build timed out after {:.1} s (watchdog {} s); still on the legacy output",
                        job.elapsed().as_secs_f32(),
                        BUILD_TIMEOUT.as_secs()
                    );
                    eprintln!("{error}");
                    self.failed = Some((running, error.clone()));
                    self.state = State::Failed(error);
                    return Step::Nothing;
                }
            }
        }
        if need.is_empty() {
            self.state = State::Ready;
            self.failed = None;
            return Step::Nothing;
        }
        if let Some((failed, error)) = &self.failed {
            if failed == need {
                self.state = State::Failed(error.clone());
                return Step::Nothing;
            }
            self.failed = None;
        }
        self.state = State::Building;
        Step::Start(need.clone())
    }

    pub(crate) fn spawn(&mut self, need: Need, inputs: Inputs) {
        let label = format!(
            "{}{}{} material program(s)",
            if need.post { "post + " } else { "" },
            if need.hdr { "HDR world + " } else { "" },
            need.programs.len()
        );
        match Job::spawn("post_pipelines", label, BUILD_TIMEOUT, move |p| {
            build(inputs, p)
        }) {
            Ok(job) => self.job = Some((need, job)),
            Err(e) => {
                let error = format!("post pipeline build could not start: {e:#}");
                self.failed = Some((need, error.clone()));
                self.state = State::Failed(error);
            }
        }
    }

    /// The running build for `observe`.
    pub(crate) fn job_observation(&self) -> Option<Value> {
        self.job.as_ref().map(|(_, job)| job.observation())
    }

    pub fn observation(&self) -> Value {
        let round = |v: f64| (v * 1000.0).round() / 1000.0;
        json!({
            "state": match &self.state {
                State::Ready => "ready",
                State::Building => "building",
                State::Failed(_) => "failed",
            },
            "error": match &self.state {
                State::Failed(e) => Value::String(e.clone()),
                _ => Value::Null,
            },
            "job": self.job_observation(),
            "builds": self.builds,
            "worker_ms": round(self.last_ms),
            "request_frame_ms": round(self.request_frame_ms),
            "install_frame_ms": round(self.install_frame_ms),
            "before_frame_ms": round(self.before_frame_ms),
            "fallback_frames": self.fallback_frames,
        })
    }

    /// Status-bar text while building or after a failure.
    pub fn status(&self) -> Option<(String, bool)> {
        match &self.state {
            State::Ready => None,
            State::Building => {
                let (done, total) = self.job.as_ref().map_or((0, 0), |(_, job)| job.progress());
                Some((format!("post pipelines {done}/{total}"), false))
            }
            State::Failed(e) => Some((e.clone(), true)),
        }
    }
}

/// What `settings` needs that is not installed yet (see `Renderer::post_need`).
pub fn need(
    settings: &Settings,
    post_built: bool,
    hdr_built: bool,
    programs: Vec<(String, String)>,
) -> Need {
    let hdr = settings.hdr.is_some();
    Need {
        post: settings.any() && !post_built,
        hdr: hdr && !hdr_built,
        programs: if hdr { programs } else { vec![] },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(post: bool, hdr: bool) -> Need {
        Need {
            post,
            hdr,
            programs: vec![],
        }
    }

    #[test]
    fn requests_start_once_supersede_and_failures_wait_for_a_change() {
        let mut b = PostBuild::default();
        assert!(matches!(
            b.update(&Need::default(), Duration::ZERO),
            Step::Nothing
        ));
        assert_eq!(b.state, State::Ready);
        // A need starts a build (the renderer spawns it).
        assert!(
            matches!(b.update(&need(true, true), Duration::ZERO), Step::Start(n) if n == need(true, true))
        );
        // A failed need is not retried until it changes.
        b.failed = Some((need(true, true), "boom".into()));
        assert!(matches!(
            b.update(&need(true, true), Duration::ZERO),
            Step::Nothing
        ));
        assert_eq!(b.state, State::Failed("boom".into()));
        assert!(matches!(
            b.update(&need(true, false), Duration::ZERO),
            Step::Start(_)
        ));
        assert!(b.failed.is_none());
    }

    #[test]
    fn a_changed_request_cancels_the_running_job_and_the_latest_one_wins() {
        let mut b = PostBuild::default();
        // A job that runs until cancelled, then returns.
        let job = Job::spawn("post_pipelines", "test", BUILD_TIMEOUT, |p: &Progress| {
            while !p.cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            Built::default()
        })
        .unwrap();
        b.job = Some((need(true, true), job));
        assert!(matches!(
            b.update(&need(true, true), Duration::ZERO),
            Step::Nothing
        ));
        assert_eq!(b.state, State::Building);
        assert!(b.status().unwrap().0.starts_with("post pipelines"));
        // A change cancels it; nothing new starts while it is stopping (one job at a time).
        let mut step = b.update(&need(true, false), Duration::ZERO);
        for _ in 0..200 {
            if b.job.is_none() {
                break;
            }
            step = b.update(&need(true, false), Duration::from_millis(50));
        }
        assert!(b.job.is_none(), "the cancelled job ended");
        // What it built before stopping is handed over (here: nothing).
        assert!(matches!(step, Step::Install(ref built) if built.post.is_none()));
        // Then the current request (the latest) starts; turned off = nothing to build.
        assert!(
            matches!(b.update(&need(true, false), Duration::ZERO), Step::Start(n) if n == need(true, false))
        );
        assert!(matches!(
            b.update(&Need::default(), Duration::ZERO),
            Step::Nothing
        ));
        assert_eq!(b.state, State::Ready);
    }

    #[test]
    fn need_lists_only_what_is_missing() {
        use axiom_core::post::Hdr;
        let off = Settings::default();
        assert!(super::need(&off, false, false, vec![("m".into(), "h".into())]).is_empty());
        let fxaa = Settings {
            hdr: None,
            fxaa: true,
        };
        assert_eq!(
            super::need(&fxaa, false, false, vec![("m".into(), "h".into())]),
            need(true, false)
        );
        let hdr = Settings {
            hdr: Some(Hdr::default()),
            fxaa: false,
        };
        let n = super::need(&hdr, true, false, vec![("m".into(), "h".into())]);
        assert!(!n.post && n.hdr && n.programs.len() == 1);
        assert!(super::need(&hdr, true, true, vec![]).is_empty());
    }
}
