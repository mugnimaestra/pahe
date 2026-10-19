import { describe, expect, it } from "vitest";

import { failureOf, failureText } from "./failure";
import type {
  DownloadJob,
  FailureReason,
  ResolverFailureKind,
} from "./ipc/client";

function job(overrides: Partial<DownloadJob> = {}): DownloadJob {
  return {
    id: "job-1",
    animeId: "anime-id",
    title: "Seitokai Yakuindomo",
    episode: 1,
    quality: "1080p",
    state: "failed",
    progress: null,
    error: "Something specific went wrong.",
    failure: "failed",
    ...overrides,
  };
}

describe("a failed Download job's text", () => {
  it("names every failure reason explicitly", () => {
    const expected: Record<FailureReason, string> = {
      deadSource: "Dead Source · Something specific went wrong.",
      expiredCdnUrl: "CDN URL expired · Something specific went wrong.",
      insufficientDisk:
        "Not enough disk space · Something specific went wrong.",
      unreachable: "Base URL unreachable · Something specific went wrong.",
      challengeTimedOut:
        "Cloudflare check timed out · Something specific went wrong.",
      failed: "Failed · Something specific went wrong.",
    };

    for (const [failure, text] of Object.entries(expected)) {
      expect(failureText(job({ failure: failure as FailureReason }))).toBe(text);
    }
  });

  it("falls back to the message when the job carries no reason", () => {
    expect(failureText(job({ failure: null }))).toBe(
      "Failed · Something specific went wrong.",
    );
    expect(failureText(job({ failure: null, error: null }))).toBe(
      "Failed · The download failed.",
    );
  });
});

describe("a Resolver failure from a rejected IPC call", () => {
  it("keeps every kind the Resolver can name", () => {
    const kinds: ResolverFailureKind[] = [
      "unreachable",
      "deadSource",
      "challengeTimedOut",
      "failed",
    ];

    for (const kind of kinds) {
      expect(failureOf({ kind, message: "why" })).toEqual({
        kind,
        message: "why",
      });
    }
  });

  it("falls back to a plain message for anything else", () => {
    expect(failureOf("the site is down")).toEqual({
      kind: "failed",
      message: "the site is down",
    });
    expect(failureOf({ kind: "surprise", message: "why" })).toEqual({
      kind: "failed",
      message: "why",
    });
  });
});
