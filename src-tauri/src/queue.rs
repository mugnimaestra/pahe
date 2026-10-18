use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::download::{Job, JobState, Request};

pub const QUEUE_FILE: &str = "queue.json";

/// Why an Episode and Quality was not enqueued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    InLibrary,
    InQueue,
}

/// One refused request, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub anime_id: String,
    pub episode: u32,
    pub quality: String,
    pub reason: SkipReason,
}

/// The accepted jobs in enqueue order, plus the refusals.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Enqueued {
    pub jobs: Vec<Job>,
    pub skipped: Vec<Skipped>,
}

/// What the Queue view reads: every job plus whether the worker is allowed to
/// run and whether a Challenge holds it. `active` is false after a restart
/// until the user starts or resumes something.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub jobs: Vec<Job>,
    pub active: bool,
    pub challenge: bool,
}

struct State {
    jobs: Vec<Job>,
    requests: HashMap<String, Request>,
}

/// A serial Download job queue: at most one job runs, the rest wait in order.
pub struct Queue {
    state: Mutex<State>,
    wake: Notify,
    report: Box<dyn Fn(&Job) + Send + Sync>,
    store: Option<PathBuf>,
    active: AtomicBool,
    challenge: AtomicBool,
}

impl Queue {
    /// An in-memory queue, for tests that do not need a restart.
    #[cfg(test)]
    pub fn new(report: impl Fn(&Job) + Send + Sync + 'static) -> Self {
        Self::build(None, report)
    }

    /// Loads the queue persisted at `path`. Jobs that were running or paused
    /// when the app died come back interrupted, and nothing runs until the user
    /// starts the queue or resumes a job.
    pub fn open(path: impl Into<PathBuf>, report: impl Fn(&Job) + Send + Sync + 'static) -> Self {
        Self::build(Some(path.into()), report)
    }

    fn build(store: Option<PathBuf>, report: impl Fn(&Job) + Send + Sync + 'static) -> Self {
        let (jobs, requests) = store
            .as_deref()
            .map(restore)
            .unwrap_or_else(|| (Vec::new(), HashMap::new()));

        Self {
            state: Mutex::new(State { jobs, requests }),
            wake: Notify::new(),
            report: Box::new(report),
            store,
            active: AtomicBool::new(false),
            challenge: AtomicBool::new(false),
        }
    }

    /// Every job in enqueue order, with its latest state.
    pub fn jobs(&self) -> Vec<Job> {
        self.state.lock().expect("queue lock poisoned").jobs.clone()
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            jobs: self.jobs(),
            active: self.active.load(Ordering::SeqCst),
            challenge: self.challenge.load(Ordering::SeqCst),
        }
    }

    /// Queues every request that is neither owned nor already active.
    pub fn enqueue<O>(&self, requests: Vec<Request>, owned: O) -> Enqueued
    where
        O: Fn(&Request) -> bool,
    {
        let mut outcome = Enqueued {
            jobs: Vec::new(),
            skipped: Vec::new(),
        };
        let mut state = self.state.lock().expect("queue lock poisoned");

        for request in requests {
            if state.jobs.iter().any(|job| is_active(job, &request)) {
                outcome
                    .skipped
                    .push(Skipped::refusing(&request, SkipReason::InQueue));
                continue;
            }
            if !request.redownload && owned(&request) {
                outcome
                    .skipped
                    .push(Skipped::refusing(&request, SkipReason::InLibrary));
                continue;
            }

            let job = Job::new(&request);
            state.requests.insert(job.id.clone(), request);
            state.jobs.push(job.clone());
            outcome.jobs.push(job);
        }
        self.persist(&state);
        drop(state);

        if !outcome.jobs.is_empty() {
            // Adding a job is an explicit user action, so it starts the worker.
            self.active.store(true, Ordering::SeqCst);
        }

        for job in &outcome.jobs {
            (self.report)(job);
        }
        if !outcome.jobs.is_empty() {
            self.wake.notify_one();
        }

        outcome
    }

    /// Records a job's latest state and publishes it. Progress reports land here
    /// so a late Queue view still sees bytes and speed.
    ///
    /// A Challenge holds the stored job at Paused even while the engine keeps
    /// reporting it as Running.
    pub fn report(&self, job: &Job) {
        let published = {
            let mut state = self.state.lock().expect("queue lock poisoned");
            let mut published = job.clone();
            if let Some(stored) = state.jobs.iter_mut().find(|stored| stored.id == job.id) {
                if stored.state == JobState::Paused && job.state == JobState::Running {
                    published.state = JobState::Paused;
                }
                *stored = published.clone();
            }
            self.persist(&state);
            published
        };
        (self.report)(&published);
    }

    /// Lets the worker run queued jobs. Used by Start and by Resume.
    pub fn start(&self) -> QueueSnapshot {
        self.active.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        self.snapshot()
    }

    /// Puts an interrupted, paused, failed, or queued job back in line and
    /// starts the worker. A running or completed job is refused.
    pub fn resume(&self, id: &str) -> Result<Job, String> {
        let resumed = {
            let mut state = self.state.lock().expect("queue lock poisoned");
            let Some(index) = state.jobs.iter().position(|job| job.id == id) else {
                return Err("That Download job is no longer in the queue.".to_owned());
            };
            let job = &mut state.jobs[index];
            match job.state {
                JobState::Interrupted | JobState::Paused | JobState::Failed | JobState::Queued => {
                    job.state = JobState::Queued;
                    job.error = None;
                }
                JobState::Running => {
                    return Err("That Download job is already running.".to_owned());
                }
                JobState::Completed => {
                    return Err("That Episode is already downloaded.".to_owned());
                }
            }

            let resumed = job.clone();
            self.persist(&state);
            resumed
        };

        self.active.store(true, Ordering::SeqCst);
        (self.report)(&resumed);
        self.wake.notify_one();

        Ok(resumed)
    }

    /// Marks the queue paused for a Challenge, or clears it. Pausing holds the
    /// running job (and its bytes) without failing it.
    pub fn set_challenge(&self, active: bool) {
        if self.challenge.swap(active, Ordering::SeqCst) == active {
            return;
        }

        let mut changed = Vec::new();
        {
            let mut state = self.state.lock().expect("queue lock poisoned");
            for job in &mut state.jobs {
                match (active, job.state) {
                    (true, JobState::Running) => {
                        job.state = JobState::Paused;
                        changed.push(job.clone());
                    }
                    (false, JobState::Paused) => {
                        job.state = JobState::Running;
                        changed.push(job.clone());
                    }
                    _ => {}
                }
            }
            if !changed.is_empty() {
                self.persist(&state);
            }
        }

        for job in &changed {
            (self.report)(job);
        }
        if !active {
            self.wake.notify_one();
        }
    }

    /// Waits for the next queued job, runs it, and stores its final state.
    pub async fn run_next<F, Fut>(&self, execute: F) -> Job
    where
        F: Fn(Job, Request) -> Fut,
        Fut: Future<Output = Job>,
    {
        let (job, request) = loop {
            let next = {
                let mut state = self.state.lock().expect("queue lock poisoned");
                if !self.active.load(Ordering::SeqCst) || self.challenge.load(Ordering::SeqCst) {
                    None
                } else {
                    state
                        .jobs
                        .iter()
                        .position(|job| job.state == JobState::Queued)
                        .map(|index| {
                            let job = &mut state.jobs[index];
                            job.state = JobState::Running;
                            let job = job.clone();
                            let request = state
                                .requests
                                .get(&job.id)
                                .cloned()
                                .expect("a queued job keeps its request");
                            self.persist(&state);
                            (job, request)
                        })
                }
            };

            match next {
                Some(next) => break next,
                None => self.wake.notified().await,
            }
        };

        (self.report)(&job);
        let finished = execute(job, request).await;
        self.report(&finished);
        finished
    }

    /// Runs queued jobs forever, one at a time. Spawn exactly one of these.
    pub async fn serve<F, Fut>(&self, execute: F)
    where
        F: Fn(Job, Request) -> Fut,
        Fut: Future<Output = Job>,
    {
        loop {
            self.run_next(&execute).await;
        }
    }

    fn persist(&self, state: &State) {
        let Some(path) = self.store.as_deref() else {
            return;
        };
        let jobs = state
            .jobs
            .iter()
            .filter_map(|job| {
                state.requests.get(&job.id).map(|request| PersistedJob {
                    job: job.clone(),
                    request: request.clone(),
                })
            })
            .collect();

        if let Err(error) = save(path, &PersistedQueue { jobs }) {
            eprintln!("queue: {error}");
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PersistedQueue {
    jobs: Vec<PersistedJob>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedJob {
    job: Job,
    request: Request,
}

/// Reads the persisted queue, treating a job that was running or paused when
/// the app died as interrupted.
fn restore(path: &Path) -> (Vec<Job>, HashMap<String, Request>) {
    let parsed = fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<PersistedQueue>(&raw).ok())
        .unwrap_or_default();

    let mut jobs = Vec::new();
    let mut requests = HashMap::new();
    for mut entry in parsed.jobs {
        if matches!(entry.job.state, JobState::Running | JobState::Paused) {
            entry.job.state = JobState::Interrupted;
        }
        requests.insert(entry.job.id.clone(), entry.request);
        jobs.push(entry.job);
    }

    (jobs, requests)
}

fn save(path: &Path, queue: &PersistedQueue) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| format!("could not create {dir:?}: {error}"))?;
    }
    let json = serde_json::to_vec_pretty(queue)
        .map_err(|error| format!("could not encode the queue: {error}"))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(|error| format!("could not save the queue: {error}"))?;
    fs::rename(&temp, path).map_err(|error| format!("could not save the queue: {error}"))
}

fn is_active(job: &Job, request: &Request) -> bool {
    matches!(
        job.state,
        JobState::Queued | JobState::Running | JobState::Paused
    ) && job.anime_id == request.anime_id
        && job.episode == request.episode
        && job.quality.eq_ignore_ascii_case(&request.quality)
}

impl Skipped {
    fn refusing(request: &Request, reason: SkipReason) -> Self {
        Self {
            anime_id: request.anime_id.clone(),
            episode: request.episode,
            quality: request.quality.clone(),
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::task::JoinHandle;

    fn request(episode: u32, quality: &str) -> Request {
        Request {
            anime_id: "anime-id".to_owned(),
            title: "Seitokai Yakuindomo".to_owned(),
            episode,
            session: format!("session-{episode}"),
            quality: quality.to_owned(),
            redownload: false,
        }
    }

    fn queue() -> (Arc<Queue>, Arc<Mutex<Vec<Job>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let queue = Arc::new(Queue::new(move |job| {
            sink.lock().unwrap().push(job.clone());
        }));

        (queue, events)
    }

    fn queue_at(path: &Path) -> (Arc<Queue>, Arc<Mutex<Vec<Job>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let queue = Arc::new(Queue::open(path, move |job| {
            sink.lock().unwrap().push(job.clone());
        }));

        (queue, events)
    }

    fn episodes(jobs: &[Job]) -> Vec<u32> {
        jobs.iter().map(|job| job.episode).collect()
    }

    async fn wait_until(queue: &Queue, done: impl Fn(&[Job]) -> bool) {
        for _ in 0..200 {
            let jobs = queue.jobs();
            if done(&jobs) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "the queue did not reach the expected state: {:?}",
            queue.jobs()
        );
    }

    /// A worker whose fake executor records overlap and order.
    struct CompletionWorker {
        task: JoinHandle<()>,
        peak: Arc<AtomicUsize>,
        started: Arc<Mutex<Vec<u32>>>,
        finished: Arc<Mutex<Vec<u32>>>,
    }

    fn completion_worker(queue: Arc<Queue>) -> CompletionWorker {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Mutex::new(Vec::new()));
        let finished = Arc::new(Mutex::new(Vec::new()));

        let task = tokio::spawn({
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let started = Arc::clone(&started);
            let finished = Arc::clone(&finished);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        let active = Arc::clone(&active);
                        let peak = Arc::clone(&peak);
                        let started = Arc::clone(&started);
                        let finished = Arc::clone(&finished);
                        async move {
                            let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(running, Ordering::SeqCst);
                            started.lock().unwrap().push(job.episode);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            finished.lock().unwrap().push(job.episode);
                            active.fetch_sub(1, Ordering::SeqCst);
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        CompletionWorker {
            task,
            peak,
            started,
            finished,
        }
    }

    #[tokio::test]
    async fn enqueues_jobs_as_queued_in_order() {
        let (queue, events) = queue();

        let outcome = queue.enqueue(
            vec![request(1, "1080p"), request(2, "720p"), request(3, "1080p")],
            |_| false,
        );

        assert_eq!(episodes(&outcome.jobs), vec![1, 2, 3]);
        assert_eq!(
            outcome.jobs.iter().map(|job| job.state).collect::<Vec<_>>(),
            vec![JobState::Queued; 3]
        );
        assert!(outcome.skipped.is_empty());
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .map(|job| job.state)
                .collect::<Vec<_>>(),
            vec![JobState::Queued; 3]
        );
    }

    #[tokio::test]
    async fn runs_one_job_at_a_time_in_enqueue_order() {
        let (queue, _) = queue();
        queue.enqueue(
            vec![request(1, "1080p"), request(2, "720p"), request(3, "1080p")],
            |_| false,
        );
        let worker = completion_worker(Arc::clone(&queue));

        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.task.abort();

        assert_eq!(*worker.started.lock().unwrap(), vec![1, 2, 3]);
        assert_eq!(*worker.finished.lock().unwrap(), vec![1, 2, 3]);
        assert_eq!(worker.peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn never_enqueues_an_active_episode_and_quality_twice() {
        let (queue, _) = queue();
        queue.enqueue(vec![request(1, "1080p")], |_| false);

        let while_queued = queue.enqueue(vec![request(1, "1080p"), request(1, "720p")], |_| false);

        assert_eq!(episodes(&while_queued.jobs), vec![1]);
        assert_eq!(while_queued.jobs[0].quality, "720p");
        assert_eq!(
            while_queued.skipped,
            vec![Skipped {
                anime_id: "anime-id".to_owned(),
                episode: 1,
                quality: "1080p".to_owned(),
                reason: SkipReason::InQueue,
            }]
        );

        let gate = Arc::new(Notify::new());
        let running = tokio::spawn({
            let queue = Arc::clone(&queue);
            let gate = Arc::clone(&gate);
            async move {
                queue
                    .run_next(move |mut job, _request| {
                        let gate = Arc::clone(&gate);
                        async move {
                            gate.notified().await;
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await
            }
        });

        wait_until(&queue, |jobs| {
            jobs.iter().any(|job| job.state == JobState::Running)
        })
        .await;

        let while_running = queue.enqueue(vec![request(1, "1080p")], |_| false);

        assert!(while_running.jobs.is_empty());
        assert_eq!(while_running.skipped[0].reason, SkipReason::InQueue);

        gate.notify_one();
        running.await.unwrap();
    }

    #[tokio::test]
    async fn skips_an_episode_the_library_owns() {
        let (queue, _) = queue();

        let outcome = queue.enqueue(vec![request(1, "1080p"), request(2, "1080p")], |request| {
            request.episode == 2
        });

        assert_eq!(episodes(&outcome.jobs), vec![1]);
        assert_eq!(
            outcome.skipped,
            vec![Skipped {
                anime_id: "anime-id".to_owned(),
                episode: 2,
                quality: "1080p".to_owned(),
                reason: SkipReason::InLibrary,
            }]
        );
    }

    #[tokio::test]
    async fn an_explicit_redownload_ignores_the_library_copy() {
        let (queue, _) = queue();
        let mut request = request(2, "1080p");
        request.redownload = true;

        let outcome = queue.enqueue(vec![request], |_| true);

        assert_eq!(episodes(&outcome.jobs), vec![2]);
        assert!(outcome.skipped.is_empty());
    }

    #[tokio::test]
    async fn a_failed_job_does_not_stop_the_queue() {
        let (queue, _) = queue();
        queue.enqueue(vec![request(1, "1080p"), request(2, "1080p")], |_| false);

        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            async move {
                queue
                    .serve(move |mut job, _request| async move {
                        if job.episode == 1 {
                            job.state = JobState::Failed;
                            job.error = Some("Every Source is dead.".to_owned());
                        } else {
                            job.state = JobState::Completed;
                        }
                        job
                    })
                    .await;
            }
        });

        wait_until(&queue, |jobs| {
            jobs.iter()
                .all(|job| matches!(job.state, JobState::Completed | JobState::Failed))
        })
        .await;
        worker.abort();

        let jobs = queue.jobs();
        assert_eq!(jobs[0].state, JobState::Failed);
        assert_eq!(jobs[0].error.as_deref(), Some("Every Source is dead."));
        assert_eq!(jobs[1].state, JobState::Completed);
    }

    #[tokio::test]
    async fn publishes_queued_running_and_completed() {
        let (queue, events) = queue();
        queue.enqueue(vec![request(1, "1080p")], |_| false);

        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            async move {
                queue
                    .serve(move |mut job, _request| async move {
                        job.state = JobState::Completed;
                        job
                    })
                    .await;
            }
        });

        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .map(|job| job.state)
                .collect::<Vec<_>>(),
            vec![JobState::Queued, JobState::Running, JobState::Completed]
        );
    }

    #[tokio::test]
    async fn a_progress_report_updates_the_stored_job() {
        let (queue, _) = queue();
        queue.enqueue(vec![request(1, "1080p")], |_| false);

        let mut job = queue.jobs()[0].clone();
        job.progress = Some(crate::download::Progress {
            bytes: 10,
            total: Some(100),
            speed: Some(5.0),
            eta_seconds: Some(18),
        });
        queue.report(&job);

        assert_eq!(queue.jobs()[0].progress, job.progress);
    }

    #[tokio::test]
    async fn restores_running_jobs_as_interrupted_and_never_auto_starts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("queue.json");

        {
            let (queue, _) = queue_at(&path);
            let outcome = queue.enqueue(
                vec![request(1, "1080p"), request(2, "720p"), request(3, "1080p")],
                |_| false,
            );

            let mut running = outcome.jobs[0].clone();
            running.state = JobState::Running;
            running.progress = Some(crate::download::Progress {
                bytes: 512,
                total: Some(2_048),
                speed: Some(128.0),
                eta_seconds: Some(12),
            });
            queue.report(&running);

            let mut failed = outcome.jobs[2].clone();
            failed.state = JobState::Failed;
            failed.error = Some("Every Source is dead.".to_owned());
            queue.report(&failed);
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let (queue, _) = queue_at(&path);
        assert!(!queue.snapshot().active, "a restored queue starts idle");

        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let calls = Arc::clone(&calls);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async move {
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let jobs = queue.jobs();
        assert_eq!(
            jobs.iter().map(|job| job.state).collect::<Vec<_>>(),
            vec![JobState::Interrupted, JobState::Queued, JobState::Failed]
        );
        assert_eq!(jobs[0].progress.as_ref().unwrap().bytes, 512);
        assert_eq!(jobs[2].error.as_deref(), Some("Every Source is dead."));

        worker.abort();
    }

    #[tokio::test]
    async fn resume_requeues_a_restored_job_with_its_request() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("queue.json");

        let id = {
            let (queue, _) = queue_at(&path);
            let outcome = queue.enqueue(vec![request(7, "1080p")], |_| false);
            let mut running = outcome.jobs[0].clone();
            running.state = JobState::Running;
            queue.report(&running);
            running.id
        };

        let (queue, _) = queue_at(&path);
        let started = Arc::new(Mutex::new(Vec::new()));
        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let started = Arc::clone(&started);
            async move {
                queue
                    .serve(move |mut job, request| {
                        let started = Arc::clone(&started);
                        async move {
                            started.lock().unwrap().push((job.episode, request.session));
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        assert_eq!(queue.jobs()[0].state, JobState::Interrupted);
        let resumed = queue.resume(&id).unwrap();
        assert_eq!(resumed.state, JobState::Queued);
        assert!(queue.snapshot().active);

        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(
            *started.lock().unwrap(),
            vec![(7, "session-7".to_owned())],
            "the restored request supplies the Episode and Session"
        );
    }

    #[tokio::test]
    async fn restored_queued_jobs_wait_for_an_explicit_start() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("queue.json");

        {
            let (queue, _) = queue_at(&path);
            queue.enqueue(vec![request(1, "1080p"), request(2, "1080p")], |_| false);
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let (queue, _) = queue_at(&path);
        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let calls = Arc::clone(&calls);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async move {
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            queue.jobs().iter().map(|job| job.state).collect::<Vec<_>>(),
            vec![JobState::Queued; 2]
        );

        queue.start();
        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_challenge_pauses_the_running_job_without_failing_it() {
        let (queue, _) = queue();
        queue.enqueue(vec![request(1, "1080p"), request(2, "1080p")], |_| false);

        let gate = Arc::new(Notify::new());
        let started = Arc::new(Mutex::new(Vec::new()));
        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let gate = Arc::clone(&gate);
            let started = Arc::clone(&started);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        let gate = Arc::clone(&gate);
                        let started = Arc::clone(&started);
                        async move {
                            started.lock().unwrap().push(job.episode);
                            if job.episode == 1 {
                                gate.notified().await;
                            }
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        wait_until(&queue, |jobs| {
            jobs.iter().any(|job| job.state == JobState::Running)
        })
        .await;

        queue.set_challenge(true);

        let paused = queue.jobs()[0].clone();
        assert_eq!(paused.state, JobState::Paused);
        assert_eq!(paused.error, None);
        assert_eq!(queue.jobs()[1].state, JobState::Queued);
        assert!(queue.snapshot().challenge);

        // The engine keeps reporting Running while it waits for clearance; the
        // stored job must stay Paused.
        let mut progress = paused.clone();
        progress.state = JobState::Running;
        progress.progress = Some(crate::download::Progress {
            bytes: 1_024,
            total: Some(2_048),
            speed: Some(64.0),
            eta_seconds: Some(16),
        });
        queue.report(&progress);
        assert_eq!(queue.jobs()[0].state, JobState::Paused);
        assert_eq!(queue.jobs()[0].progress.as_ref().unwrap().bytes, 1_024);

        queue.set_challenge(false);
        assert_eq!(queue.jobs()[0].state, JobState::Running);
        assert!(!queue.snapshot().challenge);

        gate.notify_one();
        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(*started.lock().unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn a_challenge_keeps_queued_jobs_from_starting() {
        let (queue, _) = queue();
        queue.enqueue(vec![request(1, "1080p"), request(2, "1080p")], |_| false);
        queue.set_challenge(true);

        let calls = Arc::new(AtomicUsize::new(0));
        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let calls = Arc::clone(&calls);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async move {
                            job.state = JobState::Completed;
                            job
                        }
                    })
                    .await;
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        queue.set_challenge(false);
        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_requeues_a_failed_job_and_runs_it_again() {
        let (queue, _) = queue();
        let outcome = queue.enqueue(vec![request(1, "1080p")], |_| false);
        let id = outcome.jobs[0].id.clone();

        let attempts = Arc::new(AtomicUsize::new(0));
        let worker = tokio::spawn({
            let queue = Arc::clone(&queue);
            let attempts = Arc::clone(&attempts);
            async move {
                queue
                    .serve(move |mut job, _request| {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        async move {
                            if attempt == 0 {
                                job.state = JobState::Failed;
                                job.error = Some("The connection dropped.".to_owned());
                            } else {
                                job.state = JobState::Completed;
                                job.error = None;
                            }
                            job
                        }
                    })
                    .await;
            }
        });

        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Failed)
        })
        .await;

        let retried = queue.resume(&id).unwrap();
        assert_eq!(retried.state, JobState::Queued);
        assert_eq!(retried.error, None);

        wait_until(&queue, |jobs| {
            jobs.iter().all(|job| job.state == JobState::Completed)
        })
        .await;
        worker.abort();

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}
