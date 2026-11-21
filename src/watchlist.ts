import type { WatchStatus, WatchlistEntry } from "./ipc/client";

export const WATCH_STATUSES: readonly WatchStatus[] = [
  "watching",
  "planToWatch",
  "completed",
  "onHold",
  "dropped",
] as const;

export const WATCH_STATUS_LABELS: Record<WatchStatus, string> = {
  watching: "Watching",
  planToWatch: "Plan to Watch",
  completed: "Completed",
  onHold: "On Hold",
  dropped: "Dropped",
};

export function formatClock(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const rest = String(whole % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
}

export function progressPercent(
  position: number,
  duration: number | null,
): number {
  if (duration === null || !Number.isFinite(duration) || duration <= 0) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round((position / duration) * 100)));
}

function slugOf(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

export function findWatchlistEntry(
  entries: WatchlistEntry[],
  target: { stableId: number | null; animeId: string | null; title: string },
): WatchlistEntry | undefined {
  if (target.stableId !== null) {
    const byStable = entries.find(
      (entry) =>
        entry.stableId === target.stableId ||
        entry.key === `#${target.stableId}`,
    );
    if (byStable) return byStable;
  }
  if (target.animeId) {
    const byAnimeId = entries.find((entry) => entry.animeId === target.animeId);
    if (byAnimeId) return byAnimeId;
  }
  const slug = slugOf(target.title);
  if (slug) {
    return entries.find((entry) => slugOf(entry.title) === slug);
  }
  return undefined;
}
