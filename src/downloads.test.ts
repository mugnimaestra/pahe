import { describe, expect, it } from "vitest";
import { mergeJobs, upsertJob } from "./downloads";
import type { DownloadJob } from "./ipc/client";

function job(overrides: Partial<DownloadJob> = {}): DownloadJob {
  return {
    id: "job-1",
    animeId: "anime-id",
    title: "Fairy Tail",
    episode: 1,
    quality: "1080p",
    state: "queued",
    progress: null,
    error: null,
    failure: null,
    ...overrides,
  };
}

describe("download jobs", () => {
  it("replaces a job by id and appends a new one", () => {
    const first = job({ state: "running" });
    const jobs = upsertJob(upsertJob([], first), job({ id: "job-2" }));

    expect(jobs.map((entry) => entry.id)).toEqual(["job-1", "job-2"]);
    expect(upsertJob(jobs, job({ state: "completed" }))[0].state).toBe(
      "completed",
    );
  });

  it("keeps the queue order and the newer event copy when a snapshot arrives", () => {
    const running = job({ state: "running" });
    const queued = job({ id: "job-2", episode: 2 });

    const merged = mergeJobs([running], [queued, job()]);

    expect(merged.map((entry) => entry.id)).toEqual(["job-2", "job-1"]);
    expect(merged[1].state).toBe("running");
  });
});
