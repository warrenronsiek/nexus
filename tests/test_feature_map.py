# @feature architecture-tooling
# @spec docs/features/architecture-tooling.md
# @boundary lint-fixture-dynamic-data
"""Behavior tests for the feature-map explorer and linter."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from types import ModuleType


def load_feature_map() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "feature_map.py"
    spec = importlib.util.spec_from_file_location("feature_map", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load feature-map module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class FeatureMapTests(unittest.TestCase):
    def test_discovers_typescript_symbols_but_skips_generated_output(self) -> None:
        feature_map = load_feature_map()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            typescript = root / "pi-extension" / "src" / "component.ts"
            generated = root / "pi-extension" / "dist" / "app.js"
            for path in (typescript, generated):
                path.parent.mkdir(parents=True, exist_ok=True)
            typescript.write_text(
                "export interface Bucket { count: number }\n"
                "export type Renderer = (buckets: Bucket[]) => void;\n"
                "export function render(buckets: Bucket[]): void {}\n",
                encoding="utf-8",
            )
            generated.write_text("export function bundled() {}\n", encoding="utf-8")

            sources = feature_map.read_sources(root)
            self.assertEqual(
                [str(source.path) for source in sources],
                ["pi-extension/src/component.ts"],
            )
            self.assertEqual(
                [(symbol.kind, symbol.name) for symbol in sources[0].symbols],
                [
                    ("interface", "Bucket"),
                    ("type", "Renderer"),
                    ("function", "render"),
                ],
            )

    def test_lint_accepts_linked_feature_and_typed_python(self) -> None:
        feature_map = load_feature_map()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs" / "features").mkdir(parents=True)
            (root / "docs" / "features" / "demo.md").write_text(
                "---\nfeature: demo\n---\n# Demo\n", encoding="utf-8"
            )
            source = root / "demo.py"
            source.write_text(
                "# @feature demo\n"
                "# @spec docs/features/demo.md\n"
                "# @entrypoint run\n"
                "def run(value: str) -> str:\n"
                "    return value\n",
                encoding="utf-8",
            )
            sources = feature_map.read_sources(root)
            self.assertEqual(feature_map.lint(root, sources), [])

    def test_lint_rejects_missing_types_and_unmarked_dynamic_data(self) -> None:
        feature_map = load_feature_map()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs" / "features").mkdir(parents=True)
            (root / "docs" / "features" / "demo.md").write_text(
                "---\nfeature: demo\n---\n# Demo\n", encoding="utf-8"
            )
            (root / "demo.py").write_text(
                "# @feature demo\n"
                "# @spec docs/features/demo.md\n"
                "# @entrypoint run\n"
                "from typing import Any\n"
                "def run(value):\n"
                "    return value\n",
                encoding="utf-8",
            )
            errors = feature_map.lint(root, feature_map.read_sources(root))
            self.assertTrue(any("@boundary" in error for error in errors))
            self.assertTrue(any("argument 'value'" in error for error in errors))
            self.assertTrue(any("return type" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
