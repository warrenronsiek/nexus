# Functional Rust

Apply the architecture skill's value-oriented design using Rust's ordinary
ownership, enums, pattern matching, and iterator idioms. Keep the complete domain
statically typed while allowing useful local inference.

## Express transformations directly

- Prefer borrowed inputs, immutable bindings, and returned values. Consume an
  owned value when ownership transfer naturally describes the operation.
- Prefer `match` for domain alternatives and exhaustive enum dispatch. Use
  `if let` or `let ... else` for a single relevant pattern. Keep ordinary `if`
  for a real predicate when inventing an enum would add no domain meaning.
- Use `Option<T>` for absence and `Result<T, DomainError>` for failure. Prefer
  `?`, `map`, `and_then`, `map_or_else`, `ok_or_else`, or `transpose` as the
  operation requires. For example, `find_user(id).ok_or(Error::MissingUser)?`
  expresses a required lookup without `is_some()` followed by `unwrap()`.
  Do not replace a deliberate error with a default merely to shorten the code.
- Prefer `iter`/`into_iter` with `map`, `filter`, `filter_map`, `collect`, `sum`,
  `reduce`, `fold`, or `try_fold` over `for`, `while`, and `loop` with an
  accumulator. Use recursion for naturally recursive data when depth is bounded.
  Keep streaming, allocation, early-exit, and stack constraints intact.
- A `.for_each` closure mutating captured state is still imperative. Do not
  replace a linear owned loop with a fold that repeatedly clones or reallocates
  its accumulator. Iterators themselves advance internal state; the objective
  is clear domain reasoning, not machine-level purity.

## Give choices and fields names

Use named structs for domain fields, enums for alternatives, and newtypes when
identities, units, or validated values must not be interchangeable. A genuinely
dynamic-key collection with a concrete value type is valid; a
`HashMap<String, serde_json::Value>`, raw JSON value, or `Box<dyn Any>` is not a
substitute for an enumerated domain model. Decode those representations at a
named adapter boundary.

Replace boolean behavior arguments with a named enum and match on that choice:

```rust
enum CachePolicy { UseStored, Refresh }

let content = match policy {
    CachePolicy::UseStored => read_cached(key)?,
    CachePolicy::Refresh => fetch_and_store(key)?,
};
```

The caller passes `CachePolicy::Refresh` instead of an unexplained `true`.
Keep boolean predicates and boolean domain facts when they are intrinsically
binary; decode external flags before invoking domain operations.

Keep item signatures explicit. Local inference such as `let count = items.len()`
or `collect::<Vec<_>>()` still resolves static types. Prefer ordinary functions
and static dispatch; introduce generics, traits, lifetimes, or `dyn` only for an
actual contract or variation point. Do not curry arguments or add builders,
macros, or fluent layers merely to resemble Clojure.

## Keep required effects honest

Avoid `Cell`, `RefCell`, mutexes, atomics, and shared ownership as substitutes for
passing values. Retain them when a real resource or concurrency invariant needs
them, with ownership and transitions explicit. Do not erase a transaction,
cleanup path, or protocol state to make an implementation appear stateless.

Preserve explicit records, typed errors, and exhaustive matches even when they
add syntax. Review score changes within the same behavioral scope; do not hide
inputs in captures, replace records with tuples, compress lines, move work into
macros, or merge unrelated operations to improve a number.

When semantics affect a design decision, consult the official Rust documentation
on [iterators](https://doc.rust-lang.org/book/ch13-02-iterators.html),
[type inference](https://doc.rust-lang.org/reference/types/inferred.html), and
[interior mutability](https://doc.rust-lang.org/reference/interior-mutability.html).
