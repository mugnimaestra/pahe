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
    invoke.mockResolvedValue({ baseUrl: "https://animepahe.pw" });

    await expect(ipc.getSettings()).resolves.toEqual({
      baseUrl: "https://animepahe.pw",
    });
    expect(invoke).toHaveBeenCalledWith("get_settings");
  });

  it("saves settings through the save_settings command", async () => {
    const settings = { baseUrl: "https://mirror.example" };
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
      episodes: [{ number: 1, airDate: "2018-06-11" }],
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
});
