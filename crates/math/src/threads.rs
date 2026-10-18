//! How many threads the machine offers, the one place that asks.

/// The number of threads this machine runs at once: what the standard
/// library reports, 1 where it cannot tell or where there are no threads
/// (wasm). Every caller caps it to what its own work wants; the results
/// are the same for any count (AGENTS.md rule 5).
pub fn available_threads() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    }
}
