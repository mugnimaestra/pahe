import { useEffect, useState } from "react";
import { ipc } from "../ipc/client";

/**
 * Whether a Cloudflare Challenge is pending. The Resolver window is already
 * open when this flips true, so the view only has to say so.
 */
export function useResolverChallenge(): [
  boolean,
  (challenge: boolean) => void,
] {
  const [challenge, setChallenge] = useState(false);

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    ipc
      .onResolverStatus((status) => {
        if (active) setChallenge(status.state === "challenge");
      })
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        // Resolver status is advisory; the request reports its own outcome.
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  return [challenge, setChallenge];
}
