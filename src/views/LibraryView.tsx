import { useEffect, useState } from "react";
import Poster from "../components/Poster";
import { ipc, type LibraryAnime } from "../ipc/client";
import { messageOf } from "../message";

type State =
  | { kind: "loading" }
  | { kind: "ready"; anime: LibraryAnime[] }
  | { kind: "failed"; message: string };

export default function LibraryView() {
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let active = true;

    ipc
      .loadLibrary()
      .then((anime) => {
        if (!active) return;
        setState({ kind: "ready", anime });
        // Enrichment is network-bound, so the scan result stays visible while
        // it runs, and a failure leaves the placeholders in place.
        return ipc.enrichLibrary();
      })
      .then((anime) => {
        if (active && anime) setState({ kind: "ready", anime });
      })
      .catch((error) => {
        if (!active) return;
        setState((current) =>
          current.kind === "ready"
            ? current
            : { kind: "failed", message: messageOf(error) },
        );
      });

    return () => {
      active = false;
    };
  }, []);

  return (
    <section className="view view-library">
      <header>
        <h1>Library</h1>
        <p className="muted">Downloaded Episodes, grouped by Anime.</p>
      </header>

      {state.kind === "loading" && (
        <p role="status">Scanning the Library folder…</p>
      )}

      {state.kind === "failed" && <p role="alert">{state.message}</p>}

      {state.kind === "ready" && state.anime.length === 0 && (
        <p role="status">No Episodes in the Library yet.</p>
      )}

      {state.kind === "ready" && state.anime.length > 0 && (
        <ul className="results" aria-label="Library">
          {state.anime.map((anime) => (
            <li key={anime.animeId ?? anime.title}>
              <Poster url={anime.poster} alt={`${anime.title} poster`} />
              <span className="result-title">{anime.title}</span>
              <span className="muted">
                {anime.episodeCount === 1
                  ? "1 Episode"
                  : `${anime.episodeCount} Episodes`}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
