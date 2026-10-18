use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::Notify;

use crate::download::{Job, JobState, Request};

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

struct State {
    jobs: Vec<Job>,
    requests: HashMap<String, Request>,
}

/// A serial Download job queue: at most one job runs, the rest wait in order.
pub struct Queue {
    state: Mutex<State>,
    wake: Notify,
    report: Box<dyn Fn(&Job) + Send + Sync>,
}

impl Queue {
    /// `report` receives every job change, so the UI can follow it by job id.
    pub fn new(report: impl Fn(&Job) + Send + Sync + 'static) -> Self {
        Self {
            state: Mutex::new(State {
                jobs: Vec::new(),
                requests: HashMap::new(),
            }),
            wake: Notify::new(),
            report: Box::new(report),
        }
    }

    /// Every job in enqueue order, with its latest state.
    pub fn jobs(&self) -> Vec<Job> {
        self.state.lock().expect("queue lock poisoned").jobs.clone()
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
            if state.jobs.iter().any(|job| active(job, &request)) {
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
        drop(state);

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
    pub fn report(&self, job: &Job) {
        {
            let mut state = self.state.lock().expect("queue lock poisoned");
            if let Some(stored) = state.jobs.iter_mut().find(|stored| stored.id == job.id) {
                *stored = job.clone();
            }
        }
        (self.report)(job);
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
                            .remove(&job.id)
                            .expect("a queued job keeps its request");
                        (job, request)
                    })
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
}

fn active(job: &Job, request: &Request) -> bool {
    matches!(job.state, JobState::Queued | JobState::Running)
        && job.anime_id == request.anime_id
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
}
