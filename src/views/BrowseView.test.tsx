// @vitest-environment jsdom
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
    searchAnime: vi.fn(),
    loadAnime: vi.fn(),
    getSettings: vi.fn(),
    onResolverStatus: vi.fn(),
    onDownloadJob: vi.fn(),
    ownedEpisodes: vi.fn(),
    enqueueDownloads: vi.fn(),
    browseLatest: vi.fn(),
    browseIndex: vi.fn(),
    libraryOwns: vi.fn(),
  },
}));

import BrowseView, { letterOf } from "./BrowseView";
import { ipc, type SearchResult } from "../ipc/client";

const mocked = vi.mocked(ipc);

const FRIEREN: SearchResult = {
  animeId: "11111111-1111-1111-1111-111111111111",
  stableId: 5646,
  title: "Frieren",
  year: 2023,
  poster: "https://i.animepahe.si/snapshots/frieren.jpg",
};
const ONE_PIECE: SearchResult = {
  animeId: "22222222-2222-2222-2222-222222222222",
  stableId: 4,
  title: "One Piece",
  year: 1999,
  poster: "https://i.animepahe.si/snapshots/op.jpg",
};

function entry(animeId: string, title: string): SearchResult {
  return { animeId, stableId: null, title, year: null, poster: null };
}

describe("browse view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.onResolverStatus.mockResolvedValue(() => {});
    mocked.onDownloadJob.mockResolvedValue(() => {});
    mocked.fetchPoster.mockResolvedValue("data:image/jpeg;base64,abc");
    mocked.getSettings.mockResolvedValue({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/Users/me/Downloads/animepahe",
      defaultQuality: "1080p",
      proxy: "auto",
      resolverUserAgent: "",
    });
    mocked.ownedEpisodes.mockResolvedValue([]);
    mocked.loadAnime.mockReturnValue(new Promise(() => {}));
    mocked.browseLatest.mockResolvedValue({
      releases: [
        { anime: FRIEREN, episode: 12 },
        { anime: ONE_PIECE, episode: 1100 },
      ],
      page: 1,
      lastPage: 3,
    });
    mocked.libraryOwns.mockImplementation(async (anime) =>
      anime.map((item) => item.stableId === 5646),
    );
  });

  it("groups titles by first letter", () => {
    expect(letterOf("frieren")).toBe("F");
    expect(letterOf(" Zom 100")).toBe("Z");
    expect(letterOf("86")).toBe("#");
    expect(letterOf(".hack//Sign")).toBe("#");
  });

  it("lists the latest releases with episodes and In Library badges", async () => {
    render(<BrowseView />);

    const list = await screen.findByRole("list", { name: "Latest releases" });
    expect(list.textContent).toContain("Frieren");
    expect(list.textContent).toContain("Episode 12");
    expect(list.textContent).toContain("Episode 1100");
    expect(screen.getAllByText("In Library")).toHaveLength(1);
    expect(mocked.browseLatest).toHaveBeenCalledWith(1);
    expect(mocked.libraryOwns).toHaveBeenCalledWith([
      { animeId: FRIEREN.animeId, stableId: 5646, title: "Frieren" },
      { animeId: ONE_PIECE.animeId, stableId: 4, title: "One Piece" },
    ]);
    expect(screen.getByText("Page 1 of 3")).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Newer" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("pages to older releases", async () => {
    render(<BrowseView />);
    await screen.findByText("Page 1 of 3");

    mocked.browseLatest.mockResolvedValue({
      releases: [{ anime: ONE_PIECE, episode: 1099 }],
      page: 2,
      lastPage: 3,
    });
    fireEvent.click(screen.getByRole("button", { name: "Older" }));

    expect(await screen.findByText("Page 2 of 3")).toBeTruthy();
    expect(mocked.browseLatest).toHaveBeenLastCalledWith(2);
    expect(screen.getByText("Episode 1099")).toBeTruthy();
  });

  it("still lists releases when the Library check fails", async () => {
    mocked.libraryOwns.mockRejectedValue(new Error("disk"));
    render(<BrowseView />);

    expect(await screen.findByText("Episode 12")).toBeTruthy();
    expect(screen.queryByText("In Library")).toBeNull();
  });

  it("shows a Resolver failure", async () => {
    mocked.browseLatest.mockRejectedValue({
      kind: "unreachable",
      message: "animepahe is unreachable.",
    });
    render(<BrowseView />);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "animepahe is unreachable.",
    );
  });

  it("opens a latest release in the detail view", async () => {
    render(<BrowseView />);

    fireEvent.click(await screen.findByRole("button", { name: /One Piece/ }));

    expect(await screen.findByRole("button", { name: /Back/ })).toBeTruthy();
    expect(mocked.loadAnime).toHaveBeenCalledWith(ONE_PIECE.animeId);
  });

  it("filters the A-Z index by letter and by text", async () => {
    mocked.browseIndex.mockResolvedValue([
      entry("a1", "Akira"),
      entry("a2", "Ao Haru Ride"),
      entry("b1", "Bleach"),
      entry("n1", "86 Eighty-Six"),
    ]);
    mocked.libraryOwns.mockImplementation(async (anime) =>
      anime.map((item) => item.animeId === "a2"),
    );
    render(<BrowseView />);

    fireEvent.click(screen.getByRole("tab", { name: "A\u2013Z" }));
    const list = await screen.findByRole("list", { name: "Anime index" });
    expect(list.textContent).toContain("Akira");
    expect(list.textContent).not.toContain("Bleach");
    expect(await screen.findByText("In Library")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "#" }));
    expect(screen.getByRole("list", { name: "Anime index" }).textContent).toBe(
      "86 Eighty-Six",
    );

    fireEvent.change(screen.getByLabelText("Filter"), {
      target: { value: "bleach" },
    });
    expect(screen.getByRole("list", { name: "Anime index" }).textContent).toBe(
      "Bleach",
    );

    fireEvent.change(screen.getByLabelText("Filter"), {
      target: { value: "zzz" },
    });
    expect(screen.getByText("No Anime here.")).toBeTruthy();
    expect(mocked.browseIndex).toHaveBeenCalledTimes(1);
  });

  it("opens an index entry with the matching search result", async () => {
    const bare = entry(FRIEREN.animeId, "Frieren");
    mocked.browseIndex.mockResolvedValue([bare]);
    mocked.searchAnime.mockResolvedValue([ONE_PIECE, FRIEREN]);
    render(<BrowseView />);

    fireEvent.click(screen.getByRole("tab", { name: "A\u2013Z" }));
    fireEvent.click(await screen.findByRole("button", { name: "F" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^Frieren/ }));
    });

    expect(mocked.searchAnime).toHaveBeenCalledWith("Frieren");
    expect(await screen.findByRole("button", { name: /Back/ })).toBeTruthy();
    expect(mocked.loadAnime).toHaveBeenCalledWith(FRIEREN.animeId);
  });

  it("opens the bare index entry when search fails", async () => {
    const bare = entry("f1", "Frieren");
    mocked.browseIndex.mockResolvedValue([bare]);
    mocked.searchAnime.mockRejectedValue(new Error("offline"));
    render(<BrowseView />);

    fireEvent.click(screen.getByRole("tab", { name: "A\u2013Z" }));
    fireEvent.click(await screen.findByRole("button", { name: "F" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^Frieren/ }));
    });

    expect(await screen.findByRole("button", { name: /Back/ })).toBeTruthy();
    expect(mocked.loadAnime).toHaveBeenCalledWith("f1");
  });
});
