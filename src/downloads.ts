import type { DownloadJob, DownloadProgress } from "./ipc/client";

/** Replaces a job in place, appending one seen for the first time. */
export function upsertJob(
  jobs: DownloadJob[],
  job: DownloadJob,
): DownloadJob[] {
  const index = jobs.findIndex((current) => current.id === job.id);
  if (index === -1) return [...jobs, job];

  const next = jobs.slice();
  next[index] = job;
  return next;
}

/**
 * Merges a queue snapshot with jobs already seen as events. The snapshot keeps
 * the queue order and the event copy wins, because it is newer.
 */
export function mergeJobs(
  current: DownloadJob[],
  loaded: DownloadJob[],
): DownloadJob[] {
  const seen = new Map(current.map((job) => [job.id, job]));
  const merged = loaded.map((job) => seen.get(job.id) ?? job);
  const extra = current.filter(
    (job) => !loaded.some((loadedJob) => loadedJob.id === job.id),
  );

  return [...merged, ...extra];
}

export function progressText(progress: DownloadProgress): string {
  const total =
    progress.total === null ? "" : ` / ${formatBytes(progress.total)}`;
  const speed =
    progress.speed === null ? "" : ` · ${formatBytes(progress.speed)}/s`;
  const eta =
    progress.etaSeconds === null ? "" : ` · ${formatEta(progress.etaSeconds)}`;

  return `${formatBytes(progress.bytes)}${total}${speed}${eta}`;
}

export function formatBytes(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

export function formatEta(seconds: number): string {
  const rounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(rounded / 60);
  const remaining = rounded % 60;

  return minutes > 0 ? `${minutes}m ${remaining}s left` : `${remaining}s left`;
}
