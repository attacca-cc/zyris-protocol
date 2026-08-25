# zyris-macros

The `#[zyris::capability]` proc-macro for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol.

**Do not depend on this crate directly.** It is re-exported by
[`zyris`](https://crates.io/crates/zyris) (and by [`zyris-core`](https://crates.io/crates/zyris-core)),
and the code it generates names paths inside that crate — reaching for the macro on its own gives
you an attribute whose output will not resolve.

One trait becomes the descriptor, the server wrapper, and the client:

```rust
#[zyris::capability(name = "hello", version = 1)]
pub trait Hello {
    /// Return a random friendly greeting, optionally addressed to `name`.
    async fn greet(&self, name: Option<String>) -> zyris::Result<Greeting>;
}
```

Doc comments become the tool and field descriptions a model reads, so write them for the model.

## License

MIT or Apache-2.0, at your option.
