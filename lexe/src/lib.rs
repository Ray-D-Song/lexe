#[cfg(not(target_family = "wasm"))]
pub mod executable;
#[cfg(not(target_family = "wasm"))]
pub mod upgrade;
