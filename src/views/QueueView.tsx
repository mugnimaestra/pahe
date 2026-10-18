import { useEffect, useState } from "react";
import { mergeJobs, progressText, upsertJob } from "../downloads";
import { ipc, type DownloadJob } from "../ipc/client";
import { messageOf } from "../message";

type State =
  | { kind: "loading" }
  | { kind: "ready" }
  | { kind: "failed"; message: string };

export default function QueueView() {
  const [jobs, setJobs] = useState<DownloadJob[]>([]);
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    ipc
      .loadQueue()
      .then((loaded) => {
        if (!active) return;
        setJobs((current) => mergeJobs(current, loaded));
        setState({ kind: "ready" });
      })
      .catch((error) => {
        if (active) setState({ kind: "failed", message: messageOf(error) });
      });

    ipc
      .onDownloadJob((job) => {
        if (active) setJobs((current) => upsertJob(current, job));
      })
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        // The loaded snapshot still shows the queue if events cannot attach.
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  return (
    <section className="view view-queue">
      <header>
        <h1>Queue</h1>
        <p className="muted">Download jobs run one at a time.</p>
      </header>

      {state.kind === "loading" && <p role="status">Reading the queue…</p>}
      {state.kind === "failed" && <p role="alert">{state.message}</p>}

      {state.kind === "ready" && jobs.length === 0 && (
        <p role="status">No Download jobs yet.</p>
      )}

      {jobs.length > 0 && (
        <ul className="queue" aria-label="Download queue">
          {jobs.map((job) => (
            <li key={job.id}>
              <span className="result-title">{job.title}</span>
              <span>Episode {job.episode}</span>
              <span className="muted">{job.quality}</span>
              <JobStatus job={job} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
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
          Failed · {job.error ?? "The download failed."}
        </span>
      );
  }
}
