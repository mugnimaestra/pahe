// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ipc, type WatchlistEntry } from "../ipc/client";
import WatchlistView from "./WatchlistView";

vi.mock("../ipc/client", () => ({
  ipc: {
    listWatchlist: vi.fn(),
    getSettings: vi.fn(),
    onWatchlistUpdated: vi.fn(),
    upsertWatchlistEntry: vi.fn(),
    removeWatchlistEntry: vi.fn(),
    libraryItems: vi.fn(),
    fetchPoster: vi.fn(),
    streamEpisode: vi.fn(),
    playbackPosition: vi.fn(),
    savePlaybackPosition: vi.fn(),
    recordWatchProgress: vi.fn(),
    checkLibraryFile: vi.fn(),
    libraryFileUrl: vi.fn(),
  },
}));

const ENTRIES: WatchlistEntry[] = [
  {
    key: "#100",
    stableId: 100,
    animeId: "dandadan-sess",
    title: "Dandadan",
    poster: null,
    status: "watching",
    progress: 3,
    totalEpisodes: 12,
    score: 9,
    notes: "",
    lastWatched: {
      episode: 4,
      session: "ep4-sess",
      quality: "1080p",
      positionSeconds: 720,
      durationSeconds: 1440,
      sourceKind: "stream",
      updatedAt: 2000,
    },
    updatedAt: 2000,
  },
  {
    key: "#42",
    stableId: 42,
    animeId: "frieren-sess",
    title: "Frieren",
    poster: null,
    status: "completed",
    progress: 28,
    totalEpisodes: 28,
    score: 10,
    notes: "",
    lastWatched: null,
    updatedAt: 1000,
  },
];

describe("watchlist view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(ipc.listWatchlist).mockResolvedValue(ENTRIES);
    vi.mocked(ipc.getSettings).mockResolvedValue({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/tmp/animepahe",
      defaultQuality: "1080p",
      proxy: "auto",
      resolverUserAgent: "",
    });
    vi.mocked(ipc.onWatchlistUpdated).mockResolvedValue(() => {});
    vi.mocked(ipc.libraryItems).mockResolvedValue([]);
    vi.mocked(ipc.streamEpisode).mockResolvedValue({
      url: "http://127.0.0.1:9999/stream",
      quality: "1080p",
      group: "SubsPlease",
    });
    vi.mocked(ipc.playbackPosition).mockResolvedValue(720);
    vi.mocked(ipc.savePlaybackPosition).mockResolvedValue(undefined);
  });

  afterEach(cleanup);

  it("shows Continue Watching rail and status tabs", async () => {
    render(<WatchlistView />);

    expect(await screen.findByText("Continue Watching")).toBeTruthy();
    expect(screen.getByText(/Episode 4 · 12:00 \/ 24:00/)).toBeTruthy();
    expect(screen.getByRole("tab", { name: "All (2)" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Watching (1)" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Completed (1)" })).toBeTruthy();
  });

  it("filters by status tab and increments episode progress", async () => {
    vi.mocked(ipc.upsertWatchlistEntry).mockResolvedValue({
      ...ENTRIES[0],
      progress: 4,
    });

    render(<WatchlistView />);
    await screen.findByText("Continue Watching");

    fireEvent.click(screen.getByRole("tab", { name: "Watching (1)" }));
    expect(screen.queryByText("Frieren")).toBeNull();
    expect(screen.getAllByText("Dandadan").length).toBeGreaterThan(0);

    fireEvent.click(
      screen.getByRole("button", { name: "Increment Episode for Dandadan" }),
    );
    await waitFor(() =>
      expect(ipc.upsertWatchlistEntry).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Dandadan", progress: 4 }),
      ),
    );
  });

  it("resumes a Continue Watching stream directly in PlayerView", async () => {
    render(<WatchlistView />);
    await screen.findByText("Continue Watching");

    fireEvent.click(screen.getByRole("button", { name: "Resume Ep 4" }));
    expect(await screen.findByRole("heading", { name: "Episode 4" })).toBeTruthy();
  });

  it("removes an entry from the Watchlist", async () => {
    vi.mocked(ipc.removeWatchlistEntry).mockResolvedValue(true);
    render(<WatchlistView />);
    await screen.findByText("Continue Watching");

    fireEvent.click(screen.getByRole("button", { name: "Remove Frieren" }));
    await waitFor(() =>
      expect(ipc.removeWatchlistEntry).toHaveBeenCalledWith("#42"),
    );
  });
});
