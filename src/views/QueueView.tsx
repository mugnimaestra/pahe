import { useEffect, useState } from "react";
import { upsertJob, mergeJobs, progressText } from "../downloads";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import { ipc, type DownloadJob, type QueueSnapshot } from "../ipc/client";
import { failureText } from "../failure";
import { messageOf } from "../message";

type State =
  | { kind: "loading" }
  | { kind: "ready" }
  | { kind: "failed"; message: string };

export default function QueueView() {
  const [jobs, setJobs] = useState<DownloadJob[]>([]);
  const [active, setActive] = useState(false);
  const [state, setState] = useState<State>({ kind: "loading" });
  const [challenge, setChallenge] = useResolverChallenge();

  function apply(snapshot: QueueSnapshot) {
    // A command response is the queue's own state at that moment, so it wins.
    setJobs(snapshot.jobs);
    setActive(snapshot.active);
    setChallenge(snapshot.challenge);
  }

  async function run(action: () => Promise<QueueSnapshot>) {
    try {
      apply(await action());
    } catch (error) {
      setState({ kind: "failed", message: messageOf(error) });
    }
  }

  useEffect(() => {
    let mounted = true;
    let stop: (() => void) | undefined;

    ipc
      .loadQueue()
      .then((snapshot) => {
        if (!mounted) return;
        setJobs((current) => mergeJobs(current, snapshot.jobs));
        setActive(snapshot.active);
        setChallenge(snapshot.challenge);
        setState({ kind: "ready" });
      })
      .catch((error) => {
        if (mounted) setState({ kind: "failed", message: messageOf(error) });
      });

    ipc
      .onDownloadJob((job) => {
        if (mounted) setJobs((current) => upsertJob(current, job));
      })
      .then((unlisten) => {
        if (mounted) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        // The loaded snapshot still shows the queue if events cannot attach.
      });

    return () => {
      mounted = false;
      stop?.();
    };
  }, []);

  const restartable = !active && jobs.some((job) => job.state === "queued");

  return (
    <section className="view view-queue">
      <header>
        <h1>Queue</h1>
        <p className="muted">Download jobs run one at a time.</p>
      </header>

      {state.kind === "loading" && <p role="status">Reading the queue…</p>}
      {state.kind === "failed" && <p role="alert">{state.message}</p>}

      {challenge && (
        <p role="status" className="challenge">
          Cloudflare needs a human check. The Resolver window is open — finish
          the check and the queued downloads will continue.
        </p>
      )}

      {state.kind === "ready" && jobs.length === 0 && (
        <p role="status">No Download jobs yet.</p>
      )}

      {restartable && (
        <button
          type="button"
          className="queue-start"
          onClick={() => run(() => ipc.startQueue())}
        >
          Start downloads
        </button>
      )}

      {jobs.length > 0 && (
        <ul className="queue" aria-label="Download queue">
          {jobs.map((job) => (
            <li key={job.id}>
              <span className="result-title">{job.title}</span>
              <span>Episode {job.episode}</span>
              <span className="muted">{job.quality}</span>
              <JobStatus job={job} />
              {actionFor(job) && (
                <button
                  type="button"
                  onClick={() => run(() => ipc.resumeDownload(job.id))}
                >
                  {actionFor(job)}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** The user action a job's state offers, if any. */
function actionFor(job: DownloadJob): string | null {
  switch (job.state) {
    case "failed":
      return "Retry";
    case "interrupted":
    case "paused":
      return "Resume";
    default:
      return null;
  }
}

function JobStatus({ job }: { job: DownloadJob }) {
  switch (job.state) {
    case "queued":
      return <span className="download-progress">Queued</span>;
    case "running":
      return (
        <span className="download-progress">
          {job.progress === null
            ? "Downloading…"
            : `Downloading · ${progressText(job.progress)}`}
        </span>
      );
    case "completed":
      return <span className="download-done">Completed</span>;
    case "failed":
      return (
        <span role="alert" className="download-error">
          {failureText(job)}
        </span>
      );
    case "interrupted":
      return (
        <span className="download-paused">
          Interrupted · pahe closed while this Download ran. Resume restarts it
          from the beginning.
        </span>
      );
    case "paused":
      return (
        <span className="download-paused">
          Paused · a Cloudflare check is pending. Finish it in the Resolver
          window, then Resume.
        </span>
      );
  }
}
