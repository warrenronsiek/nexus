# @feature complexity-analysis
# @spec docs/features/complexity-analysis.md
# @boundary bca-node-output
"""Read Rust syntax evidence from the already-required BCA parser."""

from __future__ import annotations

import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Sequence


NODE_TYPES = (
    "function_item", "function_signature_item", "mutable_specifier",
    "assignment_expression", "compound_assignment_expr", "type_identifier",
    "scoped_type_identifier", "parameter", "self_parameter", "closure_expression",
    "function_type", "scoped_identifier", "impl_item", "trait_item",
    "use_declaration", "mod_item", "block", "struct_item", "enum_item",
    "type_item", "type_parameter", "line_comment", "block_comment",
    "if_expression", "let_condition", "for_expression", "while_expression", "loop_expression",
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
class SourceSpan:
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

    def contains(self, other: SourceSpan) -> bool:
        return (self.start_line, self.start_column) <= (other.start_line, other.start_column) and (self.end_line, self.end_column) >= (other.end_line, other.end_column)

    @classmethod
    def whole_lines(cls, start: int, end: int) -> SourceSpan:
        return cls(start, 1, end, sys.maxsize)


@dataclass(frozen=True)
class RustNode:
    kind: str
    span: SourceSpan


@dataclass(frozen=True)
class RustSignals:
    functions: int = 0
    arguments: int = 0
    mutation_sites: int = 0
    state_types: int = 0
    dynamic_types: int = 0
    ordinary_conditionals: int = 0
    imperative_loops: int = 0
    boolean_parameters: int = 0


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
    return RustNode(match[1], SourceSpan(*(int(match[i]) for i in range(2, 6))))


@dataclass(frozen=True)
class TypeBinding:
    name: str
    qualified: str | None
    scope: SourceSpan


@dataclass(frozen=True)
class TypeNames:
    bindings: tuple[TypeBinding, ...] = ()
    qualified: tuple[SourceSpan, ...] = ()

    @classmethod
    def from_source(cls, nodes: Sequence[RustNode], lines: Sequence[str]) -> TypeNames:
        imports = tuple(node for node in nodes if node.kind == "use_declaration")
        declared = filter(None, (local_binding(node, nodes, lines) for node in nodes if node.kind in {"struct_item", "enum_item", "trait_item", "type_item", "type_parameter"}))
        return cls(
            bindings=tuple(binding for node in imports for binding in imported_bindings(node, nodes, lines)) + tuple(declared),
            qualified=tuple(node.span for node in nodes if node.kind == "scoped_type_identifier"),
        )

    def is_dynamic(self, node: RustNode, text: str) -> bool:
        if node.kind not in {"type_identifier", "scoped_type_identifier"}:
            return False
        if node.kind == "type_identifier" and any(span.contains(node.span) for span in self.qualified):
            return False
        parts = text.lstrip(":").split("::")
        matches = (binding for binding in self.bindings if binding.name == parts[0] and binding.scope.contains(node.span))
        binding = min(matches, key=lambda item: span_size(item.scope), default=None)
        if binding is None:
            return text.lstrip(":") in DYNAMIC_TYPES
        return binding.qualified is not None and "::".join((binding.qualified, *parts[1:])) in DYNAMIC_TYPES


def span_size(span: SourceSpan) -> tuple[int, int]:
    return span.end_line - span.start_line, span.end_column - span.start_column


def binding_scope(node: RustNode, nodes: Sequence[RustNode], line_count: int) -> SourceSpan:
    kinds = {"block", "mod_item"}
    if node.kind == "type_parameter":
        kinds |= {"function_item", "function_signature_item", "struct_item", "enum_item", "trait_item", "impl_item"}
    scopes = (item.span for item in nodes if item.kind in kinds and item.span.contains(node.span))
    return min(scopes, key=span_size, default=SourceSpan.whole_lines(1, line_count))


def local_binding(node: RustNode, nodes: Sequence[RustNode], lines: Sequence[str]) -> TypeBinding | None:
    names = (item for item in nodes if item.kind == "type_identifier" and node.span.contains(item.span))
    name = next(names, None)
    return None if name is None else TypeBinding(name.span.source(lines), None, binding_scope(node, nodes, len(lines)))


def uncommented_source(node: RustNode, nodes: Sequence[RustNode], lines: Sequence[str]) -> str:
    text = node.span.source(lines)
    for comment in (item for item in nodes if item.kind in {"line_comment", "block_comment"} and node.span.contains(item.span)):
        text = text.replace(comment.span.source(lines), " ")
    return text


def imported_bindings(node: RustNode, nodes: Sequence[RustNode], lines: Sequence[str]) -> tuple[TypeBinding, ...]:
    text = uncommented_source(node, nodes, lines)
    tokens = re.findall(r"(?:r#)?\w+|::|[{},*]", text)
    scope = binding_scope(node, nodes, len(lines))
    return tuple(TypeBinding(name, path, scope) for name, path in import_bindings(iter(tokens[tokens.index("use") + 1 :])))


def import_leaf(path: tuple[str, ...], alias: str | None = None) -> tuple[tuple[str, str], ...]:
    if not path or path[-1] == "*":
        return ()
    qualified = path[:-1] if path[-1] == "self" else path
    return ((alias or qualified[-1], "::".join(qualified)),)


def import_bindings(tokens: Iterator[str], prefix: tuple[str, ...] = ()) -> tuple[tuple[str, str], ...]:
    bindings: list[tuple[str, str]] = []
    path: tuple[str, ...] | None = None
    for token in tokens:
        pending = path or ()
        if token == "}":
            return (*bindings, *import_leaf(pending))
        if token == ",":
            bindings.extend(import_leaf(pending))
            path = None
        elif token == "{":
            bindings.extend(import_bindings(tokens, pending or prefix))
            path = None
        elif token == "as":
            bindings.extend(import_leaf(pending, next(tokens)))
            path = None
        elif token != "::":
            path = (*prefix, token) if path is None else (*path, token)
    return (*bindings, *import_leaf(path or ()))


def enclosing_node(node: RustNode, owners: Sequence[RustNode]) -> RustNode | None:
    return min((owner for owner in owners if owner.span.contains(node.span)), key=lambda owner: span_size(owner.span), default=None)


def named_parameters(nodes: Sequence[RustNode]) -> tuple[RustNode, ...]:
    owners = tuple(node for node in nodes if node.kind in {"function_item", "function_signature_item", "closure_expression", "function_type"})
    return tuple(
        node for node in nodes if node.kind in {"parameter", "self_parameter"}
        if (owner := enclosing_node(node, owners)) is not None and owner.kind in {"function_item", "function_signature_item"}
    )


def pattern_conditionals(nodes: Sequence[RustNode]) -> frozenset[SourceSpan]:
    owners = tuple(node for node in nodes if node.kind in {"if_expression", "while_expression", "block", "closure_expression"})
    return frozenset(
        owner.span for node in nodes if node.kind == "let_condition"
        if (owner := enclosing_node(node, owners)) is not None and owner.kind == "if_expression"
    )


def is_boolean_parameter(node: RustNode, nodes: Sequence[RustNode], lines: Sequence[str]) -> bool:
    parameter_type = re.split(r"(?<!:):(?!:)", uncommented_source(node, nodes, lines), maxsplit=1)[-1].strip()
    return re.fullmatch(r"(?:&\s*(?:'\w+\s*)?(?:mut\s+)?)*bool", parameter_type) is not None


def is_state_type(node: RustNode, text: str) -> bool:
    if node.kind == "type_identifier":
        return text in STATE_TYPES
    parts = text.split("::")
    return node.kind == "scoped_identifier" and len(parts) >= 2 and parts[-2] in STATE_TYPES and parts[-1] in {"new", "default", "from"}


def node_signals(node: RustNode, lines: Sequence[str], names: TypeNames, patterns: frozenset[SourceSpan]) -> RustSignals:
    text = node.span.source(lines)
    return RustSignals(
        functions=int(node.kind in {"function_item", "function_signature_item"}),
        mutation_sites=int(node.kind in {"mutable_specifier", "assignment_expression", "compound_assignment_expr"}),
        state_types=int(is_state_type(node, text)),
        dynamic_types=int(names.is_dynamic(node, text)),
        ordinary_conditionals=int(node.kind == "if_expression" and node.span not in patterns),
        imperative_loops=int(node.kind in {"for_expression", "while_expression", "loop_expression"}),
    )


def signals_in_span(nodes: Sequence[RustNode], lines: Sequence[str], span: SourceSpan, names: TypeNames) -> RustSignals:
    selected = tuple(node for node in nodes if span.contains(node.span))
    parameters = named_parameters(selected)
    patterns = pattern_conditionals(selected)
    evidence = tuple(node_signals(node, lines, names, patterns) for node in selected)
    return RustSignals(
        functions=sum(item.functions for item in evidence),
        arguments=len(parameters),
        mutation_sites=sum(item.mutation_sites for item in evidence),
        state_types=sum(item.state_types for item in evidence),
        dynamic_types=sum(item.dynamic_types for item in evidence),
        ordinary_conditionals=sum(item.ordinary_conditionals for item in evidence),
        imperative_loops=sum(item.imperative_loops for item in evidence),
        boolean_parameters=sum(is_boolean_parameter(node, selected, lines) for node in parameters),
    )
