# UI and QML flow review

This report lists the findings of a flow-based review of the desktop UI: the QML front end, the Rust bridges and the controller it drives. The review started from each user action and each path that ends in a write, a notice or a focus change, and traced it through the code it reaches. It was run as a read-only workflow that produced 281 findings across 74 flows.

**Status: reduced to the open findings.** This file keeps only the findings that are not finished: UI-147 (confirmed, partly fixed) and UI-104 (partial). The other 279 of the 281 findings were fixed, already fixed, not real, or duplicates, and have been removed. Their text and status history are in git: the last version with all 90 open findings is commit `a5c9b239`, `docs/plans/UI_FLOW_REVIEW.md`. Each finding was checked against the live code at the cited sites, and against the Qt source under `C:\Qt\6.12.1\Src` where a finding depends on Qt behaviour.

## Method

1. A mapping step listed the user flows and the cited sites on each.
2. Each flow was traced from its entry point through the code it reaches, and each finding records a trigger, a consequence and the sites it cites.
3. Each finding was then validated: the cited lines were read in context (about four lines either side), and the claim was traced into the callees and into Qt where needed. Where a finding cites lines past the end of the file, its mechanism was checked where the code could be found elsewhere.

## Coverage

| Measure                       | Value |
|-------------------------------|------:|
| Flows traced                  |    74 |
| Findings remaining            |     2 |
| Flows with remaining findings |     2 |
| Sites recorded as not flows   |    18 |

Sites recorded as not flows were not traced; the mapping output gives a reason for each one.

## Summary

These counts cover the 2 remaining findings.

| Severity | Count |
|----------|------:|
| low      |     2 |

| Category       | Count |
|----------------|------:|
| stale-result   |     2 |

| Verdict   | Count |
|-----------|------:|
| CONFIRMED |     1 |
| PARTIAL   |     1 |
| REFUTED   |     0 |

Duplicates: none remain.

| Fix status (per finding's Status line) | Count |
|----------------------------------------|------:|
| Partial                                |     2 |

Of the 281 original findings, 279 have been removed: 183 in the first clean-up (fixed, already fixed or not real), 8 more (4 duplicates, and 4 not real or already fixed: UI-049, UI-136, UI-249, UI-254), and 88 since marked done or already fixed.

## Flows

| ID  | Name                                                      | Findings |
|-----|-----------------------------------------------------------|---------:|
| U28 | Remove a folder (with confirmation)                       |        1 |
| U39 | Turn a photo in the viewer                                |        1 |

## U28: Remove a folder (with confirmation)

Entry: `crates/desktop/qml/LibraryPanel.qml:71 onClicked removeRequested -> Main.qml:700 askRemoveFolder (Main.qml:214-218)`

### UI-104 [low/stale-result] Grid keeps listing the removed folder's images until the removal job ends — PARTIAL

- Status: Partial (build and tests). The hide stays under the park protocol, so the lanes are parked before the files are hidden. Residual: the hide waits for the removals thread, so a folder's photos can stay until another folder's delete pass finishes.
- Needs: Fix (checked: confirmed; the stale window ends at hide_pages, not at the final publish). Hide the folder's rows when the removal starts, or skip the reload at Begun::Started. Files: crates/desktop/src/library.rs (290-292), crates/desktop/qml/Main.qml (155), crates/library/src/coordinator.rs (959, 1012).
- Verdict: PARTIAL
- Note: The removal's final publish is Scanned (coordinator.rs:847), so the grid refreshes only then. Whether the Begun::Started reload still lists the removed folder's files depends on the listing query, which I did not trace.
- Confidence: medium
- Trigger: The user confirms the removal of a folder with many files. The removal runs as a background job (hide, delete in batches) that can take a long time.
- Consequence: The reload at Begun::Started re-lists the grid while the removed folder's files are still visible. Hiding and deletion then happen with no grid refresh until the job's final Scanned. The user sees images the dialog said were being deleted, and the tree no longer shows their folder.
- Sites: `crates/desktop/src/library.rs:276`; `crates/desktop/src/library.rs:277`; `crates/desktop/qml/Main.qml:129`; `crates/library/src/coordinator.rs:847`

## U39: Turn a photo in the viewer

Entry: `crates/desktop/qml/Viewer.qml:277-278 keys TurnForward and TurnBack -> turn(+1/-1) (Viewer.qml:190); RatingBar.qml:66 and :91 turnAsked -> Hud.qml:42 -> Viewer.qml:508`

### UI-147 [low/stale-result] A turn can be counted twice if a refresh read lands between the catalog write and turn_queued — CONFIRMED

- Status: Partial (build and tests). Each press has a sequence and the catalog's file stamp; a read counts a press exactly when its stamp is not older than the press. Residual: a read applied between a press's commit and its turn_done counts it twice until turn_done corrects it.
- Needs: Fix (checked: confirmed; the window is narrow). Key pending turns by revision, or stop counting in turn_queued what the refresh read already includes. Files: crates/desktop/src/gallery/edits.rs, crates/client/src/grid.rs.
- Verdict: CONFIRMED
- Note: edits.rs:44 adds the turn to pending_turns from the model thread, and turned_request reads that state. refresh_turned applies only when the revision is unchanged (grid.rs:405-408). The interleaving with turn_queued and whether edit() bumps the revision were not fully traced. Low severity, as the finding says.
- Confidence: low
- Trigger: Press R while a FilesChanged refresh is reading the same pending file. The read is taken after the turn is committed to the catalog, and its result is applied before turn_queued runs on the model thread.
- Consequence: The entry's pending_turns counts the new turn twice, so the HUD and the image show an extra quarter turn. It self-corrects when the writer writes the queued turns and FilesChanged re-reads the file. The window is very narrow unless the worker thread is preempted between the commit and the queue call.
- Sites: `crates/desktop/src/gallery/edits.rs:30`; `crates/desktop/src/gallery/edits.rs:44`; `crates/client/src/grid.rs:405`
