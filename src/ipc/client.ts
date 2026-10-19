import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Settings {
  baseUrl: string;
  libraryFolder: string;
  /** The Quality new Download jobs start from. */
  defaultQuality: string;
  proxy: ProxyPolicy;
}

/** When the Resolver uses pahe's local DoH proxy. */
export type ProxyPolicy = "auto" | "always" | "never";

/**
 * The verdict of a TLS probe against a Base URL: `clean` is reachable, and
 * `poisoned` means the address does not belong to the host.
 */
export type ProbeOutcome = "clean" | "poisoned" | "unreachable";

export interface SearchResult {
  animeId: string;
  title: string;
  year: number | null;
  poster: string | null;
}

export type ResolverFailureKind =
  | "unreachable"
  | "deadSource"
  | "challengeTimedOut"
  | "failed";

export interface ResolverFailure {
  kind: ResolverFailureKind;
  message: string;
}

export interface Episode {
  number: number;
  session: string;
  airDate: string | null;
}

export interface AnimeDetail {
  synopsis: string | null;
  episodes: Episode[];
}

export interface LibraryAnime {
  animeId: string | null;
  title: string;
  poster: string | null;
  episodeCount: number;
}

export type ResolverPhase = "idle" | "challenge";

export interface ResolverStatus {
  state: ResolverPhase;
}

export type DownloadState =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "interrupted"
  | "paused";

export interface DownloadProgress {
  bytes: number;
  total: number | null;
  /** Bytes per second. */
  speed: number | null;
  etaSeconds: number | null;
}

export interface DownloadJob {
  id: string;
  animeId: string;
  title: string;
  episode: number;
  quality: string;
  state: DownloadState;
  progress: DownloadProgress | null;
  error: string | null;
  /** Why the job failed, when it did. */
  failure: FailureReason | null;
}

/** Why a Download job failed, so the UI can name the state. */
export type FailureReason =
  | "deadSource"
  | "expiredCdnUrl"
  | "insufficientDisk"
  | "unreachable"
  | "challengeTimedOut"
  | "failed";

export interface DownloadRequest {
  animeId: string;
  title: string;
  episode: number;
  session: string;
  quality: string;
  /** Fetches again even though the Library owns this Episode and Quality. */
  redownload: boolean;
}

export type SkipReason = "inLibrary" | "inQueue";

export interface SkippedDownload {
  animeId: string;
  episode: number;
  quality: string;
  reason: SkipReason;
}

export interface Enqueued {
  jobs: DownloadJob[];
  skipped: SkippedDownload[];
}

export interface OwnedEpisode {
  episode: number;
  quality: string;
}

/** A downloaded Episode file the Library knows about. */
export interface LibraryItem {
  animeId: string | null;
  titleSlug: string;
  poster: string | null;
  episode: number;
  version: number;
  quality: string;
  group: string;
  path: string;
}

/** Every Download job, plus whether the worker runs and a Challenge holds it. */
export interface QueueSnapshot {
  jobs: DownloadJob[];
  /** False after a restart until the user starts or resumes a job. */
  active: boolean;
  challenge: boolean;
}

/**
 * The only bridge to Rust. UI code imports this module, never `@tauri-apps/api`.
 */
export const ipc = {
  getSettings: (): Promise<Settings> => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings): Promise<Settings> =>
    invoke<Settings>("save_settings", { settings }),
  /** Probes a Base URL: reachable, Poisoned resolver, or no answer. */
  testBaseUrl: (baseUrl: string): Promise<ProbeOutcome> =>
    invoke<ProbeOutcome>("test_base_url", { baseUrl }),
  searchAnime: (query: string): Promise<SearchResult[]> =>
    invoke<SearchResult[]>("search_anime", { query }),
  loadAnime: (animeId: string): Promise<AnimeDetail> =>
    invoke<AnimeDetail>("load_anime", { animeId }),
  /** Returns a data URL for the poster, or rejects when it cannot be fetched. */
  fetchPoster: (url: string): Promise<string> =>
    invoke<string>("fetch_poster", { url }),
  /** Scans the Library folder and returns its Animes. Rejects when unreadable. */
  loadLibrary: (): Promise<LibraryAnime[]> =>
    invoke<LibraryAnime[]>("load_library"),
  /** Caches Anime metadata for Library items; rejects only when the index cannot be saved. */
  enrichLibrary: (): Promise<LibraryAnime[]> =>
    invoke<LibraryAnime[]>("enrich_library"),
  /** Every Episode and Quality the Library already holds for an Anime. */
  ownedEpisodes: (animeId: string, title: string): Promise<OwnedEpisode[]> =>
    invoke<OwnedEpisode[]>("owned_episodes", { animeId, title }),
  /** Queues Download jobs, skipping Episodes the Library or an active job owns. */
  enqueueDownloads: (requests: DownloadRequest[]): Promise<Enqueued> =>
    invoke<Enqueued>("enqueue_downloads", { requests }),
  /** Every Library item of an Anime, in Episode then Quality order. */
  libraryItems: (
    animeId: string | null,
    title: string,
  ): Promise<LibraryItem[]> =>
    invoke<LibraryItem[]>("library_items", { animeId, title }),
  /** Rejects with a plain-language message when the item's file is gone. */
  checkLibraryFile: (path: string): Promise<void> =>
    invoke<void>("check_library_file", { path }),
  /** Hands a Library item's file to the OS default player. */
  openExternally: (path: string): Promise<void> =>
    invoke<void>("open_externally", { path }),
  /** A `src` the webview may load for a Library item's file. */
  libraryFileUrl: (path: string): string => convertFileSrc(path),
  /** The current Download jobs in enqueue order. */
  loadQueue: (): Promise<QueueSnapshot> => invoke<QueueSnapshot>("load_queue"),
  /** Starts the restored queue; restored jobs never run on launch by themselves. */
  startQueue: (): Promise<QueueSnapshot> => invoke<QueueSnapshot>("start_queue"),
  /** Resumes an interrupted or paused job, or retries a failed one. */
  resumeDownload: (jobId: string): Promise<QueueSnapshot> =>
    invoke<QueueSnapshot>("resume_download", { jobId }),
  onDownloadJob: (handler: (job: DownloadJob) => void): Promise<UnlistenFn> =>
    listen<DownloadJob>("download://job", (event) => handler(event.payload)),
  onResolverStatus: (
    handler: (status: ResolverStatus) => void,
  ): Promise<UnlistenFn> =>
    listen<ResolverStatus>("resolver://status", (event) =>
      handler(event.payload),
    ),
};
