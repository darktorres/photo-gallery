# Findings (index)

The measurements log was split by topic. Entries moved unchanged; only some headings changed level, and the
text above each topic file now says where its results were measured. This file lists where each former section
now lives. Code and docs quote sections by their old title, for example `IO.md "I/O"`, so search for the
title here to find it.

| Former section | Now in |
|---|---|
| Environment; Test set; Labels; Eval viewer: port collision (was under "Bugs found") | [TEST_SET.md](TEST_SET.md) |
| I/O; Read buffers | [IO.md](IO.md) |
| Decode benchmark; The rule that matters: index and query images must go through the same decode | [JPEG_DECODERS.md](JPEG_DECODERS.md) |
| Thumbnails | [THUMBNAILS.md](THUMBNAILS.md) |
| Model speed; TensorRT and ONNX Runtime; A second model (SigLIP 2 / CLIP, NaFlex, fp16 precision) | [MODELS.md](MODELS.md) |
| Bug: SSCD breaks in fp16 (was under "Bugs found") | [MODELS.md](MODELS.md) |
| Nearest-neighbour search, first attempt: invalid | [TIER2_ANN_BAKEOFF_PLAN.md](TIER2_ANN_BAKEOFF_PLAN.md) |
| Quality: true negatives; PDQ; Quality: model comparison; Quality: SSCD fp16 detail; Shortcut and grouping; Synthetic edits, all models; Synthetic edits, SSCD fp16 | [QUALITY.md](QUALITY.md) |
| Bug: R@k metric undercounted groups of 3+ (was under "Bugs found") | [QUALITY.md](QUALITY.md) |
| Verifier sections (prototype, match vs unrelated, same-shoot frames, synthetic edits, candidate pairs, speed, TensorRT and rotations) | [VERIFIER.md](VERIFIER.md) |
| Verifier after the resampling fix (was under "Pending") | [VERIFIER.md](VERIFIER.md) |
| Border cleaning; Trimming before embedding; Border detector in the gallery | [BORDERS.md](BORDERS.md) |
| M1 import | [IMPORT.md](IMPORT.md) |
| Videos and animations; Video player options (web research) | [VIDEO_PLAYER.md](VIDEO_PLAYER.md) |
| Reverse image search: Yandex | [REVERSE_SEARCH.md](REVERSE_SEARCH.md) |
| Startup | [START_PROFILE.md](START_PROFILE.md) |
| Decisions | [DECISIONS.md](DECISIONS.md) |
| Design conclusions so far | [PLAN.md](PLAN.md) |
| Phase 0 baseline; Refactor S-02.3 to S-16.3 | [REFACTOR_MEASUREMENTS.md](REFACTOR_MEASUREMENTS.md) |
| Pending | [OPEN_ITEMS.md](OPEN_ITEMS.md) |
