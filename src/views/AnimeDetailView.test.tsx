// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
    loadAnime: vi.fn(),
    onResolverStatus: vi.fn(),
    onDownloadJob: vi.fn(),
    startDownload: vi.fn(),
  },
}));

import AnimeDetailView from "./AnimeDetailView";
import {
  ipc,
  type AnimeDetail,
  type DownloadJob,
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
    { number: 1, session: "session-one", airDate: "2018-06-11" },
    { number: 2, session: "session-two", airDate: null },
  ],
};

let emitDownload: ((job: DownloadJob) => void) | undefined;

function job(overrides: Partial<DownloadJob> = {}): DownloadJob {
  return {
    id: "job-1",
    animeId: RESULT.animeId,
    episode: 1,
    quality: "1080p",
    state: "running",
    progress: null,
    error: null,
    ...overrides,
  };
}

function renderDetail(onBack = vi.fn()) {
  render(<AnimeDetailView result={RESULT} onBack={onBack} />);
  return onBack;
}

async function download(jobToEmit: DownloadJob) {
  await waitFor(() => expect(emitDownload).toBeDefined());
  await act(async () => {
    emitDownload?.(jobToEmit);
  });
}

describe("anime detail view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    emitDownload = undefined;
    mocked.onResolverStatus.mockResolvedValue(() => {});
    mocked.onDownloadJob.mockImplementation(async (handler) => {
      emitDownload = handler;
      return () => {};
    });
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

  it("offers a Download control and a 1080p Quality on every Episode", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    expect(
      await screen.findByRole("button", { name: "Download Episode 1" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Download Episode 2" }),
    ).toBeTruthy();
    expect(
      (screen.getByLabelText("Quality for Episode 1") as HTMLSelectElement)
        .value,
    ).toBe("1080p");
  });

  it("starts a 1080p Download for the Episode and its Session", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.startDownload.mockResolvedValue(job());
    renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "Download Episode 1" }),
    );

    await waitFor(() =>
      expect(mocked.startDownload).toHaveBeenCalledWith({
        animeId: RESULT.animeId,
        title: RESULT.title,
        episode: 1,
        session: "session-one",
        quality: "1080p",
      }),
    );
  });

  it("sends a chosen Quality override", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.startDownload.mockResolvedValue(job({ quality: "720p" }));
    renderDetail();

    fireEvent.change(
      await screen.findByLabelText("Quality for Episode 1"),
      { target: { value: "720p" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Download Episode 1" }));

    await waitFor(() =>
      expect(mocked.startDownload).toHaveBeenCalledWith(
        expect.objectContaining({ episode: 1, quality: "720p" }),
      ),
    );
  });

  it("shows preparing while the Source is resolved", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    await download(job());

    expect(await screen.findByText("Preparing…")).toBeTruthy();
  });

  it("shows progress as bytes, speed, and ETA", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    await download(
      job({
        progress: {
          bytes: 42_500_000,
          total: 135_000_000,
          speed: 8_500_000,
          etaSeconds: 11,
        },
      }),
    );

    expect(
      await screen.findByText("42.5 MB / 135.0 MB · 8.5 MB/s · 11s left"),
    ).toBeTruthy();
  });

  it("shows a completed Download", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    await download(
      job({
        state: "completed",
        progress: {
          bytes: 135_000_000,
          total: 135_000_000,
          speed: null,
          etaSeconds: null,
        },
      }),
    );

    expect(await screen.findByText("Saved to the Library.")).toBeTruthy();
  });

  it("shows why a Download job failed", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    renderDetail();

    await download(
      job({
        state: "failed",
        error:
          "Every download Source for this Episode is dead. kwik did not redirect.",
      }),
    );

    expect(
      (await screen.findByRole("alert")).textContent,
    ).toContain("Every download Source for this Episode is dead.");
  });

  it("shows why a Download could not start", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.startDownload.mockRejectedValue(
      "Open the Anime again and pick an Episode.",
    );
    renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "Download Episode 1" }),
    );

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Open the Anime again and pick an Episode.",
    );
  });
});
