//! Paleta y estilo de la GUI (fase 9).
//!
//! Colores fijos, sin assets externos: la ventana se ve igual en Windows y
//! bajo WSLg, y no depende de imágenes ni fuentes del sistema (egui trae sus
//! fuentes por defecto).

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Ui};

/// Fondo general de la ventana.
pub const BG: Color32 = Color32::from_rgb(13, 15, 19);
/// Fondo de paneles.
pub const PANEL: Color32 = Color32::from_rgb(22, 25, 31);
/// Gabinete del cajero.
pub const CABINET: Color32 = Color32::from_rgb(30, 34, 42);
/// Bordes y separadores.
pub const BORDER: Color32 = Color32::from_rgb(58, 64, 76);
/// Fondo de la pantalla del cajero.
pub const SCREEN_BG: Color32 = Color32::from_rgb(7, 24, 45);
/// Texto de la pantalla del cajero.
pub const SCREEN_TEXT: Color32 = Color32::from_rgb(125, 220, 255);
/// Texto secundario.
pub const MUTED: Color32 = Color32::from_rgb(148, 156, 170);
/// Acento de la marca.
pub const ACCENT: Color32 = Color32::from_rgb(255, 170, 60);
/// Estado correcto / conectado.
pub const OK: Color32 = Color32::from_rgb(70, 190, 110);
/// Advertencia / intermedio.
pub const WARN: Color32 = Color32::from_rgb(230, 175, 65);
/// Error / cancelar.
pub const ERR: Color32 = Color32::from_rgb(225, 95, 95);

/// Aplica la paleta y el espaciado a un contexto egui.
pub fn setup(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = SCREEN_BG;
    visuals.override_text_color = Some(Color32::from_rgb(226, 229, 236));
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.animation_time = 0.08;
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.corner_radius = CornerRadius::same(5);
        }
    });
}

/// Etiqueta compacta de estado (conexión, modo, sesión).
pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.18))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color).strong());
        });
}
