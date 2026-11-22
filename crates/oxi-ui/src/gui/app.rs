//! Ventana principal de la GUI (fase 9): gabinete, pantalla, teclado y logs.
//!
//! egui es immediate-mode: el estado vive en `GuiState` y en el perfil, y
//! cada frame se redibuja. El runtime de tokio sigue corriendo en sus hilos;
//! la GUI sólo drena `UiUpdate` y envía eventos con `try_send`.

use std::time::Duration;

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke, Vec2};
use oxi_core::{Fdk, OperationalMode};
use oxi_engine::{Event, Key as AtmKey};
use oxi_net::ConnectionState;
use tokio::sync::mpsc::error::TryRecvError;

use crate::config::SimConfig;
use crate::gui::theme;
use crate::gui_state::GuiState;
use crate::runtime::{card_event, RuntimeHandle};

/// FDKs del lado izquierdo del gabinete.
const FDKS_LEFT: [Fdk; 4] = [Fdk::A, Fdk::B, Fdk::C, Fdk::D];
/// FDKs del lado derecho del gabinete.
const FDKS_RIGHT: [Fdk; 5] = [Fdk::E, Fdk::F, Fdk::G, Fdk::H, Fdk::I];
/// Teclas físicas que equivalen al teclado numérico.
const DIGIT_KEYS: [(egui::Key, char); 10] = [
    (egui::Key::Num0, '0'),
    (egui::Key::Num1, '1'),
    (egui::Key::Num2, '2'),
    (egui::Key::Num3, '3'),
    (egui::Key::Num4, '4'),
    (egui::Key::Num5, '5'),
    (egui::Key::Num6, '6'),
    (egui::Key::Num7, '7'),
    (egui::Key::Num8, '8'),
    (egui::Key::Num9, '9'),
];
/// Teclas físicas F1–F9 que equivalen a las FDK A–I.
const FDK_KEYS: [(egui::Key, Fdk); 9] = [
    (egui::Key::F1, Fdk::A),
    (egui::Key::F2, Fdk::B),
    (egui::Key::F3, Fdk::C),
    (egui::Key::F4, Fdk::D),
    (egui::Key::F5, Fdk::E),
    (egui::Key::F6, Fdk::F),
    (egui::Key::F7, Fdk::G),
    (egui::Key::F8, Fdk::H),
    (egui::Key::F9, Fdk::I),
];

/// Arranca el runtime y abre la ventana. Bloquea hasta que el usuario la cierra.
pub fn run(config: SimConfig) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("oxi-runtime")
        .build()
        .map_err(|err| anyhow::anyhow!("no se pudo crear el runtime de tokio: {err}"))?;
    // `spawn` usa tokio::spawn: hay que entrar al contexto antes de llamarlo.
    let session = {
        let _enter = runtime.enter();
        crate::runtime::spawn(&config)
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("oxi · simulador NDC")
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([980.0, 680.0]),
        renderer: eframe::Renderer::Glow,
        ..eframe::NativeOptions::default()
    };

    eframe::run_native(
        "oxi",
        options,
        Box::new(move |cc| Ok(Box::new(OxiApp::new(cc, config, runtime, session)))),
    )
    .map_err(|err| anyhow::anyhow!("no se pudo abrir la ventana: {err}"))
}

/// Aplicación egui con la sesión de oxi.
struct OxiApp {
    // El orden importa: la sesión se apaga antes de soltar el runtime.
    session: Option<RuntimeHandle>,
    runtime: tokio::runtime::Runtime,
    state: GuiState,
    config: SimConfig,
    selected_card: usize,
}

impl OxiApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        config: SimConfig,
        runtime: tokio::runtime::Runtime,
        session: RuntimeHandle,
    ) -> Self {
        theme::setup(&cc.egui_ctx);
        let mut state = GuiState::default();
        state.push_log(format!(
            "oxi GUI lista; conectando a {} ...",
            config.host_addr()
        ));
        state.push_log("atajos: F1–F9 = FDK A–I · Enter · Backspace = CLR · Esc = CAN");
        state.push_log("atajos: Ctrl+I = insertar tarjeta · Ctrl+E = retirar efectivo");
        state.push_log("atajos: Ctrl+R = reconectar cuando la sesión esté cerrada");
        Self {
            session: Some(session),
            runtime,
            state,
            config,
            selected_card: 0,
        }
    }

    /// Reconecta desde cero (el botón aparece cuando la sesión terminó).
    fn reconnect(&mut self) {
        if let Some(previous) = self.session.take() {
            previous.shutdown();
        }
        self.state = GuiState::default();
        self.state
            .push_log(format!("reconectando a {} ...", self.config.host_addr()));
        let session = {
            let _enter = self.runtime.enter();
            crate::runtime::spawn(&self.config)
        };
        self.session = Some(session);
    }

    /// Envía un evento al motor sin bloquear el hilo de dibujo.
    fn send(&mut self, event: Event) {
        if self.state.session_closed {
            self.state
                .push_log("sesión cerrada; reconectá con Ctrl+R o el botón");
            return;
        }
        let Some(session) = self.session.as_ref() else {
            self.state.push_log("sin sesión activa (usá Reconectar)");
            return;
        };
        if !session.try_send(event) {
            self.state
                .push_log("evento descartado: runtime ocupado o cerrado");
        }
    }

    fn press(&mut self, key: AtmKey) {
        self.send(Event::KeyPressed(key));
    }

    /// Inserta la tarjeta seleccionada en el perfil (botón o Ctrl+I).
    fn insert_selected_card(&mut self) {
        if let Some(card) = self.config.cards.get(self.selected_card).cloned() {
            let name = card.name.clone();
            self.send(card_event(&card));
            self.state.push_log(format!("tarjeta '{name}' insertada"));
        } else {
            self.state.push_log("el perfil no define tarjetas");
        }
    }

    /// Retira el efectivo pendiente (botón o Ctrl+E).
    fn take_cash(&mut self) {
        if self.state.cash_ready.is_some() {
            self.send(Event::CashTaken);
            self.state.cash_taken();
        }
    }

    /// Procesa los atajos de teclado físico (los botones usan el mouse).
    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if self.state.session_closed {
            if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::R)) {
                self.reconnect();
            }
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::I)) {
            self.insert_selected_card();
        }
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::E)) {
            self.take_cash();
        }
        let mut events = Vec::new();
        ctx.input(|input| {
            for (key, digit) in DIGIT_KEYS {
                if input.key_pressed(key) {
                    events.push(Event::KeyPressed(AtmKey::Digit(digit)));
                }
            }
            if input.key_pressed(egui::Key::Enter) {
                events.push(Event::KeyPressed(AtmKey::Enter));
            }
            if input.key_pressed(egui::Key::Backspace) {
                events.push(Event::KeyPressed(AtmKey::Clear));
            }
            if input.key_pressed(egui::Key::Escape) {
                events.push(Event::KeyPressed(AtmKey::Cancel));
            }
            for (key, fdk) in FDK_KEYS {
                if input.key_pressed(key) {
                    events.push(Event::KeyPressed(AtmKey::Fdk(fdk)));
                }
            }
        });
        for event in events {
            self.send(event);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("oxi")
                    .size(23.0)
                    .strong()
                    .color(theme::ACCENT),
            );
            ui.label(
                RichText::new("simulador NDC")
                    .size(13.0)
                    .color(theme::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.state.session_closed && ui.button("Reconectar").clicked() {
                    self.reconnect();
                }
                self.badges(ui);
            });
        });
    }

    fn badges(&self, ui: &mut egui::Ui) {
        if self.state.session_closed {
            theme::badge(ui, "sesión cerrada", theme::ERR);
        } else {
            match self.state.connection {
                Some(ConnectionState::Connected) => theme::badge(ui, "conectado", theme::OK),
                Some(ConnectionState::Connecting { attempt }) => {
                    theme::badge(ui, &format!("conectando ({attempt})"), theme::WARN);
                }
                None => theme::badge(ui, "sin conexión", theme::MUTED),
            }
        }
        if let Some(mode) = self.state.mode {
            theme::badge(ui, mode_label(mode), mode_color(mode));
        }
    }

    fn footer(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("host {}", self.config.host_addr()))
                    .monospace()
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("datos 100 % sintéticos · fase 9")
                        .size(10.0)
                        .color(theme::MUTED),
                );
                ui.label(
                    RichText::new(
                        "F1–F9 FDK · Ctrl+I tarjeta · Ctrl+E efectivo · Ctrl+R reconectar",
                    )
                    .size(10.0)
                    .color(theme::MUTED),
                );
            });
        });
    }

    fn atm_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE
            .fill(theme::CABINET)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.set_min_height(ui.available_height());
                let height = ui.available_height();
                let fdk_width = 66.0;
                let middle = (ui.available_width() - 2.0 * (fdk_width + 12.0) - 24.0).max(430.0);
                ui.horizontal(|ui| {
                    self.fdk_column(ui, &FDKS_LEFT);
                    let _ = ui.allocate_ui_with_layout(
                        Vec2::new(middle, height),
                        Layout::top_down(Align::Center),
                        |ui| {
                            ui.set_width(middle);
                            let screen_height = (height * 0.38).clamp(220.0, 300.0);
                            self.screen(ui, screen_height);
                            ui.add_space(8.0);
                            self.keypad(ui);
                            ui.add_space(8.0);
                            self.hardware_row(ui);
                        },
                    );
                    self.fdk_column(ui, &FDKS_RIGHT);
                });
            });
    }

    fn fdk_column(&mut self, ui: &mut egui::Ui, keys: &[Fdk]) {
        ui.vertical(|ui| {
            ui.set_width(66.0);
            ui.label(RichText::new("FDK").size(9.0).color(theme::MUTED));
            ui.add_space(2.0);
            for &fdk in keys {
                let label = fdk.as_char().to_string();
                let button = egui::Button::new(RichText::new(label).size(19.0).strong())
                    .min_size(Vec2::new(58.0, 50.0));
                if ui.add(button).clicked() {
                    self.press(AtmKey::Fdk(fdk));
                }
                ui.add_space(6.0);
            }
        });
    }

    fn screen(&mut self, ui: &mut egui::Ui, height: f32) {
        egui::Frame::NONE
            .fill(theme::SCREEN_BG)
            .stroke(Stroke::new(2.0, theme::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(ui.available_width(), height));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PANTALLA").size(9.0).color(theme::MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let id = self
                            .state
                            .screen_id
                            .map(|id| id.to_wire())
                            .unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(id).monospace().size(9.0).color(theme::MUTED));
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((height - 46.0).max(120.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if self.state.screen_text.is_empty() {
                            ui.label(
                                RichText::new("— sin pantalla —")
                                    .monospace()
                                    .size(18.0)
                                    .color(theme::MUTED),
                            );
                        } else {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.state.screen_text)
                                        .monospace()
                                        .size(19.0)
                                        .color(theme::SCREEN_TEXT),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            });
    }

    fn keypad(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            egui::Grid::new("atm-keypad")
                .spacing(Vec2::new(8.0, 7.0))
                .show(ui, |ui| {
                    for row in [['1', '2', '3'], ['4', '5', '6'], ['7', '8', '9']] {
                        for digit in row {
                            self.digit_button(ui, digit);
                        }
                        ui.end_row();
                    }
                    self.action_button(ui, "CLR", AtmKey::Clear, theme::WARN);
                    self.digit_button(ui, '0');
                    self.action_button(ui, "CAN", AtmKey::Cancel, theme::ERR);
                    ui.end_row();
                });
            ui.add_space(4.0);
            let enter = egui::Button::new(RichText::new("ENTER").size(15.0).strong())
                .min_size(Vec2::new(226.0, 34.0));
            if ui.add(enter).clicked() {
                self.press(AtmKey::Enter);
            }
        });
    }

    fn digit_button(&mut self, ui: &mut egui::Ui, digit: char) {
        let button = egui::Button::new(RichText::new(digit.to_string()).size(20.0))
            .min_size(Vec2::new(70.0, 38.0));
        if ui.add(button).clicked() {
            self.press(AtmKey::Digit(digit));
        }
    }

    fn action_button(&mut self, ui: &mut egui::Ui, label: &str, key: AtmKey, color: Color32) {
        let button = egui::Button::new(RichText::new(label).size(14.0).strong().color(color))
            .fill(color.gamma_multiply(0.12))
            .min_size(Vec2::new(70.0, 38.0));
        if ui.add(button).clicked() {
            self.press(key);
        }
    }

    fn hardware_row(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            self.card_block(&mut columns[0]);
            self.cash_block(&mut columns[1]);
        });
    }

    fn card_block(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.label(RichText::new("TARJETA").size(10.0).color(theme::MUTED));
                ui.add_space(2.0);
                let cards: Vec<String> = self
                    .config
                    .cards
                    .iter()
                    .map(|card| card.name.clone())
                    .collect();
                let selected = cards
                    .get(self.selected_card)
                    .cloned()
                    .unwrap_or_else(|| "—".into());
                egui::ComboBox::from_id_salt("card-select")
                    .selected_text(selected)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        for (index, name) in cards.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_card, index, name);
                        }
                    });
                ui.add_space(4.0);
                let can_insert = !cards.is_empty() && !self.state.session_closed;
                let button = egui::Button::new("Insertar tarjeta").min_size(Vec2::new(170.0, 30.0));
                if ui.add_enabled(can_insert, button).clicked() {
                    self.insert_selected_card();
                }
                if cards.is_empty() {
                    ui.label(
                        RichText::new("el perfil no define tarjetas")
                            .size(10.0)
                            .color(theme::WARN),
                    );
                }
            });
    }

    fn cash_block(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.label(RichText::new("DISPENSADOR").size(10.0).color(theme::MUTED));
                ui.add_space(2.0);
                if let Some(counts) = self.state.cash_ready {
                    let total: u16 = counts.iter().sum();
                    ui.label(
                        RichText::new(format!("{total} billete(s) listos"))
                            .size(14.0)
                            .color(theme::OK),
                    );
                    ui.label(
                        RichText::new(format!(
                            "casete 1: {} · 2: {} · 3: {} · 4: {}",
                            counts[0], counts[1], counts[2], counts[3]
                        ))
                        .size(10.0)
                        .color(theme::MUTED),
                    );
                    ui.add_space(4.0);
                    let button =
                        egui::Button::new("Retirar efectivo").min_size(Vec2::new(170.0, 30.0));
                    if ui.add(button).clicked() {
                        self.take_cash();
                    }
                } else {
                    ui.label(
                        RichText::new("sin efectivo pendiente")
                            .size(12.0)
                            .color(theme::MUTED),
                    );
                    ui.add_space(34.0);
                }
            });
    }

    fn logs_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Registro");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Limpiar").clicked() {
                    self.state.clear_logs();
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "{} líneas · {}",
                self.state.logs.len(),
                if self.state.session_closed {
                    "sesión cerrada"
                } else {
                    "sesión activa"
                }
            ))
            .size(10.0)
            .color(theme::MUTED),
        );
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for line in &self.state.logs {
                    ui.label(RichText::new(line).monospace().size(11.0));
                }
            });
    }
}

impl eframe::App for OxiApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut disconnected = false;
        if let Some(session) = self.session.as_mut() {
            loop {
                match session.updates.try_recv() {
                    Ok(update) => self.state.apply(update),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.state.mark_closed();
        }
        self.handle_keyboard(ctx);
        // El motor puede emitir actualizaciones en cualquier momento: se
        // consulta el canal a ritmo fijo (80 ms) sin bloquear el hilo de UI.
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("footer").show(ui, |ui| self.footer(ui));
        egui::Panel::right("logs")
            .default_size(340.0)
            .min_size(240.0)
            .resizable(true)
            .show(ui, |ui| self.logs_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.atm_panel(ui));
    }
}

impl Drop for OxiApp {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
    }
}

/// Texto del modo operativo para la barra de estado.
fn mode_label(mode: OperationalMode) -> &'static str {
    match mode {
        OperationalMode::PowerUp => "Power-Up",
        OperationalMode::OutOfService => "fuera de servicio",
        OperationalMode::Supply => "abastecimiento",
        OperationalMode::InService => "en servicio",
    }
}

/// Color del badge de modo.
fn mode_color(mode: OperationalMode) -> Color32 {
    match mode {
        OperationalMode::PowerUp => theme::MUTED,
        OperationalMode::OutOfService | OperationalMode::Supply => theme::WARN,
        OperationalMode::InService => theme::OK,
    }
}
