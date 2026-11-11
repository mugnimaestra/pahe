import { useEffect, useMemo, useState } from "react";
import Poster from "../components/Poster";
import { failureOf } from "../failure";
import { useResolverChallenge } from "../hooks/useResolverChallenge";
import { ipc, type LatestPage, type SearchResult } from "../ipc/client";
import AnimeDetailView from "./AnimeDetailView";

type Tab = "latest" | "index";

type Load<T> =
  | { kind: "loading" }
  | { kind: "ready"; value: T }
  | { kind: "failed"; message: string };

const LETTERS = ["#", ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"];

/** The A-Z group of a title: its first letter, or "#" for anything else. */
export function letterOf(title: string): string {
  const first = title.trim().charAt(0).toUpperCase();
  return first >= "A" && first <= "Z" ? first : "#";
}

function owns(anime: SearchResult[]): Promise<boolean[]> {
  // A failed Library lookup only hides the badges.
  return ipc
    .libraryOwns(
      anime.map(({ animeId, stableId, title }) => ({ animeId, stableId, title })),
    )
    .catch(() => anime.map(() => false));
}

/**
 * The index has no poster or numeric id, so look the title up once and take
 * the search result for the same Anime when there is one.
 */
async function withSearchDetails(entry: SearchResult): Promise<SearchResult> {
  try {
    const results = await ipc.searchAnime(entry.title);
    return (
      results.find((result) => result.animeId === entry.animeId) ??
      results.find((result) => result.title === entry.title) ??
      entry
    );
  } catch {
    return entry;
  }
}

export default function BrowseView() {
  const [tab, setTab] = useState<Tab>("latest");
  const [page, setPage] = useState(1);
  const [latest, setLatest] = useState<Load<LatestPage>>({ kind: "loading" });
  const [latestOwned, setLatestOwned] = useState<boolean[]>([]);
  const [index, setIndex] = useState<Load<SearchResult[]> | null>(null);
  const [letter, setLetter] = useState("A");
  const [filter, setFilter] = useState("");
  const [indexOwned, setIndexOwned] = useState<Record<string, boolean>>({});
  const [selected, setSelected] = useState<SearchResult | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [challenge] = useResolverChallenge();

  useEffect(() => {
    if (tab !== "latest") return;
    let current = true;
    setLatest({ kind: "loading" });
    ipc
      .browseLatest(page)
      .then(async (value) => {
        const owned = await owns(value.releases.map((release) => release.anime));
        if (!current) return;
        setLatest({ kind: "ready", value });
        setLatestOwned(owned);
      })
      .catch((error: unknown) => {
        if (current) setLatest({ kind: "failed", message: failureOf(error).message });
      });
    return () => {
      current = false;
    };
  }, [tab, page]);

  useEffect(() => {
    if (tab !== "index" || index !== null) return;
    setIndex({ kind: "loading" });
    ipc
      .browseIndex()
      .then((value) => setIndex({ kind: "ready", value }))
      .catch((error: unknown) =>
        setIndex({ kind: "failed", message: failureOf(error).message }),
      );
  }, [tab, index]);

  const shown = useMemo(() => {
    if (index?.kind !== "ready") return [];
    const words = filter.trim().toLowerCase();
    return index.value
      .filter((entry) =>
        words
          ? entry.title.toLowerCase().includes(words)
          : letterOf(entry.title) === letter,
      )
      .sort((left, right) =>
        left.title.localeCompare(right.title, undefined, { sensitivity: "base" }),
      );
  }, [index, letter, filter]);

  useEffect(() => {
    const missing = shown.filter((entry) => !(entry.animeId in indexOwned));
    if (missing.length === 0) return;
    let current = true;
    owns(missing).then((owned) => {
      if (!current) return;
      setIndexOwned((known) => {
        const next = { ...known };
        missing.forEach((entry, i) => {
          next[entry.animeId] = owned[i];
        });
        return next;
      });
    });
    return () => {
      current = false;
    };
  }, [shown, indexOwned]);

  async function openIndexEntry(entry: SearchResult) {
    setOpening(entry.animeId);
    const result = await withSearchDetails(entry);
    setOpening(null);
    setSelected(result);
  }

  if (selected !== null) {
    return <AnimeDetailView result={selected} onBack={() => setSelected(null)} />;
  }

  return (
    <section className="view view-browse">
      <header className="view-intro">
        <h1>Browse</h1>
        <p className="muted">
          See the latest releases or every Anime from A to Z.
        </p>
      </header>

      <div className="tabs" role="tablist" aria-label="Browse by">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "latest"}
          onClick={() => setTab("latest")}
        >
          Latest releases
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "index"}
          onClick={() => setTab("index")}
        >
          A–Z
        </button>
      </div>

      {challenge && (
        <p role="status" className="challenge">
          Cloudflare needs a human check. The Resolver window is open — finish
          the check and Browse will continue.
        </p>
      )}

      {tab === "latest" && (
        <>
          {latest.kind === "loading" && (
            <p role="status">Loading the latest releases…</p>
          )}
          {latest.kind === "failed" && <p role="alert">{latest.message}</p>}
          {latest.kind === "ready" && (
            <>
              <ul className="results releases" aria-label="Latest releases">
                {latest.value.releases.map((release, i) => (
                  <li key={`${release.anime.animeId}-${release.episode ?? i}`}>
                    <button
                      type="button"
                      className="result"
                      onClick={() => setSelected(release.anime)}
                    >
                      <Poster
                        url={release.anime.poster}
                        alt={`${release.anime.title} poster`}
                      />
                      <span className="result-title">{release.anime.title}</span>
                      {release.episode !== null && (
                        <span className="muted">Episode {release.episode}</span>
                      )}
                      {latestOwned[i] && (
                        <span className="download-done">In Library</span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
              <div className="pager" role="group" aria-label="Pages">
                <button
                  type="button"
                  disabled={latest.value.page <= 1}
                  onClick={() => setPage(latest.value.page - 1)}
                >
                  Newer
                </button>
                <span className="muted">
                  Page {latest.value.page} of {latest.value.lastPage}
                </span>
                <button
                  type="button"
                  disabled={latest.value.page >= latest.value.lastPage}
                  onClick={() => setPage(latest.value.page + 1)}
                >
                  Older
                </button>
              </div>
            </>
          )}
        </>
      )}

      {tab === "index" && (
        <>
          {index?.kind === "loading" && (
            <p role="status">Loading the Anime index…</p>
          )}
          {index?.kind === "failed" && <p role="alert">{index.message}</p>}
          {index?.kind === "ready" && (
            <>
              <div className="search-row">
                <label htmlFor="browse-filter">Filter</label>
                <input
                  id="browse-filter"
                  type="search"
                  value={filter}
                  onChange={(event) => setFilter(event.currentTarget.value)}
                />
              </div>
              <div className="letters" role="group" aria-label="Letters">
                {LETTERS.map((item) => (
                  <button
                    key={item}
                    type="button"
                    aria-pressed={!filter.trim() && item === letter}
                    onClick={() => {
                      setFilter("");
                      setLetter(item);
                    }}
                  >
                    {item}
                  </button>
                ))}
              </div>
              {shown.length === 0 ? (
                <p role="status">No Anime here.</p>
              ) : (
                <ul className="index" aria-label="Anime index">
                  {shown.map((entry) => (
                    <li key={entry.animeId}>
                      <button
                        type="button"
                        className="index-entry"
                        disabled={opening !== null}
                        onClick={() => void openIndexEntry(entry)}
                      >
                        <span>{entry.title}</span>
                        {opening === entry.animeId && (
                          <span className="muted"> Opening…</span>
                        )}
                        {indexOwned[entry.animeId] && (
                          <span className="download-done">In Library</span>
                        )}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
