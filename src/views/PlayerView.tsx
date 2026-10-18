import { useEffect, useState } from "react";
import { ipc, type LibraryItem } from "../ipc/client";
import { messageOf } from "../message";

type State = "loading" | "idle" | "playing" | "error";

interface PlayerViewProps {
  item: LibraryItem;
  title: string;
  onBack: () => void;
}

export default function PlayerView({ item, title, onBack }: PlayerViewProps) {
  const [state, setState] = useState<State>("loading");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  useEffect(() => {
    let active = true;
    setState("loading");
    setError(null);
    setNotice(null);
    setSrc(null);

    ipc
      .checkLibraryFile(item.path)
      .then(() => {
        if (active) setSrc(ipc.libraryFileUrl(item.path));
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setError(messageOf(failure));
        setState("error");
      });

    return () => {
      active = false;
    };
  }, [item.path]);

  async function openExternally() {
    if (opening) return;

    setNotice(null);
    setOpening(true);
    try {
      await ipc.openExternally(item.path);
    } catch (failure) {
      setNotice(messageOf(failure));
    } finally {
      setOpening(false);
    }
  }

  function mediaFailed() {
    setError(playbackFailure());
    setState("error");
  }

  return (
    <article className="view view-player" data-state={state}>
      <button type="button" className="back" onClick={onBack}>
        ← Back to Episodes
      </button>

      <header className="player-head">
        <h1>Episode {item.episode}</h1>
        <p className="muted">
          {title} · {item.quality}
          {item.group === "" ? "" : ` · ${item.group}`}
        </p>
        <button type="button" onClick={openExternally} disabled={opening}>
          Open externally
        </button>
      </header>

      {state === "loading" && <p role="status">Opening Episode {item.episode}…</p>}
      {state === "error" && error !== null && <p role="alert">{error}</p>}
      {notice !== null && <p role="alert">{notice}</p>}

      {src !== null && error === null && (
        <video
          className="player-video"
          src={src}
          controls
          autoPlay
          onLoadedData={() =>
            setState((current) => (current === "loading" ? "idle" : current))
          }
          onPlaying={() => setState("playing")}
          onPause={() =>
            setState((current) => (current === "playing" ? "idle" : current))
          }
          onError={mediaFailed}
        />
      )}
    </article>
  );
}

/** The system codecs are the difference Linux users hit first. */
function playbackFailure(): string {
  return /Linux/.test(navigator.userAgent)
    ? "This Episode could not be played in the app. On Linux, the system GStreamer codecs may be missing — use Open externally instead."
    : "This Episode could not be played in the app. The file may be damaged or its format unsupported — use Open externally instead.";
}
