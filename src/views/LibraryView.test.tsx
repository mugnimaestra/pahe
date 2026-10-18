// @vitest-environment jsdom
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    loadLibrary: vi.fn(),
    enrichLibrary: vi.fn(),
    fetchPoster: vi.fn(),
  },
}));

import LibraryView from "./LibraryView";
import { ipc, type LibraryAnime } from "../ipc/client";

const mocked = vi.mocked(ipc);

const LIBRARY: LibraryAnime[] = [
  {
    animeId: "anime-id",
    title: "Seitokai ni mo Ana wa Aru!",
    poster: "https://i.animepahe.pw/posters/one.jpg",
    episodeCount: 3,
  },
];

describe("library view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.fetchPoster.mockResolvedValue("data:image/jpeg;base64,abc");
  });

  it("shows each Anime with its poster and Episode count", async () => {
    mocked.loadLibrary.mockResolvedValue(LIBRARY);
    mocked.enrichLibrary.mockResolvedValue(LIBRARY);

    render(<LibraryView />);

    expect(await screen.findByText("Seitokai ni mo Ana wa Aru!")).toBeTruthy();
    expect(screen.getByText("3 Episodes")).toBeTruthy();
    await waitFor(() =>
      expect(mocked.fetchPoster).toHaveBeenCalledWith(
        "https://i.animepahe.pw/posters/one.jpg",
      ),
    );
    expect(await screen.findByAltText("Seitokai ni mo Ana wa Aru! poster"))
      .toBeTruthy();
  });

  it("uses the singular for one Episode", async () => {
    mocked.loadLibrary.mockResolvedValue([
      { ...LIBRARY[0], episodeCount: 1 },
    ]);
    mocked.enrichLibrary.mockResolvedValue([{ ...LIBRARY[0], episodeCount: 1 }]);

    render(<LibraryView />);

    expect(await screen.findByText("1 Episode")).toBeTruthy();
  });

  it("keeps the scan result when enrichment fails", async () => {
    mocked.loadLibrary.mockResolvedValue(LIBRARY);
    mocked.enrichLibrary.mockRejectedValue("animepahe is unreachable");

    render(<LibraryView />);

    expect(await screen.findByText("Seitokai ni mo Ana wa Aru!")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows an explicit error when the Library folder cannot be read", async () => {
    mocked.loadLibrary.mockRejectedValue(
      "The Library folder /missing does not exist. Create it or choose another folder in Settings.",
    );

    render(<LibraryView />);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "does not exist",
    );
    expect(mocked.enrichLibrary).not.toHaveBeenCalled();
  });

  it("says so when the Library is empty", async () => {
    mocked.loadLibrary.mockResolvedValue([]);
    mocked.enrichLibrary.mockResolvedValue([]);

    render(<LibraryView />);

    expect(await screen.findByText("No Episodes in the Library yet."))
      .toBeTruthy();
  });

  it("keeps the placeholder when an item has no poster", async () => {
    const unenriched = [{ ...LIBRARY[0], poster: null }];
    mocked.loadLibrary.mockResolvedValue(unenriched);
    mocked.enrichLibrary.mockResolvedValue(unenriched);

    const { container } = render(<LibraryView />);

    expect(await screen.findByText("Seitokai ni mo Ana wa Aru!")).toBeTruthy();
    expect(container.querySelector(".poster-empty")).toBeTruthy();
    expect(mocked.fetchPoster).not.toHaveBeenCalled();
  });
});
