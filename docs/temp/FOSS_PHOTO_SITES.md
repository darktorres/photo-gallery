# FOSS photo libraries: homepages and GitHub Pages

Task: inspect the homepage or GitHub Pages site of each project in the comparison list, to improve our own site
(`site/index.html`, published at <https://darktorres.github.io/photo-gallery/>). Inspection is done with Playwright.

Status key: **inspected** = homepage read through Playwright; **README only** = no separate homepage found, so the
GitHub README is the public page; **unreachable** = the site did not load.

## Source list

- Comparison page: <https://meichthys.github.io/foss_photo_libraries/>
- Its source: <https://github.com/meichthys/foss_photo_libraries>

## Projects

| # | Project | Repository | Homepage / Pages | Status |
|---|---------|------------|------------------|--------|
| 1 | Chevereto | <https://github.com/chevereto/chevereto> | <https://chevereto.com/> | inspected |
| 2 | Damselfly | <https://github.com/Webreaper/Damselfly> | <https://damselfly.info/> (blog-style site) | inspected |
| 3 | Ente | <https://github.com/ente-io/ente> | <https://ente.com/> | inspected |
| 4 | Foldergram | <https://github.com/foldergram/foldergram> | <https://foldergram.github.io/> (GitHub Pages; demo: <https://foldergram.intentdeep.com/>) | inspected |
| 5 | HomeGallery | <https://github.com/xemle/home-gallery> | <https://home-gallery.org/> | inspected |
| 6 | Immich | <https://github.com/immich-app/immich> | <https://immich.app/> | inspected |
| 7 | LibrePhotos | <https://github.com/LibrePhotos/librephotos> | <https://docs.librephotos.com/> | inspected |
| 8 | Lychee | <https://github.com/LycheeOrg/Lychee> | <https://lycheeorg.dev/> | inspected |
| 9 | Nextcloud Photos | <https://github.com/nextcloud/photos> | none found; vendor blog post <https://nextcloud.com/blog/how-we-got-to-the-new-nextcloud-photos/> | README only |
| 10 | Nextcloud Memories | <https://github.com/pulsejet/memories> | <https://memories.gallery/> | inspected |
| 11 | Photonix | <https://github.com/photonixapp/photonix> | <https://photonix.org/> | inspected |
| 12 | Photofield | <https://github.com/SmilyOrg/photofield> | <https://photofield.dev/> | inspected |
| 13 | PiGallery2 | <https://github.com/bpatrik/pigallery2> | <https://bpatrik.github.io/pigallery2/> (GitHub Pages docs; demo: <https://pigallery2.onrender.com/>) | inspected |
| 14 | Photoprism | <https://github.com/photoprism/photoprism> | <https://www.photoprism.app/> | inspected |
| 15 | Photoview | <https://github.com/photoview/photoview> | <https://photoview.github.io/> (GitHub Pages) | inspected |
| 16 | Piwigo | <https://github.com/Piwigo/Piwigo> | <https://piwigo.org/> | inspected |
| 17 | Snapcrescent | <https://github.com/snapcrescent/snapcrescent> | <https://snapcrescent.app/> (DNS fails) | unreachable |

## Notes from the inspected homepages

Each entry is what the page shows at the top and how it is organized. Only what Playwright returned is recorded here.

- **Damselfly** (blog-style site): titled "Damselfly – Server based Digital Asset Management"; no meta description and
  no og:image; nav of Home, GitHub Repo, Docker, Reddit; a "Recent posts" list of release announcements (v2.1.0, v2.2.0
  multi-user support, v2.3.0 search, a Blazor migration write-up); feature, screenshot, workflow and planned-features
  sections taken from the README; a "Buy Me a Coffee" button; about 10 images.
- **Chevereto**: a hero headline about a private image and video sharing platform; "Get started free" and "Try Cloud"
  buttons; sections by audience (stores, photographers, creators, realtors, organizations, agencies); a release news
  section; a pricing link.
- **Ente**: a marketing page with a section per feature (face recognition, zero-knowledge AI, curated memories, family
  plans, widgets, public links, smart albums, guest view, hidden photos, legacy contacts); about 295 images; a
  "start with 10GB free" meta description.
- **HomeGallery**: a documentation-style site with no meta description and no og:image; nav of Why?, Docs, Demo,
  Download; six feature sections (files and folders, open source, AI discovery, tagging and face recognition,
  geo information, flexible search).
- **Immich**: needed a longer wait before the page rendered; hero "Self-hosted photo and video management solution";
  sections for the mobile app, sharing and facial recognition; a support section (product key, merch, contribute).
- **LibrePhotos** (docs site): hero "The photo library that stays on your machine."; a "Start here" section with
  Install, First steps and Contribute; a Windows download option; a "No company, no product key" statement.
- **Lychee**: hero "Self-hosted photo-management done right."; a Manage, Share, View feature row; buttons for
  Download, Live Demo, See on GitHub and Deploy now; a blog and news section; a Docker section.
- **Nextcloud Memories**: a feature list (privacy first, auto upload, timeline, albums, auto tagging, metadata editing,
  video transcoding, map, "fast and battle tested", "no lock-in"); Install and Demo buttons; docs-style nav (installation,
  configuration, file type support, hardware transcoding, troubleshooting).
- **Photonix**: hero "Photonix"; a "Main features" grid of eight items; a "Try live demo now" button; a Patreon link.
- **Photofield**: hero "Self-Hosted Personal Photo Gallery"; a list of short feature headings (zoomable interface,
  progressive loading, simple to run, MIT license, non-destructive, fast indexing, semantic search); no meta
  description and no og:image.
- **Photoprism**: hero "AI-powered, privacy-first app for browsing, organizing and sharing photos and videos";
  "Try our demo", "Learn more" and "Compare Team Plans" buttons; sections for teams and organizations.
- **Photoview** (GitHub Pages): feature headings (syncs with the file system, users and sharing, maps, mobile app,
  RAW support, EXIF, video, ARM, performance, security, privacy, free and open source); nav of Documentation and
  Find on GitHub; no og:image; one console error.
- **Piwigo**: hero "Manage, share and organize your photos and videos. Your way."; a choice between Piwigo Cloud and
  self-hosted; "no lock-in" and "data sovereignty" sections; seven numbered feature steps; a "who uses Piwigo"
  section; free trial and demo buttons; 69 images; no og:image; two console errors.

- **Foldergram** (GitHub Pages docs): hero "Foldergram: local-first photo and video gallery"; sections on local-first
  storage, fast feed-style browsing, photos and videos, and optional access control; an "at a glance" block (what ships
  today, how indexing works, formats and storage, access modes); a six-step "how Foldergram works" list; nav of Quick
  Start, Installation, Configuration, How It Works, API, Security, Demo; no og:image; no buttons.
- **PiGallery2** (GitHub Pages docs): titled "PiGallery2 Documentation"; a navigation list of Features, FAQ, Getting
  Started, User Guide, Development, Blog, References and Support; one image; no og:image. Its About section links the
  documentation, the live demo, and install guides for Docker (recommended) and native (unsupported).

## Notes from the README-only projects

No homepage was found, so these are the GitHub README sections. Nextcloud Photos is the only one left.

- **Nextcloud Photos**: "Why is this so awesome?", "Installation", "Mobile photos", "Maintainers", "Development setup".
  Searches found no product homepage. `nextcloud.com/photos/` redirects to an image file, and `apps.nextcloud.com/apps/photos`
  returns 404. The closest page is the vendor's blog post "How we got to the new Nextcloud Photos" (version 2.0: collaborative
  albums, face and object recognition, photo editing). It is an article, not a landing page, so it is recorded but not
  counted as a homepage.
- **Snapcrescent**: the site does not resolve, so only the repository description is known.

## Correction to the first pass

The first pass filtered out every link whose URL contained "github", which also hid `github.io` project sites. That is
how PiGallery2's and Foldergram's Pages sites were missed. The second pass used a filter that excludes only GitHub's own
domains. Damselfly's site, <https://damselfly.info/>, was not linked in its GitHub About section (the About section links only
social pages), so it was supplied by the user. Nextcloud Photos has no homepage in its About section and stays README-only.

## Our site, for comparison

- `site/index.html` and `site/style.css`: one static page, no build step.
- Hero with the app icon, a lede, a promise line ("Non-destructive by design"), and two buttons.
- Sections: "What it is made of" (three cards), "Build" (command block), "Documentation" (links), mascots.
- No screenshot yet (commented out in `site/index.html`).

## Screenshots

Playwright may only save files under `D:\GitHub\photo-gallery\.playwright-mcp\`, which is git-ignored.
