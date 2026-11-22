# [AGENTS.md](http://AGENTS.md) — oxi: simulador de cajeros NDC en Rust

## Objetivo

`oxi` es un simulador de ATM que implementa el protocolo NDC (APTRA Advance NDC)

en Rust, para probar hosts y drivers de cajeros. El dueño del proyecto conoce

ISO 8583 y ATMs, pero está aprendiendo Rust.

## Material de referencia (CONFIDENCIAL)

- `docs/manual/`: manual oficial de desarrollo NDC. Es la fuente de verdad del protocolo.

- `reference/atm/`: simulador previo del dueño, solo como referencia.

- Puedes LEER y analizar ambos para entender el protocolo y armar el plan.

- NUNCA los copies, pegues ni reproduzcas en bloque dentro de archivos versionados

  (código, docs, README, comentarios, tests, issues).

- En los docs escribe resúmenes con tus propias palabras y cita solo la referencia

  (capítulo/sección), no el texto del manual.

- No hagas `git add` de `docs/manual/` ni `reference/`; están en `.gitignore`.

- No subas estos archivos a servicios externos ni los uses como ejemplos públicos.

- Para tests usa datos sintéticos construidos por ti, no extractos literales ni

  datos reales de la empresa.

## Reglas del protocolo

- No inventes campos, códigos ni comportamientos. Si algo no está en el manual,

  dilo y pregunta.

- Cada decisión de protocolo debe indicar su referencia (capítulo/sección).

## Flujo de trabajo

- Trabaja por fases pequeñas según `docs/[ROADMAP.md](http://ROADMAP.md)`; una fase a la vez.

- Antes de codificar una fase, resume el plan en pocas líneas y espera mi confirmación.

- Una función o módulo por vez; no generes grandes bloques sin que yo los revise.

- Explica brevemente cada concepto nuevo de Rust (ownership, lifetimes, traits,

  async) la primera vez que aparezca, y por qué se eligió ese diseño.

- Ante errores del compilador, explica la causa antes de corregir.

## Convenciones de código

- Workspace de Cargo en `crates/`: `oxi-core` (parser/serializador), `oxi-engine`

  (estados, pantallas, FITs, hardware virtual), `oxi-net` (TCP con tokio),

  `oxi-host-mock` (host falso para pruebas), `oxi-ui` (interfaz).

- `oxi-core` y `oxi-engine` no dependen de la interfaz ni de `tokio`.

- Errores con `thiserror`; nada de `unwrap()` fuera de tests.

- Configuración serializada con `serde`.

- Todo cambio debe pasar `cargo fmt`, `cargo clippy -- -D warnings` y `cargo test`.

- Tests unitarios junto al código; de integración en `tests/` usando `oxi-host-mock`.

- Comentarios y documentación en español; identificadores en inglés.

## Fuera de alcance por ahora

- Interfaz gráfica (se hace después del núcleo).

- Cualquier cosa no listada en el ROADMAP sin mi aprobación.