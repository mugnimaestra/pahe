// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    checkLibraryFile: vi.fn(),
    libraryFileUrl: vi.fn(),
    openExternally: vi.fn(),
  },
}));

import PlayerView from "./PlayerView";
import { ipc, type LibraryItem } from "../ipc/client";

const mocked = vi.mocked(ipc);

const ITEM: LibraryItem = {
  animeId: null,
  titleSlug: "Seitokai_Yakuindomo",
  poster: null,
  episode: 3,
  version: 1,
  quality: "1080p",
  group: "B-Bass",
  path: "/library/AnimePahe_Seitokai_Yakuindomo_-_03_BD_1080p_B-Bass.mp4",
};

function open() {
  const { container } = render(
    <PlayerView item={ITEM} title="Seitokai Yakuindomo" onBack={() => {}} />,
  );
  return container;
}

async function findVideo(container: HTMLElement): Promise<HTMLVideoElement> {
  await waitFor(() => expect(container.querySelector("video")).not.toBeNull());
  return container.querySelector("video") as HTMLVideoElement;
}

describe("player view", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocked.checkLibraryFile.mockResolvedValue();
    mocked.libraryFileUrl.mockReturnValue("asset://localhost/library%2Fone.mp4");
    mocked.openExternally.mockResolvedValue();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("loads the Episode through the Library file URL and follows playback", async () => {
    const container = open();
    const player = container.querySelector(".view-player");
    expect(player?.getAttribute("data-state")).toBe("loading");
    expect(mocked.checkLibraryFile).toHaveBeenCalledWith(ITEM.path);

    const video = await findVideo(container);
    expect(video.getAttribute("src")).toBe("asset://localhost/library%2Fone.mp4");
    expect(video.hasAttribute("controls")).toBe(true);
    expect(mocked.libraryFileUrl).toHaveBeenCalledWith(ITEM.path);

    fireEvent.loadedData(video);
    expect(player?.getAttribute("data-state")).toBe("idle");

    fireEvent.playing(video);
    expect(player?.getAttribute("data-state")).toBe("playing");
    expect(screen.queryByRole("status")).toBeNull();

    fireEvent.pause(video);
    expect(player?.getAttribute("data-state")).toBe("idle");
  });

  it("shows the Library's missing-file error instead of a silent video", async () => {
    mocked.checkLibraryFile.mockRejectedValue(
      "This Episode's file is missing from the Library folder. Scan the Library again or download it again.",
    );

    const container = open();

    expect((await screen.findByRole("alert")).textContent).toContain("missing");
    expect(container.querySelector("video")).toBeNull();
    expect(mocked.libraryFileUrl).not.toHaveBeenCalled();
  });

  it("explains a media failure that is not a missing file", async () => {
    const container = open();
    const video = await findVideo(container);

    fireEvent.error(video);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Open externally",
    );
  });

  it("names the GStreamer codecs when playback fails on Linux", async () => {
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (X11; Linux x86_64) WebKitGTK/2.0",
    });
    const container = open();
    const video = await findVideo(container);

    fireEvent.error(video);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("GStreamer");
    expect(alert.textContent).toContain("Open externally");
  });

  it("opens the Episode externally through the IPC client", async () => {
    open();

    fireEvent.click(
      await screen.findByRole("button", { name: "Open externally" }),
    );

    expect(mocked.openExternally).toHaveBeenCalledWith(ITEM.path);
  });

  it("reports a failed external open", async () => {
    mocked.openExternally.mockRejectedValue(
      "The system could not open this Episode: no handler",
    );
    open();

    fireEvent.click(
      await screen.findByRole("button", { name: "Open externally" }),
    );

    expect((await screen.findByRole("alert")).textContent).toContain(
      "could not open",
    );
  });
});
