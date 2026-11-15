#!/usr/bin/env python3
# @feature complexity-analysis
# @spec docs/features/complexity-analysis.md
# @entrypoint main
# @boundary bca-json-output
"""Analyze BCA metrics and repeated implementation shapes by feature or path."""

from __future__ import annotations

import argparse
import difflib
import json
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import asdict, dataclass, field, fields, replace
from enum import Enum
from operator import attrgetter
from pathlib import Path
from typing import Mapping, Sequence

import feature_map
import rust_structure

ANALYZABLE_SUFFIXES = {".js", ".jsx", ".mjs", ".py", ".rs", ".ts", ".tsx"}
BCA_METRICS = "cognitive,cyclomatic,lloc,nargs,nexits,nom,tokens,halstead,mi,abc"
TOKEN_PATTERN = re.compile(
    r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|'
    r"[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|"
    r"::|->|=>|==|!=|<=|>=|&&|\|\||[{}()\[\];,.?:+\-*/%<>=!&|]"
)
KEYWORDS = {
    "and",
    "as",
    "async",
    "await",
    "break",
    "class",
    "const",
    "continue",
    "crate",
    "def",
    "else",
    "enum",
    "except",
    "false",
    "fn",
    "for",
    "from",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "none",
    "not",
    "or",
    "pass",
    "pub",
    "raise",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "with",
    "yield",
}


class AnalysisError(RuntimeError):
    """An actionable analysis setup or execution failure."""


@dataclass(frozen=True)
class RankingWeights:
    lloc: float
    arguments: float
    functions: float
    cyclomatic: float
    cognitive: float
    mutation_sites: float
    state_types: float
    dynamic_types: float


@dataclass(frozen=True)
class RankingSignals:
    lloc: float = 0.0
    arguments: float = 0.0
    functions: float = 0.0
    cyclomatic: float = 0.0
    cognitive: float = 0.0
    mutation_sites: float = 0.0
    state_types: float = 0.0
    dynamic_types: float = 0.0


def ranking_formula(weights: RankingWeights) -> str:
    labels = {"lloc": "LLOC", "cyclomatic": "branching"}
    return " + ".join(f"{getattr(weights, item.name):g}*{labels.get(item.name, item.name)}" for item in fields(RankingSignals))


def contributions(weights: RankingWeights, signals: RankingSignals) -> RankingSignals:
    return RankingSignals(**{item.name: getattr(weights, item.name) * getattr(signals, item.name) for item in fields(RankingSignals)})


@dataclass(frozen=True)
class SourceEvidence:
    path: Path
    lines: Sequence[str]
    nodes: Sequence[rust_structure.RustNode]
    span: rust_structure.SourceSpan
    names: rust_structure.TypeNames

    def scoped(self, space: Mapping[str, object], owner: rust_structure.RustNode | None) -> SourceEvidence:
        if owner is None:
            span = rust_structure.SourceSpan.whole_lines(as_integer(space.get("start_line")), as_integer(space.get("end_line")))
            return replace(self, span=span)
        actual = (rust_structure.SPACE_KINDS[owner.kind], owner.span.start_line, owner.span.end_line)
        expected = (space.get("kind"), space.get("start_line"), space.get("end_line"))
        if actual != expected:
            raise AnalysisError(f"BCA metric/node scope mismatch in {self.path}: {expected} != {actual}")
        return SourceEvidence(self.path, self.lines, tuple(node for node in self.nodes if owner.span.contains(node.span)), owner.span, self.names)

    def metric_units(self, root: Path, document: Mapping[str, object], weights: RankingWeights) -> list[UnitMetric]:
        spaces = spaces_in_document(document)
        owners: Sequence[rust_structure.RustNode | None] = [None] * len(spaces)
        if self.path.suffix == ".rs":
            owners = [None, *(node for node in self.nodes if node.kind in rust_structure.SPACE_KINDS)]
            if len(owners) != len(spaces):
                raise AnalysisError(f"BCA metric/node scope count mismatch in {self.path}")
        return [unit_from_space(root, self.scoped(space, owner), space, weights) for space, owner in zip(spaces, owners)]

    def ranking_signals(self, space: Mapping[str, object]) -> RankingSignals:
        metrics = as_mapping(space.get("metrics"))
        loc = as_mapping(metrics.get("loc"))
        nargs = as_mapping(metrics.get("nargs"))
        nom = as_mapping(metrics.get("nom"))
        cyclomatic = as_mapping(metrics.get("cyclomatic"))
        cognitive = as_mapping(metrics.get("cognitive"))
        syntax = rust_structure.signals_in_span(self.nodes, self.lines, self.span, self.names)
        return RankingSignals(
            lloc=as_number(loc.get("lloc")),
            arguments=syntax.arguments if self.path.suffix == ".rs" else as_number(nargs.get("function_args", nargs.get("total"))),
            functions=syntax.functions if self.path.suffix == ".rs" else as_number(nom.get("functions")),
            cyclomatic=max(as_number(cyclomatic.get("sum", cyclomatic.get("value"))) - space_count(space), 0.0),
            cognitive=as_number(cognitive.get("sum", cognitive.get("value"))),
            mutation_sites=syntax.mutation_sites,
            state_types=syntax.state_types,
            dynamic_types=syntax.dynamic_types,
        )


@dataclass(frozen=True)
class RepetitionPolicy:
    minimum_tokens: int
    warning_similarity: float
    fail_similarity: float


@dataclass(frozen=True)
class AnalysisPolicy:
    ranking: RankingWeights
    repetition: RepetitionPolicy


@dataclass(frozen=True)
class UnitMetric:
    path: str
    name: str
    kind: str
    span: rust_structure.SourceSpan
    architectural_load: float
    metrics: Mapping[str, object]
    signals: RankingSignals = field(default_factory=RankingSignals)
    contributions: RankingSignals = field(default_factory=RankingSignals)

    def contains(self, other: UnitMetric) -> bool:
        return self.path == other.path and self.span.contains(other.span)

    def label(self) -> str:
        return f"{self.path}:{self.span.start_line}:{self.name}"


@dataclass(frozen=True)
class RepetitionCandidate:
    left: str
    right: str
    left_tokens: int
    right_tokens: int
    sequence_similarity: float
    shingle_containment: float

    def similarity(self) -> float:
        return max(self.sequence_similarity, self.shingle_containment)

    def breaches(self, threshold: float) -> bool:
        return (
            self.sequence_similarity >= threshold
            and self.shingle_containment >= threshold
        )


def as_mapping(value: object) -> Mapping[str, object]:
    return value if isinstance(value, dict) else {}


def as_number(value: object) -> float:
    return float(value) if isinstance(value, (int, float)) else 0.0


def as_integer(value: object) -> int:
    return int(as_number(value))


def load_policy(root: Path) -> AnalysisPolicy:
    config_path = root / "complexity.toml"
    with config_path.open("rb") as config_file:
        config = tomllib.load(config_file)
    ranking = as_mapping(config.get("ranking"))
    repetition = as_mapping(config.get("repetition"))
    return AnalysisPolicy(
        ranking=RankingWeights(
            lloc=as_number(ranking.get("lloc")),
            arguments=as_number(ranking.get("arguments")),
            functions=as_number(ranking.get("functions")),
            cyclomatic=as_number(ranking.get("cyclomatic")),
            cognitive=as_number(ranking.get("cognitive")),
            mutation_sites=as_number(ranking.get("mutation_sites")),
            state_types=as_number(ranking.get("state_types")),
            dynamic_types=as_number(ranking.get("dynamic_types")),
        ),
        repetition=RepetitionPolicy(
            minimum_tokens=as_integer(repetition.get("minimum_tokens")),
            warning_similarity=as_number(repetition.get("warning_similarity")),
            fail_similarity=as_number(repetition.get("fail_similarity")),
        ),
    )


def select_paths(root: Path, feature: str | None, requested_path: Path | None) -> list[Path]:
    sources = feature_map.read_sources(root)
    if feature is not None:
        matches = [source for source in sources if feature in source.features]
        if not matches:
            available = sorted({tag for source in sources for tag in source.features})
            raise AnalysisError(
                f"unknown feature {feature!r}; available: {', '.join(available)}"
            )
        paths = [root / source.path for source in matches]
    elif requested_path is not None:
        absolute = requested_path if requested_path.is_absolute() else root / requested_path
        absolute = absolute.resolve()
        if not absolute.exists():
            raise AnalysisError(f"path does not exist: {requested_path}")
        if absolute.is_file():
            paths = [absolute]
        else:
            paths = [
                root / source.path
                for source in sources
                if (root / source.path).resolve().is_relative_to(absolute)
            ]
    else:
        paths = [root / source.path for source in sources]
    return sorted(path for path in paths if path.suffix in ANALYZABLE_SUFFIXES)


def require_bca() -> str:
    executable = shutil.which("bca")
    if executable is None:
        raise AnalysisError(
            "bca is not installed; run "
            "`uv tool install big-code-analysis-cli==2.2.0`"
        )
    return executable


def relative_path_input(root: Path, paths: Sequence[Path]) -> str:
    return "\n".join(str(path.resolve().relative_to(root)) for path in paths)


def run_bca_metrics(root: Path, paths: Sequence[Path]) -> list[Mapping[str, object]]:
    completed = subprocess.run(
        [
            require_bca(),
            "metrics",
            "--format",
            "json",
            "--metrics",
            BCA_METRICS,
            "--cyclomatic-count-try=false",
            "--no-config",
            "--paths-from",
            "-",
        ],
        cwd=root,
        input=relative_path_input(root, paths),
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise AnalysisError(completed.stderr.strip() or "bca metrics failed")
    try:
        decoded = json.loads(completed.stdout)
        if isinstance(decoded, list):
            return [as_mapping(item) for item in decoded]
        return [as_mapping(decoded)]
    except json.JSONDecodeError as aggregate_error:
        documents: list[Mapping[str, object]] = []
        for line in completed.stdout.splitlines():
            if line.strip():
                documents.append(as_mapping(json.loads(line)))
        if not documents:
            raise aggregate_error
        return documents


def run_bca_check(root: Path, paths: Sequence[Path]) -> int:
    completed = subprocess.run(
        [
            require_bca(),
            "check",
            "--no-config",
            "--config",
            "bca.toml",
            "--baseline",
            ".bca-baseline.toml",
            "--strict",
            "--cyclomatic-count-try=false",
            "--paths-from",
            "-",
        ],
        cwd=root,
        input=relative_path_input(root, paths),
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.stdout:
        print(completed.stdout, end="", file=sys.stderr)
    if completed.stderr:
        print(completed.stderr, end="", file=sys.stderr)
    return completed.returncode


def space_count(space: Mapping[str, object]) -> int:
    children = space.get("spaces")
    return 1 + sum(space_count(as_mapping(child)) for child in children) if isinstance(children, list) else 1


def unit_from_space(root: Path, source: SourceEvidence, space: Mapping[str, object], weights: RankingWeights) -> UnitMetric:
    signals = source.ranking_signals(space)
    weighted = contributions(weights, signals)
    return UnitMetric(
        path=str(source.path.relative_to(root)),
        name=str(space.get("name", source.path.name)),
        kind=str(space.get("kind", "unit")),
        span=source.span,
        architectural_load=sum(asdict(weighted).values()),
        metrics=as_mapping(space.get("metrics")),
        signals=signals,
        contributions=weighted,
    )


def spaces_in_document(space: Mapping[str, object]) -> list[Mapping[str, object]]:
    spaces = [space]
    children = space.get("spaces")
    if isinstance(children, list):
        for child in children:
            spaces.extend(spaces_in_document(as_mapping(child)))
    return spaces


def collect_units(
    root: Path,
    documents: Sequence[Mapping[str, object]],
    weights: RankingWeights,
) -> list[UnitMetric]:
    paths = [root / str(document.get("name", "")) for document in documents]
    try:
        nodes = rust_structure.read_nodes(require_bca(), root, paths)
    except RuntimeError as error:
        raise AnalysisError(str(error)) from error
    units: list[UnitMetric] = []
    for document, path in zip(documents, paths):
        lines = path.read_text(encoding="utf-8").splitlines()
        file_nodes = nodes.get(path, ())
        span = rust_structure.SourceSpan.whole_lines(as_integer(document.get("start_line")), as_integer(document.get("end_line")))
        source = SourceEvidence(path, lines, file_nodes, span, rust_structure.TypeNames.from_source(file_nodes, lines))
        units.extend(source.metric_units(root, document, weights))
    return units


def scope_document(units: Sequence[UnitMetric]) -> dict[str, object]:
    selected: list[UnitMetric] = []
    for unit in sorted(units, key=lambda item: (item.path, item.span.start_line, item.span.start_column, -item.span.end_line, -item.span.end_column)):
        if not any(parent.contains(unit) for parent in selected):
            selected.append(unit)
    totals = RankingSignals(**{item.name: sum(getattr(unit.contributions, item.name) for unit in selected) for item in fields(RankingSignals)})
    return {
        "architectural_load": sum(unit.architectural_load for unit in selected),
        "contributions": asdict(totals),
        "units": [unit.label() for unit in selected],
    }


def normalized_tokens(text: str) -> tuple[str, ...]:
    without_comments = re.sub(r"(?m)//.*$|#.*$", "", text)
    tokens: list[str] = []
    for token in TOKEN_PATTERN.findall(without_comments):
        lowered = token.lower()
        if token.startswith(("\"", "'")):
            tokens.append("STRING")
        elif token[0].isdigit():
            tokens.append("NUMBER")
        elif re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", token):
            tokens.append(lowered if lowered in KEYWORDS else "IDENTIFIER")
        else:
            tokens.append(token)
    return tuple(tokens)


def token_shingles(tokens: Sequence[str], width: int = 5) -> set[tuple[str, ...]]:
    if len(tokens) < width:
        return set()
    return {tuple(tokens[index : index + width]) for index in range(len(tokens) - width + 1)}


class AnalysisPurpose(Enum):
    REVIEW = "review"
    ENFORCE = "enforce"


@dataclass(frozen=True)
class ComparableUnit:
    unit: UnitMetric
    tokens: tuple[str, ...]
    shingles: frozenset[tuple[str, ...]]

    def contains_or_is_contained_by(self, other: ComparableUnit) -> bool:
        return self.unit.contains(other.unit) or other.unit.contains(self.unit)


def comparable_units(root: Path, units: Sequence[UnitMetric], minimum_tokens: int) -> list[ComparableUnit]:
    lines_by_path: dict[str, list[str]] = {}
    result: list[ComparableUnit] = []
    for unit in units:
        if unit.kind not in {"function", "method", "closure"}:
            continue
        if unit.path not in lines_by_path:
            lines_by_path[unit.path] = (root / unit.path).read_text(encoding="utf-8").splitlines()
        lines = lines_by_path[unit.path]
        tokens = normalized_tokens(unit.span.source(lines))
        if len(tokens) >= minimum_tokens:
            result.append(ComparableUnit(unit, tokens, frozenset(token_shingles(tokens))))
    return result


def compare_units(left: ComparableUnit, right: ComparableUnit, policy: RepetitionPolicy, purpose: AnalysisPurpose) -> RepetitionCandidate | None:
    if left.contains_or_is_contained_by(right):
        return None
    total = len(left.tokens) + len(right.tokens)
    upper_bound = 2 * min(len(left.tokens), len(right.tokens)) / total if total else 1.0
    if purpose is AnalysisPurpose.ENFORCE and upper_bound < policy.fail_similarity:
        return None
    denominator = min(len(left.shingles), len(right.shingles))
    containment = len(left.shingles & right.shingles) / denominator if denominator else 0.0
    if purpose is AnalysisPurpose.ENFORCE and containment < policy.fail_similarity:
        return None
    sequence = difflib.SequenceMatcher(None, left.tokens, right.tokens, autojunk=False).ratio()
    candidate = RepetitionCandidate(left.unit.label(), right.unit.label(), len(left.tokens), len(right.tokens), sequence, containment)
    accepted = candidate.breaches(policy.fail_similarity) if purpose is AnalysisPurpose.ENFORCE else candidate.similarity() >= policy.warning_similarity
    return candidate if accepted else None


def find_repetition(root: Path, units: Sequence[UnitMetric], policy: RepetitionPolicy, purpose: AnalysisPurpose = AnalysisPurpose.REVIEW) -> list[RepetitionCandidate]:
    comparable = comparable_units(root, units, policy.minimum_tokens)
    candidates: list[RepetitionCandidate] = []
    for index, left in enumerate(comparable):
        for right in comparable[index + 1:]:
            candidate = compare_units(left, right, policy, purpose)
            if candidate is not None:
                candidates.append(candidate)
    return sorted(candidates, key=lambda candidate: candidate.similarity(), reverse=True)


def nested_metric(unit: UnitMetric, metric: str, field: str) -> float:
    return as_number(as_mapping(unit.metrics.get(metric)).get(field))


def unit_document(unit: UnitMetric) -> dict[str, object]:
    return {
        "path": unit.path,
        "name": unit.name,
        "kind": unit.kind,
        **asdict(unit.span),
        "architectural_load": unit.architectural_load,
        "metrics": dict(unit.metrics),
        "signals": asdict(unit.signals),
        "contributions": asdict(unit.contributions),
    }


def repetition_document(candidate: RepetitionCandidate) -> dict[str, object]:
    return {
        "left": candidate.left,
        "right": candidate.right,
        "left_tokens": candidate.left_tokens,
        "right_tokens": candidate.right_tokens,
        "sequence_similarity": candidate.sequence_similarity,
        "shingle_containment": candidate.shingle_containment,
        "similarity": candidate.similarity(),
    }


def print_text(
    scope: str,
    units: Sequence[UnitMetric],
    candidates: Sequence[RepetitionCandidate],
    limit: int,
    weights: RankingWeights,
) -> None:
    print(f"scope: {scope}")
    print(f"ranking: {ranking_formula(weights)}")
    print(f"scope architectural load: {scope_document(units)['architectural_load']:.1f} (outermost units counted once)")
    print("hotspots (review leads; inspect raw metrics before changing code):")
    ranked = sorted(units, key=attrgetter("architectural_load"), reverse=True)
    for unit in ranked[:limit]:
        print(
            f"  {unit.architectural_load:7.1f}  {unit.path}:{unit.span.start_line} "
            f"{unit.kind} {unit.name} | "
            f"lloc={nested_metric(unit, 'loc', 'lloc'):g} "
            f"args={unit.signals.arguments:g} "
            f"functions={unit.signals.functions:g} "
            f"mutation={unit.signals.mutation_sites:g} state={unit.signals.state_types:g} dynamic={unit.signals.dynamic_types:g} "
            f"branching={unit.signals.cyclomatic:g} "
            f"cognitive={unit.signals.cognitive:g} "
            f"abc={nested_metric(unit, 'abc', 'value'):.1f}"
        )
        print("    costs: " + " ".join(f"{name}={value:g}" for name, value in asdict(unit.contributions).items()))
    print("repetition candidates (verify shared invariants before abstracting):")
    if not candidates:
        print("  none")
    for candidate in candidates[:limit]:
        print(
            f"  {candidate.similarity():.3f}  {candidate.left} <> {candidate.right} "
            f"| sequence={candidate.sequence_similarity:.3f} "
            f"shingles={candidate.shingle_containment:.3f}"
        )


def parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    scope = parser.add_mutually_exclusive_group()
    scope.add_argument("--feature")
    scope.add_argument("--path", type=Path)
    parser.add_argument("--symbol")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--limit", type=int, default=20)
    return parser.parse_args(argv)


def selected_units(root: Path, paths: Sequence[Path], weights: RankingWeights, symbol: str | None) -> list[UnitMetric]:
    units = collect_units(root, run_bca_metrics(root, paths), weights)
    if symbol is not None:
        units = [unit for unit in units if unit.name == symbol]
        if not units:
            raise AnalysisError(f"symbol not found in selected scope: {symbol}")
    return units


def report_document(scope: str, purpose: AnalysisPurpose, units: Sequence[UnitMetric], candidates: Sequence[RepetitionCandidate], policy: AnalysisPolicy) -> dict[str, object]:
    return {
        "scope": scope,
        "comparison": purpose.value,
        "formula": ranking_formula(policy.ranking),
        "weights": asdict(policy.ranking),
        "total": scope_document(units),
        "units": [unit_document(unit) for unit in units],
        "repetition": [repetition_document(candidate) for candidate in candidates],
        "thresholds": asdict(policy.repetition),
        "violations": [repetition_document(candidate) for candidate in candidates if candidate.breaches(policy.repetition.fail_similarity)],
    }


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(argv)
    root = arguments.root.resolve()
    try:
        policy = load_policy(root)
        paths = select_paths(root, arguments.feature, arguments.path)
        if not paths:
            raise AnalysisError("the selected scope has no BCA-analyzable source files")
        units = selected_units(root, paths, policy.ranking, arguments.symbol)
        purpose = AnalysisPurpose.ENFORCE if arguments.check else AnalysisPurpose.REVIEW
        candidates = find_repetition(root, units, policy.repetition, purpose)
        violations = [
            candidate
            for candidate in candidates
            if candidate.breaches(policy.repetition.fail_similarity)
        ]
        scope = (
            f"feature:{arguments.feature}"
            if arguments.feature is not None
            else f"path:{arguments.path}"
            if arguments.path is not None
            else "repository"
        )
        if arguments.format == "json":
            print(json.dumps(report_document(scope, purpose, units, candidates, policy), indent=2))
        else:
            print(f"comparison: {purpose.value} (omit --check for advisory comparisons)")
            print_text(scope, units, candidates, arguments.limit, policy.ranking)

        bca_status = run_bca_check(root, paths) if arguments.check else 0
        if bca_status == 1:
            return 1
        if arguments.check and (violations or bca_status in range(2, 6)):
            return 2
        return bca_status
    except (AnalysisError, json.JSONDecodeError, OSError, tomllib.TOMLDecodeError) as error:
        print(f"complexity analysis failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
