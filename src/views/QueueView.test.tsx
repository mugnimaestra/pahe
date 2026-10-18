// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    loadQueue: vi.fn(),
    startQueue: vi.fn(),
    resumeDownload: vi.fn(),
    onResolverStatus: vi.fn(),
    onDownloadJob: vi.fn(),
  },
}));

import QueueView from "./QueueView";
import {
  ipc,
  type DownloadJob,
  type QueueSnapshot,
  type ResolverStatus,
} from "../ipc/client";

const mocked = vi.mocked(ipc);

let emit: ((job: DownloadJob) => void) | undefined;
let emitStatus: ((status: ResolverStatus) => void) | undefined;

function job(overrides: Partial<DownloadJob> = {}): DownloadJob {
  return {
    id: "job-1",
    animeId: "anime-id",
    title: "Seitokai Yakuindomo",
    episode: 1,
    quality: "1080p",
    state: "queued",
    progress: null,
    error: null,
    ...overrides,
  };
}

function snapshot(overrides: Partial<QueueSnapshot> = {}): QueueSnapshot {
  return { jobs: [], active: true, challenge: false, ...overrides };
}

async function emitJob(jobToEmit: DownloadJob) {
  await waitFor(() => expect(emit).toBeDefined());
  await act(async () => {
    emit?.(jobToEmit);
  });
}

describe("queue view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    emit = undefined;
    emitStatus = undefined;
    mocked.loadQueue.mockResolvedValue(snapshot());
    mocked.onDownloadJob.mockImplementation(async (handler) => {
      emit = handler;
      return () => {};
    });
    mocked.onResolverStatus.mockImplementation(async (handler) => {
      emitStatus = handler;
      return () => {};
    });
  });

  it("says so when nothing has been queued", async () => {
    render(<QueueView />);

    expect(await screen.findByText("No Download jobs yet.")).toBeTruthy();
  });

  it("lists each job with its Anime, Episode, Quality, and state in order", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({
        jobs: [
          job({ id: "job-1", episode: 1, state: "completed" }),
          job({ id: "job-2", episode: 2, quality: "720p", state: "queued" }),
          job({
            id: "job-3",
            episode: 3,
            state: "failed",
            error: "Every download Source for this Episode is dead.",
          }),
        ],
      }),
    );
    render(<QueueView />);

    await waitFor(() =>
      expect(screen.getAllByRole("listitem")).toHaveLength(3),
    );
    const rows = screen.getAllByRole("listitem").map((row) => row.textContent);
    expect(rows[0]).toContain("Seitokai Yakuindomo");
    expect(rows[0]).toContain("Episode 1");
    expect(rows[0]).toContain("1080p");
    expect(rows[0]).toContain("Completed");
    expect(rows[1]).toContain("Episode 2");
    expect(rows[1]).toContain("720p");
    expect(rows[1]).toContain("Queued");
    expect(rows[2]).toContain("Episode 3");
    expect(rows[2]).toContain("Failed");
    expect(rows[2]).toContain(
      "Every download Source for this Episode is dead.",
    );
  });

  it("shows live progress for the job an event names", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({
        jobs: [job({ id: "job-1", episode: 1 }), job({ id: "job-2", episode: 2 })],
      }),
    );
    render(<QueueView />);

    await waitFor(() =>
      expect(screen.getAllByRole("listitem")).toHaveLength(2),
    );

    await emitJob(
      job({
        id: "job-2",
        episode: 2,
        state: "running",
        progress: {
          bytes: 42_500_000,
          total: 135_000_000,
          speed: 8_500_000,
          etaSeconds: 11,
        },
      }),
    );

    expect(
      await screen.findByText(
        "Downloading · 42.5 MB / 135.0 MB · 8.5 MB/s · 11s left",
      ),
    ).toBeTruthy();
    expect(screen.getAllByRole("listitem")[0].textContent).toContain("Queued");
  });

  it("replaces a job's row as progress arrives", async () => {
    mocked.loadQueue.mockResolvedValue(snapshot({ jobs: [job()] }));
    render(<QueueView />);

    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
    await emitJob(job({ state: "running" }));
    await emitJob(
      job({
        state: "running",
        progress: {
          bytes: 5_000_000,
          total: 100_000_000,
          speed: 2_000_000,
          etaSeconds: 47,
        },
      }),
    );

    expect(
      await screen.findByText("Downloading · 5.0 MB / 100.0 MB · 2.0 MB/s · 47s left"),
    ).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  it("surfaces a queue read failure", async () => {
    mocked.loadQueue.mockRejectedValue("The queue is unavailable.");
    render(<QueueView />);

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The queue is unavailable.",
    );
  });

  it("shows an interrupted job with Resume, and resumes it", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({
        active: false,
        jobs: [job({ state: "interrupted" })],
      }),
    );
    mocked.resumeDownload.mockResolvedValue(
      snapshot({ jobs: [job({ state: "queued" })] }),
    );
    render(<QueueView />);

    expect(await screen.findByText("Interrupted")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));

    await waitFor(() =>
      expect(mocked.resumeDownload).toHaveBeenCalledWith("job-1"),
    );
    expect(await screen.findByText("Queued")).toBeTruthy();
  });

  it("shows a paused job with Resume", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({ jobs: [job({ state: "paused" })] }),
    );
    render(<QueueView />);

    expect(await screen.findByText("Paused")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("retries a failed job through Resume", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({
        jobs: [
          job({
            state: "failed",
            error: "The download was interrupted: connection reset.",
          }),
        ],
      }),
    );
    mocked.resumeDownload.mockResolvedValue(
      snapshot({ jobs: [job({ state: "completed" })] }),
    );
    render(<QueueView />);

    fireEvent.click(await screen.findByRole("button", { name: "Retry" }));

    await waitFor(() =>
      expect(mocked.resumeDownload).toHaveBeenCalledWith("job-1"),
    );
    expect(await screen.findByText("Completed")).toBeTruthy();
  });

  it("keeps restored queued jobs waiting until Start is pressed", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({
        active: false,
        jobs: [job({ state: "queued" }), job({ id: "job-2", episode: 2 })],
      }),
    );
    mocked.startQueue.mockResolvedValue(
      snapshot({
        active: true,
        jobs: [job({ state: "queued" }), job({ id: "job-2", episode: 2 })],
      }),
    );
    render(<QueueView />);

    const start = await screen.findByRole("button", { name: "Start downloads" });
    fireEvent.click(start);

    await waitFor(() => expect(mocked.startQueue).toHaveBeenCalled());
    expect(
      screen.queryByRole("button", { name: "Start downloads" }),
    ).toBeNull();
  });

  it("says a Challenge holds the queue while it waits", async () => {
    mocked.loadQueue.mockResolvedValue(snapshot({ jobs: [job()] }));
    render(<QueueView />);

    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
    await waitFor(() => expect(emitStatus).toBeDefined());
    await act(async () => {
      emitStatus?.({ state: "challenge" });
    });

    expect(
      await screen.findByText(
        /Cloudflare needs a human check.*queued downloads will continue/s,
      ),
    ).toBeTruthy();
  });

  it("starts with a Challenge already pending after a restart", async () => {
    mocked.loadQueue.mockResolvedValue(
      snapshot({ jobs: [job({ state: "paused" })], challenge: true }),
    );
    render(<QueueView />);

    expect(
      await screen.findByText(/Cloudflare needs a human check/),
    ).toBeTruthy();
  });
});
