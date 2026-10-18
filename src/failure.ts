import type { ResolverFailure, ResolverFailureKind } from "./ipc/client";
import { messageOf } from "./message";

function isFailureKind(value: unknown): value is ResolverFailureKind {
  return (
    value === "unreachable" ||
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
