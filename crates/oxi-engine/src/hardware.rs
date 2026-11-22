//! Hardware virtual del terminal: dispensador, lector, impresoras, sensores
//! y catálogo de fallas (SPEC §13; manual §9-63…§9-91).
//!
//! Los formatos de estado siguen las tablas del manual:
//! - Cash handler 'E' (Tabla 9-31): T-code + 4×2 dígitos de conteos.
//! - Receipt printer 'G' (Tabla 9-33): status 0/1/2/4 + supplies 4 chars.
//! - Journal printer 'H' (Tabla 9-34): status + supplies.
//! - Sensors 'P' (Tabla 9-41): cambio de modo o alarmas TI.
//! - Clock 'A' (Tabla 9-28) y Power failure 'B' (Tabla 9-29).

/// Cantidad de casetes del dispensador (Tabla 9-8: tipos 1-4).
pub const CASSETTE_COUNT: usize = 4;

/// Un casete con su denominación y notas restantes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cassette {
    /// Denominación en la moneda del perfil (0 = no configurado).
    pub denomination: u32,
    /// Notas presentes.
    pub count: u16,
}

/// Dispensador de efectivo (cash handler 'E', Tabla 9-31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispenser {
    /// Casetes 1-4 (perfil sintético por defecto).
    pub cassettes: [Cassette; CASSETTE_COUNT],
    /// Notas retraídas (no retiradas por el cardholder).
    pub retract_bin: u16,
    /// Total dispensado por casete desde el último reset (contadores g6).
    pub dispensed_total: [u32; CASSETTE_COUNT],
    /// Notas rechazadas por casete (contadores g5).
    pub rejected: [u16; CASSETTE_COUNT],
    /// Notas de la última transacción (g7).
    pub last_dispense: [u16; CASSETTE_COUNT],
    /// Falla inyectada: el mecanismo no dispensa.
    pub jammed: bool,
    /// Falla inyectada: dispensa menos de lo pedido.
    pub partial: bool,
}

impl Default for Dispenser {
    /// Perfil sintético de 100 notas por casete con denominaciones
    /// arbitrarias de prueba (el perfil real llega con la config, fase 6).
    fn default() -> Self {
        let denominations = [10, 20, 50, 100];
        let mut cassettes = [Cassette::default(); CASSETTE_COUNT];
        for (cassette, denomination) in cassettes.iter_mut().zip(denominations) {
            *cassette = Cassette {
                denomination,
                count: 100,
            };
        }
        Self {
            cassettes,
            retract_bin: 0,
            dispensed_total: [0; CASSETTE_COUNT],
            rejected: [0; CASSETTE_COUNT],
            last_dispense: [0; CASSETTE_COUNT],
            jammed: false,
            partial: false,
        }
    }
}

/// Resultado de un intento de dispensado (T-code de la Tabla 9-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispenseOutcome {
    /// '0': completo (puede haber excepción reportada aparte).
    Complete,
    /// '1': dispensado corto (menos notas de las pedidas).
    Short,
    /// '2': no se dispensó nada.
    None,
    /// '5': notas retraídas tras el time-out de retiro.
    Retracted,
}

impl Dispenser {
    /// Verifica si el pedido puede dispensarse completo.
    pub fn can_dispense(&self, counts: [u16; CASSETTE_COUNT]) -> bool {
        !self.jammed
            && counts
                .iter()
                .zip(&self.cassettes)
                .all(|(requested, cassette)| *requested <= cassette.count)
    }

    /// Aplica el dispensado: descuenta notas y actualiza contadores.
    /// Devuelve el resultado y los conteos REALMENTE dispensados.
    pub fn dispense(&mut self, counts: [u16; CASSETTE_COUNT]) -> (DispenseOutcome, [u16; 4]) {
        if self.jammed {
            self.last_dispense = [0; 4];
            return (DispenseOutcome::None, [0; 4]);
        }
        let mut actual = [0u16; 4];
        for index in 0..CASSETTE_COUNT {
            let available = if self.partial {
                self.cassettes[index].count.min(counts[index] / 2)
            } else {
                self.cassettes[index].count.min(counts[index])
            };
            actual[index] = available;
            self.cassettes[index].count -= available;
            self.dispensed_total[index] += u32::from(available);
        }
        self.last_dispense = actual;
        let requested: u16 = counts.iter().sum();
        let delivered: u16 = actual.iter().sum();
        let outcome = if delivered == requested {
            DispenseOutcome::Complete
        } else if delivered == 0 {
            DispenseOutcome::None
        } else {
            DispenseOutcome::Short
        };
        (outcome, actual)
    }

    /// Retracta las notas presentadas que no fueron retiradas (timer 07).
    pub fn retract(&mut self, counts: [u16; CASSETTE_COUNT]) -> DispenseOutcome {
        self.retract_bin += counts.iter().sum::<u16>();
        DispenseOutcome::Retracted
    }

    /// T-code + conteos de 2 dígitos por casete (e2 de la Tabla 9-31).
    pub fn status_field(outcome: DispenseOutcome, counts: [u16; 4]) -> String {
        let code = match outcome {
            DispenseOutcome::Complete => '0',
            DispenseOutcome::Short => '1',
            DispenseOutcome::None => '2',
            DispenseOutcome::Retracted => '5',
        };
        let counts_text: String = counts
            .iter()
            .map(|count| format!("{:02}", (*count).min(99)))
            .collect();
        format!("{code}{counts_text}")
    }

    /// Conteos de notas restantes en casetes, 4×5 dígitos (campo g4, Tabla 9-8).
    pub fn notes_in_cassettes(&self) -> String {
        self.cassettes
            .iter()
            .map(|cassette| format!("{:05}", cassette.count))
            .collect()
    }

    /// Conteos de notas rechazadas, 4×5 (campo g5).
    pub fn notes_rejected(&self) -> String {
        self.rejected
            .iter()
            .map(|count| format!("{:05}", *count))
            .collect()
    }

    /// Conteos de notas dispensadas, 4×5 (campo g6).
    pub fn notes_dispensed(&self) -> String {
        self.dispensed_total
            .iter()
            .map(|count| format!("{:05}", *count))
            .collect()
    }

    /// Notas de la última transacción, 4×5 (campo g7).
    pub fn last_notes_dispensed(&self) -> String {
        self.last_dispense
            .iter()
            .map(|count| format!("{:05}", *count))
            .collect()
    }
}

/// Estado físico del lector de tarjetas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderState {
    /// Sin tarjeta.
    Idle,
    /// Tarjeta presentada esperando ser retirada.
    Presented,
    /// Tarjeta capturada.
    Captured,
}

/// Lector/grabador de tarjetas (device 'D', Tabla 9-30).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reader {
    /// Estado físico.
    pub state: Option<ReaderState>,
    /// Tarjetas capturadas desde el reset (campo g8 de Tabla 9-8).
    pub captures: u32,
    /// Falla inyectada: atasco (no lee ni expulsa).
    pub jammed: bool,
    /// Falla inyectada: error de lectura de pistas.
    pub read_error: bool,
}

impl Reader {
    /// Estado del lector (Idle si `None`).
    pub fn state(&self) -> ReaderState {
        self.state.unwrap_or(ReaderState::Idle)
    }
}

/// Estado del papel de una impresora (Tabla 9-33, char 0 de supplies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperState {
    /// '1': papel suficiente.
    Sufficient,
    /// '2': papel bajo.
    Low,
    /// '3': sin papel.
    Exhausted,
}

/// Falla de impresión (papel agotado o falla inyectada).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("impresión fallida")]
pub struct PrinterFault;

/// Una impresora con spool virtual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    /// Líneas/datos impresos (v1: spool en memoria).
    pub spool: Vec<String>,
    /// Estado del papel.
    pub paper: PaperState,
    /// Falla inyectada: la impresora falla.
    pub failed: bool,
}

impl Default for Printer {
    fn default() -> Self {
        Self {
            spool: Vec::new(),
            paper: PaperState::Sufficient,
            failed: false,
        }
    }
}

impl Printer {
    /// Intenta imprimir; falla si no hay papel o hay falla inyectada.
    pub fn print(&mut self, data: &str) -> Result<(), PrinterFault> {
        if self.failed || self.paper == PaperState::Exhausted {
            return Err(PrinterFault);
        }
        self.spool.push(data.to_owned());
        Ok(())
    }

    /// Supplies status de 4 chars (Tabla 9-33): papel + ribbon/head/knife OK.
    pub fn supplies_status(&self) -> String {
        let paper = match self.paper {
            PaperState::Sufficient => '1',
            PaperState::Low => '2',
            PaperState::Exhausted => '3',
        };
        format!("{paper}111")
    }
}

/// Sensores y alarmas (device 'P', Tabla 9-41).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sensors {
    /// Modo supervisor activo.
    pub supervisor_mode: bool,
    /// Sensor de puerta.
    pub door_open: bool,
    /// Tamper detectado.
    pub tamper: bool,
}

impl Sensors {
    /// Campo de status de cambio de modo supervisor: 'P21' entrada,
    /// 'P20' salida (Tabla 9-41, byte 1 = '2').
    pub fn mode_change_field(&self) -> String {
        format!("P2{}", if self.supervisor_mode { '1' } else { '0' })
    }

    /// Campo de alarmas TI: 'P3' + 12 bytes de sensores (Tabla 9-41).
    pub fn alarm_field(&self) -> String {
        let byte = |active: bool| if active { '1' } else { '0' };
        // Bytes 2-13: supervisor, vibración/calor, puerta, señal silenciosa,
        // gabinete electrónico, bins (depósito/tarjetas/rechazo) y 4 casetes.
        let snapshot = format!(
            "{}{}{}{}{}1111111",
            byte(self.supervisor_mode),
            byte(self.tamper),
            byte(self.door_open),
            '0',
            '0'
        );
        format!("P3{snapshot}")
    }
}

/// Falla inyectable desde el perfil o los tests (decisión Q9: catálogo
/// completo de los dispositivos modelados).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Atasco del lector de tarjetas (device 'D').
    CardReaderJam,
    /// Error de lectura de pistas (device 'D', código '7').
    CardReaderReadError,
    /// Atasco del dispensador (device 'E', código '2').
    CashJam,
    /// Dispensado parcial (device 'E', código '1' en el próximo dispense).
    CashPartial,
    /// Sin papel en el recibo (device 'G', supplies 3111).
    ReceiptPaperOut,
    /// Falla de la impresora de recibo (device 'G', código '1').
    ReceiptPrinterFailure,
    /// Falla de la impresora de journal (device 'H', código '1').
    JournalPrinterFailure,
    /// Reloj detenido (device 'A', código '2').
    ClockFailure,
    /// Fallo de energía (device 'B' + config ID, Tabla 9-29).
    PowerFailure,
    /// Tamper detectado (device 'P', alarma TI).
    Tamper,
    /// Puerta abierta (device 'P', alarma TI).
    DoorOpen,
}

/// Condición de dispositivo lista para un status (Tabla 9-27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCondition {
    /// Device Identification Graphic (1 char).
    pub device: char,
    /// Device status (e2/g2).
    pub status: String,
    /// Error severity (e3/g3).
    pub severity: char,
    /// Diagnostic status (e4/g4); vacío en v1.
    pub diagnostic: String,
    /// Supplies status (e5/g5), si aplica.
    pub supplies: Option<String>,
}

impl DeviceCondition {
    /// Campos de status information (el primero DIG+status, luego el resto).
    pub fn status_fields(&self) -> Vec<String> {
        let mut fields = vec![format!("{}{}", self.device, self.status)];
        fields.push(self.severity.to_string());
        fields.push(self.diagnostic.clone());
        if let Some(supplies) = &self.supplies {
            fields.push(supplies.clone());
        }
        fields
    }
}

/// Todo el hardware virtual del terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hardware {
    /// Dispensador.
    pub dispenser: Dispenser,
    /// Lector de tarjetas.
    pub reader: Reader,
    /// Impresora de recibos.
    pub receipt: Printer,
    /// Impresora de journal.
    pub journal: Printer,
    /// Sensores.
    pub sensors: Sensors,
}

impl Hardware {
    /// Aplica una falla y devuelve la condición a reportar (si aplica un
    /// unsolicited status inmediato). `None` = la falla solo afecta la
    /// próxima operación.
    pub fn inject(&mut self, fault: Fault, config_id: u16) -> Option<DeviceCondition> {
        match fault {
            Fault::CardReaderJam => {
                self.reader.jammed = true;
                Some(DeviceCondition {
                    device: 'D',
                    status: "2".into(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: None,
                })
            }
            Fault::CardReaderReadError => {
                self.reader.read_error = true;
                Some(DeviceCondition {
                    device: 'D',
                    status: "7".into(),
                    severity: '2',
                    diagnostic: String::new(),
                    supplies: None,
                })
            }
            Fault::CashJam => {
                self.dispenser.jammed = true;
                Some(DeviceCondition {
                    device: 'E',
                    status: "2".into(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: Some(self.dispenser.notes_in_cassettes()),
                })
            }
            Fault::CashPartial => {
                self.dispenser.partial = true;
                None
            }
            Fault::ReceiptPaperOut => {
                self.receipt.paper = PaperState::Exhausted;
                Some(DeviceCondition {
                    device: 'G',
                    status: "0".into(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: Some(self.receipt.supplies_status()),
                })
            }
            Fault::ReceiptPrinterFailure => {
                self.receipt.failed = true;
                Some(DeviceCondition {
                    device: 'G',
                    status: "1".into(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: Some(self.receipt.supplies_status()),
                })
            }
            Fault::JournalPrinterFailure => {
                self.journal.failed = true;
                Some(DeviceCondition {
                    device: 'H',
                    status: "1".into(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: Some(self.journal.supplies_status()),
                })
            }
            Fault::ClockFailure => Some(DeviceCondition {
                device: 'A',
                status: "2".into(),
                severity: '4',
                diagnostic: String::new(),
                supplies: None,
            }),
            Fault::PowerFailure => Some(DeviceCondition {
                device: 'B',
                status: format!("{config_id:04}"),
                severity: '0',
                diagnostic: String::new(),
                supplies: None,
            }),
            Fault::Tamper => {
                self.sensors.tamper = true;
                Some(DeviceCondition {
                    device: 'P',
                    status: self.sensors.alarm_field()[1..].to_owned(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: None,
                })
            }
            Fault::DoorOpen => {
                self.sensors.door_open = true;
                Some(DeviceCondition {
                    device: 'P',
                    status: self.sensors.alarm_field()[1..].to_owned(),
                    severity: '4',
                    diagnostic: String::new(),
                    supplies: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispense_should_deduct_and_track_counts() {
        let mut dispenser = Dispenser::default();
        let (outcome, actual) = dispenser.dispense([1, 0, 2, 0]);
        assert_eq!(outcome, DispenseOutcome::Complete);
        assert_eq!(actual, [1, 0, 2, 0]);
        assert_eq!(dispenser.cassettes[0].count, 99);
        assert_eq!(dispenser.cassettes[2].count, 98);
        assert_eq!(dispenser.notes_dispensed(), "00001000000000200000");
    }

    #[test]
    fn dispense_should_be_none_when_jammed() {
        let mut dispenser = Dispenser {
            jammed: true,
            ..Dispenser::default()
        };
        let (outcome, actual) = dispenser.dispense([1, 0, 0, 0]);
        assert_eq!(outcome, DispenseOutcome::None);
        assert_eq!(actual, [0; 4]);
    }

    #[test]
    fn partial_dispense_should_report_short() {
        let mut dispenser = Dispenser {
            partial: true,
            ..Dispenser::default()
        };
        let (outcome, actual) = dispenser.dispense([10, 0, 0, 0]);
        assert_eq!(outcome, DispenseOutcome::Short);
        assert_eq!(actual[0], 5);
    }

    /// T-code + 2 dígitos por casete (Tabla 9-31).
    #[test]
    fn status_field_should_match_table_9_31() {
        let field = Dispenser::status_field(DispenseOutcome::Short, [1, 2, 3, 4]);
        assert_eq!(field, "101020304");
    }

    /// Sin papel: supplies 3111 y severidad 4 (ejemplo de la Tabla 9-33).
    #[test]
    fn receipt_paper_out_should_report_supplies_3111() {
        let mut hardware = Hardware::default();
        let condition = hardware
            .inject(Fault::ReceiptPaperOut, 1)
            .expect("condición inmediata");
        assert_eq!(condition.device, 'G');
        assert_eq!(condition.supplies.as_deref(), Some("3111"));
        assert_eq!(condition.severity, '4');
    }

    /// Modo supervisor: 'P21' entrada / 'P20' salida (Tabla 9-41).
    #[test]
    fn supervisor_mode_change_should_use_p2_codes() {
        let mut sensors = Sensors {
            supervisor_mode: true,
            ..Sensors::default()
        };
        assert_eq!(sensors.mode_change_field(), "P21");
        sensors.supervisor_mode = false;
        assert_eq!(sensors.mode_change_field(), "P20");
    }
}
