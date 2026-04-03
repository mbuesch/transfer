# File Transfer App

## Implementation process

- When implementing, always think carefully and do not take shortcuts.
- When unsure, ask rather than guess.
- Never use the character `—`; use `-` instead.
- Use `vec![]` instead of `Vec::new()`.
- When cloning an `Arc`, use `Arc::clone(&arc)` instead of `arc.clone()`.
- When using multiple items from the same crate, prefer a single `use` statement with curly braces.
- `mod` statements shall come after `use` statements.
- `use` statements shall be at the top of the file.
- Generally avoid `unwrap()`.
- When unwrapping a lock guard, use `expect("Lock poisoned")` instead of `unwrap()`.
- Prefer `use` statements to bring crate paths into scope rather than using crate-root paths directly.
- Always run `cargo clippy` after making changes and address warnings; prefer clippy over relying solely on `cargo build`.
- After changing code, run `cargo fmt` to ensure consistent formatting.
- After changing Dioxus code, run `dx fmt` to ensure consistent formatting.

## Crate versions

- Use the latest crate versions from crates.io.
