import { useState } from "react";
import LibraryView from "./views/LibraryView";
import QueueView from "./views/QueueView";
import SearchView from "./views/SearchView";
import SettingsView from "./views/SettingsView";
import "./App.css";

const VIEWS = [
  { id: "search", label: "Search" },
  { id: "library", label: "Library" },
  { id: "queue", label: "Queue" },
  { id: "settings", label: "Settings" },
] as const;

type View = (typeof VIEWS)[number]["id"];

export default function App() {
  const [view, setView] = useState<View>("search");

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
        ) : view === "library" ? (
          <LibraryView />
        ) : view === "queue" ? (
          <QueueView />
        ) : (
          <SettingsView />
        )}
      </main>
    </div>
  );
}
