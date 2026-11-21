import { useEffect, useState } from "react";
import {
  ipc,
  type LibraryAnime,
  type LibraryItem,
  type WatchlistEntry,
} from "../ipc/client";
import { messageOf } from "../message";
import { formatClock } from "../watchlist";
import PlayerView from "./PlayerView";

type State =
  | { kind: "loading" }
  | { kind: "ready"; items: LibraryItem[] }
  | { kind: "failed"; message: string };

interface LibraryAnimeViewProps {
  anime: LibraryAnime;
  onBack: () => void;
}

export default function LibraryAnimeView({
  anime,
  onBack,
}: LibraryAnimeViewProps) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [playing, setPlaying] = useState<LibraryItem | null>(null);
  const [watchlistEntry, setWatchlistEntry] = useState<WatchlistEntry | null>(
    null,
  );

  useEffect(() => {
    let active = true;
    setState({ kind: "loading" });

    ipc
      .libraryItems(anime.animeId, anime.stableId, anime.title)
      .then((items) => {
        if (active) setState({ kind: "ready", items });
      })
      .catch((error) => {
        if (active) setState({ kind: "failed", message: messageOf(error) });
      });

    return () => {
      active = false;
    };
  }, [anime.animeId, anime.stableId, anime.title]);

  useEffect(() => {
    let active = true;
    Promise.resolve(
      ipc.getWatchlistEntry?.(anime.stableId, anime.animeId, anime.title),
    )
      .then((entry) => {
        if (active) setWatchlistEntry(entry ?? null);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [anime.animeId, anime.stableId, anime.title, playing]);

  if (playing !== null) {
    const items = state.kind === "ready" ? state.items : [];
    const nextItem =
      items.find((candidate) => candidate.episode > playing.episode) ?? null;
    return (
      <PlayerView
        playback={{ kind: "library", item: playing }}
        title={anime.title}
        poster={anime.poster}
        nextEpisode={nextItem ? { episode: nextItem.episode } : null}
        onNextEpisode={nextItem ? () => setPlaying(nextItem) : undefined}
        onBack={() => setPlaying(null)}
      />
    );
  }

  return (
    <article className="view view-library-episodes">
      <button type="button" className="back" onClick={onBack}>
        ← Back to the Library
      </button>

      <header>
        <h1>{anime.title}</h1>
        <p className="muted">Downloaded Episodes.</p>
      </header>

      {state.kind === "loading" && <p role="status">Listing Episodes…</p>}
      {state.kind === "failed" && <p role="alert">{state.message}</p>}

      {state.kind === "ready" && state.items.length === 0 && (
        <p role="status">
          No Library items for this Anime. Scan the Library again.
        </p>
      )}

      {state.kind === "ready" && state.items.length > 0 && (
        <ul className="episodes" aria-label="Library Episodes">
          {state.items.map((item) => (
            <li key={item.path}>
              <span>Episode {item.episode}</span>
              <span className="muted">
                {item.quality}
                {item.group === "" ? "" : ` · ${item.group}`}
              </span>
              {watchlistEntry !== null &&
                item.episode <= watchlistEntry.progress && (
                  <span className="episode-watched">Watched</span>
                )}
              {watchlistEntry?.lastWatched?.episode === item.episode &&
                watchlistEntry.lastWatched.positionSeconds > 0 && (
                  <span className="watchlist-badge">
                    Paused {formatClock(watchlistEntry.lastWatched.positionSeconds)}
                  </span>
                )}
              <div className="episode-download">
                <button type="button" onClick={() => setPlaying(item)}>
                  Play
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </article>
  );
}
