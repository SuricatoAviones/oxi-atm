//! `oxi-ui`: ensamblado del simulador e interfaz (CLI en v1).
//!
//! El runtime glue conecta `oxi-net` ↔ `oxi-engine` ↔ teclado/pantalla con
//! canales de tokio (DESIGN §6): el engine sigue siendo puro; este crate es
//! el único que toca todo.

pub mod config;
pub mod runtime;
pub mod screen;

pub use config::SimConfig;
pub use runtime::{spawn, RuntimeHandle, UiUpdate};
