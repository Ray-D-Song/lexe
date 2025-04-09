// export build for test
pub mod build {
    include!("build.rs");
}

// re-export core functionality
pub use crate::build::*;