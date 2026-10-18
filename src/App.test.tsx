// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./ipc/client", () => ({
  ipc: {
    getSettings: vi.fn(),
    saveSettings: vi.fn(),
    searchAnime: vi.fn(),
    fetchPoster: vi.fn(),
    onResolverStatus: vi.fn(),
  },
}));

import App from "./App";
import { ipc } from "./ipc/client";

const mocked = vi.mocked(ipc);

describe("app shell", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.getSettings.mockResolvedValue({ baseUrl: "https://animepahe.pw" });
    mocked.onResolverStatus.mockResolvedValue(() => {});
  });

  it("renders the navigation between the main views", () => {
    render(<App />);

    const nav = screen.getByRole("navigation", { name: "Main" });
    for (const label of ["Search", "Library", "Queue", "Settings"]) {
      expect(nav.textContent).toContain(label);
    }
    expect(screen.getByRole("heading", { name: "Search" })).toBeTruthy();
  });

  it("switches to the settings view", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
  });

  it("shows the persisted base url", async () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    const input = (await screen.findByLabelText("Base URL")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("https://animepahe.pw"));
  });

  it("saves an edited base url through the IPC client", async () => {
    mocked.saveSettings.mockResolvedValue({ baseUrl: "https://mirror.example" });
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const input = await screen.findByLabelText("Base URL");
    fireEvent.change(input, { target: { value: "https://mirror.example" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.saveSettings).toHaveBeenCalledWith({
        baseUrl: "https://mirror.example",
      }),
    );
    expect((await screen.findByRole("status")).textContent).toBe(
      "Settings saved.",
    );
  });

  it("surfaces a settings load failure", async () => {
    mocked.getSettings.mockRejectedValue("could not read settings");

    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "could not read settings",
    );
  });
});
