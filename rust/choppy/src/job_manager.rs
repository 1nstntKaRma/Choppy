//! Runs jobs one at a time in the order they came in (JobManager.cs): two copies fighting over the same disk or line
//! are slower than one after the other. Every queued job is on disk until it ends, so a crash can be resumed.
#![deny(unsafe_code)]

use std::sync::{Arc, Mutex, Weak};

use crate::engine::copy_job::CopyJob;
use crate::engine::lock;
use crate::engine::model::{ConflictAnswer, ConflictInfo, ConflictPolicy, FileItem, JobPhase, JobSpec};
use crate::engine::settings::{Settings, journal};

pub type AskFn = Arc<dyn Fn(&Arc<CopyJob>, &ConflictInfo) -> Option<ConflictAnswer> + Send + Sync>;
pub type AskLowSpaceFn = Arc<dyn Fn(&Arc<CopyJob>, i64, i64) -> bool + Send + Sync>;
pub type AskElevateFn = Arc<dyn Fn(&Arc<CopyJob>) -> bool + Send + Sync>;
pub type JobFn = Arc<dyn Fn(&Arc<CopyJob>) + Send + Sync>;
pub type AllDoneFn = Arc<dyn Fn(Option<&Arc<CopyJob>>) + Send + Sync>;
/// Runs a piece of work on the UI thread (C#: Application.Current.Dispatcher.BeginInvoke).
pub type DispatchFn = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

#[derive(Default)]
struct State {
    waiting: Vec<Arc<JobSpec>>,
    finished: Vec<Arc<CopyJob>>,
    current: Option<Arc<CopyJob>>,
    last_finished: Option<Arc<CopyJob>>,
}

#[derive(Default)]
pub struct JobManager {
    gate: Mutex<State>,
    changed: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    /// Conflict questions go to the UI thread (set by App).
    pub ask: Option<AskFn>,
    pub ask_low_space: Option<AskLowSpaceFn>,
    pub ask_elevate: Option<AskElevateFn>,
    pub job_started: Option<JobFn>,
    pub all_done: Option<AllDoneFn>,
    /// The UI thread (step 4/5 sets it); None = everything runs right where it is (C#: no Application).
    pub dispatch: Option<DispatchFn>,
    /// True on the UI thread itself (C#'s Dispatcher.CheckAccess): Changed then runs at once instead of later.
    pub on_ui_thread: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl JobManager {
    pub fn current(&self) -> Option<Arc<CopyJob>> {
        lock(&self.gate).current.clone()
    }
    pub fn last_finished(&self) -> Option<Arc<CopyJob>> {
        lock(&self.gate).last_finished.clone()
    }
    /// Runs `f` whenever the queue or a job changes (C#'s Changed event; on the UI thread when there is one).
    pub fn on_changed(&self, f: Arc<dyn Fn() + Send + Sync>) {
        lock(&self.changed).push(f);
    }

    pub fn waiting_count(&self) -> usize {
        lock(&self.gate).waiting.len()
    }
    pub fn has_waiting(&self) -> bool {
        self.waiting_count() > 0
    }
    pub fn busy(&self) -> bool {
        let g = lock(&self.gate);
        g.current.as_ref().is_some_and(|c| !c.is_finished()) || !g.waiting.is_empty()
    }
    pub fn waiting_snapshot(&self) -> Vec<Arc<JobSpec>> {
        lock(&self.gate).waiting.clone()
    }
    /// The last 8 finished, the newest first.
    pub fn finished_snapshot(&self) -> Vec<Arc<CopyJob>> {
        lock(&self.gate).finished.iter().rev().take(8).cloned().collect()
    }

    pub fn enqueue(self: &Arc<Self>, mut spec: JobSpec) {
        {
            let s = Settings::current();
            if spec.speed_limit_mbps <= 0.0 {
                spec.speed_limit_mbps = s.speed_limit_mbps;
            }
        }
        journal::save_soon(&mut spec);
        let start;
        {
            let mut g = lock(&self.gate);
            g.waiting.push(Arc::new(spec));
            start = g.current.as_ref().is_none_or(|c| c.is_finished());
        }
        if start {
            self.start_next();
        } else {
            self.raise();
        }
    }

    pub fn remove(&self, spec: &Arc<JobSpec>) {
        lock(&self.gate).waiting.retain(|s| !Arc::ptr_eq(s, spec));
        journal::remove(spec);
        self.raise();
    }

    pub fn cancel_current(&self) {
        if let Some(c) = self.current()
            && !c.is_finished()
        {
            c.cancel();
        }
    }

    pub fn retry_failed(self: &Arc<Self>, job: &Arc<CopyJob>) {
        let policy = job.policy();
        let spec = JobSpec {
            op: job.spec.op,
            dest: job.spec.dest.clone(),
            verify: job.spec.verify,
            policy: if policy == ConflictPolicy::Ask { ConflictPolicy::Ask } else { policy },
            origin: "Retry".into(),
            explicit_items: Some(
                lock(&job.failed_items).iter().map(|f| FileItem::new(f.src.clone(), f.rel.clone(), f.size())).collect(),
            ),
            sources: job.spec.sources.clone(),
            ..Default::default()
        };
        self.enqueue(spec);
    }

    /// Speed limit changes apply to the running copy at once.
    pub fn apply_live_settings(&self) {
        if let Some(c) = self.current()
            && !c.is_finished()
        {
            c.set_speed_limit(Settings::current().speed_limit_mbps);
        }
    }

    fn start_next(self: &Arc<Self>) {
        let spec;
        {
            let mut g = lock(&self.gate);
            if g.current.as_ref().is_some_and(|c| !c.is_finished()) {
                return;
            }
            if g.waiting.is_empty() {
                g.current = None;
                spec = None;
            } else {
                spec = Some(g.waiting.remove(0));
            }
        }
        let Some(spec) = spec else {
            self.raise();
            if let Some(h) = &self.all_done {
                h(self.last_finished().as_ref());
            }
            return;
        };

        let mut job = CopyJob::new(spec);
        let me: Weak<JobManager> = Arc::downgrade(self);
        let m = me.clone();
        job.ask_conflict = Some(Box::new(move |j, info| {
            match m.upgrade().and_then(|m| m.ask.clone()) {
                Some(ask) => ask(j, info),
                None => Some(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false }),
            }
        }));
        let m = me.clone();
        job.ask_low_space = Some(Box::new(move |j, need, free| match m.upgrade().and_then(|m| m.ask_low_space.clone()) {
            Some(ask) => ask(j, need, free),
            None => true,
        }));
        let m = me.clone();
        job.ask_elevate = Some(Box::new(move |j| m.upgrade().and_then(|m| m.ask_elevate.clone()).is_some_and(|ask| ask(j))));
        let job = Arc::new(job);
        let m = me.clone();
        job.on_phase_changed(Arc::new(move |_| {
            if let Some(m) = m.upgrade() {
                m.raise();
            }
        }));
        let m = me.clone();
        job.on_finished(Arc::new(move |j| {
            if let Some(m) = m.upgrade() {
                m.on_finished(j);
            }
        }));
        lock(&self.gate).current = Some(job.clone());
        if let Some(st) = &self.job_started {
            st(&job);
        }
        job.start();
        self.raise();
    }

    fn on_finished(self: &Arc<Self>, job: &Arc<CopyJob>) {
        // canceled by the user or done = nothing to resume. A hard failure (destination gone) keeps its record.
        if job.phase() != JobPhase::Failed {
            journal::remove(&job.spec);
        }
        {
            let mut g = lock(&self.gate);
            g.finished.push(job.clone());
            g.last_finished = Some(job.clone());
            if g.current.as_ref().is_some_and(|c| Arc::ptr_eq(c, job)) {
                g.current = None;
            }
        }
        match &self.dispatch {
            Some(d) => {
                let me = self.clone();
                d(Box::new(move || me.start_next()));
            }
            None => self.start_next(),
        }
    }

    fn raise(&self) {
        let hs: Vec<Arc<dyn Fn() + Send + Sync>> = lock(&self.changed).clone();
        if hs.is_empty() {
            return;
        }
        let here = self.on_ui_thread.as_ref().is_some_and(|f| f()); // unknown = posted, as from any other thread
        match &self.dispatch {
            Some(d) if !here => d(Box::new(move || {
                for h in &hs {
                    h();
                }
            })),
            _ => {
                for h in &hs {
                    h();
                }
            }
        }
    }
}
