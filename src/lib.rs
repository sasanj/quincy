#[cfg(unix)]
use jemallocator::Jemalloc;

#[cfg(unix)]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

pub mod auth;
pub mod client;
pub mod config;
pub mod constants;
pub mod interface;
pub mod server;
pub mod utils;
