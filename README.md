# async-safe-defer

Minimal async- and sync-capable `defer` crate with:

- ✅ async support
- ✅ no `unsafe` code
- ✅ `no_std` + `alloc` compatible
- ✅ optional `no_alloc` mode
- ✅ zero dependencies

Inspired by [`defer`](https://crates.io/crates/defer), but designed for embedded and async contexts.

## Usage

### Sync
```rust
use async_defer::defer;

fn main() {
    defer!(println!("cleanup"));
    println!("work");
}
```

### Async
```rust
use async_defer::async_scope;

async_scope!(scope, {
    scope.defer(|| async { println!("async cleanup") });
    println!("async work");
}).await;
```

### No-alloc
```rust
use async_defer::no_alloc::AsyncScopeNoAlloc;

fn task() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
    Box::pin(async { println!("no_alloc") })
}

let mut scope = AsyncScopeNoAlloc::<2>::new();
scope.defer(task);
scope.run().await;
```
