# Manual smoke checklist

CI builds the app for macOS, Windows, and Linux on every push, launches each
build against an empty app store, and checks that cold start settles and adopts
an existing Library file (`.github/actions/cold-start-smoke/`). Everything the
app does against the network — search past Cloudflare, resolution, downloading,
resuming — and everything the webview does with video cannot be exercised from
CI. Those parts need a human on a machine of each target platform.

Run this list against the release artifacts before calling a release shipped.
Record where it was run and what failed; a checklist that was not run is a gap,
and gaps belong in the ticket or the release notes, not in silence.

## Artifacts and setup

| Platform | Artifact | Install |
| --- | --- | --- |
| macOS | `pahe_<version>_<arch>.dmg` | Open the dmg, drag **pahe** to Applications. Unsigned: first launch needs right-click → **Open**, or `xattr -dr com.apple.quarantine /Applications/pahe.app`. |
| Windows | `pahe_<version>_x64-setup.exe` | Run the installer. Unsigned: SmartScreen needs **More info** → **Run anyway**. |
| Linux | `pahe_<version>_amd64.deb`, `pahe_<version>_amd64.AppImage` | `sudo apt install ./pahe_<version>_amd64.deb`, or `chmod +x pahe_<version>_amd64.AppImage && ./pahe_<version>_amd64.AppImage`. |

Before the first run, know which network you are on. On an ISP that poisons
`animepahe.pw`, pahe still works — its local DoH proxy tunnels the Resolver —
but a poisoned resolver is the regime where Settings → **Test** should say
*Poisoned resolver*. Cloudflare may ask for a human check on `/api` even when
DNS is clean. Both behaviours are expected; surprise at either is a finding.

Record at the top of a run:

- Version and artifact filename:
- OS and version:
- Clean or poisoned DNS (`dig animepahe.pw`), and whether the local proxy was on:
- Optional: take a screenshot of the Resolver window when a Challenge appears.

## 1. Cold start (all platforms)

CI already covers launch-and-settle plus Library adoption on a clean data
store. These steps cover what CI cannot see: the window, the first look, the
second launch.

- [ ] Install the artifact and launch it. A window titled **pahe** appears with
      the sidebar (Search, Library, Queue, Settings) and the Search view. No
      crash, no blank window, no unhandled-error dialog.
- [ ] Settings shows the expected defaults: Base URL
      `https://animepahe.pw`, Library folder `<Downloads>/animepahe`, Default
      Quality `1080p`, Proxy `Auto`.
- [ ] Library lists the Episodes already in the Library folder (the app adopts
      `AnimePahe_*.mp4` files from the CLI without renaming them). Count the
      groups against the folder once; afterwards the app should agree with the
      folder.
- [ ] Quit and launch again. Settings, Library, and Queue come back as they
      were; nothing is re-scanned from network on startup (startup does not
      touch the network at all).
- [ ] Launch a second copy while the first is running. The existing window
      comes to the front; no second instance and no duplicate data.
- [ ] macOS: launch once from the dmg itself, then from Applications. Both
      start; the app does not depend on being in `/Applications`.
- [ ] Windows: after the installer finishes, pahe appears in the Start menu and
      uninstalls cleanly (Settings → Apps → uninstall). No leftover first-run
      error dialog.

## 2. Challenge (all platforms)

- [ ] With the app up, search a common title and open a result. Somewhere on
      this path — search, Anime detail, or the first Download job — Cloudflare
      may present a human check.
- [ ] When it does: the **Resolver window** surfaces by itself, the view that
      is waiting shows *Cloudflare needs a human check. The Resolver window is
      open — finish the check and … will continue*, and the search or download
      does **not** fail while the check is pending.
- [ ] Clear the check in the Resolver window (the widget is a page-level
      challenge: if the homepage shows nothing, navigate inside the window once
      — a document navigation is what triggers it for this egress).
- [ ] The window hides itself again and the waiting search or Download job
      finishes without another click.
- [ ] Queue behaviour while the check is pending: open Queue with jobs queued.
      A running job shows **Paused · a Cloudflare check is pending…**; queued
      jobs wait instead of failing. If you ignore the check long enough, the job
      fails with the Challenge-timeout reason rather than a generic error.
- [ ] Offline variant: disconnect the network, start a search. The app reports
      the unreachable reason in plain language (not a generic failure), and
      recovers when the network returns.

## 3. Clearance reuse (all platforms)

- [ ] After the check in section 2 is cleared, run a second search and a second
      Download job inside the same session. Neither shows a new Challenge.
- [ ] Quit the app, launch it again, and search again. Clearance survives the
      restart (its cookies live in pahe's own data store, not a shared browser
      profile).
- [ ] Change the Base URL in Settings to another mirror and save, then search.
      The app clears the old clearance and opens the Resolver for the new
      origin — the new origin gets its own check, and the old one cannot leak
      cookies across.
- [ ] Force a check on purpose (delete the app's data for the origin, or clear
      the cookies in the Resolver window with devtools if you have them open)
      and confirm the flow in section 2 repeats cleanly. A Challenge is not a
      dead end.

## 4. Download (all platforms)

- [ ] Search, open an Anime, pick one Episode, download it. The Queue row goes
      `queued` → `running` with bytes, percentage, speed, and ETA, and the row
      settles on `completed`.
- [ ] The file lands in the Library folder as
      `AnimePahe_<title_slug>_-_<NN>_<quality>_<group>.mp4`; no `.part` file is
      left behind.
- [ ] The Episode shows up in Library for that Anime at the chosen Quality,
      and the same Episode is marked as owned on the Anime detail page.
- [ ] Download all (or select several Episodes) and queue them. Jobs run one at
      a time in order; the app stays responsive while one runs.
- [ ] Queue the same Episode and Quality twice. The second is recognised as
      already owned and does not download again.
- [ ] Change the Quality selector to a lower Quality for one Episode and
      download it. The row and the resulting file both carry that Quality.
- [ ] Windows and Linux: watch the whole of the smallest Episode's download
      through to completion. (macOS is the machine the app was developed on;
      the other two platforms are where path, permission, and proxy plumbing
      differs.)
- [ ] Linux: while the AppImage runs, the app is readable from a non-root
      account and the Library folder is writable — a permission failure must
      read as a plain-language message, not a silent stop.
- [ ] Disk-space behaviour, if you can produce it (small volume): a Download
      job that cannot fit fails **before** writing, naming the disk reason.

## 5. Resume (all platforms)

- [ ] Start a Download job and let it reach a visible percentage, then quit the
      app while it runs. A `.part` file stays on disk.
- [ ] Launch the app again. The job reads **Interrupted · pahe closed while this
      Download ran…** and the queue does not restart by itself.
- [ ] Press **Resume** on that job. It continues from the portion already on
      disk (the byte counter starts near where it stopped, not at zero) and
      finishes with a correct file that plays to the end.
- [ ] Pause-by-Challenge path: with a job running, force a Challenge (or start
      a download when the site is already asking). The job pauses instead of
      failing; after clearance it continues in place.
- [ ] Kill the network mid-download. The job fails with the unreachable reason
      and offers **Retry**; Retry re-resolves and either resumes or starts
      cleanly, without leaving a corrupt final file.
- [ ] A truncated CDN URL case, if you can reproduce it (let a download sit
      until the URL expires): the app re-resolves once and retries before
      failing as a dead Source.

## 6. Playback (all platforms)

- [ ] Library → an Anime → an Episode → **Play**. The in-app player opens and
      the first frame arrives; `data-state` moves `playing`.
- [ ] Pause, play, seek to the middle, and mute/unmute. Audio and video follow
      the controls and the position bar tracks playback.
- [ ] Play a second Episode from a different Anime. The player replaces the
      first cleanly (no stuck previous frame, no duplicate video element).
- [ ] Move an Episode's file out of the Library folder and press Play. The app
      says the file is missing from the Library folder and tells you to scan
      again or download it again — not a codec error and not silence.
- [ ] Press **Open externally**. On macOS the default player (QuickTime or
      IINA) opens exactly that file; on Windows, the default player opens it; on
      Linux, `xdg-open`'s target opens it.
- [ ] Windows playback, specifically: play an Episode in-app with the media
      feature of WebView2 — Windows 10/11 N editions and some server images
      lack it, and the failure must be the player error with the external
      fallback mentioned, not a crash.
- [ ] **Linux playback**: play an Episode in-app.
  - Where the system GStreamer codecs are present (`gst-play-1.0 <file>`
    plays, typically `gstreamer1.0-plugins-base` + `-good` + `-libav`, or the
    distro's codec package), the player must play in-app. Record the distro,
    the GStreamer packages (`dpkg -l 'gstreamer1.0*'`), and whether it played.
  - Where they are missing or incomplete (`gst-play-1.0` fails on the same
    file), in-app playback shows *This Episode could not be played in the app.
    On Linux, the system GStreamer codecs may be missing — use Open externally
    instead.* — this is the documented fallback, and **Open externally**
    completes the task with a system player (mpv, VLC, …). Record the missing
    packages.
  - A distro without both in-app playback and a working external player is a
    release blocker for that distro; record it as one.

## Recording results

Run this per platform and paste the outcome where the release is tracked —
a comment on the release or on the closing issue. Include:

- The four setup facts (version, OS, DNS regime, proxy policy), then per section
  either **pass** or the step number, what you saw, and what you expected.
- For playback, the codec package list on Linux and whether the external
  fallback was needed.
- Anything you could not run, and why. "Not run" is a valid entry; an unmarked
  checklist is not.
