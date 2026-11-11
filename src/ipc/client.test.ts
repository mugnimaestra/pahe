import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, listen, convertFileSrc } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  convertFileSrc: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { ipc, type Settings } from "./client";

describe("ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    convertFileSrc.mockReset();
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
    const settings: Settings = {
      baseUrl: "https://mirror.example",
      libraryFolder: "/Users/me/Downloads/animepahe",
      defaultQuality: "1080p",
      proxy: "auto",
      resolverUserAgent: "",
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
        stableId: 68,
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

    await expect(ipc.ownedEpisodes("anime-id", 68, "Fairy Tail")).resolves.toEqual(
      owned,
    );
    expect(invoke).toHaveBeenCalledWith("owned_episodes", {
      animeId: "anime-id",
      stableId: 68,
      title: "Fairy Tail",
    });
  });

  it("reads a latest-releases page through the browse_latest command", async () => {
    const page = { releases: [], page: 2, lastPage: 9 };
    invoke.mockResolvedValue(page);

    await expect(ipc.browseLatest(2)).resolves.toEqual(page);
    expect(invoke).toHaveBeenCalledWith("browse_latest", { page: 2 });
  });

  it("reads the A-Z index through the browse_index command", async () => {
    invoke.mockResolvedValue([]);

    await expect(ipc.browseIndex()).resolves.toEqual([]);
    expect(invoke).toHaveBeenCalledWith("browse_index");
  });

  it("checks Library ownership through the library_owns command", async () => {
    const anime = [{ animeId: "a", stableId: 68, title: "Fairy Tail" }];
    invoke.mockResolvedValue([true]);

    await expect(ipc.libraryOwns(anime)).resolves.toEqual([true]);
    expect(invoke).toHaveBeenCalledWith("library_owns", { anime });
  });

  it("lists a Library Anime's items through the library_items command", async () => {
    const items = [
      {
        animeId: null,
        stableId: null,
        titleSlug: "Seitokai_Yakuindomo",
        poster: null,
        episode: 1,
        version: 1,
        quality: "1080p",
        group: "B-Bass",
        path: "/Users/me/Downloads/animepahe/AnimePahe_Seitokai_Yakuindomo_-_01_BD_1080p_B-Bass.mp4",
      },
    ];
    invoke.mockResolvedValue(items);

    await expect(
      ipc.libraryItems(null, null, "Seitokai Yakuindomo"),
    ).resolves.toEqual(items);
    expect(invoke).toHaveBeenCalledWith("library_items", {
      animeId: null,
      stableId: null,
      title: "Seitokai Yakuindomo",
    });
  });

  it("checks a Library item's file through the check_library_file command", async () => {
    invoke.mockResolvedValue(null);

    await ipc.checkLibraryFile("/library/one.mp4");

    expect(invoke).toHaveBeenCalledWith("check_library_file", {
      path: "/library/one.mp4",
    });
  });

  it("opens a Library item externally through the open_externally command", async () => {
    invoke.mockResolvedValue(null);

    await ipc.openExternally("/library/one.mp4");

    expect(invoke).toHaveBeenCalledWith("open_externally", {
      path: "/library/one.mp4",
    });
  });

  it("turns a Library item's path into a webview file URL", () => {
    convertFileSrc.mockReturnValue("asset://localhost/%2Flibrary%2Fone.mp4");

    expect(ipc.libraryFileUrl("/library/one.mp4")).toBe(
      "asset://localhost/%2Flibrary%2Fone.mp4",
    );
    expect(convertFileSrc).toHaveBeenCalledWith("/library/one.mp4");
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

  it("reads and saves playback positions through their commands", async () => {
    invoke.mockResolvedValueOnce(754).mockResolvedValueOnce(undefined);

    await expect(ipc.playbackPosition(["#68:3", "anime:3"])).resolves.toBe(754);
    await ipc.savePlaybackPosition(["#68:3", "anime:3"], 800, 1420);

    expect(invoke).toHaveBeenNthCalledWith(1, "playback_position", {
      keys: ["#68:3", "anime:3"],
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "save_playback_position", {
      keys: ["#68:3", "anime:3"],
      seconds: 800,
      duration: 1420,
    });
  });

  it("streams an Episode through the stream_episode command", async () => {
    const source = {
      url: "http://127.0.0.1:5000/abc",
      quality: "1080p",
      group: "SubsPlease",
    };
    invoke.mockResolvedValue(source);

    await expect(ipc.streamEpisode("anime", "session", "1080p")).resolves.toEqual(
      source,
    );
    expect(invoke).toHaveBeenCalledWith("stream_episode", {
      animeId: "anime",
      session: "session",
      quality: "1080p",
    });
  });
});
