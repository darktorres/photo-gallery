"""Tests for tools/check_feature_coverage.py: each rule that makes a feature linted, on synthetic cargo metadata."""

import unittest
from unittest import mock

import check_feature_coverage as cov

WORKSPACE_CLIPPY = ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]


def package(name, features, deps=()):
    """A workspace package: `deps` are (name, kind, optional, features) tuples, as cargo metadata lists them."""
    return {
        "name": name,
        "features": features,
        "dependencies": [{"name": n, "kind": kind, "optional": optional, "features": feats} for n, kind, optional, feats in deps],
    }


def meta(*packages):
    return {"packages": list(packages)}


# A crate with a default feature `x`, a plain feature `y`, and `z`, which enables an optional dependency.
A = package("a", {"default": ["x"], "x": [], "y": [], "z": ["dep:foo"]})


class DefaultsAndSteps(unittest.TestCase):
    def test_the_workspace_step_compiles_the_default_features(self):
        uncovered, stale = cov.check(meta(A), [WORKSPACE_CLIPPY])
        self.assertEqual(uncovered, [("a", "y"), ("a", "z")])
        self.assertEqual(stale, [])

    def test_a_package_step_compiles_the_features_it_names(self):
        uncovered, _ = cov.check(meta(A), [WORKSPACE_CLIPPY, ["cargo", "clippy", "-p", "a", "--features", "y,z"]])
        self.assertEqual(uncovered, [])

    def test_no_default_features_leaves_the_default_out(self):
        steps = [["cargo", "check", "-p", "a", "--no-default-features"]]
        uncovered, _ = cov.check(meta(A), steps)
        self.assertEqual(uncovered, [("a", "x"), ("a", "y"), ("a", "z")])

    def test_all_features_compiles_every_feature_of_the_named_package(self):
        uncovered, _ = cov.check(meta(A), [["cargo", "build", "-p", "a", "--all-features"]])
        self.assertEqual(uncovered, [])

    def test_a_feature_enabled_by_another_feature_is_compiled_with_it(self):
        b = package("b", {"all": ["y", "z"], "y": [], "z": []})
        uncovered, _ = cov.check(meta(b), [["cargo", "check", "-p", "b", "--features", "all"]])
        self.assertEqual(uncovered, [])

    def test_doc_does_not_compile_code(self):
        uncovered, _ = cov.check(meta(A), [["cargo", "doc", "--workspace", "--no-deps"], WORKSPACE_CLIPPY])
        self.assertEqual(uncovered, [("a", "y"), ("a", "z")])


class Dependencies(unittest.TestCase):
    def test_a_dev_dependency_turns_its_feature_on_for_the_tests(self):
        b = package("b", {}, deps=[("a", "dev", False, ["y"])])
        uncovered, _ = cov.check(meta(A, b), [])
        self.assertNotIn(("a", "y"), uncovered)

    def test_an_optional_dependency_that_defaults_do_not_turn_on_gives_nothing(self):
        b = package("b", {"default": []}, deps=[("a", None, True, ["z"])])
        uncovered, _ = cov.check(meta(A, b), [WORKSPACE_CLIPPY])
        self.assertIn(("a", "z"), uncovered)

    def test_an_optional_dependency_that_defaults_turn_on_gives_its_features(self):
        b = package("b", {"default": ["dep:a"]}, deps=[("a", None, True, ["z"])])
        uncovered, _ = cov.check(meta(A, b), [])
        self.assertNotIn(("a", "z"), uncovered)

    def test_a_dependency_outside_the_workspace_is_ignored(self):
        b = package("b", {}, deps=[("serde", None, False, ["derive"])])
        uncovered, _ = cov.check(meta(A, b), [WORKSPACE_CLIPPY])
        self.assertEqual(uncovered, [("a", "y"), ("a", "z")])


class Allowed(unittest.TestCase):
    def test_an_allowed_feature_is_not_reported(self):
        with mock.patch.dict(cov.ALLOWED, {("a", "z"): "needs a device"}):
            uncovered, stale = cov.check(meta(A), [WORKSPACE_CLIPPY])
        self.assertEqual(uncovered, [("a", "y")])
        self.assertEqual(stale, [])

    def test_an_allowed_entry_that_names_no_feature_is_stale(self):
        with mock.patch.dict(cov.ALLOWED, {("a", "gone"): "removed", ("nope", "x"): "removed"}):
            _, stale = cov.check(meta(A), [WORKSPACE_CLIPPY])
        self.assertEqual(stale, [("a", "gone"), ("nope", "x")])


class Parsing(unittest.TestCase):
    def test_a_flag_takes_comma_or_space_separated_values(self):
        step = ["cargo", "clippy", "--features", "a,b", "--features", "c d"]
        self.assertEqual(cov.values_after(step, ("--features", "-F")), ["a", "b", "c", "d"])

    def test_closure_does_not_follow_dependency_entries(self):
        features = {"f": ["dep:x", "x/feat", "g"], "g": []}
        self.assertEqual(cov.closure(features, {"f"}), {"f", "g"})


class RealWorkspace(unittest.TestCase):
    def test_every_feature_of_the_workspace_is_linted_or_allowed(self):
        """The check on the repository itself: a new feature fails `just lint` until a step compiles it."""
        uncovered, stale = cov.check(cov.workspace_metadata(), _lint_steps())
        self.assertEqual(uncovered, [], "features with no lint step")
        self.assertEqual(stale, [], "ALLOWED names features that do not exist")


def _lint_steps():
    import lint

    return lint.STEPS


if __name__ == "__main__":
    unittest.main()
