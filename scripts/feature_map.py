#!/usr/bin/env python3
# @feature architecture-tooling
# @spec docs/features/architecture-tooling.md
# @entrypoint main
# @boundary dynamic-type-detector
"""Explore and lint feature annotations without third-party dependencies."""

from __future__ import annotations

import argparse
import ast
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

SUPPORTED_SUFFIXES = {
    ".go",
    ".java",
    ".js",
    ".jsx",
    ".kt",
    ".py",
    ".rs",
    ".sh",
    ".sql",
    ".toml",
    ".ts",
    ".tsx",
    ".yaml",
    ".yml",
}
SUPPORTED_NAMES = {"Dockerfile", "Makefile", "pre-commit", "pre-push"}
SKIPPED_DIRECTORIES = {
    ".git",
    ".idea",
    ".vscode",
    "__pycache__",
    "docs",
    "node_modules",
    "target",
    "vendor",
}
SKIPPED_FILES = {"Cargo.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock"}
HEADER_LINE_LIMIT = 80

ANNOTATION_PATTERNS = {
    "feature": re.compile(r"@feature\s+([a-z0-9][a-z0-9._-]*)"),
    "spec": re.compile(r"@spec\s+([^\s]+)"),
    "entrypoint": re.compile(r"@entrypoint\s+(.+?)\s*$"),
    "boundary": re.compile(r"@boundary\s+([a-z0-9][a-z0-9._-]*)"),
}

DYNAMIC_PATTERNS = (
    re.compile(r"serde_json::Value"),
    re.compile(r"serde_json::\{[^}]*\bValue\b"),
    re.compile(r"toml::Value"),
    re.compile(r"toml::\{[^}]*\bValue\b"),
    re.compile(r"\bdyn\s+Any\b"),
    re.compile(r"\btyping\.Any\b"),
    re.compile(r"\bfrom\s+typing\s+import[^\n]*\bAny\b"),
    re.compile(r"\bRecord\s*<\s*string\s*,\s*any\s*>"),
    re.compile(r"\bMap\s*<\s*string\s*,\s*any\s*>"),
)


@dataclass(frozen=True)
class Symbol:
    kind: str
    name: str
    line: int


@dataclass(frozen=True)
class SourceFile:
    path: Path
    features: tuple[str, ...]
    specs: tuple[Path, ...]
    entrypoints: tuple[str, ...]
    boundaries: tuple[str, ...]
    symbols: tuple[Symbol, ...]
    text: str


def unique(values: Iterable[str]) -> tuple[str, ...]:
    return tuple(dict.fromkeys(values))


def source_paths(root: Path) -> list[Path]:
    paths: list[Path] = []
    for path in root.rglob("*"):
        if not path.is_file() or path.name in SKIPPED_FILES:
            continue
        relative = path.relative_to(root)
        if any(part in SKIPPED_DIRECTORIES for part in relative.parts[:-1]):
            continue
        if path.suffix in SUPPORTED_SUFFIXES or path.name in SUPPORTED_NAMES:
            paths.append(path)
    return sorted(paths)


def annotations(text: str, key: str) -> tuple[str, ...]:
    comment_lines = (
        line
        for line in text.splitlines()[:HEADER_LINE_LIMIT]
        if line.lstrip().startswith(("#", "//", "--", "/*", "*"))
    )
    return unique(
        match.group(1).strip()
        for line in comment_lines
        if (match := ANNOTATION_PATTERNS[key].search(line))
    )


def discover_rust_symbols(text: str) -> list[Symbol]:
    definitions = re.compile(
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?"
        r"(?:(async|const|unsafe|extern\s+\"[^\"]+\")\s+)*"
        r"(fn|struct|enum|trait|union|type|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
    )
    implementations = re.compile(r"^\s*impl(?:<[^>]+>)?\s+(.+?)\s*\{?\s*$")
    symbols: list[Symbol] = []
    for line_number, line in enumerate(text.splitlines(), 1):
        match = definitions.match(line)
        if match:
            symbols.append(Symbol(match.group(2), match.group(3), line_number))
            continue
        implementation = implementations.match(line)
        if implementation:
            symbols.append(Symbol("impl", implementation.group(1), line_number))
    return symbols


def discover_python_symbols(text: str) -> list[Symbol]:
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return []
    symbols: list[Symbol] = []
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            symbols.append(Symbol("function", node.name, node.lineno))
        elif isinstance(node, ast.ClassDef):
            symbols.append(Symbol("class", node.name, node.lineno))
    return sorted(symbols, key=lambda symbol: (symbol.line, symbol.name))


def discover_symbols(path: Path, text: str) -> tuple[Symbol, ...]:
    if path.suffix == ".rs":
        return tuple(discover_rust_symbols(text))
    if path.suffix == ".py":
        return tuple(discover_python_symbols(text))
    if path.suffix == ".sql":
        pattern = re.compile(
            r"^\s*CREATE\s+(TABLE|INDEX)\s+(?:IF\s+NOT\s+EXISTS\s+)?([^\s(]+)",
            re.IGNORECASE,
        )
        return tuple(
            Symbol(match.group(1).lower(), match.group(2), line_number)
            for line_number, line in enumerate(text.splitlines(), 1)
            if (match := pattern.match(line))
        )
    return ()


def read_source(root: Path, path: Path) -> SourceFile:
    text = path.read_text(encoding="utf-8")
    return SourceFile(
        path=path.relative_to(root),
        features=annotations(text, "feature"),
        specs=tuple(Path(value) for value in annotations(text, "spec")),
        entrypoints=annotations(text, "entrypoint"),
        boundaries=annotations(text, "boundary"),
        symbols=discover_symbols(path, text),
        text=text,
    )


def read_sources(root: Path) -> list[SourceFile]:
    return [read_source(root, path) for path in source_paths(root)]


def spec_feature(path: Path) -> str | None:
    if not path.is_file():
        return None
    header = "\n".join(path.read_text(encoding="utf-8").splitlines()[:20])
    match = re.search(r"^feature:\s*([a-z0-9][a-z0-9._-]*)\s*$", header, re.MULTILINE)
    return match.group(1) if match else None


def python_type_errors(source: SourceFile) -> list[str]:
    if source.path.suffix != ".py":
        return []
    try:
        tree = ast.parse(source.text)
    except SyntaxError as error:
        return [f"{source.path}:{error.lineno}: invalid Python: {error.msg}"]
    errors: list[str] = []
    for node in ast.walk(tree):
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        arguments = [*node.args.posonlyargs, *node.args.args, *node.args.kwonlyargs]
        for argument in arguments:
            if argument.arg in {"self", "cls"}:
                continue
            if argument.annotation is None:
                errors.append(
                    f"{source.path}:{argument.lineno}: {node.name} argument "
                    f"'{argument.arg}' lacks a type annotation"
                )
        for argument in (node.args.vararg, node.args.kwarg):
            if argument is not None and argument.annotation is None:
                errors.append(
                    f"{source.path}:{argument.lineno}: {node.name} argument "
                    f"'{argument.arg}' lacks a type annotation"
                )
        if node.returns is None:
            errors.append(
                f"{source.path}:{node.lineno}: {node.name} lacks a return type annotation"
            )
    return errors


def lint(root: Path, sources: Sequence[SourceFile]) -> list[str]:
    errors: list[str] = []
    entrypoints_by_feature: dict[str, set[str]] = {}
    declared_features: set[str] = set()

    for source in sources:
        if not source.features:
            errors.append(f"{source.path}: missing @feature")
        if not source.specs:
            errors.append(f"{source.path}: missing @spec")

        spec_features: set[str] = set()
        for relative_spec in source.specs:
            full_spec = root / relative_spec
            feature = spec_feature(full_spec)
            if feature is None:
                if full_spec.exists():
                    errors.append(f"{source.path}: {relative_spec} has no feature frontmatter")
                else:
                    errors.append(f"{source.path}: missing spec {relative_spec}")
            else:
                spec_features.add(feature)

        for feature in source.features:
            declared_features.add(feature)
            if feature not in spec_features:
                errors.append(
                    f"{source.path}: @feature {feature} has no matching referenced spec"
                )
            if source.entrypoints:
                entrypoints_by_feature.setdefault(feature, set()).update(source.entrypoints)

        if any(pattern.search(source.text) for pattern in DYNAMIC_PATTERNS):
            if not source.boundaries:
                errors.append(
                    f"{source.path}: dynamic type requires an explicit @boundary annotation"
                )
        errors.extend(python_type_errors(source))

    for feature in sorted(declared_features):
        if not entrypoints_by_feature.get(feature):
            errors.append(f"feature {feature}: no @entrypoint declared")
    return errors


def print_feature(root: Path, sources: Sequence[SourceFile], feature: str) -> int:
    matches = [source for source in sources if feature in source.features]
    if not matches:
        available = sorted({tag for source in sources for tag in source.features})
        print(f"unknown feature: {feature}", file=sys.stderr)
        print(f"available: {', '.join(available)}", file=sys.stderr)
        return 2

    specs = sorted(
        {
            spec
            for source in matches
            for spec in source.specs
            if spec_feature(root / spec) == feature
        }
    )
    entrypoints = sorted({entrypoint for source in matches for entrypoint in source.entrypoints})
    print(f"feature: {feature}")
    print("specs:")
    for spec in specs:
        print(f"  - {spec}")
    print("entrypoints:")
    for entrypoint in entrypoints:
        print(f"  - {entrypoint}")
    print("files:")
    for source in matches:
        print(f"  - {source.path}")
        for symbol in source.symbols:
            print(f"      {symbol.line}: {symbol.kind} {symbol.name}")
    return 0


def print_features(sources: Sequence[SourceFile]) -> None:
    features = sorted({feature for source in sources for feature in source.features})
    for feature in features:
        count = sum(feature in source.features for source in sources)
        print(f"{feature}\t{count} files")


def parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--feature")
    mode.add_argument("--lint", action="store_true")
    mode.add_argument("--list", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(argv)
    root = arguments.root.resolve()
    sources = read_sources(root)
    if arguments.lint:
        errors = lint(root, sources)
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            print(f"feature-map lint failed with {len(errors)} error(s)", file=sys.stderr)
            return 1
        print(f"feature-map lint passed for {len(sources)} files")
        return 0
    if arguments.feature:
        return print_feature(root, sources, arguments.feature)
    print_features(sources)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
