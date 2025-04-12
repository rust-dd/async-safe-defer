//! This library provides two implementations of RAII-style deferred execution:
//! one using dynamic allocation (the default) and one that avoids allocation
//! entirely (`no_alloc`), with a fixed-capacity array of deferred function pointers.

#![cfg_attr(not(test), no_std)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

/// RAII-style guard for executing a closure at the end of a scope.
///
/// This synchronous variant does not require async. Inspired by Go's `defer`.
#[must_use = "Defer must be stored in a variable to execute the closure"]
pub fn defer<F>(f: F) -> impl Drop
where
    F: FnOnce(),
{
    /// Inner type that holds the closure until drop.
    struct Defer<F: FnOnce()> {
        f: Option<F>,
    }

    impl<F: FnOnce()> Drop for Defer<F> {
        fn drop(&mut self) {
            if let Some(f) = self.f.take() {
                f();
            }
        }
    }

    Defer { f: Some(f) }
}

/// Macro for creating a synchronous defer guard.
#[macro_export]
macro_rules! defer {
    ($e:expr) => {
        let _guard = $crate::defer(|| $e);
        let _ = &_guard;
    };
}

/// An async-aware scope guard for storing and running deferred async closures.
///
/// This version uses dynamic allocation with `Vec<Box<...>>`. It requires the
/// global allocator (heap).
pub struct AsyncScope {
    defer: Vec<Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> + 'static>>,
}

impl AsyncScope {
    /// Creates a new `AsyncScope` for collecting async deferred tasks.
    pub fn new() -> Self {
        AsyncScope { defer: Vec::new() }
    }

    /// Registers an async closure that will be executed when `run()` is called.
    ///
    /// Deferred tasks are run in reverse order (LIFO).
    pub fn defer<F>(&mut self, f: F)
    where
        F: FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> + 'static,
    {
        self.defer.push(Box::new(move || Box::pin(f())));
    }

    /// Executes all stored async tasks in reverse (stack-like) order.
    pub async fn run(mut self) {
        while let Some(f) = self.defer.pop() {
            f().await;
        }
    }
}

/// Macro that creates an async scope to automatically await all async defers.
///
/// Inside this block, you can register async cleanup tasks by calling
/// `scope.defer(...)`, and they will be executed (awaited) at the end of the block
/// in reverse order.
#[macro_export]
macro_rules! async_scope {
    ($scope:ident, $body:block) => {
        async {
            let mut $scope = $crate::AsyncScope::new();
            $body
            $scope.run().await;
        }
    };
}

/// A module providing a no-alloc implementation of a fixed-capacity async scope.
///
/// This variant does not rely on dynamic allocation. It stores `'static` function
/// pointers in a fixed-size array and executes them in LIFO order. Each deferred
/// function pointer must return a `Pin<Box<dyn Future<Output=()> + 'static>>`.
#[cfg(any(feature = "no_alloc", test))]
pub mod no_alloc {
    use alloc::boxed::Box;
    use core::{future::Future, pin::Pin};

    /// Type alias for a `'static` function pointer returning a pinned async future.
    pub type DeferredFn = fn() -> Pin<Box<dyn Future<Output = ()> + 'static>>;

    /// A fixed-capacity async scope that does not use dynamic allocation.
    ///
    /// - `N` is the maximum number of deferred tasks.
    /// - Each deferred function must be a `'static` function pointer.
    pub struct AsyncScopeNoAlloc<const N: usize> {
        tasks: [Option<DeferredFn>; N],
        len: usize,
    }

    impl<const N: usize> AsyncScopeNoAlloc<N> {
        /// Creates a new `AsyncScopeNoAlloc` with a capacity of `N`.
        pub const fn new() -> Self {
            Self {
                tasks: [None; N],
                len: 0,
            }
        }

        /// Registers a `'static` function pointer to be called later.
        ///
        /// Panics if the capacity `N` is exceeded.
        pub fn defer(&mut self, f: DeferredFn) {
            if self.len >= N {
                panic!("No space left for more tasks.");
            }
            self.tasks[self.len] = Some(f);
            self.len += 1;
        }

        /// Executes all stored tasks in reverse order, awaiting each one.
        pub async fn run(&mut self) {
            while self.len > 0 {
                self.len -= 1;
                let task = self.tasks[self.len].take().unwrap();
                (task)().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use self::std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use super::*;

    #[test]
    fn test_sync_defer() {
        println!("test_sync_defer start");
        let val = Arc::new(AtomicUsize::new(0));
        {
            println!("in scope, val={}", val.load(Ordering::SeqCst));
            let v = val.clone();
            defer!(v.store(42, Ordering::SeqCst));
        }
        println!("out of scope, val={}", val.load(Ordering::SeqCst));
        assert_eq!(val.load(Ordering::SeqCst), 42);
    }

    #[tokio::test]
    async fn test_async_scope_order() {
        println!("test_async_scope_order start");
        let log = Arc::new(Mutex::new(Vec::new()));
        {
            let mut scope = AsyncScope::new();

            let l1 = log.clone();
            scope.defer(move || {
                println!("push(1) scheduled");
                let l1 = l1.clone();
                Box::pin(async move {
                    println!("push(1) running");
                    l1.lock().unwrap().push(1);
                })
            });

            let l2 = log.clone();
            scope.defer(move || {
                println!("push(2) scheduled");
                let l2 = l2.clone();
                Box::pin(async move {
                    println!("push(2) running");
                    l2.lock().unwrap().push(2);
                })
            });

            scope.run().await;
        }
        let result = log.lock().unwrap().clone();
        println!("final log: {:?}", result);
        assert_eq!(result, vec![2, 1]);
    }

    #[tokio::test]
    async fn test_async_scope_macro() {
        println!("test_async_scope_macro start");
        use crate::async_scope;
        let flag = Arc::new(AtomicUsize::new(0));
        {
            let f = Arc::clone(&flag);
            async_scope!(scope, {
                let f2 = Arc::clone(&f);
                scope.defer(move || {
                    println!("store(1) scheduled");
                    Box::pin(async move {
                        println!("store(1) running");
                        f2.store(1, Ordering::SeqCst);
                    })
                });
                println!("in scope, flag={}", f.load(Ordering::SeqCst));
            })
            .await;
        }
        println!("out of scope, flag={}", flag.load(Ordering::SeqCst));
        assert_eq!(flag.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "no_alloc")]
    #[tokio::test]
    async fn test_no_alloc_scope() {
        println!("test_no_alloc_scope start");
        use super::no_alloc::{AsyncScopeNoAlloc, DeferredFn};
        use core::future::Future;
        use core::pin::Pin;

        fn task_one() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
            Box::pin(async {
                println!("task_one running");
            })
        }
        fn task_two() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
            Box::pin(async {
                println!("task_two running");
            })
        }

        let mut scope = AsyncScopeNoAlloc::<2>::new();
        scope.defer(task_one as DeferredFn);
        scope.defer(task_two as DeferredFn);
        scope.run().await;
    }
}
