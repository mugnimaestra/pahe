// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
    loadAnime: vi.fn(),
    getSettings: vi.fn(),
    onResolverStatus: vi.fn(),
    onDownloadJob: vi.fn(),
    ownedEpisodes: vi.fn(),
    enqueueDownloads: vi.fn(),
  },
}));

import AnimeDetailView from "./AnimeDetailView";
import {
  ipc,
  type AnimeDetail,
  type DownloadJob,
  type Enqueued,
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
    title: RESULT.title,
    episode: 1,
    quality: "1080p",
    state: "running",
    progress: null,
    error: null,
    failure: null,
    ...overrides,
  };
}

function enqueued(overrides: Partial<Enqueued> = {}): Enqueued {
  return { jobs: [], skipped: [], ...overrides };
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
    mocked.getSettings.mockResolvedValue({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/Users/me/Downloads/animepahe",
      defaultQuality: "1080p",
      proxy: "auto",
      resolverUserAgent: "",
    });
    mocked.ownedEpisodes.mockResolvedValue([]);
    mocked.enqueueDownloads.mockResolvedValue(enqueued());
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

  it("starts a new Download at the saved Default Quality", async () => {
    mocked.getSettings.mockResolvedValue({
      baseUrl: "https://animepahe.pw",
      libraryFolder: "/Users/me/Downloads/animepahe",
      defaultQuality: "720p",
      proxy: "auto",
      resolverUserAgent: "",
    });
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.enqueueDownloads.mockResolvedValue(enqueued({ jobs: [job()] }));
    renderDetail();

    expect(
      (
        (await screen.findByLabelText(
          "Quality for Episode 1",
        )) as HTMLSelectElement
      ).value,
    ).toBe("720p");

    fireEvent.click(screen.getByRole("button", { name: "Download Episode 2" }));

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        expect.objectContaining({ episode: 2, quality: "720p" }),
      ]),
    );
  });

  it("enqueues a 1080p Download for the Episode and its Session", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.enqueueDownloads.mockResolvedValue(enqueued({ jobs: [job()] }));
    renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "Download Episode 1" }),
    );

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        {
          animeId: RESULT.animeId,
          title: RESULT.title,
          episode: 1,
          session: "session-one",
          quality: "1080p",
          redownload: false,
        },
      ]),
    );
  });

  it("sends a chosen Quality override", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.enqueueDownloads.mockResolvedValue(
      enqueued({ jobs: [job({ quality: "720p" })] }),
    );
    renderDetail();

    fireEvent.change(
      await screen.findByLabelText("Quality for Episode 1"),
      { target: { value: "720p" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Download Episode 1" }));

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        expect.objectContaining({ episode: 1, quality: "720p" }),
      ]),
    );
  });

  it("queues the selected Episodes in order with their own Qualities", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.enqueueDownloads.mockResolvedValue(
      enqueued({ jobs: [job(), job({ id: "job-2", episode: 2 })] }),
    );
    renderDetail();

    fireEvent.click(await screen.findByLabelText("Select Episode 2"));
    fireEvent.change(screen.getByLabelText("Quality for Episode 2"), {
      target: { value: "720p" },
    });
    fireEvent.click(screen.getByLabelText("Select Episode 1"));
    fireEvent.click(
      screen.getByRole("button", { name: "Download selected" }),
    );

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        expect.objectContaining({ episode: 1, quality: "1080p" }),
        expect.objectContaining({ episode: 2, quality: "720p" }),
      ]),
    );
  });

  it("queues every Episode the Library does not own when Download all is used", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.ownedEpisodes.mockResolvedValue([{ episode: 1, quality: "1080p" }]);
    mocked.enqueueDownloads.mockResolvedValue(
      enqueued({ jobs: [job({ episode: 2 })] }),
    );
    renderDetail();

    expect(await screen.findByText("Owned 1080p")).toBeTruthy();
    expect(
      (await screen.findByLabelText("Select Episode 1") as HTMLInputElement)
        .disabled,
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Download all" }));

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        expect.objectContaining({ episode: 2, redownload: false }),
      ]),
    );
  });

  it("redownloads an owned Episode only when asked explicitly", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.ownedEpisodes.mockResolvedValue([{ episode: 1, quality: "1080p" }]);
    mocked.enqueueDownloads.mockResolvedValue(enqueued({ jobs: [job()] }));
    renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "Redownload Episode 1" }),
    );

    await waitFor(() =>
      expect(mocked.enqueueDownloads).toHaveBeenCalledWith([
        expect.objectContaining({ episode: 1, redownload: true }),
      ]),
    );
  });

  it("says which Episodes the Library or the queue already holds", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.enqueueDownloads.mockResolvedValue(
      enqueued({
        skipped: [
          {
            animeId: RESULT.animeId,
            episode: 1,
            quality: "1080p",
            reason: "inLibrary",
          },
          {
            animeId: RESULT.animeId,
            episode: 2,
            quality: "1080p",
            reason: "inQueue",
          },
        ],
      }),
    );
    renderDetail();

    fireEvent.click(
      await screen.findByRole("button", { name: "Download all" }),
    );

    expect(
      (await screen.findByRole("status")).textContent,
    ).toBe(
      "Already in the Library: Episode 1 (1080p). Already queued: Episode 2 (1080p).",
    );
  });

  it("marks an Episode owned when its Download finishes", async () => {
    mocked.loadAnime.mockResolvedValue(DETAIL);
    mocked.ownedEpisodes
      .mockResolvedValueOnce([])
      .mockResolvedValue([{ episode: 1, quality: "1080p" }]);
    renderDetail();

    await download(job({ state: "completed" }));

    expect(await screen.findByText("Owned 1080p")).toBeTruthy();
    expect(mocked.ownedEpisodes).toHaveBeenCalledTimes(2);
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
    mocked.enqueueDownloads.mockRejectedValue(
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
