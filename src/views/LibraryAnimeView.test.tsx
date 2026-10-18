// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    libraryItems: vi.fn(),
    checkLibraryFile: vi.fn(),
    libraryFileUrl: vi.fn(),
    openExternally: vi.fn(),
  },
}));

import LibraryAnimeView from "./LibraryAnimeView";
import {
  ipc,
  type LibraryAnime,
  type LibraryItem,
} from "../ipc/client";

const mocked = vi.mocked(ipc);

const ANIME: LibraryAnime = {
  animeId: null,
  title: "Seitokai Yakuindomo",
  poster: null,
  episodeCount: 2,
};

const ITEMS: LibraryItem[] = [
  {
    animeId: null,
    titleSlug: "Seitokai_Yakuindomo",
    poster: null,
    episode: 1,
    version: 1,
    quality: "1080p",
    group: "B-Bass",
    path: "/library/AnimePahe_Seitokai_Yakuindomo_-_01_BD_1080p_B-Bass.mp4",
  },
  {
    animeId: null,
    titleSlug: "Seitokai_Yakuindomo",
    poster: null,
    episode: 2,
    version: 1,
    quality: "720p",
    group: "B-Bass",
    path: "/library/AnimePahe_Seitokai_Yakuindomo_-_02_BD_720p_B-Bass.mp4",
  },
];

describe("library Anime view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.libraryItems.mockResolvedValue(ITEMS);
    mocked.checkLibraryFile.mockResolvedValue();
    mocked.libraryFileUrl.mockReturnValue("asset://localhost/library%2Ftwo.mp4");
    mocked.openExternally.mockResolvedValue();
  });

  it("lists the downloaded Episodes with their Quality", async () => {
    render(<LibraryAnimeView anime={ANIME} onBack={() => {}} />);

    expect(await screen.findByText("Episode 1")).toBeTruthy();
    expect(screen.getByText("Episode 2")).toBeTruthy();
    expect(screen.getByText("1080p · B-Bass")).toBeTruthy();
    expect(screen.getByText("720p · B-Bass")).toBeTruthy();
    expect(mocked.libraryItems).toHaveBeenCalledWith(
      null,
      "Seitokai Yakuindomo",
    );
  });

  it("plays a picked Episode and comes back to the list", async () => {
    render(<LibraryAnimeView anime={ANIME} onBack={() => {}} />);
    await screen.findByText("Episode 2");

    fireEvent.click(screen.getAllByRole("button", { name: "Play" })[1]);

    expect(
      await screen.findByRole("heading", { name: "Episode 2" }),
    ).toBeTruthy();
    expect(mocked.checkLibraryFile).toHaveBeenCalledWith(ITEMS[1].path);

    fireEvent.click(screen.getByRole("button", { name: "← Back to Episodes" }));

    expect(await screen.findByText("Episode 1")).toBeTruthy();
  });

  it("shows an explicit error when the Library cannot list the Episodes", async () => {
    mocked.libraryItems.mockRejectedValue(
      "The Library index could not be read.",
    );

    render(<LibraryAnimeView anime={ANIME} onBack={() => {}} />);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "could not be read",
    );
  });

  it("says so when the Anime holds no Library items any more", async () => {
    mocked.libraryItems.mockResolvedValue([]);

    render(<LibraryAnimeView anime={ANIME} onBack={() => {}} />);

    expect(
      await screen.findByText(
        "No Library items for this Anime. Scan the Library again.",
      ),
    ).toBeTruthy();
  });
});
