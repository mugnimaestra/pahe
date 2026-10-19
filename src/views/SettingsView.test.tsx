// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    getSettings: vi.fn(),
    saveSettings: vi.fn(),
    testBaseUrl: vi.fn(),
  },
}));

import SettingsView from "./SettingsView";
import { ipc, type Settings } from "../ipc/client";

const mocked = vi.mocked(ipc);

const SETTINGS: Settings = {
  baseUrl: "https://animepahe.pw",
  libraryFolder: "/Users/me/Downloads/animepahe",
  defaultQuality: "1080p",
  proxy: "auto",
};

function value(label: string): string {
  return (screen.getByLabelText(label) as HTMLInputElement).value;
}

function choose(label: string, option: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value: option } });
}

describe("settings view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.getSettings.mockResolvedValue(SETTINGS);
    mocked.saveSettings.mockImplementation(async (settings) => settings);
  });

  it("shows every persisted setting", async () => {
    render(<SettingsView />);

    expect(await screen.findByLabelText("Base URL")).toBeTruthy();
    expect(value("Base URL")).toBe("https://animepahe.pw");
    expect(value("Library folder")).toBe("/Users/me/Downloads/animepahe");
    expect(value("Default Quality")).toBe("1080p");
    expect(value("Proxy policy")).toBe("auto");
  });

  it("saves every edited setting through the IPC client", async () => {
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://mirror.example" },
    });
    fireEvent.change(screen.getByLabelText("Library folder"), {
      target: { value: "/media/anime" },
    });
    choose("Default Quality", "720p");
    choose("Proxy policy", "always");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.saveSettings).toHaveBeenCalledWith({
        baseUrl: "https://mirror.example",
        libraryFolder: "/media/anime",
        defaultQuality: "720p",
        proxy: "always",
      }),
    );
    expect((await screen.findByRole("status")).textContent).toBe(
      "Settings saved.",
    );
  });

  it("keeps the saved values when a save is refused", async () => {
    mocked.saveSettings.mockRejectedValue("Base URL must be a valid http(s) URL");
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");

    // Anything but http(s) passes the browser's own check and reaches Rust.
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "ftp://mirror.example" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Base URL must be a valid http(s) URL",
    );
    expect(value("Base URL")).toBe("ftp://mirror.example");
  });

  it("tests the Base URL and calls a valid certificate reachable", async () => {
    mocked.testBaseUrl.mockResolvedValue("clean");
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");

    fireEvent.click(screen.getByRole("button", { name: "Test" }));

    await waitFor(() =>
      expect(mocked.testBaseUrl).toHaveBeenCalledWith("https://animepahe.pw"),
    );
    expect((await screen.findByRole("status")).textContent).toContain(
      "Reachable",
    );
  });

  it("distinguishes a Poisoned resolver from a dead mirror", async () => {
    mocked.testBaseUrl.mockResolvedValue("poisoned");
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");
    fireEvent.click(screen.getByRole("button", { name: "Test" }));

    const poisoned = await screen.findByRole("alert");
    expect(poisoned.textContent).toContain("Poisoned resolver");
    expect(poisoned.textContent).not.toContain("mirror");

    mocked.testBaseUrl.mockResolvedValue("unreachable");
    fireEvent.click(screen.getByRole("button", { name: "Test" }));

    const dead = await screen.findByRole("alert");
    expect(dead.textContent).toContain("No answer from this Base URL");
    expect(dead.textContent).not.toContain("Poisoned");
  });

  it("tests the Base URL currently typed, not the saved one", async () => {
    mocked.testBaseUrl.mockResolvedValue("clean");
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://mirror.example" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Test" }));

    await waitFor(() =>
      expect(mocked.testBaseUrl).toHaveBeenCalledWith("https://mirror.example"),
    );
  });

  it("surfaces a failed test as an alert", async () => {
    mocked.testBaseUrl.mockRejectedValue("Base URL must be a valid http(s) URL");
    render(<SettingsView />);
    await screen.findByLabelText("Base URL");

    fireEvent.click(screen.getByRole("button", { name: "Test" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Base URL must be a valid http(s) URL",
    );
  });

  it("surfaces a settings load failure", async () => {
    mocked.getSettings.mockRejectedValue("could not read settings");

    render(<SettingsView />);

    expect((await screen.findByRole("alert")).textContent).toBe(
      "could not read settings",
    );
  });
});
