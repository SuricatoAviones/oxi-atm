# ROADMAP — oxi

> **Estado**: propuesta v0.1 para revisión del dueño.
> Depende de: `docs/SPEC.md` (protocolo, preguntas Q1–Q11), `docs/ARCHITECTURE.md` (estructura),
> `docs/DESIGN.md` (componentes y ADRs).
>
> Regla de oro (AGENTS.md): **una fase a la vez**. Antes de codificar cada fase se resume el
> plan en pocas líneas y se espera confirmación. Cada tarea se muestra al dueño antes de su
> commit.

## 0. Flujo de trabajo Git

- **Fase = rama**: `phase/NN-short-name` (inglés, como los identificadores), creada desde
  `main` al iniciar la fase. Ej.: `phase/01-core-parser`.
- **Tarea = commit**: cada tarea del ROADMAP (T x.y) termina en exactamente un commit, después
  de pasar el checklist de calidad y de que el dueño revise el cambio.
- **Mensajes de commit**: conventional commits con referencia a la tarea:
  `feat(core): parse Transaction Reply (T1.5)` · `test(engine): timeout state matrix (T3.7)`.
- **Cierre de fase**: la rama se mergea a `main` (merge o rebase según preferencia del dueño)
  solo cuando el criterio de aceptación de la fase está verde y el dueño da el OK.
- Si una tarea revela un problema de diseño, se detiene la fase y se discute antes de seguir
  (nada de "arreglaremos luego").

## 1. Definition of Done (por tarea)

1. `cargo fmt --check` verde.
2. `cargo clippy --all-targets --all-features -- -D warnings` verde.
3. `cargo test` verde; tests junto al código, nombres descriptivos, idealmente una aserción por
   test (skill `rust-best-practices`, Cap. 5).
4. Sin `unwrap()`/`expect()` fuera de tests (AGENTS.md + skill, Cap. 4).
5. Errores con `thiserror` en librerías; `anyhow` solo en los binarios (`oxi-ui`, mock).
6. Comentarios/docs en español; identificadores en inglés; los comentarios explican el *por qué*
   (skill, Cap. 8); cada decisión de protocolo cita SPEC (ej. `// SPEC §10-58: MCN obligatorio`).
7. Datos de test 100 % sintéticos: builder de fixtures, nunca extractos del manual ni datos
   reales (AGENTS.md).
8. Se muestra el diff al dueño → se aprueba → se hace el commit.

## 2. Fases

### Fase 0 — Cimientos del workspace · rama `phase/00-scaffolding`

Asegurar que el esqueleto existente de 5 crates compila limpio y fija la infraestructura mínima.

| Tarea | Contenido |
|---|---|
| T0.1 | Revisar/ajustar `Cargo.toml` de cada crate: deps mínimas (`serde`, `thiserror` en core/engine; `tokio` solo en net/host-mock/ui) y metadata del workspace |
| T0.2 | Lints del workspace en `[workspace.lints]` (partiendo de los recomendados por la skill, Cap. 2) + `.rustfmt.toml` si hace falta |
| T0.3 | Script/task documentado para correr el trío de calidad (fmt + clippy + test) |

**Criterio de aceptación**: `cargo build`, fmt, clippy `-D warnings` y test verdes sobre los
esqueletos; `cargo tree` confirma que ni core ni engine dependen de tokio.
**Tests**: los esqueletos traen un test humo por crate.

### Fase 1 — Parser y serializador (`oxi-core`) · rama `phase/01-core-parser`

Objetivo: hablar el idioma del cable sin red ni motor. Ref: SPEC §2–§4, DESIGN §2.

| Tarea | Contenido |
|---|---|
| T1.1 | Tipos base: `StateNumber` (000–254/256–999, 255 = disabled), `Luno`, `Mcn`, `Fdk`, `NoteCounts`, `ScreenId`, `PrintGroup`, `CardReturnFlag` + `ParseError` (thiserror) |
| T1.2 | `RawFields`: split por FS (0x1C) / GS (0x1D) y reconstrucción inversa; propiedad roundtrip con proptest |
| T1.3 | `HostMessage::TerminalCommand` + parser clase 1 (SPEC §4.1) |
| T1.4 | `HostMessage::CustomisationData` + parser clase 3 (ids 1, 2, 3, 5, 6, A, B, C, E, G — SPEC §4.2) |
| T1.5 | `HostMessage::TransactionReply` + function IDs + hasta 13 grupos de impresión + MCN/card flag (SPEC §4.3) |
| T1.6 | `HostMessage::Unknown` (tolerant reader) + ITR + EKC/Extended (estructura mínima) |
| T1.7 | `TerminalMessage` + serializador de TRQ y Solicited Status (descriptors 8/9/A/B/C/F) (SPEC §3) |
| T1.8 | Matriz de modos operativos `disposition(msg, mode)` (SPEC §4.5) como tabla pura |

**Criterio de aceptación**: roundtrip `parse → serialize → parse` idéntico para todas las
variantes (proptest); mensajes malformados devuelven `ParseError` (nunca pánico); sin tokio en
deps. **Tests**: unitarios por variante + proptest + tabla de casos de la matriz de modos.

### Fase 2 — Transporte (`oxi-net`) · rama `phase/02-net`

Objetivo: bytes ida y vuelta contra cualquier peer TCP. Ref: SPEC §1, DESIGN §4.

| Tarea | Contenido |
|---|---|
| T2.1 | Trait `FrameCodec` + impl `LenPrefix2BE` (codec **confirmado por el dueño**, Q1 resuelta) + tests de framing: frames partidas, varias en un segmento, longitud truncada |
| T2.2 | Loop de conexión con reconexión exponencial y estado observable; test con servidor TCP de juguete |
| T2.3 | Loops de lectura/escritura con `mpsc` + tracing con dump hex bidireccional; test loopback end-to-end |

**Criterio de aceptación**: contra un servidor de test, se conecta, re-conecta tras corte y
entrega frames completas al canal de eventos aunque lleguen fragmentadas.
**Tests**: unitarios de codec + integración TCP en `oxi-net/tests/`.

### Fase 3 — Máquina de estados (`oxi-engine`) · rama `phase/03-engine-states`

Objetivo: el ATM "piensa" sin red. Ref: SPEC §5, DESIGN §3.

| Tarea | Contenido |
|---|---|
| T3.1 | `Event`/`Action`, `Terminal` con stores de customización y buffers; default close como recuperación global (SPEC §5.1) |
| T3.2 | Aplicar loads: State Tables, Screens, FITs, ECPL, Config ID sobre los stores (SPEC §4.2) |
| T3.3 | Modos operativos + power-up + Go In/Out of Service + Solicited Status Ready (SPEC §4.5) |
| T3.4 | Intérprete de tipos — **set verificado del driver real** (reference/download): A (read conditions en orden, SPEC §5.2), D, J, Y, X, F, H + extensiones Z |
| T3.5 | Tipo I: armado de TRQ (MCN rotativo, pistas, op-code, importe, buffers — SPEC §3, **+ transaction status info por opción 15 activa en el driver real**) y ejecución de Transaction Reply (function IDs 1–5/7–9, MCN check → reject genérico **o specific command reject C/B09 por opción 12 activa**) |
| T3.6 | Switches K/`_`/W + FIT match por FIID (BTreeMap) + estados B/M con PIN remoto vía `PinCrypto` stub (SPEC §10, §12) |
| T3.7 | Timers virtuales + Time-Out State (C00, timer 01, FDK A/B/Cancel — SPEC §5.4) + timer 03 sin respuesta |

**Criterio de aceptación**: una transacción sintética completa (A→B→F→I→J) se ejecuta en un
test solo con eventos (incluye TRQ bien formado, dispensado simulado, close), y los casos de
excepción (MCN, timeout) emiten los rejects correctos. **Tests**: por tipo de estado, tabla de
transiciones, inyección directa de `TimerFired`.

### Fase 4 — Hardware virtual (en `oxi-engine`) · rama `phase/04-hardware`

Objetivo: dispensar, leer, imprimir, fallar. Ref: DESIGN §3.5, SPEC §13.

| Tarea | Contenido |
|---|---|
| T4.1 | Dispensador: 4 casetes con conteos, `Dispense`/`CashTaken`/retract con timer 07, límite 40 notas (SPEC §4.3) |
| T4.2 | Lector/grabador: tarjetas sintéticas del perfil, captura/devolución (bandera del TR, timers 09), Track 1/2/3 sintéticas |
| T4.3 | Impresoras como spool estructurado (receipt/journal) + grupos de impresión del TR (SPEC §8) |
| T4.4 | Sensores y suministros + unsolicited básico + supply counters (comando 4) |
| T4.5 | **Catálogo completo de fallas (Q9: "todas las posibles")**: una por dispositivo modelado — lector (jam, error de lectura), cash handler (jam, dispensado parcial, empty), depository, receipt/journal (sin papel, falla), encryptor, reloj, energía, tamper/sensores — cada una con su unsolicited/solicited status correcto (SPEC §9-63…§9-91) + inyección por perfil (`[faults]` en TOML) |

**Criterio de aceptación**: TR con function 2/8 dispensa y "entrega" efectivo virtual; cada
falla del catálogo genera el mensaje de estado correcto; contadores consistentes.
**Tests**: eventos + aserciones sobre spools, conteos y mensajes unsolicited; una prueba por
falla del catálogo.

### Fase 5 — Host falso (`oxi-host-mock`) · rama `phase/05-host-mock`

Objetivo: un peer de verdad para probar. Ref: DESIGN §5.

| Tarea | Contenido |
|---|---|
| T5.1 | `fixtures::builder`: state tables, pantallas, FITs y tarjetas sintéticas (única fuente de customización de test del repo) |
| T5.2 | `ScriptedHost` como librería: acepta conexión, espera power-up, baja customización (cada load espera Ready), manda In-Service |
| T5.3 | Auto-respuesta del TRQ (aprobar/rechazar configurable) + terminal commands 3/4/7/8 con datos sintéticos |
| T5.4 | Binario CLI del mock (opciones de guion) |

**Criterio de aceptación**: un cliente de test que habla `oxi-core` (sin engine) completa el
ciclo power-up → customización → transacción contra el mock.
**Tests**: integración en `oxi-host-mock/tests/` con cliente de test.

### Fase 6 — Ensamblado e interfaz CLI (`oxi-ui`) · rama `phase/06-ui-cli`

Objetivo: usar oxi de verdad contra un host. Ref: DESIGN §6.

| Tarea | Contenido |
|---|---|
| T6.1 | `runtime::spawn`: glue tokio (canales net→engine→UI, scheduler de timers) reutilizable por bin y tests |
| T6.2 | Config TOML (serde + perfil de terminal: framing, LUNO, tarjetas, claves de prueba, fallas) |
| T6.3 | CLI v1: `connect`, `card`, `pin`, `fdk A..I`, `enter/clear/cancel`, `dump states|screens|fits|buffers`, render de pantalla en texto |
| T6.4 | Test E2E full-stack: `oxi` (bin) ↔ `oxi-host-mock` (bin) en localhost: descarga + retiro aprobado + rechazo + timeout |

**Criterio de aceptación**: demo manual completa con dos terminales abiertas y logs legibles;
test E2E verde en CI.
**Tests**: `oxi-ui/tests/e2e_*.rs` con el `ScriptedHost`.

### Fase 7 — Seguridad de PIN (`oxi-engine` + core) · rama `phase/07-security`

Objetivo: PIN block ISO-0 y claves de prueba. Ref: SPEC §12, DESIGN §3.6.
**Q8/Q4 resueltas**: por defecto ISO-0 + 3DES con claves de prueba, sin MAC; las demás opciones
quedan documentadas y configurables por perfil (SPEC §12).

| Tarea | Contenido |
|---|---|
| T7.1 | `PinCrypto` real: ISO-0 (XOR PIN/PAN, pad ANSI), 3DES-ECB con claves de prueba del perfil |
| T7.2 | KVV (6 hex de 3DES sobre ceros) + verificación local vía FIT (PEKEY) como **opción documentada, desactivada por defecto** |
| T7.3 | Encryption Key Change básico (descifrar comms key bajo master; claves en decimal→hex) + documentación de MAC/TVN como opción futura |

**Criterio de aceptación**: TRQ con PIN encriptado que el mock valida con la misma clave;
KVV coincidente. MAC/TVN y EPP real quedan fuera (SPEC §0).

### Fase 8 — Refinos protocolo (opcional, con aprobación) · rama `phase/08-extras`

ITR (3/2) con teclas activas binarias y pantalla dinámica · tipos E/G/C (no usados por el
driver real pero definidos en el manual) · estados R/S/T/b/m · reject fino completo (más
calificadores de la Tabla 10-37) · teclado numérico completo · Override Reserved Screens.
Cada ítem = tarea propia, se planifica al cerrar la fase 7.

### Fase 9 — GUI (después del núcleo, fuera de v1)

Reemplaza la capa CLI de `oxi-ui` sin tocar core/engine (ARCHITECTURE §6). Se diseña aparte
cuando llegue el momento.

## 3. Dependencias entre fases y preguntas abiertas

| Fase | Estado de las preguntas |
|---|---|
| 0, 1, 2 | **Sin bloqueos**: Q1 resuelta (codec 2B BE confirmado; el trait lo deja configurable) |
| 3, 4 | Detalles abiertos no bloqueantes: Q5 (tipos P/Q/i/Z) y Q10 (default close); Q9 resuelta → T4.5 catálogo completo de fallas |
| 5, 6 | Q2/Q3 resueltas: el mock habla **solo Advance NDC**; fixture "Download" real (Q11) como acceptance opcional local |
| 7 | Q4/Q8 resueltas: default ISO-0 + 3DES sin MAC; opciones documentadas en el perfil |
| 8 | Abierta no bloqueante: Q6 (asociación pantalla↔teclado) |

## 4. Regla de avance

1. El dueño aprueba el ROADMAP → se mergea este documento.
2. Se abre la rama de la fase 0 → resumen corto de la fase en el chat → confirmación → tareas.
3. Cada tarea: implementación pequeña → DoD → diff al dueño → commit → siguiente.
4. Fin de fase: revisión del criterio de aceptación → merge → siguiente fase.
