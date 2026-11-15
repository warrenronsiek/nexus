# @feature complexity-analysis
# @spec docs/features/complexity-analysis.md
# @boundary rust-syntax-fixtures
"""Behavior tests for the feature-aware BCA wrapper."""

from __future__ import annotations

import importlib.util
import io
import json
import subprocess
from contextlib import redirect_stdout
import sys
import tempfile
import unittest
from pathlib import Path
from types import ModuleType
from unittest.mock import patch


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
    def test_rust_ranking_prefers_deep_immutable_typed_operations(self) -> None:
        analysis = load_complexity_analysis()
        policy = analysis.load_policy(Path(__file__).parents[1])
        cases = {
            "deep.rs": "fn work(x: i32) -> i32 {\n let a = x + 1;\n let b = a + 2;\n b * 3\n}\n",
            "wide.rs": "fn work(x: i32, y: i32, z: i32) -> i32 { x + y + z }\n",
            "split.rs": "fn work(x: i32) -> i32 { helper(x) }\nfn helper(x: i32) -> i32 { (x + 1 + 2) * 3 }\n",
            "mutable.rs": "fn work(mut x: i32) -> i32 { x += 1; x }\n",
            "pure.rs": "fn work(x: i32) -> i32 { x + 1 }\n",
            "record.rs": "struct Input { value: i32 }\nfn work(x: Input) -> i32 { x.value }\n",
            "dynamic.rs": "fn work(x: serde_json::Value) -> serde_json::Value { x }\n",
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            paths = []
            for name, source in cases.items():
                path = root / name
                path.write_text(source)
                paths.append(path)
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, paths), policy.ranking)
            scores = {unit.path: unit.architectural_load for unit in units if unit.kind == "unit"}
            self.assertLess(scores["deep.rs"], scores["wide.rs"])
            self.assertLess(scores["deep.rs"], scores["split.rs"])
            self.assertLess(scores["pure.rs"], scores["mutable.rs"])
            self.assertLess(scores["record.rs"], scores["dynamic.rs"])

    def test_rust_syntax_signals_ignore_literals_comments_and_record_fields(self) -> None:
        analysis = load_complexity_analysis()
        policy = analysis.load_policy(Path(__file__).parents[1])
        source = '''struct Input { first: i32, second: bool }
trait Work { fn run(&self, value: Input) -> i32; }
fn work(value: i32) -> i32 {
    // mut x = 1; RefCell serde_json::Value
    let text = r###"mut x = 1; Cell serde_json::Value"###;
    let next = value + 1;
    next
}
'''
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text(source)
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), policy.ranking)
            document = analysis.unit_document(units[0])
            self.assertEqual(document["signals"]["functions"], 2)
            self.assertEqual(document["signals"]["arguments"], 3)
            self.assertEqual(document["signals"]["mutation_sites"], 0)
            self.assertEqual(document["signals"]["state_types"], 0)
            self.assertEqual(document["signals"]["dynamic_types"], 0)

    def test_scope_score_counts_outermost_units_once(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text("fn work(x: i32) -> i32 { x + 1 }\n")
            policy = analysis.load_policy(Path(__file__).parents[1])
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), policy.ranking)
            total = analysis.scope_document(units)
            self.assertEqual(total["architectural_load"], units[0].architectural_load)
            self.assertEqual(len(total["units"]), 1)
            self.assertEqual(total["contributions"], analysis.unit_document(units[0])["contributions"])

    def test_state_and_branch_signals_survive_nested_function_spaces(self) -> None:
        analysis = load_complexity_analysis()
        policy = analysis.load_policy(Path(__file__).parents[1])
        source = "fn work(value: &std::sync::Mutex<i32>) -> i32 { (0..3).map(|x| if x > 1 { x } else { 0 }).sum() }\n"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text(source)
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), policy.ranking)
            signals = analysis.unit_document(units[0])["signals"]
            self.assertEqual(signals["state_types"], 1)
            self.assertEqual(signals["mutation_sites"], 0)
            self.assertEqual(signals["functions"], 1)
            self.assertEqual(signals["arguments"], 1)
            self.assertGreater(signals["cyclomatic"], 0)
            self.assertGreater(signals["cognitive"], 0)
            self.assertEqual(analysis.scope_document(units)["architectural_load"], units[0].architectural_load)

    def test_assignments_are_mutation_evidence_and_custom_costs_add_up(self) -> None:
        analysis = load_complexity_analysis()
        source = "fn work(mut value: i32) -> i32 { value = 2; value += 1; value }\n"
        weights = analysis.RankingWeights(2.0, 7.0, 11.0, 3.0, 4.0, 13.0, 17.0, 19.0)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text(source)
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), weights)
            document = analysis.unit_document(units[0])
            self.assertEqual(document["signals"]["mutation_sites"], 3)
            self.assertEqual(document["contributions"]["mutation_sites"], 39)
            self.assertEqual(document["contributions"]["functions"], 11)
            self.assertEqual(document["contributions"]["arguments"], 7)
            self.assertEqual(sum(document["contributions"].values()), document["architectural_load"])

    def test_inline_sibling_and_closure_signals_use_exact_ast_spans(self) -> None:
        analysis = load_complexity_analysis()
        weights = analysis.load_policy(Path(__file__).parents[1]).ranking
        source = "fn alpha(x: i32) -> i32 { x } fn beta(y: serde_json::Value) -> serde_json::Value { y }\nfn closures() { let first = |x: i32| x; let second = |mut y: i32| { y += 1; y }; }\n"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text(source)
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), weights)
            alpha = next(unit for unit in units if unit.name == "alpha")
            self.assertEqual(alpha.signals.functions, 1)
            self.assertEqual(alpha.signals.arguments, 1)
            self.assertEqual(alpha.signals.dynamic_types, 0)
            closures = [unit for unit in units if unit.name == "<anonymous>"]
            self.assertEqual([unit.signals.mutation_sites for unit in closures], [0, 2])
            self.assertEqual([unit.signals.arguments for unit in closures], [0, 0])
            policy_path = Path(__file__).parents[1] / "complexity.toml"
            (root / "complexity.toml").write_text(policy_path.read_text())
            output = io.StringIO()
            with redirect_stdout(output):
                status = analysis.main(["--root", str(root), "--path", "sample.rs", "--symbol", "alpha", "--format", "json"])
            report = json.loads(output.getvalue())
            self.assertEqual(status, 0)
            self.assertEqual(report["units"][0]["signals"]["dynamic_types"], 0)
            self.assertEqual(report["total"]["architectural_load"], alpha.architectural_load)

    def test_same_line_sibling_methods_both_contribute_to_symbol_total(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text("struct A; struct B; impl A { fn run(&self) {} } impl B { fn run(&self) {} }\n")
            weights = analysis.load_policy(Path(__file__).parents[1]).ranking
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), weights)
            methods = [unit for unit in units if unit.name == "run"]
            self.assertEqual(len(methods), 2)
            self.assertEqual(len(analysis.scope_document(methods)["units"]), 2)

    def test_rust_syntax_evidence_handles_inference_unicode_and_empty_modules(self) -> None:
        analysis = load_complexity_analysis()
        cases = {
            "constructors.rs": (
                "fn work() { let first = RefCell::new(0); let second = std::sync::Mutex::new(1); }\n",
                analysis.rust_structure.RustSignals(functions=1, state_types=2),
            ),
            "unicode.rs": (
                "struct Café { payload: serde_json::Value, state: std::sync::Mutex<i32> }\n",
                analysis.rust_structure.RustSignals(state_types=1, dynamic_types=1),
            ),
            "module.rs": ("pub mod child;\n", analysis.rust_structure.RustSignals()),
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            paths = [root / name for name in cases]
            for path in paths:
                path.write_text(cases[path.name][0])
            weights = analysis.load_policy(Path(__file__).parents[1]).ranking
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, paths), weights)
            for unit in (item for item in units if item.kind == "unit"):
                with self.subTest(path=unit.path):
                    expected = cases[unit.path][1]
                    self.assertEqual(
                        tuple(getattr(unit.signals, name) for name in expected.__dataclass_fields__),
                        tuple(vars(expected).values()),
                    )

    def test_missing_rust_ast_output_fails_visibly(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text("fn work() {}\n")
            documents = analysis.run_bca_metrics(root, [path])
            weights = analysis.load_policy(Path(__file__).parents[1]).ranking
            completed = subprocess.CompletedProcess(["bca"], 0, stdout="", stderr="")
            with patch.object(analysis.rust_structure.subprocess, "run", return_value=completed):
                with self.assertRaisesRegex(analysis.AnalysisError, "scope count mismatch"):
                    analysis.collect_units(root, documents, weights)

    def test_text_report_shows_the_signals_and_contributions_it_scores(self) -> None:
        analysis = load_complexity_analysis()
        weights = analysis.load_policy(Path(__file__).parents[1]).ranking
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = root / "sample.rs"
            path.write_text("fn work(mut x: i32) -> i32 { (0..3).map(|n| if n > 1 { x = n; x } else { 0 }).sum() }\n")
            units = analysis.collect_units(root, analysis.run_bca_metrics(root, [path]), weights)
            output = io.StringIO()
            with redirect_stdout(output):
                analysis.print_text("sample", units[:1], [], 1, weights)
            report = output.getvalue()
            self.assertIn(f"args={units[0].signals.arguments:g}", report)
            self.assertIn(f"branching={units[0].signals.cyclomatic:g}", report)
            self.assertIn(f"mutation_sites={units[0].contributions.mutation_sites:g}", report)

    def test_ranking_formula_reports_configured_weights(self) -> None:
        analysis = load_complexity_analysis()
        weights = analysis.RankingWeights(2.0, 7.0, 11.0, 3.0, 4.0, 13.0, 17.0, 19.0)
        formula = analysis.ranking_formula(weights)
        self.assertIn("2*LLOC", formula)
        self.assertIn("7*arguments", formula)
        self.assertIn("11*functions", formula)
        self.assertIn("13*mutation_sites", formula)

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
            root = Path(directory).resolve()
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

    def test_feature_scope_includes_authored_typescript(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "docs" / "features").mkdir(parents=True)
            (root / "docs" / "features" / "ui.md").write_text(
                "---\nfeature: ui\n---\n# UI\n", encoding="utf-8"
            )
            source = root / "ui" / "src" / "chart.ts"
            source.parent.mkdir(parents=True)
            source.write_text(
                "// @feature ui\n"
                "// @spec docs/features/ui.md\n"
                "export function render(): void {}\n",
                encoding="utf-8",
            )
            generated = root / "ui" / "dist" / "app.js"
            generated.parent.mkdir(parents=True)
            generated.write_text("function bundled() {}\n", encoding="utf-8")

            selected = analysis.select_paths(root, "ui", None)
            self.assertEqual(selected, [source])

    def test_unit_preserves_bca_metrics_and_adds_declarations(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
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
                "kind": "unit",
                "spaces": [{"name": "work", "kind": "function", "start_line": 1, "end_line": 3, "spaces": [], "metrics": raw_metrics}],
                "metrics": raw_metrics,
            }
            weights = analysis.RankingWeights(1.0, 3.0, 8.0, 5.0, 2.0)
            units = analysis.collect_units(root, [document], weights)
            self.assertEqual(units[0].declarations, 1)
            self.assertEqual(units[0].metrics, raw_metrics)
            self.assertEqual(units[0].architectural_load, 13.0)

    def test_typescript_declarations_are_counted_explicitly(self) -> None:
        analysis = load_complexity_analysis()
        source = Path("chart.ts")
        lines = [
            "export interface Bucket { count: number }",
            "export type Renderer = (bucket: Bucket) => void;",
            "const colors = {};",
            "export function render(bucket: Bucket): void {}",
        ]

        self.assertEqual(analysis.declaration_count(source, lines, 1, 4), 4)

    def test_enforcement_preserves_every_full_report_violation(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            sources = ["fn alpha() { let x = 1; let y = 2; x + y }", "fn beta() { let p = 3; let q = 4; p + q }", "fn other() { if true { loop { break; } } }", "", ""]
            units = []
            for index, source in enumerate(sources):
                name = f"unit{index}.rs"
                (root / name).write_text(source)
                units.append(analysis.UnitMetric(name, name, "function", 1, 1, 0, 0.0, {}))
            for minimum in (0, 4):
                policy = analysis.RepetitionPolicy(minimum, 0.5, 0.96)
                expected = [candidate for candidate in analysis.find_repetition(root, units, policy) if candidate.breaches(policy.fail_similarity)]
                actual = analysis.find_repetition(root, units, policy, analysis.AnalysisPurpose.ENFORCE)
                self.assertTrue(expected)
                self.assertEqual(actual, expected)

    def test_enforcement_avoids_sequence_work_for_provably_nonmatching_pairs(self) -> None:
        analysis = load_complexity_analysis()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            sources = ["fn long() { " + "let x = 1; " * 60 + "}", "fn short() { return 0; }"]
            units = []
            for index, source in enumerate(sources):
                name = f"unit{index}.rs"
                (root / name).write_text(source)
                units.append(analysis.UnitMetric(name, name, "function", 1, 1, 0, 0.0, {}))
            with patch.object(analysis.difflib, "SequenceMatcher", side_effect=AssertionError("unnecessary sequence comparison")):
                candidates = analysis.find_repetition(root, units, analysis.RepetitionPolicy(4, 0.5, 0.96), analysis.AnalysisPurpose.ENFORCE)
            self.assertEqual(candidates, [])


if __name__ == "__main__":
    unittest.main()
