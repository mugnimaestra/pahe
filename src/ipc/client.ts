import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Settings {
  baseUrl: string;
  libraryFolder: string;
}

export interface SearchResult {
  animeId: string;
  title: string;
  year: number | null;
  poster: string | null;
}

export type ResolverFailureKind =
  | "unreachable"
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

export type DownloadState = "queued" | "running" | "completed" | "failed";

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
}

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

/**
 * The only bridge to Rust. UI code imports this module, never `@tauri-apps/api`.
 */
export const ipc = {
  getSettings: (): Promise<Settings> => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings): Promise<Settings> =>
    invoke<Settings>("save_settings", { settings }),
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
  /** The current Download jobs in enqueue order. */
  loadQueue: (): Promise<DownloadJob[]> => invoke<DownloadJob[]>("load_queue"),
  onDownloadJob: (handler: (job: DownloadJob) => void): Promise<UnlistenFn> =>
    listen<DownloadJob>("download://job", (event) => handler(event.payload)),
  onResolverStatus: (
    handler: (status: ResolverStatus) => void,
  ): Promise<UnlistenFn> =>
    listen<ResolverStatus>("resolver://status", (event) =>
      handler(event.payload),
    ),
};
