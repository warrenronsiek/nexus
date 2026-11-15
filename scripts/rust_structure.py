# @feature complexity-analysis
# @spec docs/features/complexity-analysis.md
# @boundary bca-node-output
"""Read Rust syntax evidence from the already-required BCA parser."""

from __future__ import annotations

import re
import subprocess
from dataclasses import astuple, dataclass
from pathlib import Path
from typing import Sequence


NODE_TYPES = (
    "function_item", "function_signature_item", "mutable_specifier",
    "assignment_expression", "compound_assignment_expr", "type_identifier",
    "scoped_type_identifier", "parameter", "self_parameter", "closure_expression",
    "function_type", "scoped_identifier", "impl_item", "trait_item",
)
NODE_LINE = re.compile(
    r"^╰─ \{([a-z_]+):\d+\} from \((\d+), (\d+)\) to \((\d+), (\d+)\)(?: :.*)? *$"
)
SPACE_KINDS = {"function_item": "function", "closure_expression": "function", "impl_item": "impl", "trait_item": "trait"}
STATE_TYPES = frozenset({
    "Cell", "RefCell", "UnsafeCell", "Mutex", "RwLock", "OnceCell", "OnceLock",
    "AtomicBool", "AtomicI8", "AtomicI16", "AtomicI32", "AtomicI64", "AtomicIsize",
    "AtomicU8", "AtomicU16", "AtomicU32", "AtomicU64", "AtomicUsize", "AtomicPtr",
})
DYNAMIC_TYPES = frozenset({
    "serde_json::Value", "toml::Value", "std::any::Any", "core::any::Any",
})


@dataclass(frozen=True)
class RustNode:
    kind: str
    start_line: int
    start_column: int
    end_line: int
    end_column: int

    def source(self, lines: Sequence[str]) -> str:
        selected = [line.encode("utf-8") for line in lines[self.start_line - 1 : self.end_line]]
        if not selected:
            return ""
        selected[-1] = selected[-1][: self.end_column - 1]
        selected[0] = selected[0][self.start_column - 1 :]
        return b"\n".join(selected).decode("utf-8")

    def contains(self, other: RustNode) -> bool:
        return (self.start_line, self.start_column) <= (other.start_line, other.start_column) and (self.end_line, self.end_column) >= (other.end_line, other.end_column)


@dataclass(frozen=True)
class RustSignals:
    functions: int = 0
    arguments: int = 0
    mutation_sites: int = 0
    state_types: int = 0
    dynamic_types: int = 0


def read_nodes(executable: str, root: Path, paths: Sequence[Path]) -> dict[Path, tuple[RustNode, ...]]:
    rust_paths = [path for path in paths if path.suffix == ".rs"]
    if not rust_paths:
        return {}
    command = [executable, "find", "--no-config", "--color", "never", "--paths-from", "-"]
    for kind in NODE_TYPES:
        command.extend(("-t", kind))
    completed = subprocess.run(
        command, cwd=root,
        input="\n".join(str(path.relative_to(root)) for path in rust_paths),
        capture_output=True, text=True, check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "bca find failed")
    return parse_nodes(completed.stdout, root, rust_paths)


def parse_nodes(output: str, root: Path, paths: Sequence[Path]) -> dict[Path, tuple[RustNode, ...]]:
    nodes: dict[Path, list[RustNode]] = {path: [] for path in paths}
    current: Path | None = None
    for line in output.splitlines():
        if not line.strip():
            continue
        if line.startswith("== ") and line.endswith(" =="):
            current = root / line[3:-3]
        else:
            if current not in nodes:
                raise RuntimeError("BCA node output has no recognized file heading")
            nodes[current].append(parse_node(line))
    return {path: tuple(items) for path, items in nodes.items()}


def parse_node(line: str) -> RustNode:
    match = NODE_LINE.fullmatch(line)
    if match is None:
        raise RuntimeError(f"unrecognized BCA node output: {line[:120]}")
    return RustNode(match[1], *(int(match[i]) for i in range(2, 6)))


def parameter_count(nodes: Sequence[RustNode]) -> int:
    owners = tuple(node for node in nodes if node.kind in {"function_item", "function_signature_item", "closure_expression", "function_type"})
    arguments = 0
    for parameter in (node for node in nodes if node.kind in {"parameter", "self_parameter"}):
        owner = min(
            (node for node in owners if node.contains(parameter)),
            key=lambda node: (node.end_line - node.start_line, node.end_column - node.start_column),
            default=None,
        )
        arguments += owner is not None and owner.kind in {"function_item", "function_signature_item"}
    return arguments


def is_state_type(node: RustNode, text: str) -> bool:
    if node.kind == "type_identifier":
        return text in STATE_TYPES
    parts = text.split("::")
    return node.kind == "scoped_identifier" and len(parts) >= 2 and parts[-2] in STATE_TYPES and parts[-1] in {"new", "default", "from"}


def node_signals(node: RustNode, lines: Sequence[str]) -> RustSignals:
    text = node.source(lines)
    return RustSignals(
        functions=int(node.kind in {"function_item", "function_signature_item"}),
        mutation_sites=int(node.kind in {"mutable_specifier", "assignment_expression", "compound_assignment_expr"}),
        state_types=int(is_state_type(node, text)),
        dynamic_types=int(node.kind == "scoped_type_identifier" and text.lstrip(":") in DYNAMIC_TYPES),
    )


def signals_in_span(nodes: Sequence[RustNode], lines: Sequence[str], start: int, end: int) -> RustSignals:
    selected = tuple(node for node in nodes if start <= node.start_line and node.end_line <= end)
    evidence = tuple(node_signals(node, lines) for node in selected)
    totals = [sum(values) for values in zip(*(astuple(item) for item in evidence))] if evidence else [0] * 5
    totals[1] = parameter_count(selected)
    return RustSignals(*totals)
