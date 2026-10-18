import { useState } from "react";
import SearchView from "./views/SearchView";
import SettingsView from "./views/SettingsView";
import "./App.css";

const VIEWS = [
  {
    id: "search",
    label: "Search",
    description: "Find an Anime by title and open its Episodes.",
  },
  {
    id: "library",
    label: "Library",
    description: "Downloaded Episodes live here.",
  },
  {
    id: "queue",
    label: "Queue",
    description: "Download jobs and their progress appear here.",
  },
  { id: "settings", label: "Settings", description: "" },
] as const;

type View = (typeof VIEWS)[number]["id"];

export default function App() {
  const [view, setView] = useState<View>("search");
  const current = VIEWS.find((item) => item.id === view) ?? VIEWS[0];

  return (
    <div className="shell">
      <aside className="sidebar">
        <p className="wordmark">pahe</p>
        <nav aria-label="Main">
          {VIEWS.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-current={item.id === view ? "page" : undefined}
              onClick={() => setView(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>
      </aside>

      <main className="content">
        {view === "search" ? (
          <SearchView />
        ) : view === "settings" ? (
          <SettingsView />
        ) : (
          <section className="view view-quiet">
            <h1>{current.label}</h1>
            <p className="muted">{current.description}</p>
          </section>
        )}
      </main>
    </div>
  );
}
