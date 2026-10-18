import { useEffect, useState } from "react";
import Poster from "../components/Poster";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import { ipc, type AnimeDetail, type SearchResult } from "../ipc/client";
import { failureOf } from "../failure";

type State =
  | { kind: "loading" }
  | { kind: "ready"; detail: AnimeDetail }
  | { kind: "failed"; message: string };

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
            {state.detail.episodes.map((episode) => (
              <li key={episode.number}>
                <span>Episode {episode.number}</span>
                <span className="muted">
                  {episode.airDate ?? "Air date unknown"}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </article>
  );
}
