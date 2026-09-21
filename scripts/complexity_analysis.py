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
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence

import feature_map

ANALYZABLE_SUFFIXES = {".js", ".jsx", ".mjs", ".py", ".rs", ".ts", ".tsx"}
BCA_METRICS = "cognitive,cyclomatic,lloc,nargs,nexits,nom,tokens,halstead,mi,abc"
RANKING_FORMULA = (
    "LLOC + 3*arguments + 8*declarations + "
    "5*max(cyclomatic-1, 0) + 2*cognitive"
)
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
    declarations: float
    cyclomatic: float
    cognitive: float


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
    start_line: int
    end_line: int
    declarations: int
    architectural_load: float
    metrics: Mapping[str, object]

    def label(self) -> str:
        return f"{self.path}:{self.start_line}:{self.name}"


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
            declarations=as_number(ranking.get("declarations")),
            cyclomatic=as_number(ranking.get("cyclomatic")),
            cognitive=as_number(ranking.get("cognitive")),
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


def declaration_count(path: Path, lines: Sequence[str], start: int, end: int) -> int:
    body = "\n".join(lines[max(start - 1, 0) : end])
    if path.suffix == ".rs":
        pattern = re.compile(
            r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?"
            r"(?:let|const|static|struct|enum|trait|type)\b"
        )
    elif path.suffix in {".js", ".jsx", ".mjs", ".ts", ".tsx"}:
        pattern = re.compile(
            r"(?m)^\s*(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:async\s+)?"
            r"(?:function|class|interface|type|enum|const|let|var)\b"
        )
    else:
        pattern = re.compile(
            r"(?m)^\s*(?:async\s+def|def|class)\s+|"
            r"^\s*[A-Za-z_][A-Za-z0-9_]*\s*(?::[^=\n]+)?=(?!=)"
        )
    return len(pattern.findall(body))


def ranking_score(
    weights: RankingWeights,
    lloc: int,
    arguments: int,
    declarations: int,
    cyclomatic: float,
    cognitive: float,
) -> float:
    return (
        weights.lloc * lloc
        + weights.arguments * arguments
        + weights.declarations * declarations
        + weights.cyclomatic * max(cyclomatic - 1.0, 0.0)
        + weights.cognitive * cognitive
    )


def unit_from_space(
    root: Path,
    path: Path,
    lines: Sequence[str],
    space: Mapping[str, object],
    weights: RankingWeights,
) -> UnitMetric:
    metrics = as_mapping(space.get("metrics"))
    loc = as_mapping(metrics.get("loc"))
    nargs = as_mapping(metrics.get("nargs"))
    cyclomatic_metric = as_mapping(metrics.get("cyclomatic"))
    cognitive_metric = as_mapping(metrics.get("cognitive"))
    start = as_integer(space.get("start_line"))
    end = as_integer(space.get("end_line"))
    declarations = declaration_count(path, lines, start, end)
    return UnitMetric(
        path=str(path.relative_to(root)),
        name=str(space.get("name", path.name)),
        kind=str(space.get("kind", "unit")),
        start_line=start,
        end_line=end,
        declarations=declarations,
        architectural_load=ranking_score(
            weights,
            as_integer(loc.get("lloc")),
            as_integer(nargs.get("total")),
            declarations,
            as_number(cyclomatic_metric.get("value")),
            as_number(cognitive_metric.get("value")),
        ),
        metrics=metrics,
    )


def units_from_space(
    root: Path,
    path: Path,
    lines: Sequence[str],
    space: Mapping[str, object],
    weights: RankingWeights,
) -> list[UnitMetric]:
    units = [unit_from_space(root, path, lines, space, weights)]
    children = space.get("spaces")
    if isinstance(children, list):
        for child in children:
            units.extend(units_from_space(root, path, lines, as_mapping(child), weights))
    return units


def collect_units(
    root: Path,
    documents: Sequence[Mapping[str, object]],
    weights: RankingWeights,
) -> list[UnitMetric]:
    units: list[UnitMetric] = []
    for document in documents:
        path = root / str(document.get("name", ""))
        lines = path.read_text(encoding="utf-8").splitlines()
        units.extend(units_from_space(root, path, lines, document, weights))
    return units


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


def find_repetition(
    root: Path,
    units: Sequence[UnitMetric],
    policy: RepetitionPolicy,
) -> list[RepetitionCandidate]:
    comparable_kinds = {"function", "method", "closure"}
    tokenized: list[tuple[UnitMetric, tuple[str, ...]]] = []
    for unit in units:
        if unit.kind not in comparable_kinds:
            continue
        lines = (root / unit.path).read_text(encoding="utf-8").splitlines()
        body = "\n".join(lines[max(unit.start_line - 1, 0) : unit.end_line])
        tokens = normalized_tokens(body)
        if len(tokens) >= policy.minimum_tokens:
            tokenized.append((unit, tokens))

    candidates: list[RepetitionCandidate] = []
    for left_index, (left, left_tokens) in enumerate(tokenized):
        for right, right_tokens in tokenized[left_index + 1 :]:
            ranges_overlap_by_containment = left.path == right.path and (
                (left.start_line <= right.start_line and left.end_line >= right.end_line)
                or (
                    right.start_line <= left.start_line
                    and right.end_line >= left.end_line
                )
            )
            if ranges_overlap_by_containment:
                continue
            sequence = difflib.SequenceMatcher(
                None, left_tokens, right_tokens, autojunk=False
            ).ratio()
            left_shingles = token_shingles(left_tokens)
            right_shingles = token_shingles(right_tokens)
            denominator = min(len(left_shingles), len(right_shingles))
            containment = (
                len(left_shingles & right_shingles) / denominator if denominator else 0.0
            )
            if max(sequence, containment) >= policy.warning_similarity:
                candidates.append(
                    RepetitionCandidate(
                        left=left.label(),
                        right=right.label(),
                        left_tokens=len(left_tokens),
                        right_tokens=len(right_tokens),
                        sequence_similarity=sequence,
                        shingle_containment=containment,
                    )
                )
    return sorted(candidates, key=lambda candidate: candidate.similarity(), reverse=True)


def nested_metric(unit: UnitMetric, metric: str, field: str) -> float:
    return as_number(as_mapping(unit.metrics.get(metric)).get(field))


def unit_document(unit: UnitMetric) -> dict[str, object]:
    return {
        "path": unit.path,
        "name": unit.name,
        "kind": unit.kind,
        "start_line": unit.start_line,
        "end_line": unit.end_line,
        "declarations": unit.declarations,
        "architectural_load": unit.architectural_load,
        "metrics": dict(unit.metrics),
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
) -> None:
    print(f"scope: {scope}")
    print(f"ranking: {RANKING_FORMULA}")
    print("hotspots (review leads; inspect raw metrics before changing code):")
    ranked = sorted(units, key=lambda unit: unit.architectural_load, reverse=True)
    for unit in ranked[:limit]:
        print(
            f"  {unit.architectural_load:7.1f}  {unit.path}:{unit.start_line} "
            f"{unit.kind} {unit.name} | "
            f"lloc={nested_metric(unit, 'loc', 'lloc'):g} "
            f"args={nested_metric(unit, 'nargs', 'total'):g} "
            f"decls={unit.declarations} "
            f"cyclo={nested_metric(unit, 'cyclomatic', 'value'):g} "
            f"cognitive={nested_metric(unit, 'cognitive', 'value'):g} "
            f"abc={nested_metric(unit, 'abc', 'value'):.1f}"
        )
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


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(argv)
    root = arguments.root.resolve()
    try:
        policy = load_policy(root)
        paths = select_paths(root, arguments.feature, arguments.path)
        if not paths:
            raise AnalysisError("the selected scope has no BCA-analyzable source files")
        units = collect_units(root, run_bca_metrics(root, paths), policy.ranking)
        if arguments.symbol is not None:
            units = [unit for unit in units if unit.name == arguments.symbol]
            if not units:
                raise AnalysisError(f"symbol not found in selected scope: {arguments.symbol}")
        candidates = find_repetition(root, units, policy.repetition)
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
            print(
                json.dumps(
                    {
                        "scope": scope,
                        "formula": RANKING_FORMULA,
                        "units": [unit_document(unit) for unit in units],
                        "repetition": [
                            repetition_document(candidate) for candidate in candidates
                        ],
                        "thresholds": {
                            "minimum_tokens": policy.repetition.minimum_tokens,
                            "warning_similarity": policy.repetition.warning_similarity,
                            "fail_similarity": policy.repetition.fail_similarity,
                        },
                        "violations": [
                            repetition_document(candidate) for candidate in violations
                        ],
                    },
                    indent=2,
                )
            )
        else:
            print_text(scope, units, candidates, arguments.limit)

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
