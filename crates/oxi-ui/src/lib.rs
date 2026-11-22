//! `oxi-ui`: ensamblado del simulador e interfaces (CLI y GUI opcional).
//!
//! El runtime glue conecta `oxi-net` ↔ `oxi-engine` ↔ teclado/pantalla con
//! canales de tokio (DESIGN §6): el engine sigue siendo puro; este crate es
//! el único que toca todo.

pub mod config;
#[cfg(feature = "gui")]
pub mod gui;
pub mod gui_state;
pub mod runtime;
pub mod screen;

pub use config::SimConfig;
pub use gui_state::GuiState;
pub use runtime::{spawn, RuntimeHandle, UiUpdate};
