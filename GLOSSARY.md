# pahe

Desktop client for animepahe: finds episodes, resolves their media URLs, downloads them, and plays them from a local library.

## Language

**Anime**:
A series page on animepahe, identified by its anime id.
_Avoid_: show, series, title

**Episode**:
A numbered entry of an Anime, identified by the anime id plus the episode number.
_Avoid_: chapter, installment

**Release**:
The animepahe record for one Episode (session, number, air date) returned by the release API.
_Avoid_: entry, item

**Session**:
The animepahe token that names one Episode's play page: `/play/<anime id>/<session>`.
_Avoid_: token, play id

**Source**:
A quality-labeled download link offered for a Session, pointing at a kwik shortlink.
_Avoid_: mirror, server, link

**Quality**:
The resolution label of a Source, such as `1080p`.
_Avoid_: resolution

**Challenge**:
A Cloudflare human-verification gate that blocks the Resolver until the user clears it.
_Avoid_: captcha, block

**Resolver**:
The component that turns a Source into a CDN URL by driving a browser context through animepahe and kwik.
_Avoid_: scraper, crawler

**CDN URL**:
The signed direct media URL the Resolver extracts for a Source.
_Avoid_: stream URL, download link

**Download job**:
A request to fetch one Episode at one Quality, tracked as queued, running, completed, or failed.
_Avoid_: task, transfer

**Library item**:
A downloaded Episode file pahe knows about, with its Anime, Episode, and Quality.
_Avoid_: download, entry

**Library**:
The folder of downloaded Episode files plus the metadata index pahe keeps.
_Avoid_: collection, vault

**Base URL**:
The animepahe origin pahe talks to, default `https://animepahe.pw`, configurable to survive domain changes.
_Avoid_: host, site URL
