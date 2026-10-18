import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { ipc } from "./client";

describe("ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("reads settings through the get_settings command", async () => {
    invoke.mockResolvedValue({ baseUrl: "https://animepahe.pw" });

    await expect(ipc.getSettings()).resolves.toEqual({
      baseUrl: "https://animepahe.pw",
    });
    expect(invoke).toHaveBeenCalledWith("get_settings");
  });

  it("saves settings through the save_settings command", async () => {
    const settings = { baseUrl: "https://mirror.example" };
    invoke.mockResolvedValue(settings);

    await expect(ipc.saveSettings(settings)).resolves.toEqual(settings);
    expect(invoke).toHaveBeenCalledWith("save_settings", { settings });
  });
});
