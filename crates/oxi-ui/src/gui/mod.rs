//! Interfaz gráfica de oxi (fase 9), detrás de la feature `gui`.
//!
//! Comparte el runtime y el perfil con la CLI; el motor y `oxi-core` siguen
//! sin depender de ninguna biblioteca de ventanas (ARCHITECTURE §6).

mod app;
mod theme;

pub use app::run;
