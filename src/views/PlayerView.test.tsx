// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    checkLibraryFile: vi.fn(),
    libraryFileUrl: vi.fn(),
    openExternally: vi.fn(),
    playbackPosition: vi.fn(),
    savePlaybackPosition: vi.fn(),
    streamEpisode: vi.fn(),
  },
}));

import PlayerView from "./PlayerView";
import { ipc, type LibraryItem } from "../ipc/client";

const mocked = vi.mocked(ipc);

const ITEM: LibraryItem = {
  animeId: null,
  stableId: null,
  titleSlug: "Seitokai_Yakuindomo",
  poster: null,
  episode: 3,
  version: 1,
  quality: "1080p",
  group: "B-Bass",
  path: "/library/AnimePahe_Seitokai_Yakuindomo_-_03_BD_1080p_B-Bass.mp4",
};

function open() {
  const { container, unmount } = render(
    <PlayerView playback={{ kind: "library", item: ITEM }} title="Seitokai Yakuindomo" onBack={() => {}} />,
  );
  openUnmount = unmount;
  return container;
}

let openUnmount: () => void = () => {};

/** jsdom has no media pipeline; give the element a time and a length. */
function stubMedia(video: HTMLVideoElement, currentTime: number, duration: number) {
  Object.defineProperty(video, "currentTime", {
    value: currentTime,
    writable: true,
    configurable: true,
  });
  Object.defineProperty(video, "duration", {
    value: duration,
    configurable: true,
  });
}

const KEY = ["Seitokai_Yakuindomo:3"];

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
    mocked.playbackPosition.mockResolvedValue(null);
    mocked.savePlaybackPosition.mockResolvedValue();
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

  it("resumes where the Episode stopped and can start over", async () => {
    mocked.playbackPosition.mockResolvedValue(754);
    const container = open();
    const video = await findVideo(container);
    expect(mocked.playbackPosition).toHaveBeenCalledWith(KEY);

    stubMedia(video, 0, 1420);
    fireEvent.loadedMetadata(video);

    expect(video.currentTime).toBe(754);
    expect((await screen.findByText(/Resumed at 12:34/)).textContent).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Start over" }));
    expect(video.currentTime).toBe(0);
    expect(screen.queryByText(/Resumed at/)).toBeNull();
  });

  it("keys an enriched item on the numeric id and migrates older keys", async () => {
    const item = { ...ITEM, animeId: "old-session", stableId: 68 };
    const { container } = render(
      <PlayerView playback={{ kind: "library", item }} title="Seitokai Yakuindomo" onBack={() => {}} />,
    );
    await findVideo(container);

    expect(mocked.playbackPosition).toHaveBeenCalledWith([
      "#68:3",
      "Seitokai_Yakuindomo:3",
      "old-session:3",
    ]);
  });

  it("starts at the beginning when no position is kept", async () => {
    mocked.playbackPosition.mockRejectedValue("positions lock poisoned");
    const container = open();
    const video = await findVideo(container);

    stubMedia(video, 0, 1420);
    fireEvent.loadedMetadata(video);

    expect(video.currentTime).toBe(0);
    expect(screen.queryByText(/Resumed at/)).toBeNull();
  });

  it("keeps the position when playback pauses", async () => {
    const container = open();
    const video = await findVideo(container);

    stubMedia(video, 812.4, 1420);
    fireEvent.pause(video);

    expect(mocked.savePlaybackPosition).toHaveBeenCalledWith(KEY, 812.4, 1420);
  });

  it("keeps the last position when the player closes", async () => {
    const container = open();
    const video = await findVideo(container);

    stubMedia(video, 300, 1420);
    fireEvent.timeUpdate(video);
    stubMedia(video, 302, 1420);
    fireEvent.timeUpdate(video);
    openUnmount();

    expect(mocked.savePlaybackPosition).toHaveBeenLastCalledWith(KEY, 302, 1420);
  });

  describe("streaming", () => {
    const STREAM = {
      kind: "stream" as const,
      animeId: "anime-id",
      stableId: 42,
      episode: 5,
      session: "session-five",
      quality: "1080p",
    };

    function openStream() {
      const { container } = render(
        <PlayerView playback={STREAM} title="Fairy Tail" onBack={() => {}} />,
      );
      return container;
    }

    it("plays the relay URL and shares the position key with downloads", async () => {
      mocked.streamEpisode.mockResolvedValue({
        url: "http://127.0.0.1:5000/token",
        quality: "1080p",
        group: "SubsPlease",
      });
      const container = openStream();

      expect(screen.getByRole("status").textContent).toContain(
        "Finding a stream for Episode 5",
      );
      const video = await findVideo(container);
      expect(video.getAttribute("src")).toBe("http://127.0.0.1:5000/token");
      expect(mocked.streamEpisode).toHaveBeenCalledWith(
        "anime-id",
        "session-five",
        "1080p",
      );
      expect(mocked.playbackPosition).toHaveBeenCalledWith([
        "#42:5",
        "Fairy_Tail:5",
        "anime-id:5",
      ]);
      expect(screen.queryByRole("button", { name: "Open externally" })).toBeNull();
      expect(mocked.checkLibraryFile).not.toHaveBeenCalled();
    });

    it("names a Resolver failure and resolves again on Try again", async () => {
      mocked.streamEpisode
        .mockRejectedValueOnce({
          kind: "deadSource",
          message: "Every Source for this Episode is dead. Pick another Quality.",
        })
        .mockResolvedValueOnce({
          url: "http://127.0.0.1:5000/again",
          quality: "1080p",
          group: "",
        });
      const container = openStream();

      expect((await screen.findByRole("alert")).textContent).toContain(
        "Every Source for this Episode is dead",
      );
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));

      const video = await findVideo(container);
      expect(video.getAttribute("src")).toBe("http://127.0.0.1:5000/again");
      expect(mocked.streamEpisode).toHaveBeenCalledTimes(2);
    });

    it("says the CDN link may have expired when the stream stops", async () => {
      mocked.streamEpisode.mockResolvedValue({
        url: "http://127.0.0.1:5000/token",
        quality: "1080p",
        group: "",
      });
      const container = openStream();
      const video = await findVideo(container);

      fireEvent.error(video);

      expect((await screen.findByRole("alert")).textContent).toContain(
        "may have expired",
      );
      expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    });
  });
});
