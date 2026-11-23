# Plan: AI upscaling, stacked with its source

Status: **design, not built.** Nothing here is measured yet except the licence facts and the size arithmetic, which are
marked. The choices the user still has to make are in §11. The stacking rules come from [MERGE_PLAN.md](MERGE_PLAN.md) §8.

## 1. Summary

The user picks one picture, or a merged picture, and chooses **Upscale** with a factor (2× by default). The app runs a
learned super-resolution model on it and writes one larger PNG beside the source, in the library, then rescans that
folder, as the merge does. The two files are stacked (kind `upscale`): the upscaled file is the root that the gallery
shows, and the source is a hidden member. The source is never changed.

The model adds detail it has to invent. The stack keeps the true file one click away, and the plan keeps the upscaled
file visibly separate from it.

## 2. Scope

In:

- One picture, or the merged picture from MERGE_PLAN, at 2× (and 4× where the cap allows, §6).
- Output: 8-bit RGB PNG, lossless, like the merge output. Alpha is flattened onto white.
- Stacking with the source (§7), the action and its dialog, notices and text.

Out:

- Video, batches of thousands, face restoration, colour or denoise passes as separate options.
- Overwriting the source or any earlier output.
- Running Upscayl itself, or any code from it (§3).

## 3. Licence, verified 2026-11-22

Read from the GitHub API (`gh api`), not from the repo text:

| Project | Licence | What it means for us |
| --- | --- | --- |
| `upscayl/upscayl` (the app) | AGPL-3.0 | Copying its code, or shipping its binary, brings AGPL terms to the app. Not used. |
| `upscayl/upscayl-ncnn` (its backend) | AGPL-3.0 | Same. Not used. |
| `xinntao/Real-ESRGAN` (the architecture and weights' source) | BSD-3-Clause | The licence of the copy in the Upscayl repo (`Real-ESRGAN_LICENSE.txt`) is also BSD 3-Clause. Permissive; used. |

Decision (default): use the **Real-ESRGAN architecture and model weights** under their BSD-3 terms, run through ONNX
Runtime (MIT), and take nothing from Upscayl's code. Its UI is not copied either. The weights are published with the
Real-ESRGAN releases; their licence line must be read on the release page before the first build, because the repo's
LICENSE covers the code. Not verified here.

The workspace policy is `deny.toml`, whose allow list holds the mainstream permissive licences
([ALLOWLIST_PLAN.md](ALLOWLIST_PLAN.md) §"licenses"). An AGPL crate would need a separate decision there, and it does
not get one by default.

## 4. Runtime

- **ONNX Runtime through the `ort` crate.** This is the route the repo's eval notes already name for the Rust side
  ([../eval/MODELS.md](../eval/MODELS.md), "ONNX Runtime and TensorRT"). `ort` and ONNX Runtime are both permissively
  licensed.
- **Execution providers:** DirectML on Windows (any GPU vendor), CUDA where an NVIDIA GPU is present, and a CPU fallback
  that works but is slow. The CPU path must say so in the dialog before it starts.
- **Not Vulkan/ncnn.** Upscayl's route (ncnn over Vulkan) would add a second native stack, and its code is AGPL. The
  ONNX route also fits the open runtime decision for the second model ([OPEN_ITEMS.md](OPEN_ITEMS.md), "So400m"). Decide
  the runtime once for both; this plan should not pick it alone.
- **Model conversion** is a one-off step in `eval/`, like the SSCD export (`eval/accel.py`): the Real-ESRGAN PyTorch
  weights are exported to ONNX and the file is checked against the PyTorch output.
- **Speed and memory are not measured.** The repo has measured SSCD on the owner's RTX 2060 (6 GB, [../eval/MODELS.md](../eval/MODELS.md)),
  not an upscaler. §10 starts with that measurement.

## 5. Pipeline

1. **Source.** A single file is decoded upright at full size (`Decoder::decode_into`, `MinSize::Full`), as in
   MERGE_PLAN §4. A merged file is decoded the same way.
2. **Tiling.** The model runs on overlapping tiles, so memory is bounded by the tile, not the picture. Each tile's
   overlap is cropped or blended away before the next tile is written. The tile size and overlap are set by the spike
   (§10), not guessed.
3. **Output.** Each finished row band is streamed to the PNG (`Writer::stream_writer`, as MERGE_PLAN §4 step 7). The
   output is never held whole in RAM.
4. **Write and rescan.** The job writes `<stem>_x2.png` (or `_x4`) beside the source, numbered if the name is taken, then
   calls the same `LibraryService::scan` on the root that the merge calls. Metadata: copy the tags, then set orientation
   to 1 (as MERGE_PLAN §7), and **check the image-size tags** (`ExifImageWidth`, `ImageWidth` and similar): copied
   unchanged they would describe the source, not the output. Predicted, to verify on a scratch PNG.
5. **Stop.** A stop removes the partial file (the crop's `TempFile` pattern), as the merge does.

## 6. Size limits (arithmetic, verified in the code)

The decoder cap is 512 MiB of decoded pixels (`crates/codec/src/codecs/limits.rs:8`, "less the decoded size"). The
output must decode in the viewer and the thumbnails, so the output is held to the same cap. At three bytes a pixel:

- The cap is about **179 million pixels** (536,870,912 / 3).
- A 24 MP photo (6000 × 4000): **2×** gives 96 MP, 275 MiB, allowed. **3×** gives 216 MP, over. **4×** gives 384 MP, over.
- **4×** is allowed only for sources up to about **11 MP**.
- A merge of four 24 MP pieces (96 MP) is over the cap at 2×. A canvas up to about **45 MP** can be doubled.
- Two 24 MP pieces (48 MP) at 2× give 192 MP, also over the cap.

So the default is 2×, 4× only for small sources, and a refusal that names the size for anything bigger. Raising the cap
is a separate decision with memory and UI costs (§11).

## 7. Stacking

These follow [MERGE_PLAN.md](MERGE_PLAN.md) §8. Only the upscale rules are new.

- **Kind.** `upscale` is a new text key in `stacks.kind`, so no schema change beyond the migration already planned.
- **Root.** The upscaled file is the root and is shown. The source is a hidden member. The source's own row is unchanged.
- **Upscaling a merged picture.** The merged file is already the root of a `merge` stack. The upscaled file joins that
  stack and becomes its root, and the merged file becomes a member. This is a **re-root** of an existing stack, which
  §8 of the merge plan does not yet describe. Nested stacks are not allowed (§8 says stacks don't nest), so this is the
  only option that keeps the rule. Recorded as a choice in §11.
- **Ratings and marks** belong to each file. Write-back goes to the root only (MERGE_PLAN §8). So a rating given to the
  upscaled picture is written to the upscaled file, not to the source. Recorded as a choice in §11.
- **Unstack** returns the source (and the merged file, if any) to the library, as in §8 of the merge plan.
- **Duplicates.** An upscaled picture is a near-duplicate of its source, so the duplicate pass will probably pair them.
  This is the same risk as MERGE_PLAN §12, and it must be measured on the owner's own files before the feature ships.

## 8. Merge and upscale together

Recommended order (default): **lay out the pieces on the originals, compose the canvas as MERGE_PLAN §4 does, then
upscale the canvas once.** The reason is the seams. Upscaling each piece on its own treats every piece's edge as a
picture border, so the model may leave a ring or a step at the piece boundary. Upscaling the composed canvas keeps those
boundaries inside the picture. This is a prediction; the spike (§10) must check it on a merged test image.

The layout check is unchanged. It runs on the originals and never on upscaled pixels, because upscaling changes the
pixel statistics that the seam score in MERGE_PLAN §4 uses.

## 9. Gallery and dialog

- **Action.** "Upscale" is enabled for one picture, or for a merged picture. It needs a key that `keys.rs` does not use
  (check when building; MERGE_PLAN §6 lists the taken ones) and a menu entry.
- **Dialog** (`UpscaleDialog.qml`, modelled on `MergeDialog.qml`): the source's size, the factor (2× or 4×, with 4× greyed
  out where the cap refuses it), the output name and folder, the execution provider in use, and the estimated size of
  the output. **Upscale** is disabled while the size is refused, and the refusal names the cap.
- **Notices.** Three kinds, as the merge has: upscaled, refused (with the reason), failed (with the reason). The words go
  in `pg-client`'s `text` module.

## 10. Build order

0. **Spike, in `eval/`, outside the catalog.** Export Real-ESRGAN (x2 and x4) to ONNX. On five of the owner's photos and
   one generated test canvas, record: time per picture on the RTX 2060 (DirectML and CUDA), time on the CPU, peak GPU
   and RAM, tile size and overlap that give no visible tile seams, and the seam check on the canvas of §8. Compare the
   output with Upscayl's output for the same photo, run by the owner locally and not committed. Stop here if the quality
   or speed is not good enough; nothing else is built until this passes.
1. **Runtime decision.** Record the `ort` choice and the execution providers in [OPEN_ITEMS.md](OPEN_ITEMS.md) (the
   So400m entry) and in this plan.
2. **`pg-upscale` (pure).** Tiling, overlap handling, the size check of §6, and the factor. The model is behind a trait,
   so the tiling is tested with a fake model that has a known result (a nearest-neighbour upscale, which must come out
   exact). A real-model test is local only, like the merge's pair test.
3. **`pg-library` job and `pg-app` method.** Written next to the merge job, with progress, pause, stop and the rescan.
4. **Stacking.** The `upscale` kind and the re-root operation, with the shared hiding predicate of the merge plan.
   Tests: the facet total, tree count and grid length agree with the stack present.
5. **Dialog, notices, text, and the action.**
6. **Gates.** As MERGE_PLAN §11 step 8: `just lint`, `just test`, the Linux check, `just build` and `just shots` where the
   desktop changed. Validation is batched until the change list is complete.

## 11. Choices still for you

Each has a default, used if you don't choose.

1. **Model source.** Real-ESRGAN weights under BSD-3 through `ort` (default). Calling Upscayl's own binary as a separate
   program is the other route; it does not copy its code, but shipping that binary would bring AGPL duties for it, so it
   is not the default.
2. **Factor and cap.** 2× by default, 4× only where the output fits the 512 MiB cap (about 11 MP sources) (default). Raising
   the cap would allow more, at a memory cost the spike has to measure.
3. **Merged pictures.** Upscale the composed canvas once (default). Per-piece upscaling is simpler to code but risks the
   piece seams (§8).
4. **Stack root.** The upscaled file is the root, and the source is hidden (default). The other way is to keep the
   source as root and hide the upscaled file.
5. **Ratings.** Write-back to the root only, as the merge plan says (default). The other way is to write the rating to
   the source as well.
6. **Stacking a merged picture.** Re-root the merge stack (default). The other way is two separate stacks, which shows
   two pictures.
7. **Execution provider.** DirectML first, CUDA when present, CPU fallback with a warning (default).
8. **Models.** General photos only (default). An anime model can be added later as a separate choice.

## 12. Risks, and what is still open

Risks:

- **Invented detail.** A model adds texture that was not in the photo. On faces and text the result can look right and
  be wrong. The stack keeps the source, and the dialog should say the output is generated.
- **Speed on the CPU.** Likely too slow for a library of any size. The dialog must make the CPU path clear, and the
  feature may be GPU-only in practice. Not measured.
- **Output size.** A 2× PNG of a 24 MP photo is large on disk. Not measured; the spike records it.
- **Cap refusals.** Most of the owner's large photos can take 2× only, and 4× only when small (§6).
- **Duplicates and ratings** (§7). Both need checks on the owner's own files.
- **Licence drift.** Copying a line from Upscayl's code would make the app AGPL. The rule is: no code from Upscayl or
  upscayl-ncnn, only the BSD-3 model and its architecture.

Not verified:

- The licence line on the Real-ESRGAN weights release (the code licence is verified, §3).
- The exact model files for x2 and x4, and that the x2 weights exist in the same release. Check when the spike starts.
- Speed, peak memory, output size and tile seams on any hardware.
- The image-size tags after a copy (§5).
- That DirectML and CUDA providers both work through the `ort` version the repo would pin.
