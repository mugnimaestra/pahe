import { useEffect, useState } from "react";
import Poster from "../components/Poster";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import {
  ipc,
  type AnimeDetail,
  type DownloadJob,
  type DownloadProgress,
  type Episode,
  type SearchResult,
} from "../ipc/client";
import { failureOf } from "../failure";
import { messageOf } from "../message";

type State =
  | { kind: "loading" }
  | { kind: "ready"; detail: AnimeDetail }
  | { kind: "failed"; message: string };

const QUALITIES = ["1080p", "720p", "360p"];
const DEFAULT_QUALITY = "1080p";

interface AnimeDetailViewProps {
  result: SearchResult;
  onBack: () => void;
}

export default function AnimeDetailView({
  result,
  onBack,
}: AnimeDetailViewProps) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [challenge] = useResolverChallenge();
  const [qualities, setQualities] = useState<Record<number, string>>({});
  const [jobs, setJobs] = useState<DownloadJob[]>([]);
  const [startErrors, setStartErrors] = useState<Record<number, string>>({});

  useEffect(() => {
    let active = true;
    setState({ kind: "loading" });

    ipc
      .loadAnime(result.animeId)
      .then((detail) => {
        if (active) setState({ kind: "ready", detail });
      })
      .catch((error) => {
        if (active) {
          setState({ kind: "failed", message: failureOf(error).message });
        }
      });

    return () => {
      active = false;
    };
  }, [result.animeId]);

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    ipc
      .onDownloadJob((job) => {
        if (!active) return;
        setJobs((current) => upsertJob(current, job));
      })
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        // Progress is advisory; a started job still reports its own failure.
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  async function startDownload(episode: Episode) {
    setStartErrors((current) => {
      const next = { ...current };
      delete next[episode.number];
      return next;
    });

    try {
      const job = await ipc.startDownload({
        animeId: result.animeId,
        title: result.title,
        episode: episode.number,
        session: episode.session,
        quality: qualities[episode.number] ?? DEFAULT_QUALITY,
      });
      setJobs((current) => upsertJob(current, job));
    } catch (error) {
      setStartErrors((current) => ({
        ...current,
        [episode.number]: messageOf(error),
      }));
    }
  }

  return (
    <article className="view view-detail">
      <button type="button" className="back" onClick={onBack}>
        ← Back to results
      </button>

      <header className="detail-head">
        <div className="detail-poster">
          <Poster url={result.poster} alt={`${result.title} poster`} />
        </div>
        <div className="detail-summary">
          <h1>{result.title}</h1>
          {result.year !== null && <p className="muted">{result.year}</p>}
          {state.kind === "ready" && state.detail.synopsis !== null && (
            <p className="synopsis">{state.detail.synopsis}</p>
          )}
        </div>
      </header>

      {challenge && (
        <p role="status" className="challenge">
          Cloudflare needs a human check. The Resolver window is open — finish
          the check and the Episodes will load.
        </p>
      )}

      {state.kind === "loading" && <p role="status">Loading Episodes…</p>}
      {state.kind === "failed" && <p role="alert">{state.message}</p>}

      {state.kind === "ready" && state.detail.episodes.length === 0 && (
        <p role="status">No Episodes are listed for this Anime yet.</p>
      )}

      {state.kind === "ready" && state.detail.episodes.length > 0 && (
        <section aria-labelledby="episodes-heading">
          <h2 id="episodes-heading">Episodes</h2>
          <ul className="episodes">
            {state.detail.episodes.map((episode) => {
              const job = latestJob(jobs, result.animeId, episode.number);
              const busy = job?.state === "running" || job?.state === "queued";

              return (
                <li key={episode.number}>
                  <span>Episode {episode.number}</span>
                  <span className="muted">
                    {episode.airDate ?? "Air date unknown"}
                  </span>
                  <div className="episode-download">
                    <select
                      aria-label={`Quality for Episode ${episode.number}`}
                      value={qualities[episode.number] ?? DEFAULT_QUALITY}
                      onChange={(event) => {
                        const quality = event.currentTarget.value;
                        setQualities((current) => ({
                          ...current,
                          [episode.number]: quality,
                        }));
                      }}
                    >
                      {QUALITIES.map((quality) => (
                        <option key={quality} value={quality}>
                          {quality}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      aria-label={`Download Episode ${episode.number}`}
                      disabled={busy}
                      onClick={() => startDownload(episode)}
                    >
                      Download
                    </button>
                    <EpisodeDownloadStatus
                      job={job}
                      startError={startErrors[episode.number]}
                    />
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}
    </article>
  );
}

function EpisodeDownloadStatus({
  job,
  startError,
}: {
  job: DownloadJob | undefined;
  startError: string | undefined;
}) {
  if (startError !== undefined) {
    return (
      <span role="alert" className="download-error">
        {startError}
      </span>
    );
  }
  if (job === undefined) return null;

  switch (job.state) {
    case "queued":
      return <span className="download-progress">Queued</span>;
    case "completed":
      return <span className="download-done">Saved to the Library.</span>;
    case "failed":
      return (
        <span role="alert" className="download-error">
          {job.error ?? "The download failed."}
        </span>
      );
    case "running":
      return (
        <span className="download-progress">
          {job.progress === null
            ? "Preparing…"
            : progressText(job.progress)}
        </span>
      );
  }
}

function progressText(progress: DownloadProgress): string {
  const total =
    progress.total === null ? "" : ` / ${formatBytes(progress.total)}`;
  const speed =
    progress.speed === null ? "" : ` · ${formatBytes(progress.speed)}/s`;
  const eta =
    progress.etaSeconds === null ? "" : ` · ${formatEta(progress.etaSeconds)}`;

  return `${formatBytes(progress.bytes)}${total}${speed}${eta}`;
}

function formatBytes(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

function formatEta(seconds: number): string {
  const rounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(rounded / 60);
  const remaining = rounded % 60;

  return minutes > 0 ? `${minutes}m ${remaining}s left` : `${remaining}s left`;
}

function upsertJob(jobs: DownloadJob[], job: DownloadJob): DownloadJob[] {
  const index = jobs.findIndex((current) => current.id === job.id);
  if (index === -1) return [...jobs, job];

  const next = jobs.slice();
  next[index] = job;
  return next;
}

function latestJob(
  jobs: DownloadJob[],
  animeId: string,
  episode: number,
): DownloadJob | undefined {
  for (let index = jobs.length - 1; index >= 0; index -= 1) {
    const job = jobs[index];
    if (job.animeId === animeId && job.episode === episode) return job;
  }
  return undefined;
}
