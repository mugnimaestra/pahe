import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC_DIR = fileURLToPath(new URL("..", import.meta.url));
const IPC_CLIENT = join(SRC_DIR, "ipc", "client.ts");
const TAURI_IMPORT = /(?:from|import)\s*\(?\s*["']@tauri-apps\//;

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

describe("IPC boundary", () => {
  it("is crossed only by the typed IPC client", () => {
    const offenders = sourceFiles(SRC_DIR)
      .filter((file) => file !== IPC_CLIENT)
      .filter((file) => TAURI_IMPORT.test(readFileSync(file, "utf8")))
      .map((file) => relative(SRC_DIR, file));

    expect(offenders).toEqual([]);
  });
});
