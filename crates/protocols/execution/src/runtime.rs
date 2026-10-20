//! Bridge sync [`crate::venue::ExecutionVenue`] methods to async HTTP.
//!
//! On a multi-thread Tokio runtime (order-gateway), [`tokio::task::block_in_place`]
//! keeps the worker pool healthy while the async client runs. Elsewhere (unit
//! tests, current-thread runtimes) work is done on a dedicated OS thread so we
//! never nest `block_on` or panic on `block_in_place`.

use std::future::Future;

use tokio::runtime::{Builder, Handle, RuntimeFlavor};

/// Drive an async venue HTTP future from sync trait methods.
pub(crate) fn block_on_venue<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    if let Ok(handle) = Handle::try_current() {
        if handle.runtime_flavor() == RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| handle.block_on(fut));
        }
    }

    std::thread::spawn(move || {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime for venue HTTP")
            .block_on(fut)
    })
    .join()
    .expect("venue HTTP thread panicked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_without_runtime() {
        let n = block_on_venue(async { 7_u32.saturating_add(1) });
        assert_eq!(n, 8);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn block_on_inside_multi_thread_runtime() {
        let n = block_on_venue(async { 40_u32.saturating_add(2) });
        assert_eq!(n, 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_on_inside_current_thread_runtime() {
        let n = block_on_venue(async { String::from("ok") });
        assert_eq!(n, "ok");
    }
}
