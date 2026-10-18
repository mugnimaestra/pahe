import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, listen } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { ipc } from "./client";

describe("ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
  });

  it("reads settings through the get_settings command", async () => {
    invoke.mockResolvedValue({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/Users/me/Downloads/animepahe",
    });

    await expect(ipc.getSettings()).resolves.toEqual({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/Users/me/Downloads/animepahe",
    });
    expect(invoke).toHaveBeenCalledWith("get_settings");
  });

  it("saves settings through the save_settings command", async () => {
    const settings = {
      baseUrl: "https://mirror.example",
      libraryFolder: "/Users/me/Downloads/animepahe",
    };
    invoke.mockResolvedValue(settings);

    await expect(ipc.saveSettings(settings)).resolves.toEqual(settings);
    expect(invoke).toHaveBeenCalledWith("save_settings", { settings });
  });

  it("searches through the search_anime command", async () => {
    const results = [
      {
        animeId: "session-id",
        title: "Seitokai ni mo Ana wa Aru!",
        year: 2025,
        poster: "https://i.animepahe.si/posters/example.jpg",
      },
    ];
    invoke.mockResolvedValue(results);

    await expect(ipc.searchAnime("seitokai")).resolves.toEqual(results);
    expect(invoke).toHaveBeenCalledWith("search_anime", { query: "seitokai" });
  });

  it("loads an Anime detail through the load_anime command", async () => {
    const detail = {
      synopsis: "A wizard guild in the Kingdom of Fiore.",
      episodes: [{ number: 1, session: "session-one", airDate: "2018-06-11" }],
    };
    invoke.mockResolvedValue(detail);

    await expect(ipc.loadAnime("anime-id")).resolves.toEqual(detail);
    expect(invoke).toHaveBeenCalledWith("load_anime", {
      animeId: "anime-id",
    });
  });

  it("fetches a poster through the fetch_poster command", async () => {
    const dataUrl = "data:image/jpeg;base64,abc";
    invoke.mockResolvedValue(dataUrl);

    await expect(
      ipc.fetchPoster("https://i.animepahe.pw/posters/one.jpg"),
    ).resolves.toBe(dataUrl);
    expect(invoke).toHaveBeenCalledWith("fetch_poster", {
      url: "https://i.animepahe.pw/posters/one.jpg",
    });
  });

  it("loads the Library through the load_library command", async () => {
    const library = [
      {
        animeId: "anime-id",
        title: "Seitokai ni mo Ana wa Aru!",
        poster: "https://i.animepahe.pw/posters/one.jpg",
        episodeCount: 3,
      },
    ];
    invoke.mockResolvedValue(library);

    await expect(ipc.loadLibrary()).resolves.toEqual(library);
    expect(invoke).toHaveBeenCalledWith("load_library");
  });

  it("enriches the Library through the enrich_library command", async () => {
    const library = [
      {
        animeId: null,
        title: "Seitokai ni mo Ana wa Aru",
        poster: null,
        episodeCount: 1,
      },
    ];
    invoke.mockResolvedValue(library);

    await expect(ipc.enrichLibrary()).resolves.toEqual(library);
    expect(invoke).toHaveBeenCalledWith("enrich_library");
  });

  it("subscribes to Resolver status events", async () => {
    const unlisten = vi.fn();
    listen.mockResolvedValue(unlisten);
    const handler = vi.fn();

    await expect(ipc.onResolverStatus(handler)).resolves.toBe(unlisten);
    expect(listen).toHaveBeenCalledWith("resolver://status", expect.any(Function));

    const callback = listen.mock.calls[0][1] as (event: {
      payload: { state: string };
    }) => void;
    callback({ payload: { state: "challenge" } });

    expect(handler).toHaveBeenCalledWith({ state: "challenge" });
  });

  it("queues Download jobs through the enqueue_downloads command", async () => {
    const requests = [
      {
        animeId: "anime-id",
        title: "Fairy Tail",
        episode: 4,
        session: "session-four",
        quality: "1080p",
        redownload: false,
      },
    ];
    const outcome = {
      jobs: [
        {
          id: "job-1",
          animeId: "anime-id",
          title: "Fairy Tail",
          episode: 4,
          quality: "1080p",
          state: "queued",
          progress: null,
          error: null,
        },
      ],
      skipped: [],
    };
    invoke.mockResolvedValue(outcome);

    await expect(ipc.enqueueDownloads(requests)).resolves.toEqual(outcome);
    expect(invoke).toHaveBeenCalledWith("enqueue_downloads", { requests });
  });

  it("reads the queue through the load_queue command", async () => {
    const queue = { jobs: [], active: false, challenge: false };
    invoke.mockResolvedValue(queue);

    await expect(ipc.loadQueue()).resolves.toEqual(queue);
    expect(invoke).toHaveBeenCalledWith("load_queue");
  });

  it("starts the restored queue through the start_queue command", async () => {
    const queue = { jobs: [], active: true, challenge: false };
    invoke.mockResolvedValue(queue);

    await expect(ipc.startQueue()).resolves.toEqual(queue);
    expect(invoke).toHaveBeenCalledWith("start_queue");
  });

  it("resumes a job through the resume_download command", async () => {
    const queue = { jobs: [], active: true, challenge: false };
    invoke.mockResolvedValue(queue);

    await expect(ipc.resumeDownload("job-1")).resolves.toEqual(queue);
    expect(invoke).toHaveBeenCalledWith("resume_download", { jobId: "job-1" });
  });

  it("reads owned Episodes through the owned_episodes command", async () => {
    const owned = [{ episode: 1, quality: "1080p" }];
    invoke.mockResolvedValue(owned);

    await expect(ipc.ownedEpisodes("anime-id", "Fairy Tail")).resolves.toEqual(
      owned,
    );
    expect(invoke).toHaveBeenCalledWith("owned_episodes", {
      animeId: "anime-id",
      title: "Fairy Tail",
    });
  });

  it("subscribes to Download job events", async () => {
    const unlisten = vi.fn();
    listen.mockResolvedValue(unlisten);
    const handler = vi.fn();

    await expect(ipc.onDownloadJob(handler)).resolves.toBe(unlisten);
    expect(listen).toHaveBeenCalledWith("download://job", expect.any(Function));

    const callback = listen.mock.calls[0][1] as (event: {
      payload: { id: string };
    }) => void;
    callback({ payload: { id: "job-1" } });

    expect(handler).toHaveBeenCalledWith({ id: "job-1" });
  });
});
