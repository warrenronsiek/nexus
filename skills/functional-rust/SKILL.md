---
name: functional-rust
description: Write, refactor, or review Rust using immutable data transformations, explicit domain types, concise idioms, and deep interfaces. Use for Rust implementation and design work; preserve required effects and performance constraints rather than mechanically replacing every mutable operation.
---

# Functional Rust

Make the program read as typed values flowing through functions. Favor Rich
Hickey's separation of independent concerns and John Ousterhout's deep
interfaces, while using Rust's static types to document the complete domain.
Preserve behavior and the caller's requested scope.

## Values before state

- Write domain decisions as functions from explicit inputs to returned values.
  Prefer immutable bindings, borrowed inputs, owned outputs, and composition.
- Prefer expressions: `match`, `if`, iterator adapters, `collect`, `sum`, `?`,
  and `Option`/`Result` combinators when they make the transformation apparent.
  Use exhaustive matching for meaningful alternatives; avoid deeply nested
  combinator chains that conceal the decision.
- Return a new domain value instead of modifying a caller's object. Consume an
  owned value when ownership transfer naturally describes the operation.
- Keep I/O, persistence, clocks, randomness, synchronization, and framework
  objects at effect boundaries. Pass the resulting facts into domain decisions.
  A direct adapter call is enough; do not invent a trait or interpreter layer
  for every effect.
- Avoid mutable globals, interior mutability, shared ownership, and stateful
  managers. Do not turn a transformation into an object with `new`, setters,
  `run`, and `result`, or introduce required call ordering.
- Model lifecycle state only when the domain or external protocol actually has
  one. Represent that state as an explicit value with valid transitions; do not
  erase required transactions, resource cleanup, concurrency, or ordering.
- Keep unavoidable mutation narrow and owned by its effect or resource
  boundary. Explain the concrete constraint when retaining it. Do not hide it
  in `Cell`, `RefCell`, a mutex, atomics, a mutating closure, or an opaque helper
  to make the syntax look functional.
- Do not introduce repeated cloning, extra collections, unbounded recursion,
  or slower asymptotic work just to remove `mut`. Rust iterators themselves
  advance internal state; the goal is simple domain reasoning and explicit
  ownership, not a claim of machine-level purity.

## Types carry the documentation

- Enumerate domain fields in named structs and alternatives in enums. Use
  newtypes when they prevent mixing distinct identities, units, or validated
  values. Prefer small domain-specific inputs and results over framework types.
- Reject string-to-`any` equivalents such as `HashMap<String, serde_json::Value>`,
  a generic JSON value, or `Box<dyn Any>` as the domain model. A map with genuinely
  dynamic keys and a concrete value type can be a valid collection.
- Decode external input once into typed values. Keep intentionally open-ended
  extension data in a named adapter boundary and preserve the external contract.
- Use `Option` for absence and typed `Result` errors for failure. Prefer enums
  over boolean arguments that select behavior. Eliminate invalid combinations
  with a sum type instead of parallel flags or unrelated optional fields.
- Fully specified domain shapes do not require redundant local annotations.
  Rust inference resolves concrete static types; `let count = items.len()` and
  `collect::<Vec<_>>()` do not introduce dynamic typing. Keep item signatures
  explicit and local annotations useful to the reader.
- Prefer static dispatch and ordinary functions. Introduce generics, traits,
  lifetimes, or dynamic dispatch when they express an actual contract or
  variation point, rather than speculative extensibility.

## Concision with depth

- Expose one coherent operation that accepts the information its caller
  naturally owns and completes meaningful work. Count parameters, configuration,
  intermediate values the caller must manage, and call ordering as interface
  cost. An options struct earns its place by representing a real concept, not
  by disguising a long parameter list.
- Keep cohesive implementation behind that small interface. A longer function
  can be simpler than several forwarding functions. Extract for a stable domain
  concept, a genuine effect boundary, or shared invariants; do not extract merely
  to shorten a function or satisfy a line-count metric.
- Compose ordinary functions and standard iterators before creating custom
  combinators, builders, macros, fluent APIs, or a framework. Do not introduce
  one-use abstractions or curry arguments merely to resemble another language.
- Reuse a shared implementation when flow, policy, errors, and reasons to change
  agree. Keep separate implementations when their invariants differ.
- Keep names and types informative. Comments should capture rationale and
  constraints that the code cannot express, rather than narrating each step.

## Review the tradeoff, not just the score

Compare the same behavioral scope before and after a change. Prefer fewer named
callable declarations, arguments, mutable places, hidden dependencies, and
untyped boundaries when they reduce total reader and caller burden. Named
callable declarations and arguments should cost more than a logical
implementation line, so splitting a cohesive operation into thin wrappers is
not an automatic improvement.

Treat structural scores as review evidence. A named struct can add declarations
while removing ambiguity; exhaustive matching can add branches while preventing
invalid states. Preserve those benefits. Do not lower a score by removing types,
packing fields into tuples, hiding inputs in captured state, compressing lines,
moving work into macros, or merging unrelated responsibilities. State any
necessary tradeoff and verify the supported behavior with the repository's
normal checks.

For Rust semantics, consult the official documentation on
[iterators](https://doc.rust-lang.org/book/ch13-02-iterators.html),
[type inference](https://doc.rust-lang.org/reference/types/inferred.html), and
[interior mutability](https://doc.rust-lang.org/reference/interior-mutability.html)
when those details affect a design decision.
