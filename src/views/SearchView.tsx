import { useState, type FormEvent } from "react";
import Poster from "../components/Poster";
import { failureOf } from "../failure";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import { ipc, type SearchResult } from "../ipc/client";
import AnimeDetailView from "./AnimeDetailView";

type State =
  | { kind: "idle" }
  | { kind: "searching" }
  | { kind: "empty"; query: string }
  | { kind: "results"; results: SearchResult[] }
  | { kind: "unreachable"; message: string }
  | { kind: "error"; message: string };

export default function SearchView() {
  const [query, setQuery] = useState("");
  const [state, setState] = useState<State>({ kind: "idle" });
  const [selected, setSelected] = useState<SearchResult | null>(null);
  const [challenge, setChallenge] = useResolverChallenge();

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const title = query.trim();
    if (!title) {
      setState({ kind: "idle" });
      return;
    }

    setChallenge(false);
    setState({ kind: "searching" });

    try {
      const results = await ipc.searchAnime(title);
      setState(
        results.length === 0
          ? { kind: "empty", query: title }
          : { kind: "results", results },
      );
    } catch (error) {
      const failure = failureOf(error);
      setState(
        failure.kind === "unreachable"
          ? { kind: "unreachable", message: failure.message }
          : { kind: "error", message: failure.message },
      );
    } finally {
      setChallenge(false);
    }
  }

  if (selected !== null) {
    return (
      <AnimeDetailView result={selected} onBack={() => setSelected(null)} />
    );
  }

  return (
    <section className="view view-search">
      <header className="view-intro">
        <h1>Search</h1>
        <p className="muted">Find an Anime by title and open its Episodes.</p>
      </header>

      <form className="search-form" onSubmit={handleSubmit}>
        <label htmlFor="search-title">Title</label>
        <div className="search-row">
          <input
            id="search-title"
            type="search"
            required
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
          <button
            className="button"
            type="submit"
            disabled={state.kind === "searching"}
          >
            {state.kind === "searching" ? "Searching…" : "Search"}
          </button>
        </div>
      </form>

      {state.kind === "idle" && (
        <p className="muted">Search animepahe by title.</p>
      )}

      {state.kind === "searching" && (
        <p role="status">Searching animepahe…</p>
      )}

      {challenge && (
        <p role="status" className="challenge">
          Cloudflare needs a human check. The Resolver window is open — finish
          the check and the search will continue.
        </p>
      )}

      {state.kind === "empty" && (
        <p role="status">
          No Anime matched “{state.query}”. Try another title.
        </p>
      )}

      {state.kind === "unreachable" && <p role="alert">{state.message}</p>}
      {state.kind === "error" && <p role="alert">{state.message}</p>}

      {state.kind === "results" && (
        <ul className="results" aria-label="Search results">
          {state.results.map((result) => (
            <li key={result.animeId}>
              <button
                type="button"
                className="result"
                onClick={() => setSelected(result)}
              >
                <Poster
                  url={result.poster}
                  alt={`${result.title} poster`}
                />
                <span className="result-title">{result.title}</span>
                {result.year !== null && (
                  <span className="muted">{result.year}</span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
