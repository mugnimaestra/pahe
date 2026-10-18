import { useEffect, useState } from "react";
import { ipc } from "../ipc/client";

interface PosterProps {
  /** The remote poster URL, or null when the Anime has none. */
  url: string | null;
  alt: string;
}

/**
 * A poster fetched through Rust, so it survives a Poisoned resolver. Shows the
 * empty tile while loading, when there is no poster, or when the fetch fails.
 */
export default function Poster({ url, alt }: PosterProps) {
  const [source, setSource] = useState<string | null>(null);

  useEffect(() => {
    setSource(null);
    if (!url) return;

    let active = true;
    ipc
      .fetchPoster(url)
      .then((dataUrl) => {
        if (active) setSource(dataUrl);
      })
      .catch(() => {
        // A failed poster is not worth an error banner; keep the placeholder.
      });

    return () => {
      active = false;
    };
  }, [url]);

  if (!source) {
    return <div className="poster-empty" aria-hidden="true" />;
  }

  return (
    <img
      src={source}
      alt={alt}
      loading="lazy"
      onError={() => setSource(null)}
    />
  );
}
