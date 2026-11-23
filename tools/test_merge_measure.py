"""Tests for tools/merge_measure.py: the layout rules on generated images (no Pillow, no personal photos)."""

from __future__ import annotations

import math
import unittest

import merge_measure as mm


def gradient(width: int, height: int) -> mm.Image:
    """A smooth colour field that does not repeat: neighbouring pixels are close, so a true seam scores near 1, and no
    two tiles look alike. (A periodic field makes tiles interchangeable, and then refusing is the right outcome.)"""
    data = bytearray()
    for y in range(height):
        for x in range(width):
            data += bytes((255 * x // width, 255 * y // height, 255 * x * y // (width * height)))
    return mm.Image(width, height, bytes(data))


def flat(width: int, height: int, value: int = 128) -> mm.Image:
    return mm.Image(width, height, bytes([value]) * (width * height * 3))


class LayoutRules(unittest.TestCase):
    def test_a_smooth_grid_is_found_exactly(self) -> None:
        for rows, cols in ((2, 2), (3, 2), (2, 4)):
            with self.subTest(rows=rows, cols=cols):
                tiles = mm.cut(gradient(cols * 40, rows * 30), rows, cols)
                self.assertEqual(mm.outcome(tiles, rows, cols, 1.3), "correct")

    def test_a_flat_image_is_refused_not_guessed(self) -> None:
        tiles = mm.cut(flat(80, 60), 2, 2)
        self.assertEqual(mm.outcome(tiles, 2, 2, 1.3), "refused")

    def test_a_missing_link_is_refused(self) -> None:
        # Three pieces, the link from the first to the second missing: the layout cannot reach every piece.
        pieces = [(0, 0), (0, 1), (1, 0)]
        right: set[tuple[mm.Key, mm.Key]] = set()
        down = {((0, 0), (1, 0))}
        self.assertIsNone(mm.assemble(pieces, right, down))

    def test_a_wrong_link_that_closes_a_rectangle_is_caught_by_adjacency(self) -> None:
        # The 2x2 case the plan measured: both vertical links are right, one horizontal link is wrong, and no true
        # horizontal link is accepted. The positions form a full rectangle, so only the adjacency rule refuses it.
        pieces = [(0, 0), (0, 1), (1, 0), (1, 1)]
        right = {((1, 1), (1, 0))}
        down = {((0, 0), (1, 0)), ((0, 1), (1, 1))}
        pos = mm.assemble(pieces, right, down)
        assert pos is not None, "the links must reach every piece"
        self.assertTrue(mm.is_rectangle_or_ragged(pos))
        self.assertFalse(mm.adjacency_ok(pos, right, down))

    def test_a_ragged_last_row_is_allowed(self) -> None:
        pos = {(0, 0): (0, 0), (0, 1): (1, 0), (1, 0): (0, 1)}  # the last row is one piece short
        self.assertTrue(mm.is_rectangle_or_ragged(pos))

    def test_a_gap_that_is_not_in_the_last_row_or_column_is_refused(self) -> None:
        cells = [(x, y) for y in range(3) for x in range(3) if (x, y) != (1, 1)]
        pos = {(i, 0): cell for i, cell in enumerate(cells)}
        self.assertFalse(mm.is_rectangle_or_ragged(pos))

    def test_the_seam_score_is_small_for_a_true_seam_and_zero_for_flat(self) -> None:
        image = gradient(40, 30)
        left, right = mm.cut(image, 1, 2)[(0, 0)], mm.cut(image, 1, 2)[(0, 1)]
        self.assertLess(mm.h_score(left, right), 2.0)
        self.assertEqual(mm.h_score(flat(20, 10), flat(20, 10)), 0.0)

    def test_heights_must_match_for_a_horizontal_seam(self) -> None:
        self.assertEqual(mm.h_score(gradient(20, 10), gradient(20, 12)), math.inf)


if __name__ == "__main__":
    unittest.main()
