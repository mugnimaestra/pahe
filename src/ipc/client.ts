import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Settings {
  baseUrl: string;
}

export interface SearchResult {
  animeId: string;
  title: string;
  year: number | null;
  poster: string | null;
}

export type SearchFailureKind = "unreachable" | "challengeTimedOut" | "failed";

export interface SearchFailure {
  kind: SearchFailureKind;
  message: string;
}

export type ResolverPhase = "idle" | "challenge";

export interface ResolverStatus {
  state: ResolverPhase;
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
  /** Returns a data URL for the poster, or rejects when it cannot be fetched. */
  fetchPoster: (url: string): Promise<string> =>
    invoke<string>("fetch_poster", { url }),
  onResolverStatus: (
    handler: (status: ResolverStatus) => void,
  ): Promise<UnlistenFn> =>
    listen<ResolverStatus>("resolver://status", (event) =>
      handler(event.payload),
    ),
};
