//! Engine-neutral building blocks for applications composed from WebAssembly components.
//!
//! The API is intentionally small while the framework is under active development.
//!
//! ```
//! use wasm_junction as _;
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod call;
mod values;

pub use call::{Call, Caller, Trap, TypedCall};
pub use values::{TypeError, Val, Vals};
