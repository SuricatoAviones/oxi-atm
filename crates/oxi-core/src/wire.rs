//! Encuadre lógico del cable NDC: separadores FS/GS (SPEC §1, §2).
//!
//! Este módulo es deliberadamente "tonto": solo conoce los separadores y
//! preserva los campos tal cual (incluidos los vacíos, que el driver real sí
//! envía). La semántica vive en los parsers tipados de cada mensaje.

use crate::ParseError;

/// Separador de campo (FS). Manual §O-4: 0x1C.
pub const FS: u8 = 0x1C;
/// Separador de grupo (GS), para subgrupos dentro de un campo. Uso verificado
/// en los grupos de impresión del Transaction Reply (SPEC §4.3).
pub const GS: u8 = 0x1D;

/// Un mensaje crudo separado en campos por FS.
///
/// Invariantes:
/// - ningún campo contiene FS (por construcción);
/// - hay **al menos un campo** (el de clase): un cuerpo vacío se parsea como
///   un único campo vacío, igual que en el cable;
/// - los campos vacíos se preservan — el driver real termina varios mensajes
///   con FS, dejando un último campo vacío que los parsers tipados contemplan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFields {
    fields: Vec<Vec<u8>>,
}

impl RawFields {
    /// Construye desde campos ya separados. Falla si la lista está vacía
    /// (viola el invariante "al menos el campo de clase").
    pub fn new(fields: Vec<Vec<u8>>) -> Result<Self, ParseError> {
        if fields.is_empty() {
            return Err(ParseError::malformed(
                "un mensaje tiene al menos un campo (el de clase)",
            ));
        }
        Ok(Self { fields })
    }

    /// Separa un mensaje en campos por FS. No puede fallar: cualquier secuencia
    /// de bytes es separable; la validez semántica se chequea después.
    /// Un cuerpo vacío produce un único campo vacío.
    pub fn parse(bytes: &[u8]) -> Self {
        Self {
            fields: bytes.split(|&b| b == FS).map(|f| f.to_vec()).collect(),
        }
    }

    /// Campos como slices de bytes (vacíos incluidos).
    pub fn fields(&self) -> &[Vec<u8>] {
        &self.fields
    }

    /// Campo `i` como texto ASCII estricto; error si no existe o no es ASCII.
    ///
    /// NDC viaja como ASCII (manual §F): cualquier byte fuera de rango es un
    /// mensaje malformado, no un texto válido con pérdida.
    pub fn ascii_field(&self, index: usize) -> Result<&str, ParseError> {
        let raw = self
            .fields
            .get(index)
            .ok_or_else(|| ParseError::malformed(format!("falta el campo {index}")))?;
        std::str::from_utf8(raw)
            .map_err(|_| ParseError::malformed(format!("el campo {index} no es ASCII válido")))
    }

    /// Reconstruye el mensaje del cable (campos unidos por FS).
    pub fn rebuild(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, field) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push(FS);
            }
            out.extend_from_slice(field);
        }
        out
    }

    /// Divide un campo en subgrupos por GS (p.ej. grupos de impresión).
    pub fn split_groups(data: &[u8]) -> Vec<&[u8]> {
        data.split(|&b| b == GS).collect()
    }

    /// Une subgrupos con GS (inverso de [`RawFields::split_groups`]).
    pub fn join_groups(parts: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.push(GS);
            }
            out.extend_from_slice(part);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parse_should_split_fields_by_fs() {
        let raw = RawFields::parse(b"3\x1c001\x1c9");
        assert_eq!(
            raw.fields(),
            [b"3".to_vec(), b"001".to_vec(), b"9".to_vec()]
        );
    }

    /// El driver real termina mensajes con FS: queda un campo vacío final que
    /// debe preservarse para que el roundtrip sea exacto.
    #[test]
    fn parse_should_preserve_trailing_empty_field() {
        let raw = RawFields::parse(b"3\x1c1A\x1c");
        assert_eq!(raw.fields().last(), Some(&Vec::new()));
    }

    #[test]
    fn parse_should_preserve_inner_empty_fields() {
        let raw = RawFields::parse(b"1\x1c\x1c\x1c1");
        assert_eq!(raw.fields().len(), 4);
        assert_eq!(raw.fields()[1], Vec::<u8>::new());
        assert_eq!(raw.fields()[2], Vec::<u8>::new());
    }

    /// GS puede vivir DENTRO de un campo (grupos de impresión); FS no.
    #[test]
    fn parse_should_keep_gs_inside_field() {
        let raw = RawFields::parse(b"datos\x1dmas");
        assert_eq!(raw.fields(), [b"datos\x1dmas".to_vec()]);
    }

    #[test]
    fn ascii_field_should_reject_missing_field() {
        let raw = RawFields::parse(b"3");
        assert!(raw.ascii_field(1).is_err());
    }

    #[test]
    fn ascii_field_should_reject_non_ascii_bytes() {
        let raw = RawFields::parse(b"\xFF\xFE");
        assert!(raw.ascii_field(0).is_err());
    }

    #[test]
    fn split_groups_should_split_by_gs() {
        let parts = RawFields::split_groups(b"1rec\x1d2rec\x1d3rec");
        assert_eq!(
            parts,
            [b"1rec".as_slice(), b"2rec".as_slice(), b"3rec".as_slice()]
        );
    }

    #[test]
    fn join_groups_should_be_inverse_of_split() {
        let parts: Vec<&[u8]> = vec![b"1rec", b"2rec"];
        let joined = RawFields::join_groups(&parts);
        assert_eq!(RawFields::split_groups(&joined), parts);
    }

    #[test]
    fn new_should_reject_empty_field_list() {
        assert!(RawFields::new(Vec::new()).is_err());
    }

    /// Propiedad central del parser: rebuild(parse(x)) == x para cualquier
    /// mensaje con al menos el campo de clase y sin FS dentro de los campos.
    #[test]
    fn rebuild_should_be_inverse_of_parse() {
        proptest!(|(fields in proptest::collection::vec(
            proptest::collection::vec(any::<u8>().prop_filter("sin FS", |&b| b != FS), 0..8),
            1..6
        ))| {
            let raw = RawFields::new(fields).expect("al menos un campo");
            let bytes = raw.rebuild();
            let reparsed = RawFields::parse(&bytes);
            assert_eq!(reparsed.fields(), raw.fields());
        });
    }
}
