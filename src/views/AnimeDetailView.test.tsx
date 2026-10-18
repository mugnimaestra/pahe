// @vitest-environment jsdom
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
    loadAnime: vi.fn(),
    onResolverStatus: vi.fn(),
  },
}));

import AnimeDetailView from "./AnimeDetailView";
import {
  ipc,
  type AnimeDetail,
  type ResolverStatus,
  type SearchResult,
} from "../ipc/client";

const mocked = vi.mocked(ipc);

const RESULT: SearchResult = {
  animeId: "7da6a559-70e7-5ce1-4f29-3732c1fea142",
  title: "Fairy Tail",
  year: 2009,
  poster: "https://i.animepahe.pw/posters/fairy-tail.webp",
};

const SYNOPSIS = "A wizard guild in the Kingdom of Fiore.";

const DETAIL: AnimeDetail = {
  synopsis: SYNOPSIS,
  episodes: [
    { number: 1, airDate: "2018-06-11" },
    { number: 2, airDate: null },
  ],
};

function renderDetail(onBack = vi.fn()) {
  render(<AnimeDetailView result={RESULT} onBack={onBack} />);
  return onBack;
}

describe("anime detail view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.onResolverStatus.mockResolvedValue(() => {});
    mocked.fetchPoster.mockResolvedValue("data:image/webp;base64,abc");
  });

  it("shows the poster, title, year, and synopsis", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    expect(mocked.loadAnime).toHaveBeenCalledWith(RESULT.animeId);
    expect(
      await screen.findByRole("heading", { name: RESULT.title }),
    ).toBeTruthy();
    expect(screen.getByText("2009")).toBeTruthy();
    expect(await screen.findByText(SYNOPSIS)).toBeTruthy();
    expect(
      await screen.findByAltText(`${RESULT.title} poster`),
    ).toBeTruthy();
  });

  it("lists every Episode with its air date", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    expect(await screen.findByText("Episode 1")).toBeTruthy();
    expect(screen.getByText("2018-06-11")).toBeTruthy();
    expect(screen.getByText("Episode 2")).toBeTruthy();
    expect(screen.getByText("Air date unknown")).toBeTruthy();
  });

  it("says it is loading Episodes before they arrive", () => {
    mocked.loadAnime.mockReturnValue(new Promise<AnimeDetail>(() => {}));
    renderDetail();

    expect(screen.getByRole("status").textContent).toBe("Loading Episodes…");
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("shows an explicit error instead of an empty list when the fetch fails", async () => {
    mocked.loadAnime.mockRejectedValue({
      kind: "unreachable",
      message: "Could not reach https://animepahe.pw.",
    });
    renderDetail();

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Could not reach https://animepahe.pw.",
    );
    expect(screen.queryByText(/No Episodes are listed/)).toBeNull();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("says so when the Anime has no Episodes", async () => {
    mocked.loadAnime.mockResolvedValue({ ...DETAIL, episodes: [] });
    renderDetail();

    expect(
      await screen.findByText("No Episodes are listed for this Anime yet."),
    ).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("asks for a human while a Challenge is pending", async () => {
    let status: ((next: ResolverStatus) => void) | undefined;
    mocked.onResolverStatus.mockImplementation(async (handler) => {
      status = handler;
      return () => {};
    });
    mocked.loadAnime.mockReturnValue(new Promise<AnimeDetail>(() => {}));
    renderDetail();

    await act(async () => {
      status?.({ state: "challenge" });
    });

    expect(screen.getByText(/Cloudflare needs a human check/)).toBeTruthy();
  });

  it("returns to the search results when Back is used", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    const onBack = renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "← Back to results" }),
    );

    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
