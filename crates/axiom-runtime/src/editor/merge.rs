//! Field-wise conflict merge (`ops::merge`): a stale Inspector draft or a stale proposal
//! against the newer revision. Each conflicting field is picked mine / theirs, then the
//! merge commits as one revision (a proposal through `resolve_proposal {keep}`, a draft as
//! one apply). Preparing and building the merge run on background jobs.
use super::{theme, Action, Editor};
use crate::jobs::{Job, Outcome};
use axiom_core::{ops, Edit, Request, Store, World};
use egui::RichText;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

const MERGE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeSource {
    /// A pending proposal (mine = the proposal, theirs = the current revision).
    Proposal(i64),
    /// The stale Inspector draft (mine = the draft, theirs = the current revision).
    Draft,
}

impl MergeSource {
    fn name(self) -> String {
        match self {
            Self::Proposal(id) => format!("proposal:{id}"),
            Self::Draft => "draft".into(),
        }
    }
}

/// A prepared merge: the conflicts, and (for a draft) the three worlds the commit merges.
pub(crate) struct Prepared {
    pub view: ops::MergeView,
    pub base_rev: i64,
    /// The revision "theirs" is (the commit expects it).
    pub rev: i64,
    worlds: Option<[Arc<World>; 3]>,
    /// Draft merges: the draft's edit batch the conflicts were listed for (edited since =
    /// list again).
    draft: Option<String>,
}

#[derive(Default)]
pub(crate) struct MergeState {
    pub source: Option<MergeSource>,
    pub job: Option<Job<anyhow::Result<Prepared>>>,
    pub prepared: Option<Prepared>,
    /// Conflict paths picked "theirs" (the rest take mine).
    pub theirs: BTreeSet<String>,
    /// Building a draft merge's edit batch.
    pub commit: Option<Job<anyhow::Result<Vec<Edit>>>>,
    /// The built merge, emitted as a write on the next drawn frame.
    pub write: Option<Action>,
    pub error: Option<String>,
}

impl MergeState {
    fn state(&self) -> &'static str {
        match () {
            _ if self.source.is_none() => "closed",
            _ if self.commit.is_some() || self.write.is_some() => "committing",
            _ if self.job.is_some() => "preparing",
            _ if self.prepared.is_some() => "ready",
            _ => "error",
        }
    }
}

/// A conflict path as people read it: `/entities/@a~1b/components/C/F` -> `a/b · C/F`.
fn field_label(path: &str) -> String {
    let unescape = |s: &str| s.replace("~1", "/").replace("~0", "~");
    match path.strip_prefix("/entities/@") {
        Some(rest) => match rest.split_once("/components/") {
            Some((id, field)) => format!("{} · {}", unescape(id), unescape(field)),
            None => match rest.split_once('/') {
                Some((id, field)) => format!("{} · {}", unescape(id), unescape(field)),
                None => unescape(rest),
            },
        },
        None => unescape(path),
    }
}

/// (mine, theirs) column titles.
fn sides(source: MergeSource) -> (&'static str, &'static str) {
    match source {
        MergeSource::Proposal(_) => ("Proposal", "Current"),
        MergeSource::Draft => ("Mine", "Theirs"),
    }
}

impl Editor {
    /// Why a merge command cannot run now.
    pub(crate) fn merge_blocked(&self, id: &str) -> Option<&'static str> {
        let m = &self.merge;
        match id {
            "merge.pick" | "merge.commit" if m.prepared.is_none() => {
                Some("no prepared merge (merge.open proposal:N or draft first)")
            }
            "merge.commit" if m.commit.is_some() || m.write.is_some() => {
                Some("the merge is being committed")
            }
            // While edits save, the editor shows their optimistic world, not a revision: the
            // conflicts would be listed against the wrong "theirs".
            "merge.open" | "merge.commit" if self.saving > 0 => {
                Some("wait for the queued edits to save")
            }
            "merge.cancel" if m.source.is_none() => Some("no merge open"),
            _ => None,
        }
    }

    pub(crate) fn merge_command(
        &mut self,
        id: &str,
        arg: Option<&str>,
    ) -> anyhow::Result<Vec<Action>> {
        match id {
            "merge.open" => {
                let Some(arg) = arg else {
                    self.show_dock(super::Dock::Proposals);
                    return Ok(vec![]);
                };
                self.close_merge();
                if arg.trim() == "draft" {
                    self.prepare_draft_merge()?;
                    return Ok(vec![]);
                }
                let id: i64 = arg
                    .trim()
                    .strip_prefix("proposal:")
                    .map(|n| n.trim_start_matches('#'))
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("merge.open takes proposal:N or draft"))?;
                self.merge.source = Some(MergeSource::Proposal(id));
                Ok(vec![Action::PrepareMerge { proposal: id }])
            }
            "merge.pick" => {
                let arg =
                    arg.ok_or_else(|| anyhow::anyhow!("merge.pick takes PATH=mine|theirs"))?;
                let (path, side) = arg
                    .rsplit_once('=')
                    .ok_or_else(|| anyhow::anyhow!("merge.pick takes PATH=mine|theirs"))?;
                let theirs = match side.trim() {
                    "theirs" | "current" => true,
                    "mine" | "proposal" => false,
                    other => anyhow::bail!("pick mine or theirs, not {other:?}"),
                };
                let paths: Vec<String> = {
                    let conflicts = &self.merge.prepared.as_ref().unwrap().view.conflicts;
                    if path.trim() == "all" {
                        conflicts.iter().map(|c| c.path.clone()).collect()
                    } else {
                        let path = path.trim();
                        anyhow::ensure!(
                            conflicts.iter().any(|c| c.path == path),
                            "{path} is not a conflicting field"
                        );
                        vec![path.to_owned()]
                    }
                };
                for path in paths {
                    if theirs {
                        self.merge.theirs.insert(path);
                    } else {
                        self.merge.theirs.remove(&path);
                    }
                }
                Ok(vec![])
            }
            "merge.commit" => self.commit_merge(),
            "merge.cancel" => {
                self.close_merge();
                Ok(vec![])
            }
            _ => anyhow::bail!("unknown merge command {id}"),
        }
    }

    pub(crate) fn close_merge(&mut self) {
        let m = std::mem::take(&mut self.merge);
        if let Some(job) = m.job {
            job.detach();
        }
        if let Some(job) = m.commit {
            job.detach();
        }
    }

    /// Mine = the base revision of the stale draft with the draft's edits; theirs = now.
    fn prepare_draft_merge(&mut self) -> anyhow::Result<()> {
        let (base_rev, base, edits, fingerprint) = self.draft_merge_edits()?;
        let (theirs, rev, registry) = (self.world.clone(), self.rev, self.registry.clone());
        self.merge.source = Some(MergeSource::Draft);
        self.merge.job = Some(Job::spawn("merge", "draft", MERGE_TIMEOUT, move |_| {
            let mine = Arc::new(ops::apply_edits(&base, &edits, &registry)?);
            let view = ops::merge_view(&base, &mine, &theirs);
            Ok(Prepared {
                view,
                base_rev,
                rev,
                worlds: Some([base, mine, theirs]),
                draft: Some(fingerprint),
            })
        })?);
        Ok(())
    }

    /// The stale draft's base revision and its edit batch on that revision (+ the batch as
    /// JSON, to notice later edits of the draft).
    #[allow(clippy::type_complexity)]
    fn draft_merge_edits(&self) -> anyhow::Result<(i64, Arc<World>, Vec<Edit>, String)> {
        let (base_rev, base) = self.merge_base.clone().ok_or_else(|| {
            anyhow::anyhow!("no stale Inspector draft to merge (it is up to date or was reloaded)")
        })?;
        let draft = self
            .draft
            .as_ref()
            .filter(|d| d.dirty() && d.rev == base_rev)
            .ok_or_else(|| anyhow::anyhow!("no stale Inspector draft to merge"))?;
        let edits = super::inspector_edits(&base, &self.registry, draft, &self.extra)?;
        let fingerprint = serde_json::to_string(&edits)?;
        Ok((base_rev, base, edits, fingerprint))
    }

    /// The prepared merge no longer matches the editor: a newer revision arrived, edits are
    /// saving, or (draft) the draft was edited after its conflicts were listed.
    fn merge_outdated(&self) -> bool {
        let Some(p) = self.merge.prepared.as_ref() else {
            return true;
        };
        p.rev != self.rev
            || self.saving > 0
            || p.draft.as_ref().is_some_and(|listed| {
                self.draft_merge_edits()
                    .map_or(true, |(.., now)| &now != listed)
            })
    }

    /// Lists the conflicts again (the merge window stays open meanwhile).
    fn relist_merge(&mut self) -> anyhow::Result<Vec<Action>> {
        let source = self.merge.source;
        self.close_merge();
        self.set_error(
            "The world or the draft changed; conflicts listed again. Review and commit.",
        );
        match source {
            Some(MergeSource::Draft) => {
                self.prepare_draft_merge()?;
                Ok(vec![])
            }
            Some(MergeSource::Proposal(id)) => {
                self.merge.source = source;
                Ok(vec![Action::PrepareMerge { proposal: id }])
            }
            None => Ok(vec![]),
        }
    }

    fn commit_merge(&mut self) -> anyhow::Result<Vec<Action>> {
        if self.merge_outdated() {
            return self.relist_merge();
        }
        let prepared = self.merge.prepared.as_ref().unwrap();
        let keep: Vec<String> = self.merge.theirs.iter().cloned().collect();
        let prepared_rev = prepared.rev;
        match self.merge.source {
            Some(MergeSource::Proposal(id)) => {
                self.close_merge();
                Ok(vec![Action::ResolveProposal {
                    id,
                    approve: true,
                    rebase: false,
                    keep: Some(keep),
                    expected_rev: Some(prepared_rev),
                }])
            }
            Some(MergeSource::Draft) => {
                let [base, mine, theirs] = prepared.worlds.clone().unwrap();
                self.merge.commit = Some(Job::spawn(
                    "merge",
                    "draft commit",
                    MERGE_TIMEOUT,
                    move |_| ops::merge(&base, &mine, &theirs, &keep),
                )?);
                Ok(vec![])
            }
            None => Ok(vec![]),
        }
    }

    /// Once per frame (from `poll_jobs`).
    pub(crate) fn poll_merge(&mut self) {
        let failed = |stop: crate::jobs::Stop| format!("merge job {}", stop.name());
        if let Some(job) = &mut self.merge.job {
            if let Some(outcome) = job.poll() {
                self.merge.job = None;
                match outcome {
                    Outcome::Done {
                        value: Ok(prepared),
                        stop: None,
                    } => {
                        self.merge.theirs.clear();
                        self.merge.error = None;
                        self.merge.prepared = Some(prepared);
                    }
                    Outcome::Done { value: Err(e), .. } => {
                        self.merge.error = Some(format!("{e:#}"))
                    }
                    Outcome::Done { stop: Some(s), .. } | Outcome::Abandoned { stop: s } => {
                        self.merge.error = Some(failed(s))
                    }
                }
            }
        }
        if let Some(job) = &mut self.merge.commit {
            if let Some(outcome) = job.poll() {
                self.merge.commit = None;
                let rev = self.merge.prepared.as_ref().map_or(self.rev, |p| p.rev);
                match outcome {
                    Outcome::Done {
                        value: Ok(edits),
                        stop: None,
                    } if edits.is_empty() => {
                        self.close_merge();
                        self.reset_draft();
                        self.notice("Merged: the newer revision already has every change");
                    }
                    Outcome::Done {
                        value: Ok(edits),
                        stop: None,
                    } => {
                        self.merge.write = Some(Action::Write {
                            request: Request::Apply {
                                expected_rev: Some(rev),
                                edits,
                            },
                            select: None,
                        });
                    }
                    Outcome::Done { value: Err(e), .. } => {
                        self.merge.error = Some(format!("{e:#}"))
                    }
                    Outcome::Done { stop: Some(s), .. } | Outcome::Abandoned { stop: s } => {
                        self.merge.error = Some(failed(s))
                    }
                }
            }
        }
    }

    /// The merge built in the background, as this frame's write (then the merge closes).
    pub(crate) fn take_merge_write(&mut self, actions: &mut Vec<Action>) {
        if self.merge.write.is_some() && self.merge_outdated() {
            // Built while the world or the draft moved on: never commit it over that.
            self.merge.write = None;
            match self.relist_merge() {
                Ok(more) => actions.extend(more),
                Err(e) => self.set_error(format!("{e:#}")),
            }
            return;
        }
        if let Some(write) = self.merge.write.take() {
            actions.push(write);
            self.close_merge();
            // The stale draft is replaced by the merge (also when the live Inspector would
            // otherwise keep a dirty draft across the commit).
            self.reset_draft();
            self.notice("Merged draft committed as one revision");
        }
    }

    pub(crate) fn merge_jobs_observation(&self) -> impl Iterator<Item = Value> + '_ {
        self.merge
            .job
            .iter()
            .map(|j| j.observation())
            .chain(self.merge.commit.iter().map(|j| j.observation()))
            .chain(self.ghost.job.iter().map(|(_, j)| j.observation()))
    }

    pub(crate) fn merge_observation(&self) -> Value {
        let m = &self.merge;
        let prepared = m.prepared.as_ref();
        json!({
            "source":m.source.map(MergeSource::name),
            "state":m.state(),
            "base_rev":prepared.map(|p| p.base_rev),
            "rev":prepared.map(|p| p.rev),
            "conflicts":prepared.map(|p| p.view.conflicts.iter().map(|c| json!({"path":c.path,"pick":if m.theirs.contains(&c.path) {"theirs"} else {"mine"}})).collect::<Vec<_>>()),
            "mine_only":prepared.map(|p| p.view.mine_only),
            "theirs_only":prepared.map(|p| p.view.theirs_only),
            "error":m.error,
        })
    }

    /// The merge window (open while a merge is).
    pub(crate) fn merge_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        self.take_merge_write(actions);
        let Some(source) = self.merge.source else {
            return;
        };
        let (mine_title, theirs_title) = sides(source);
        let mut run: Option<(&'static str, Option<String>)> = None;
        let title = match source {
            MergeSource::Proposal(id) => format!("Merge proposal #{id}"),
            MergeSource::Draft => "Merge Inspector draft".to_owned(),
        };
        egui::Window::new(title)
            .id(egui::Id::new("merge_window"))
            .collapsible(false)
            .resizable(true)
            .default_width(720.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let state = self.merge.state();
                if let Some(error) = &self.merge.error {
                    ui.label(RichText::new(error).color(theme::ERROR));
                }
                let Some(p) = &self.merge.prepared else {
                    if state == "preparing" {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Listing conflicting fields…");
                        });
                    }
                    if ui.button("Close").clicked() {
                        run = Some(("merge.cancel", None));
                    }
                    return;
                };
                ui.label(
                    RichText::new(format!(
                        "Base revision {} → current {} · {} conflicting field(s) · {} change(s) of {} merge cleanly · {} newer change(s) kept",
                        p.base_rev,
                        p.rev,
                        p.view.conflicts.len(),
                        p.view.mine_only,
                        mine_title.to_lowercase(),
                        p.view.theirs_only
                    ))
                    .small()
                    .color(theme::MUTED),
                );
                ui.add_space(4.0);
                if p.view.conflicts.is_empty() {
                    ui.label("No field was changed on both sides: committing merges everything.");
                }
                let short = |v: &Option<Value>| match v {
                    None => "(none)".to_owned(),
                    Some(v) => {
                        let s = axiom_core::local::display_value(v).to_string();
                        if s.chars().count() > 34 {
                            format!("{}…", s.chars().take(33).collect::<String>())
                        } else {
                            s
                        }
                    }
                };
                egui::ScrollArea::vertical()
                    .id_salt("merge_rows")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        // Fixed columns (no Grid: a new Grid drops the clicks of its first frame).
                        let width = ui.available_width().max(480.0);
                        let field = egui::vec2((width * 0.34).floor(), 22.0);
                        let value = egui::vec2(((width - field.x) / 3.0 - 8.0).floor(), 22.0);
                        ui.horizontal(|ui| {
                            for (h, size) in [("FIELD", field), ("BASE", value), (mine_title, value), (theirs_title, value)] {
                                super::tuning::cell_label(ui, size, RichText::new(h.to_uppercase()).small().strong().color(theme::MUTED));
                            }
                        });
                        for (i, c) in p.view.conflicts.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let path = field_label(&c.path);
                                super::tuning::cell_label(ui, field, RichText::new(path).monospace().small())
                                    .on_hover_text(&c.path);
                                super::tuning::cell_label(ui, value, RichText::new(short(&c.base)).monospace().small().color(theme::MUTED));
                                let theirs = self.merge.theirs.contains(&c.path);
                                let pick = |on: bool| if on { theme::ACCENT_SOFT } else { egui::Color32::TRANSPARENT };
                                let text = |on: bool| if on { theme::TEXT } else { theme::MUTED };
                                let m = super::tuning::cell_button(ui, value, &short(&c.mine), text(!theirs), pick(!theirs))
                                    .on_hover_text(format!("Keep the {} value", mine_title.to_lowercase()));
                                let t = super::tuning::cell_button(ui, value, &short(&c.theirs), text(theirs), pick(theirs))
                                    .on_hover_text(format!("Keep the {} value", theirs_title.to_lowercase()));
                                self.rects.insert(format!("merge:mine:{i}"), m.rect);
                                self.rects.insert(format!("merge:theirs:{i}"), t.rect);
                                if m.clicked() {
                                    run = Some(("merge.pick", Some(format!("{}=mine", c.path))));
                                }
                                if t.clicked() {
                                    run = Some(("merge.pick", Some(format!("{}=theirs", c.path))));
                                }
                            });
                        }
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let all_mine = ui.button(format!("All {}", mine_title.to_lowercase()));
                    let all_theirs = ui.button(format!("All {}", theirs_title.to_lowercase()));
                    if all_mine.clicked() {
                        run = Some(("merge.pick", Some("all=mine".into())));
                    }
                    if all_theirs.clicked() {
                        run = Some(("merge.pick", Some("all=theirs".into())));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let commit = ui.add_enabled(
                            state == "ready",
                            egui::Button::new(RichText::new("Commit merge").color(egui::Color32::WHITE))
                                .fill(theme::ACCENT),
                        )
                        .on_hover_text("One revision: unconflicting changes from both sides plus your picks");
                        self.rects.insert("merge:commit".into(), commit.rect);
                        if commit.clicked() {
                            run = Some(("merge.commit", None));
                        }
                        if ui.button("Cancel").clicked() {
                            run = Some(("merge.cancel", None));
                        }
                        if state == "committing" {
                            ui.spinner();
                        }
                    });
                });
            });
        if let Some((id, arg)) = run {
            match self.run_command_with(id, arg.as_deref()) {
                Ok(more) => actions.extend(more),
                Err(e) => self.set_error(format!("{e:#}")),
            }
        }
    }
}

/// Host side of `Action::PrepareMerge`: reads the proposal and its base revision (two rows)
/// and lists the conflicts on a worker.
pub(crate) fn prepare_proposal(editor: &mut Editor, store: &Store, id: i64) -> anyhow::Result<()> {
    let (base_rev, edits, status) = store.proposal_raw(id)?;
    anyhow::ensure!(status == "pending", "proposal {id} is already {status}");
    let raw = store.revision_json(base_rev)?;
    let (theirs, rev, registry) = (editor.world.clone(), editor.rev, editor.registry.clone());
    editor.merge.source = Some(MergeSource::Proposal(id));
    editor.merge.job = Some(Job::spawn(
        "merge",
        format!("proposal {id}"),
        MERGE_TIMEOUT,
        move |_| {
            let base = registry.decode(serde_json::from_str(&raw)?)?;
            let edits: Vec<Edit> = serde_json::from_str(&edits)?;
            let mine = ops::apply_edits(&base, &edits, &registry)?;
            Ok(Prepared {
                view: ops::merge_view(&base, &mine, &theirs),
                base_rev,
                rev,
                worlds: None,
                draft: None,
            })
        },
    )?);
    Ok(())
}
