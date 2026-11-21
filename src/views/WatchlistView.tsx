import { startTransition, useEffect, useMemo, useOptimistic, useState } from "react";
import Poster from "../components/Poster";
import { QUALITIES } from "../downloads";
import {
  ipc,
  type SearchResult,
  type WatchStatus,
  type WatchlistEntry,
} from "../ipc/client";
import { messageOf } from "../message";
import {
  formatClock,
  progressPercent,
  WATCH_STATUSES,
  WATCH_STATUS_LABELS,
} from "../watchlist";
import AnimeDetailView from "./AnimeDetailView";
import PlayerView, { type Playback } from "./PlayerView";

type FilterTab = "all" | WatchStatus;
type SortBy = "updated" | "title" | "score";

type OptimisticAction =
  | { type: "upsert"; entry: WatchlistEntry }
  | { type: "remove"; key: string };

interface ActivePlayer {
  playback: Playback;
  title: string;
  poster: string | null;
  totalEpisodes: number | null;
}

export default function WatchlistView() {
  const [entries, setEntries] = useState<WatchlistEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [tab, setTab] = useState<FilterTab>("all");
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<SortBy>("updated");
  const [defaultQuality, setDefaultQuality] = useState(QUALITIES[0]);
  const [openingKey, setOpeningKey] = useState<string | null>(null);
  const [player, setPlayer] = useState<ActivePlayer | null>(null);
  const [detail, setDetail] = useState<SearchResult | null>(null);

  const [optimisticEntries, applyOptimistic] = useOptimistic(
    entries ?? [],
    (current: WatchlistEntry[], action: OptimisticAction): WatchlistEntry[] => {
      if (action.type === "remove") {
        return current.filter((item) => item.key !== action.key);
      }
      const idx = current.findIndex((item) => item.key === action.entry.key);
      if (idx === -1) return [action.entry, ...current];
      const next = [...current];
      next[idx] = action.entry;
      return next;
    },
  );

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    ipc
      .listWatchlist()
      .then((list) => {
        if (active) setEntries(list);
      })
      .catch((failure: unknown) => {
        if (active) setError(messageOf(failure));
      });

    ipc
      .getSettings()
      .then((settings) => {
        if (active) setDefaultQuality(settings.defaultQuality);
      })
      .catch(() => {});

    Promise.resolve(
      ipc.onWatchlistUpdated?.((updated) => {
        if (active) setEntries(updated);
      }),
    )
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten?.();
      })
      .catch(() => {});

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  const continueWatching = useMemo(() => {
    return optimisticEntries
      .filter(
        (entry) =>
          entry.lastWatched !== null &&
          entry.status !== "completed" &&
          entry.status !== "dropped",
      )
      .sort(
        (a, b) =>
          (b.lastWatched?.updatedAt ?? 0) - (a.lastWatched?.updatedAt ?? 0),
      );
  }, [optimisticEntries]);

  const counts = useMemo(() => {
    const map: Record<FilterTab, number> = {
      all: optimisticEntries.length,
      watching: 0,
      planToWatch: 0,
      completed: 0,
      onHold: 0,
      dropped: 0,
    };
    for (const entry of optimisticEntries) {
      map[entry.status] += 1;
    }
    return map;
  }, [optimisticEntries]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return optimisticEntries
      .filter((entry) => (tab === "all" ? true : entry.status === tab))
      .filter((entry) =>
        needle ? entry.title.toLowerCase().includes(needle) : true,
      )
      .sort((a, b) => {
        if (sortBy === "title") {
          return a.title.localeCompare(b.title, undefined, {
            sensitivity: "base",
          });
        }
        if (sortBy === "score") {
          return (
            (b.score ?? -1) - (a.score ?? -1) ||
            a.title.localeCompare(b.title, undefined, { sensitivity: "base" })
          );
        }
        return b.updatedAt - a.updatedAt;
      });
  }, [optimisticEntries, tab, query, sortBy]);

  function mutateEntry(next: WatchlistEntry) {
    setNotice(null);
    startTransition(async () => {
      applyOptimistic({ type: "upsert", entry: next });
      try {
        const saved = await ipc.upsertWatchlistEntry({
          stableId: next.stableId,
          animeId: next.animeId,
          title: next.title,
          poster: next.poster,
          status: next.status,
          progress: next.progress,
          totalEpisodes: next.totalEpisodes,
          score: next.score,
          notes: next.notes,
        });
        setEntries((current) => {
          const list = current ?? [];
          const idx = list.findIndex((item) => item.key === saved.key);
          if (idx === -1) return [saved, ...list];
          const copy = [...list];
          copy[idx] = saved;
          return copy;
        });
      } catch (failure) {
        setNotice(messageOf(failure));
      }
    });
  }

  function removeEntry(key: string) {
    setNotice(null);
    startTransition(async () => {
      applyOptimistic({ type: "remove", key });
      try {
        await ipc.removeWatchlistEntry(key);
        setEntries((current) =>
          (current ?? []).filter((item) => item.key !== key),
        );
      } catch (failure) {
        setNotice(messageOf(failure));
      }
    });
  }

  function openDetail(entry: WatchlistEntry) {
    if (!entry.animeId) return;
    setDetail({
      animeId: entry.animeId,
      stableId: entry.stableId,
      title: entry.title,
      year: null,
      poster: entry.poster,
    });
  }

  async function resumeEntry(entry: WatchlistEntry) {
    const lw = entry.lastWatched;
    if (!lw || openingKey !== null) return;
    setNotice(null);
    setOpeningKey(entry.key);
    try {
      const items = await ipc
        .libraryItems(entry.animeId, entry.stableId, entry.title)
        .catch(() => []);
      const localMatch = items.find((item) => item.episode === lw.episode);
      if (localMatch) {
        setPlayer({
          playback: { kind: "library", item: localMatch },
          title: entry.title,
          poster: entry.poster,
          totalEpisodes: entry.totalEpisodes,
        });
        return;
      }

      if (entry.animeId && lw.session) {
        setPlayer({
          playback: {
            kind: "stream",
            animeId: entry.animeId,
            stableId: entry.stableId,
            episode: lw.episode,
            session: lw.session,
            quality: lw.quality ?? defaultQuality,
          },
          title: entry.title,
          poster: entry.poster,
          totalEpisodes: entry.totalEpisodes,
        });
        return;
      }

      if (entry.animeId) {
        openDetail(entry);
      } else {
        setNotice(
          `Open ${entry.title} from Search or Library to resume Episode ${lw.episode}.`,
        );
      }
    } finally {
      setOpeningKey(null);
    }
  }

  if (player !== null) {
    return (
      <PlayerView
        playback={player.playback}
        title={player.title}
        poster={player.poster}
        totalEpisodes={player.totalEpisodes}
        onBack={() => setPlayer(null)}
      />
    );
  }

  if (detail !== null) {
    return <AnimeDetailView result={detail} onBack={() => setDetail(null)} />;
  }

  return (
    <section className="view view-watchlist">
      <header className="view-intro">
        <h1>Watchlist</h1>
        <p className="muted">
          Your local anime list and Continue Watching progress.
        </p>
      </header>

      {error !== null && <p role="alert">{error}</p>}
      {notice !== null && <p role="alert">{notice}</p>}
      {entries === null && error === null && (
        <p role="status">Loading Watchlist…</p>
      )}

      {entries !== null && continueWatching.length > 0 && (
        <section
          className="continue-section"
          aria-labelledby="continue-watching-heading"
        >
          <h2 id="continue-watching-heading">Continue Watching</h2>
          <ul className="continue-rail" aria-label="Continue Watching">
            {continueWatching.map((entry) => {
              const lw = entry.lastWatched!;
              const pct = progressPercent(lw.positionSeconds, lw.durationSeconds);
              const isNextUp = lw.positionSeconds <= 0;
              return (
                <li key={entry.key} className="continue-card">
                  <div className="continue-poster">
                    <Poster url={entry.poster} alt={`${entry.title} poster`} />
                  </div>
                  <div className="continue-meta">
                    <span className="continue-title">{entry.title}</span>
                    <span className="muted">
                      {isNextUp
                        ? `Next up · Episode ${lw.episode}`
                        : `Episode ${lw.episode} · ${formatClock(lw.positionSeconds)}${
                            lw.durationSeconds
                              ? ` / ${formatClock(lw.durationSeconds)}`
                              : ""
                          }`}
                    </span>
                    {!isNextUp && pct > 0 && (
                      <div
                        className="progress-bar"
                        role="progressbar"
                        aria-valuenow={pct}
                        aria-valuemin={0}
                        aria-valuemax={100}
                      >
                        <span style={{ width: `${pct}%` }} />
                      </div>
                    )}
                    <button
                      type="button"
                      className="button"
                      disabled={openingKey === entry.key}
                      onClick={() => resumeEntry(entry)}
                    >
                      {openingKey === entry.key
                        ? "Opening…"
                        : isNextUp
                          ? `Play Ep ${lw.episode}`
                          : `Resume Ep ${lw.episode}`}
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {entries !== null && (
        <>
          <div className="tabs" role="tablist" aria-label="Watchlist status">
            <button
              type="button"
              role="tab"
              aria-selected={tab === "all"}
              onClick={() => setTab("all")}
            >
              All ({counts.all})
            </button>
            {WATCH_STATUSES.map((status) => (
              <button
                key={status}
                type="button"
                role="tab"
                aria-selected={tab === status}
                onClick={() => setTab(status)}
              >
                {WATCH_STATUS_LABELS[status]} ({counts[status]})
              </button>
            ))}
          </div>

          <div className="watchlist-toolbar">
            <input
              type="search"
              aria-label="Filter Watchlist"
              placeholder="Filter by title…"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
            <select
              aria-label="Sort Watchlist"
              value={sortBy}
              onChange={(event) =>
                setSortBy(event.currentTarget.value as SortBy)
              }
            >
              <option value="updated">Last updated</option>
              <option value="title">Title</option>
              <option value="score">Score</option>
            </select>
          </div>

          {filtered.length === 0 ? (
            <p role="status" className="muted">
              {optimisticEntries.length === 0
                ? "Your Watchlist is empty. Play an Episode or add an Anime from Search or Browse."
                : "No Watchlist entries match this filter."}
            </p>
          ) : (
            <ul className="watchlist-list" aria-label="Watchlist entries">
              {filtered.map((entry) => (
                <li key={entry.key} className="watchlist-row">
                  <div className="watchlist-thumb">
                    <Poster url={entry.poster} alt={`${entry.title} poster`} />
                  </div>
                  <div className="watchlist-info">
                    {entry.animeId ? (
                      <button
                        type="button"
                        className="watchlist-title-link"
                        onClick={() => openDetail(entry)}
                      >
                        {entry.title}
                      </button>
                    ) : (
                      <span className="watchlist-title">{entry.title}</span>
                    )}
                    <span className="muted">
                      Ep {entry.progress}
                      {entry.totalEpisodes !== null
                        ? ` / ${entry.totalEpisodes}`
                        : ""}
                    </span>
                  </div>
                  <div className="watchlist-actions">
                    <select
                      aria-label={`Status for ${entry.title}`}
                      value={entry.status}
                      onChange={(event) =>
                        mutateEntry({
                          ...entry,
                          status: event.currentTarget.value as WatchStatus,
                          updatedAt: Date.now(),
                        })
                      }
                    >
                      {WATCH_STATUSES.map((status) => (
                        <option key={status} value={status}>
                          {WATCH_STATUS_LABELS[status]}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      aria-label={`Increment Episode for ${entry.title}`}
                      onClick={() => {
                        const nextProgress = entry.progress + 1;
                        const reachedEnd =
                          entry.totalEpisodes !== null &&
                          entry.totalEpisodes > 0 &&
                          nextProgress >= entry.totalEpisodes;
                        mutateEntry({
                          ...entry,
                          progress:
                            entry.totalEpisodes !== null &&
                            entry.totalEpisodes > 0
                              ? Math.min(nextProgress, entry.totalEpisodes)
                              : nextProgress,
                          status: reachedEnd
                            ? "completed"
                            : entry.status === "planToWatch"
                              ? "watching"
                              : entry.status,
                          updatedAt: Date.now(),
                        });
                      }}
                    >
                      +1 Ep
                    </button>
                    <select
                      aria-label={`Score for ${entry.title}`}
                      value={entry.score === null ? "" : String(entry.score)}
                      onChange={(event) => {
                        const raw = event.currentTarget.value;
                        mutateEntry({
                          ...entry,
                          score: raw === "" ? null : Number(raw),
                          updatedAt: Date.now(),
                        });
                      }}
                    >
                      <option value="">No score</option>
                      {[10, 9, 8, 7, 6, 5, 4, 3, 2, 1].map((n) => (
                        <option key={n} value={n}>
                          ★ {n}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      aria-label={`Remove ${entry.title}`}
                      onClick={() => removeEntry(entry.key)}
                    >
                      Remove
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
