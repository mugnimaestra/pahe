import { useEffect, useRef, useState, type SyntheticEvent } from "react";
import { ipc, type LibraryItem } from "../ipc/client";
import { messageOf } from "../message";

type State = "loading" | "idle" | "playing" | "error";

/** How often a playing Episode writes its position. */
const SAVE_INTERVAL_MS = 5000;

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
  const [resumeAt, setResumeAt] = useState<number | null>(null);
  const [resumed, setResumed] = useState<number | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const lastSave = useRef(0);
  const watched = useRef<{ seconds: number; duration: number | null } | null>(
    null,
  );
  const positionKey = `${item.animeId ?? item.titleSlug}:${item.episode}`;

  useEffect(() => {
    let active = true;
    setState("loading");
    setError(null);
    setNotice(null);
    setSrc(null);
    setResumeAt(null);
    setResumed(null);
    watched.current = null;

    Promise.all([
      ipc.checkLibraryFile(item.path),
      // A lost position only means the Episode starts at the beginning.
      ipc.playbackPosition(positionKey).catch(() => null),
    ])
      .then(([, position]) => {
        if (!active) return;
        setResumeAt(position);
        setSrc(ipc.libraryFileUrl(item.path));
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setError(messageOf(failure));
        setState("error");
      });

    return () => {
      active = false;
      // Leaving the player keeps the place, even between the timed saves.
      const last = watched.current;
      if (last !== null) {
        ipc
          .savePlaybackPosition(positionKey, last.seconds, last.duration)
          .catch(() => {});
      }
    };
  }, [item.path, positionKey]);

  function remember(video: HTMLVideoElement, now: boolean) {
    const duration = Number.isFinite(video.duration) ? video.duration : null;
    watched.current = { seconds: video.currentTime, duration };
    if (!now && Date.now() - lastSave.current < SAVE_INTERVAL_MS) return;

    lastSave.current = Date.now();
    ipc
      .savePlaybackPosition(positionKey, video.currentTime, duration)
      .catch(() => {
        // The position is a convenience; playback goes on without it.
      });
  }

  function loadedMetadata(event: SyntheticEvent<HTMLVideoElement>) {
    const video = event.currentTarget;
    if (resumeAt === null) return;
    if (Number.isFinite(video.duration) && resumeAt >= video.duration) return;

    video.currentTime = resumeAt;
    setResumed(resumeAt);
  }

  function startOver() {
    if (videoRef.current !== null) videoRef.current.currentTime = 0;
    setResumed(null);
  }

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
      {resumed !== null && error === null && (
        <p className="muted player-resumed">
          Resumed at {clock(resumed)}.{" "}
          <button type="button" onClick={startOver}>
            Start over
          </button>
        </p>
      )}

      {src !== null && error === null && (
        <video
          ref={videoRef}
          className="player-video"
          src={src}
          controls
          autoPlay
          onLoadedMetadata={loadedMetadata}
          onLoadedData={() =>
            setState((current) => (current === "loading" ? "idle" : current))
          }
          onPlaying={() => setState("playing")}
          onTimeUpdate={(event) => remember(event.currentTarget, false)}
          onPause={(event) => {
            remember(event.currentTarget, true);
            setState((current) => (current === "playing" ? "idle" : current));
          }}
          onEnded={(event) => remember(event.currentTarget, true)}
          onError={mediaFailed}
        />
      )}
    </article>
  );
}

/** `m:ss`, or `h:mm:ss` past an hour. */
function clock(seconds: number): string {
  const whole = Math.floor(seconds);
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const rest = String(whole % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
}

/** The system codecs are the difference Linux users hit first. */
function playbackFailure(): string {
  return /Linux/.test(navigator.userAgent)
    ? "This Episode could not be played in the app. On Linux, the system GStreamer codecs may be missing — use Open externally instead."
    : "This Episode could not be played in the app. The file may be damaged or its format unsupported — use Open externally instead.";
}
