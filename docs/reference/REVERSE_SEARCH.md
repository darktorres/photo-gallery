# Reverse image search

Measured results moved here from the former `FINDINGS.md` log. Each entry says where and when it was measured. Newest entries first within each section. The test set and the quick-test subset are defined in [../eval/TEST_SET.md](../eval/TEST_SET.md).

## Reverse image search: Yandex (ImageRater port, 2026-10-31)

ImageRater (`ImageRater/shared/static/frontend.js`, `reverseImageSearch`, lines 984–1026) uploads the photo to
`yandex.com/images-apphost/image-download` and opens `yandex.com/images/search?cbir_id=…` in a new tab. Only
`yandex.com` is used there; `.ru` hosts appear only in the browser's analytics traffic. The upload returns
`cbir_id` and `sizes.orig.path`. Measured here with a standalone Rust probe (not committed) and
a Playwright browser session on the same test image (a generated 64×64 PNG).

### What the results page contains

- **Two lists.** Sites (`cbir_page=sites`, paged with `&p=N`) and Similar (`cbir_page=similar`, one page).
- **Data source.** The results are an HTML-escaped state blob in the page (`originalImage&quot;:{&quot;url&quot;:…`).
  The infinite scroll renders from this blob with no further requests. The DOM stops at about 150 items, so it
  is not a complete source.
- **Sites record fields.** `title`, `description`, `url` (source page), `domain`, `thumb` (URL, width, height)
  and `originalImage.url`, the full-size file to download.
- **Similar items.** 55 items, `pos` 0–54. Each item's `img_url=` parameter is the full-size URL.
- **Overlap.** 43 of the 55 Similar images also appear in the Sites set (browser data).
- **Browser totals.** Sites pages 0–5 hold 492 unique originals in all (pages 4 and 6–10 come back empty; pages
  beyond 10 were not checked).

### Why a plain HTTP client gets fewer results

Measured with the same URLs and the same test image:

| Client                                                    | Sites p0                        | Sites p1     | Similar      |
|-----------------------------------------------------------|---------------------------------|--------------|--------------|
| Browser (Chrome, Playwright)                              | 160 records                     | 155          | 55 items     |
| Probe, no cookies                                         | 118 records (79 unique sources) | 0            | 40 items     |
| Probe, homepage visited first (`yandex.com/`, cookie jar) | 118                             | not measured | 40           |
| Probe, `yp` cookie with screen metrics only               | **160**                         | 0            | **55**       |
| Probe, `yp` + `spravka`                                   | not measured                    | **155**      | not measured |

- **Not the cause:** HTTP/1.1 vs HTTP/2 (curl gives the same 118), compression, and browser-like request headers.
- **`yp` cookie.** The browser's JavaScript writes screen metrics into `yp`, in the shape `<ts>.szm.1:WxH:WxH`.
  A fresh client gets `<ts>.ygu.1` from `yandex.com/` and nothing from `yandex.com/images/`. With `yp` in szm
  shape and no other cookie, page 0 and Similar match the browser.
- **`spravka` cookie.** Sites pages 1 and up are empty without it. It appears to be Yandex's anti-robot token;
  the browser had it, probably from the captcha it passed earlier in the session. This was not confirmed.
  The probe does not send it.
- **Captcha.** Yandex returned `showcaptcha` after a few dozen requests from the automated browser. It was
  cleared by hand. Nothing in this section gets past it.

### Crawlers script: `crawlers/yandex-reverse/search.py` (2026-10-31)

The crawlers monorepo already has a Yandex client. Its `perform_yandex_search` gets the same two groups, and
the Sites list above was the wrong target for group 1:

- **Group 1 is "other sizes".** The script reads `initialState.cbirOtherSizes` from the `ImagesApp-*`
  `data-state` attribute on the results page. Those are the same image at other resolutions, which is what
  group 1 means. The Sites list (`cbir_page=sites`) is a different section.
- **Group 2 is the similar JSON API.** `images/search?format=json&request=…&cbir_page=similar&p=N` (the request
  body names the `serpList/fetch` ajax block). Each item's `img_url` query parameter is the full-size URL.
- **Measured on a public-domain photo** (Fronalpstock, Wikimedia Commons, 14.6 MB JPEG), same settings for both
  runs (`SIMILAR_PAGES_TO_DOWNLOAD = 5`):

| Run                     | Other sizes                 | Similar                                                           |
|-------------------------|-----------------------------|-------------------------------------------------------------------|
| As written (no cookies) | **18** (largest 10109×4542) | 40, page 1 fails (`Unexpected API response structure … 'blocks'`) |
| With `yp` cookie (szm)  | **18**                      | **55**, page 1 fails the same way                                 |

- **Group 1 needs no cookie.** The other-sizes block is in the initial HTML, and the count matched with and
  without `yp`.
- **Group 2 needs the `yp` cookie** for the full 55 on page 0.
- **Page 1 of Similar is a captcha, with or without `yp`** (rechecked 2026-10-31). Yandex answers it with HTTP
  200 and JSON `{"type": "captcha", "captcha": {…}}`, which links to `showcaptcha`. The script never reports it:
  it reads that as an unexpected structure, stops paging and keeps page 0. So the script gets one page of
  similar images, whatever `SIMILAR_PAGES_TO_DOWNLOAD` says (the script's file sets 20; the runs above used 5).
- **The app's first version failed on it.** Its client reported the page-1 captcha as an error of the whole
  search, so the other sizes and page 0 were dropped and the panel said "captcha". It now does what the script
  does: a failure on a page after the first ends the list.
- **The other sizes' stated sizes are not the files'** (measured 2026-11-26, one photo, 18 other sizes, each
  downloaded with the app's client and its size read from the file). 15 matched. One listed as 4860×6480 was an
  810×1080 WebP of 28 KB (exactly a sixth), served with no redirect. One listed as 1360×1642 was a 240×240 JPEG
  (an `i0.wp.com` proxy of imagevenue: likely a placeholder). One answered 403. Sorted by the stated sizes, the
  4860×6480 one came first. So the app reads no size from Yandex: a result's size is its file's, once downloaded,
  and the results stay in Yandex's order.
- **Not measured:** whether the browser shows all 18 other sizes or about 5 (the user's "5 or so shown").
- The script's known failures (`files_not_working.md`): upload 400s, SSL EOF and remote disconnects.
  `tasks.md`: CDN 403s that load in a browser, and no captcha validation for HTML responses.

### Decision and the port (2026-10-31)

- **The `yp` szm cookie is sent** (user decision): it is a cookie the site's own JavaScript sets, and no
  challenge is passed. The Sites list is not fetched (its pages 1 and up need `spravka`). The port does not copy
  `spravka` from a passed-challenge session, since that would get around the bot check. It has no `spravka`
  handling of its own, though: it keeps every cookie Yandex sets (`crates/yandex/src/client.rs:28-29`), so a
  `spravka` that a response sets would be sent back. Whether Yandex sets it for a client that has not passed a
  captcha is not established.
- **The port** (`pg_yandex`, the panel in `crates/desktop`, usage in PLAN §8 M2): plain HTTP to `yandex.ru`
  (`crates/yandex/src/client.rs:11-15`). Every measurement above is on `yandex.com`, so the cookie and captcha
  behaviour on `.ru` is not measured. Group 1 comes from the results page's `cbirOtherSizes`, group 2 from the
  similar JSON API with `yp`, up to 5 pages. The `yp` value is fixed at `1920x1080:1920x937`
  (`client.rs:113`), not the browser's real screen metrics. Like the script, a failure on a page after the first
  (in practice the page-1 captcha) ends the list and keeps what came before. A failure on page 0 of the similar images shows the other sizes with
  a note; one on the upload or the other sizes is the search's error. A captcha is reported, never retried.

## Animated results

An animated result (GIF, APNG or animated WebP; `pg_render::player::animated`, the catalog's own rule: more than one frame)
plays in the full-size preview and in its tile, over the still, from the search's kept download (`Fetched.file`, never
waited for: `Images::kept`). The file is played as Yandex gave it, turned by its own EXIF orientation (`Orientation::parts`:
the mirror, then the quarter turns; a still is turned the same way in the preview, in QML). A tile plays only while it is on
screen, the list is at rest and the preview is closed, at its own height (the player scales each frame down to it; a
frame is never enlarged). The library's grid plays its animated files the same way (`GridCell`; videos keep their still and
badge, and play in the viewer). Stopping a player never waits for a decode: a drop closes the sink at once, and no frame
reaches it after that.
