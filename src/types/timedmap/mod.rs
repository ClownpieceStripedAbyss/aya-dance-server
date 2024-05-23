///! forked from https://github.com/zekroTJA/timedmap-rs,
/// with tokio optimizations, less type restrictions,
/// and more features.
mod timedmap;
pub use timedmap::*;

mod value;
pub use value::*;

mod tokio_cleaner;
pub use tokio_cleaner::*;

pub mod time;
