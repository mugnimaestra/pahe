import { useEffect, useState } from "react";
import {
  ipc,
  type LibraryAnime,
  type LibraryItem,
} from "../ipc/client";
import { messageOf } from "../message";
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

  useEffect(() => {
    let active = true;
    setState({ kind: "loading" });

    ipc
      .libraryItems(anime.animeId, anime.title)
      .then((items) => {
        if (active) setState({ kind: "ready", items });
      })
      .catch((error) => {
        if (active) setState({ kind: "failed", message: messageOf(error) });
      });

    return () => {
      active = false;
    };
  }, [anime.animeId, anime.title]);

  if (playing !== null) {
    return (
      <PlayerView
        item={playing}
        title={anime.title}
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
