import type {
  DownloadJob,
  FailureReason,
  ResolverFailure,
  ResolverFailureKind,
} from "./ipc/client";
import { messageOf } from "./message";

function isFailureKind(value: unknown): value is ResolverFailureKind {
  return (
    value === "unreachable" ||
    value === "deadSource" ||
    value === "challengeTimedOut" ||
    value === "failed"
  );
}

/** Reads a Resolver failure from a rejected IPC call, falling back to its message. */
export function failureOf(error: unknown): ResolverFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "kind" in error &&
    isFailureKind(error.kind) &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return { kind: error.kind, message: error.message };
  }
  return { kind: "failed", message: messageOf(error) };
}

/** The explicit name of each failure reason. The message follows it. */
const FAILURE_LABELS: Record<FailureReason, string> = {
  deadSource: "Dead Source",
  expiredCdnUrl: "CDN URL expired",
  insufficientDisk: "Not enough disk space",
  unreachable: "Base URL unreachable",
  challengeTimedOut: "Cloudflare check timed out",
  failed: "Failed",
};

/** A failed Download job's explicit state and its plain-language reason. */
export function failureText(job: DownloadJob): string {
  const label = job.failure === null ? "Failed" : FAILURE_LABELS[job.failure];

  return `${label} · ${job.error ?? "The download failed."}`;
}
