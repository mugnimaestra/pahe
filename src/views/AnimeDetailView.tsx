import { useEffect, useState } from "react";
import Poster from "../components/Poster";
import { progressText, upsertJob, QUALITIES } from "../downloads";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import {
  ipc,
  type AnimeDetail,
  type DownloadJob,
  type Enqueued,
  type Episode,
  type OwnedEpisode,
  type SearchResult,
  type SkippedDownload,
} from "../ipc/client";
import { failureOf, failureText } from "../failure";
import { messageOf } from "../message";

type State =
  | { kind: "loading" }
  | { kind: "ready"; detail: AnimeDetail }
  | { kind: "failed"; message: string };

type Notice = { kind: "info" | "error"; message: string };

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
  const [defaultQuality, setDefaultQuality] = useState(QUALITIES[0]);
  const [qualities, setQualities] = useState<Record<number, string>>({});
  const [selected, setSelected] = useState<number[]>([]);
  const [owned, setOwned] = useState<OwnedEpisode[]>([]);
  const [jobs, setJobs] = useState<DownloadJob[]>([]);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [enqueueing, setEnqueueing] = useState(false);

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

    ipc
      .getSettings()
      .then((settings) => {
        if (active) setDefaultQuality(settings.defaultQuality);
      })
      .catch(() => {
        // The tallest Quality stands in until Settings can be read.
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;

    ipc
      .ownedEpisodes(result.animeId, result.title)
      .then((episodes) => {
        if (active) setOwned(episodes);
      })
      .catch(() => {
        // Ownership is advisory; the Rust queue still refuses duplicates.
      });

    return () => {
      active = false;
    };
  }, [result.animeId, result.title]);

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    ipc
      .onDownloadJob((job) => {
        if (!active) return;
        setJobs((current) => upsertJob(current, job));
        if (job.state !== "completed" || job.animeId !== result.animeId) return;

        setSelected((current) =>
          current.filter((number) => number !== job.episode),
        );
        ipc
          .ownedEpisodes(result.animeId, result.title)
          .then((episodes) => {
            if (active) setOwned(episodes);
          })
          .catch(() => {
            // The completed job already reports its own success.
          });
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
  }, [result.animeId, result.title]);

  function qualityOf(episode: number): string {
    return qualities[episode] ?? defaultQuality;
  }

  function ownedAt(episode: number, quality: string): boolean {
    return owned.some(
      (item) => item.episode === episode && item.quality === quality,
    );
  }

  function ownedQualities(episode: number): string[] {
    return owned
      .filter((item) => item.episode === episode)
      .map((item) => item.quality);
  }

  function toggle(episode: number) {
    setSelected((current) =>
      current.includes(episode)
        ? current.filter((number) => number !== episode)
        : [...current, episode],
    );
  }

  async function enqueue(episodes: Episode[], redownload: boolean) {
    if (episodes.length === 0 || enqueueing) return;

    setNotice(null);
    setEnqueueing(true);
    try {
      const outcome = await ipc.enqueueDownloads(
        episodes.map((episode) => ({
          animeId: result.animeId,
          title: result.title,
          episode: episode.number,
          session: episode.session,
          quality: qualityOf(episode.number),
          redownload,
        })),
      );
      setJobs((current) => outcome.jobs.reduce(upsertJob, current));
      setSelected([]);
      setNotice(describe(outcome));
    } catch (error) {
      setNotice({ kind: "error", message: messageOf(error) });
    } finally {
      setEnqueueing(false);
    }
  }

  const episodes = state.kind === "ready" ? state.detail.episodes : [];
  const available = episodes.filter(
    (episode) => !ownedAt(episode.number, qualityOf(episode.number)),
  );
  const picked = available.filter((episode) =>
    selected.includes(episode.number),
  );

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

          <div className="episode-actions">
            <button
              type="button"
              disabled={enqueueing || picked.length === 0}
              onClick={() => enqueue(picked, false)}
            >
              Download selected
            </button>
            <button
              type="button"
              disabled={enqueueing || available.length === 0}
              onClick={() => enqueue(available, false)}
            >
              Download all
            </button>
          </div>

          {notice && (
            <p role={notice.kind === "error" ? "alert" : "status"}>
              {notice.message}
            </p>
          )}

          <ul className="episodes">
            {state.detail.episodes.map((episode) => {
              const quality = qualityOf(episode.number);
              const ownedHere = ownedAt(episode.number, quality);
              const job = latestJob(jobs, result.animeId, episode.number);
              const busy = job?.state === "running" || job?.state === "queued";

              return (
                <li key={episode.number}>
                  <input
                    type="checkbox"
                    aria-label={`Select Episode ${episode.number}`}
                    checked={!ownedHere && selected.includes(episode.number)}
                    disabled={ownedHere}
                    onChange={() => toggle(episode.number)}
                  />
                  <span>Episode {episode.number}</span>
                  <span className="muted">
                    {episode.airDate ?? "Air date unknown"}
                  </span>
                  <div className="episode-download">
                    <select
                      aria-label={`Quality for Episode ${episode.number}`}
                      value={quality}
                      onChange={(event) => {
                        const next = event.currentTarget.value;
                        setQualities((current) => ({
                          ...current,
                          [episode.number]: next,
                        }));
                      }}
                    >
                      {QUALITIES.map((label) => (
                        <option key={label} value={label}>
                          {label}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      aria-label={`${ownedHere ? "Redownload" : "Download"} Episode ${episode.number}`}
                      disabled={busy || enqueueing}
                      onClick={() => enqueue([episode], ownedHere)}
                    >
                      {ownedHere ? "Redownload" : "Download"}
                    </button>
                    {ownedQualities(episode.number).length > 0 && (
                      <span className="download-done">
                        Owned {ownedQualities(episode.number).join(", ")}
                      </span>
                    )}
                    <EpisodeDownloadStatus job={job} />
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

function EpisodeDownloadStatus({ job }: { job: DownloadJob | undefined }) {
  if (job === undefined) return null;

  switch (job.state) {
    case "queued":
      return <span className="download-progress">Queued</span>;
    case "completed":
      return <span className="download-done">Saved to the Library.</span>;
    case "failed":
      return (
        <span role="alert" className="download-error">
          {failureText(job)}
        </span>
      );
    case "running":
      return (
        <span className="download-progress">
          {job.progress === null ? "Preparing…" : progressText(job.progress)}
        </span>
      );
  }
}

function describe(outcome: Enqueued): Notice | null {
  const parts = [
    skippedNote(outcome.skipped, "inLibrary", "Already in the Library"),
    skippedNote(outcome.skipped, "inQueue", "Already queued"),
  ].filter((part): part is string => part !== null);

  return parts.length > 0
    ? { kind: "info", message: parts.join(" ") }
    : null;
}

function skippedNote(
  skipped: SkippedDownload[],
  reason: SkippedDownload["reason"],
  label: string,
): string | null {
  const matches = skipped.filter((item) => item.reason === reason);
  if (matches.length === 0) return null;

  const episodes = matches
    .map((item) => `Episode ${item.episode} (${item.quality})`)
    .join(", ");

  return `${label}: ${episodes}.`;
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
