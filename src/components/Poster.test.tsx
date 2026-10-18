// @vitest-environment jsdom
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  ipc: {
    fetchPoster: vi.fn(),
  },
}));

import Poster from "./Poster";
import { ipc } from "../ipc/client";

const mocked = vi.mocked(ipc);
const URL = "https://i.animepahe.pw/posters/one.jpg";

describe("poster", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("renders the data URL fetched through the IPC client", async () => {
    mocked.fetchPoster.mockResolvedValue("data:image/jpeg;base64,abc");
    render(<Poster url={URL} alt="One poster" />);

    const image = await screen.findByAltText("One poster");
    expect(image.getAttribute("src")).toBe("data:image/jpeg;base64,abc");
    expect(mocked.fetchPoster).toHaveBeenCalledWith(URL);
  });

  it("keeps the placeholder when the fetch fails", async () => {
    mocked.fetchPoster.mockRejectedValue("the poster host answered 403");
    const { container } = render(<Poster url={URL} alt="One poster" />);

    await waitFor(() => expect(mocked.fetchPoster).toHaveBeenCalledTimes(1));

    expect(screen.queryByAltText("One poster")).toBeNull();
    expect(container.querySelector(".poster-empty")).toBeTruthy();
  });

  it("shows the placeholder without fetching when there is no poster URL", () => {
    const { container } = render(<Poster url={null} alt="One poster" />);

    expect(mocked.fetchPoster).not.toHaveBeenCalled();
    expect(container.querySelector(".poster-empty")).toBeTruthy();
  });
});
