//! Perfil del simulador (serde + TOML). Todo sintético: los datos de prueba
//! viven en el perfil, nunca en el código (AGENTS.md).

use serde::Deserialize;

/// Configuración completa de una sesión de oxi.
#[derive(Debug, Clone, Deserialize)]
pub struct SimConfig {
    /// Conexión al host.
    pub net: NetConfig,
    /// Tarjetas de prueba que la CLI puede insertar.
    #[serde(default)]
    pub cards: Vec<CardProfile>,
}

/// Parámetros de red (SPEC §1).
#[derive(Debug, Clone, Deserialize)]
pub struct NetConfig {
    /// Host al que el ATM se conecta.
    pub host: String,
    /// Puerto del host.
    pub port: u16,
    /// Intentos máximos de conexión.
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
}

/// Una tarjeta sintética del perfil.
#[derive(Debug, Clone, Deserialize)]
pub struct CardProfile {
    /// Nombre corto para la CLI (`card <nombre>`).
    pub name: String,
    /// Pista 2 sintética (con sentinelas, como el driver real).
    pub track2: String,
}

fn default_attempts() -> u32 {
    8
}

impl Default for SimConfig {
    /// Perfil por defecto: host local y una tarjeta de prueba.
    fn default() -> Self {
        Self {
            net: NetConfig {
                host: "127.0.0.1".into(),
                port: 9000,
                max_attempts: 8,
            },
            cards: vec![CardProfile {
                name: "test".into(),
                track2: ";5899410000000000=2512?".into(),
            }],
        }
    }
}

impl SimConfig {
    /// Carga un perfil TOML; si no existe la ruta, usa el perfil por defecto.
    pub fn load(path: Option<&str>) -> anyhow::Result<Self> {
        match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)?;
                Ok(toml::from_str(&text)?)
            }
            None => Ok(Self::default()),
        }
    }

    /// Dirección del host en formato `host:port`.
    pub fn host_addr(&self) -> String {
        format!("{}:{}", self.net.host, self.net.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_should_be_valid() {
        let config = SimConfig::default();
        assert_eq!(config.host_addr(), "127.0.0.1:9000");
        assert_eq!(config.cards.len(), 1);
    }

    #[test]
    fn toml_profile_should_parse() {
        let text = r#"
            [net]
            host = "10.0.0.5"
            port = 1234
            [[cards]]
            name = "sintetica"
            track2 = ";0000000000000000=0000?"
        "#;
        let config: SimConfig = toml::from_str(text).expect("perfil válido");
        assert_eq!(config.host_addr(), "10.0.0.5:1234");
        assert_eq!(config.net.max_attempts, 8, "default del serde");
    }
}
