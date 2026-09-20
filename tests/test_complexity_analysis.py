# @feature complexity-analysis
# @spec docs/features/complexity-analysis.md
"""Behavior tests for the feature-aware BCA wrapper."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from types import ModuleType


def load_complexity_analysis() -> ModuleType:
    scripts = Path(__file__).parents[1] / "scripts"
    if str(scripts) not in sys.path:
        sys.path.insert(0, str(scripts))
    path = scripts / "complexity_analysis.py"
    spec = importlib.util.spec_from_file_location("complexity_analysis", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load complexity-analysis module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ComplexityAnalysisTests(unittest.TestCase):
    def test_ranking_score_uses_explicit_weights(self) -> None:
        analysis = load_complexity_analysis()
        weights = analysis.RankingWeights(1.0, 3.0, 8.0, 5.0, 2.0)
        score = analysis.ranking_score(weights, 10, 2, 3, 4.0, 5.0)
        self.assertEqual(score, 65.0)

    def test_normalized_tokens_surface_same_shape(self) -> None:
        analysis = load_complexity_analysis()
        left = analysis.normalized_tokens("fn alpha(x: i32) { let y = x + 1; y }")
        right = analysis.normalized_tokens(
            "fn beta(item: i32) { let result = item + 9; result }"
        )
        similarity = analysis.difflib.SequenceMatcher(
            None, left, right, autojunk=False
        ).ratio()
        self.assertGreater(similarity, 0.95)

    def test_feature_scope_selects_only_owned_source(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs" / "features").mkdir(parents=True)
            (root / "docs" / "features" / "demo.md").write_text(
                "---\nfeature: demo\n---\n# Demo\n", encoding="utf-8"
            )
            (root / "demo.rs").write_text(
                "// @feature demo\n"
                "// @spec docs/features/demo.md\n"
                "// @entrypoint run\n"
                "fn run() {}\n",
                encoding="utf-8",
            )
            (root / "other.rs").write_text(
                "// @feature other\nfn other() {}\n", encoding="utf-8"
            )
            selected = analysis.select_paths(root, "demo", None)
            self.assertEqual(selected, [root / "demo.rs"])

    def test_unit_preserves_bca_metrics_and_adds_declarations(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "sample.rs"
            source.write_text("fn work(value: i32) {\n    let next = value + 1;\n}\n")
            raw_metrics = {
                "loc": {"lloc": 2},
                "nargs": {"total": 1},
                "cyclomatic": {"value": 1},
                "cognitive": {"value": 0},
            }
            document = {
                "name": "sample.rs",
                "start_line": 1,
                "end_line": 3,
                "kind": "function",
                "spaces": [],
                "metrics": raw_metrics,
            }
            weights = analysis.RankingWeights(1.0, 3.0, 8.0, 5.0, 2.0)
            units = analysis.collect_units(root, [document], weights)
            self.assertEqual(units[0].declarations, 1)
            self.assertEqual(units[0].metrics, raw_metrics)
            self.assertEqual(units[0].architectural_load, 13.0)


if __name__ == "__main__":
    unittest.main()
