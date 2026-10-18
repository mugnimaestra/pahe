// @vitest-environment jsdom
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
    searchAnime: vi.fn(),
    loadAnime: vi.fn(),
    onResolverStatus: vi.fn(),
    onDownloadJob: vi.fn(),
    ownedEpisodes: vi.fn(),
    enqueueDownloads: vi.fn(),
  },
}));

import SearchView from "./SearchView";
import { ipc, type ResolverStatus, type SearchResult } from "../ipc/client";

const mocked = vi.mocked(ipc);

const RESULT: SearchResult = {
  animeId: "0b9c9a2e-6d1f-4b3c-9c4e-1a2b3c4d5e6f",
  title: "Seitokai ni mo Ana wa Aru!",
  year: 2025,
  poster: "https://i.animepahe.si/posters/example.jpg",
};

function search(title: string) {
  fireEvent.change(screen.getByLabelText("Title"), {
    target: { value: title },
  });
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
}

describe("search view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.onResolverStatus.mockResolvedValue(() => {});
    mocked.onDownloadJob.mockResolvedValue(() => {});
    mocked.fetchPoster.mockResolvedValue("data:image/jpeg;base64,abc");
    mocked.ownedEpisodes.mockResolvedValue([]);
  });

  it("starts idle", () => {
    render(<SearchView />);

    expect(screen.getByText("Search animepahe by title.")).toBeTruthy();
    expect(mocked.searchAnime).not.toHaveBeenCalled();
  });

  it("renders posters, titles, and years from the IPC client", async () => {
    mocked.searchAnime.mockResolvedValue([RESULT]);
    render(<SearchView />);

    search("seitokai");

    expect(mocked.searchAnime).toHaveBeenCalledWith("seitokai");
    expect(await screen.findByText(RESULT.title)).toBeTruthy();
    expect(screen.getByText("2025")).toBeTruthy();

    const poster = await screen.findByAltText(`${RESULT.title} poster`);
    expect(poster.getAttribute("src")).toBe("data:image/jpeg;base64,abc");
    expect(mocked.fetchPoster).toHaveBeenCalledWith(RESULT.poster);
  });

  it("keeps a result when its poster cannot be fetched", async () => {
    mocked.fetchPoster.mockRejectedValue("the poster host answered 403");
    mocked.searchAnime.mockResolvedValue([RESULT]);
    const { container } = render(<SearchView />);

    search("seitokai");

    expect(await screen.findByText(RESULT.title)).toBeTruthy();
    expect(screen.queryByAltText(`${RESULT.title} poster`)).toBeNull();
    expect(container.querySelector(".poster-empty")).toBeTruthy();
  });

  it("says so when no Anime matched", async () => {
    mocked.searchAnime.mockResolvedValue([]);
    render(<SearchView />);

    search("nothing here");

    expect(
      await screen.findByText(/No Anime matched “nothing here”\./),
    ).toBeTruthy();
  });

  it("explains an unreachable Base URL", async () => {
    mocked.searchAnime.mockRejectedValue({
      kind: "unreachable",
      message: "Could not reach https://animepahe.pw.",
    });
    render(<SearchView />);

    search("seitokai");

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Could not reach https://animepahe.pw.",
    );
  });

  it("asks for a human while a Challenge is pending, then shows results", async () => {
    let status: ((next: ResolverStatus) => void) | undefined;
    mocked.onResolverStatus.mockImplementation(async (handler) => {
      status = handler;
      return () => {};
    });
    let finish: (results: SearchResult[]) => void = () => {};
    mocked.searchAnime.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );

    render(<SearchView />);
    search("seitokai");

    await act(async () => {
      status?.({ state: "challenge" });
    });

    expect(screen.getByText(/Cloudflare needs a human check/)).toBeTruthy();

    await act(async () => {
      finish([RESULT]);
    });

    expect(screen.getByText(RESULT.title)).toBeTruthy();
    expect(screen.queryByText(/Cloudflare needs a human check/)).toBeNull();
  });

  it("opens an Anime and returns to the search results", async () => {
    mocked.searchAnime.mockResolvedValue([RESULT]);
    mocked.loadAnime.mockResolvedValue({
      synopsis: "A wizard guild in the Kingdom of Fiore.",
      episodes: [{ number: 1, session: "session-one", airDate: "2018-06-11" }],
    });
    render(<SearchView />);

    search("seitokai");
    fireEvent.click(
      await screen.findByRole("button", {
        name: new RegExp(RESULT.title),
      }),
    );

    expect(mocked.loadAnime).toHaveBeenCalledWith(RESULT.animeId);
    expect(
      await screen.findByRole("heading", { name: RESULT.title }),
    ).toBeTruthy();
    expect(screen.getByText("Episode 1")).toBeTruthy();
    expect(screen.getByText("2018-06-11")).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "← Back to results" }),
    );

    expect(screen.getByRole("heading", { name: "Search" })).toBeTruthy();
    expect(screen.getByText(RESULT.title)).toBeTruthy();
  });

  it("falls back when a failure payload is malformed", async () => {
    mocked.searchAnime.mockRejectedValue({ kind: "weird", message: 42 });
    render(<SearchView />);

    search("seitokai");

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Something went wrong.",
    );
  });

  it("surfaces an unexpected failure", async () => {
    mocked.searchAnime.mockRejectedValue(new Error("boom"));
    render(<SearchView />);

    search("seitokai");

    expect((await screen.findByRole("alert")).textContent).toBe("boom");
  });
});
