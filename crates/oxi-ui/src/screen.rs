//! Render aproximado de pantallas NDC para la CLI (v1).
//!
//! Las pantallas son texto con controles (SPEC §6). Para la consola se
//! extrae el texto legible: FF salta de línea, SI/SO y secuencias ESC se
//! descartan junto con sus argumentos, y el resto de controles se vuelven
//! espacios. La GUI (fase 9) hará el render fiel.

/// Convierte los datos crudos de una pantalla en texto legible por líneas.
pub fn render_screen(data: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0usize;
    while index < data.len() {
        let byte = data[index];
        match byte {
            // FF: limpiar pantalla → nueva línea.
            0x0C => {
                out.push('\n');
                index += 1;
            }
            // CR: fin de línea.
            0x0D => {
                out.push('\n');
                index += 1;
            }
            // LF directo.
            0x0A => {
                out.push('\n');
                index += 1;
            }
            // SI (posicionar cursor): le siguen fila y columna → descartar 2.
            0x0F => {
                index += 3;
            }
            // SO (pantalla anidada): le siguen 3 dígitos → descartar.
            0x0E => {
                index += 4;
            }
            // ESC: secuencia extendida; se descarta la letra y su argumento
            // de longitud variable (hasta el próximo control o fin).
            0x1B => {
                index += 1;
                if index < data.len() {
                    index += 1; // letra de la secuencia
                    while index < data.len() && data[index] > 0x20 {
                        index += 1;
                    }
                }
            }
            // Otros controles: espacio.
            0x00..=0x1F | 0x7F => {
                out.push(' ');
                index += 1;
            }
            _ => {
                out.push(byte as char);
                index += 1;
            }
        }
    }
    // Colapsa líneas vacías de los bordes.
    let mut lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_should_render() {
        assert_eq!(render_screen(b"\x0cBIENVENIDO"), "BIENVENIDO");
    }

    #[test]
    fn cursor_positioning_should_be_dropped() {
        // SI + fila 'G' + columna 'D' + texto.
        assert_eq!(render_screen(b"\x0c\x0fGDHOLA"), "HOLA");
    }

    #[test]
    fn nested_screen_control_should_be_dropped() {
        // SO + "011" + texto.
        assert_eq!(render_screen(b"\x0c\x0e011TEXTO"), "TEXTO");
    }
}
