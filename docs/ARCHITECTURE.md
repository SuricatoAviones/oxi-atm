# ARQUITECTURA — oxi: simulador de cajeros NDC en Rust

> **Estado**: propuesta v0.1 para revisión del dueño.
> Depende de `docs/SPEC.md` (inventario del protocolo y preguntas abiertas Q1–Q10).

## 1. Visión y principios

`oxi` simula el **lado terminal** de NDC (APTRA Advance NDC). El diseño persigue cuatro
principios, en orden de prioridad:

1. **Fidelidad al manual.** Cada comportamiento del protocolo tiene cita en SPEC (capítulo/sección).
   Lo que no está en el manual no se inventa: se pregunta (Q1–Q10).
2. **Núcleo puro y determinista.** Toda la semántica del ATM vive en código síncrono sin I/O:
   entra un evento, salen acciones. Esto hace que el 90 % del sistema se pruebe como funciones
   puras, sin red ni tiempo real.
3. **I/O solo en el borde.** Tokio, TCP y relojes viven en una capa delgada (`oxi-net` y el
   ensamblado en `oxi-ui`). El núcleo jamás espera: pide que programen un timer, pide enviar un
   mensaje, y devuelve el control.
4. **Lector tolerante.** Un simulador se enfrenta a hosts reales con variantes no documentadas:
   lo desconocido se preserva (`Unknown { .. }`) y se loguea, no se descarta ni se rompe
   (contrasta con §10-87: lo *inválido según el manual* sí genera reject).

## 2. Mapa de crates

Workspace de Cargo en `crates/` (según AGENTS.md). Grafo de dependencias:

```
                 ┌─────────────┐
                 │  oxi-core   │  (sin runtime, sin UI)
                 │  tipos NDC  │
                 │  parser/ser │
                 └──────┬──────┘
        ┌───────────────┼──────────────────┐
        │               │                   │
┌───────┴───────┐ ┌─────┴────────┐   ┌───────┴─────────┐
│  oxi-engine   │ │   oxi-net    │   │  oxi-host-mock  │
│ (states, FITs │ │ (TCP, tokio, │   │ (host falso con │
│  pantallas,   │ │  encuadre,   │   │  tokio, para    │
│  hw virtual)  │ │  reconexión) │   │  tests y CLI)   │
└───────┬───────┘ └─────┬────────┘   └─────────────────┘
        │               │
        └───────┬───────┘
        ┌───────┴───────┐
        │    oxi-ui     │  (ensambla engine+net; CLI en v1)
        │ binario + lib │
        └───────────────┘
```

| Crate | Rol | Depende de | Restricciones |
|---|---|---|---|
| `oxi-core` | Tipos de mensaje NDC (enums), parser/serializador FS/GS, validación de encuadre lógico, referencias a SPEC | `serde` (config), `thiserror` | **Sin tokio, sin I/O, sin UI** |
| `oxi-engine` | Máquina de estados del terminal, customización (states/screens/FITs/config), buffers, timers virtuales, hardware virtual, crypto de PIN | `oxi-core` | **Sin tokio, sin I/O, sin UI** |
| `oxi-net` | Conexión TCP (ATM→host), codec de encuadre configurable, reconexión con backoff, tracing de frames | `oxi-core`, `tokio` | No conoce `oxi-engine` |
| `oxi-host-mock` | Host falso: baja customización sintética, responde TRQ (auto/moderado), inyecta fallas; binario independiente + librería para tests | `oxi-core`, `tokio` | No conoce `oxi-engine` |
| `oxi-ui` | Ensamblado (runtime glue): canales entre net↔engine↔hardware↔interfaz; CLI en v1 | `oxi-engine`, `oxi-net` | Es el único que toca todo |

Reglas duras (de AGENTS.md, verificables con `cargo tree`):

- `oxi-core` y `oxi-engine` no dependen de `tokio`, de ninguna interfaz ni entre sí de nada
  externo pesado. El engine solo conoce al core.
- `oxi-net` y `oxi-host-mock` **no** dependen del engine: hablan el idioma del core (mensajes)
  pero no saben cómo funciona un ATM.
- No se agregan crates sin aprobación del dueño.

## 3. Modelo de ejecución: eventos y acciones (arquitectura hexagonal)

El corazón del sistema es un loop puro dentro de `oxi-engine`:

```
            ┌──────────── eventos ─────────────┐
            │                                   │
 (oxi-net)  │  HostFrame(bytes)                 │
 (oxi-ui)   │  KeyPressed(Fdk/Enter/Clear/…)   │   ┌─────────────┐
 (oxi-ui)   │  CardInserted(CardData)           ──►│  oxi-engine │
 (runtime)  │  CardTaken / CashTaken / …        │   │  Terminal   │
 (runtime)  │  TimerFired(timer_id)              │   │  (puro)     │
            └───────────────────────────────────┘   └──────┬──────┘
                                                    acciones│
            ┌───────────────────────────────────┐        │
 (oxi-net)  │  SendMessage(TerminalMessage)     ◄────────┤
 (oxi-ui)   │  Display(screen_id)               ◄────────┤
 (runtime)  │  StartTimer(id, duración)/Cancel  ◄────────┤
 (hw virt.) │  Dispense(casetes)/Print/Eject…   ◄────────┘
            └───────────────────────────────────┘
```

- **Eventos**: todo lo que le pasa al ATM (mensaje del host, tecla, tarjeta, timer).
- **Acciones**: todo lo que el ATM quiere hacer (enviar, mostrar, dispensar, programar timers).
- El engine es una función `fn handle(&mut self, event: Event) -> Vec<Action>` (más helpers de
  consulta de estado). Sin reloj, sin red, sin hilos: 100 % testeable con asserts sobre eventos
  y acciones.
- El **runtime glue** (en `oxi-ui`) conecta canales: tarea de red → eventos; acciones → escritura
  TCP, pantalla y planificador de timers (tokio). En tests de integración el mismo glue corre
  contra `oxi-host-mock` en localhost.

## 4. Modelo de estados del terminal

Dos capas distintas que no deben confundirse (SPEC §4.5 y §5):

1. **Modo operativo** (Power-Up / Out-of-Service / Supply / In-Service): gobierna qué mensajes
   se aceptan, rechazan o retienen (matriz de §10-95). Implementado como enum + función
   `disposition(msg, mode)` — tabla pura, testeada por casos.
2. **State tables** (la customización que baja el host): la máquina de estados de cardholder
   (tipos A…w de SPEC §5.2). El engine las interpreta: `execute_state(n) -> Transition` que
   según el tipo devuelve "ir a estado X", "esperar tecla con máscara M y timer 00", "esperar
   tarjeta", "enviar TRQ y esperar Reply (timer 03)", etc.
3. **Time-Out State** fijo (SPEC §5.4) y **default close state** como recuperaciones globales.

Los buffers (tarjeta, PIN, B, C, importe, op-code, FDK) y el MCN rotativo viven junto al estado
corriente; la customización (states/screens/FITs/config) es un almacén inmutable-que-se-
reemplaza (cada load pisa el contenido, como en el protocolo).

## 5. Uso de tokio (y dónde NO se usa)

- `oxi-net`: `TcpStream` asíncrono, `read_exact` para el prefijo de longitud, codec configurable
  (por defecto 2 bytes big-endian, pendiente Q1), reconexión con backoff exponencial.
- `oxi-host-mock`: `TcpListener` asíncrono; guiones de descarga; respuestas automáticas o
  moderadas por CLI.
- `oxi-ui` (glue): un task por fuente de eventos (red, UI, timers) que alimenta `mpsc` hacia el
  engine; las acciones se distribuyen a tasks suscriptores. `tokio::time` para timers pedidos
  por el engine (durations calculados por el engine desde la config — el engine no mide tiempo,
  lo pide).
- **`oxi-core` y `oxi-engine`: cero tokio.** Pruebas del engine: inyectamos `TimerFired` a mano;
  no hay sleeps ni async en todo el núcleo.

## 6. Separación núcleo/interfaz

- La UI es un suscriptor más de acciones (`Display`, `Log`) y un productor más de eventos
  (`KeyPressed`, `CardInserted`). El engine no sabe si hay alguien mirando.
- **v1: CLI** (`oxi-ui`): log de tracing con frames hex, comandos por stdin para insertar
  tarjeta, teclear PIN, pulsar FDKs, ver buffers y customización. Suficiente para probar hosts
  reales con netcat-like workflows.
- **v2 (después del núcleo, con aprobación): interfaz gráfica** (pantalla CRT emulado, FDKs
  clicables, estados visibles) reemplazando la capa CLI sin tocar `oxi-engine` ni `oxi-core`.
  La GUI es out-of-scope hasta entonces (AGENTS.md).

## 7. Estrategia de pruebas

| Nivel | Qué cubre | Herramientas |
|---|---|---|
| Unit (junto al código) | Parser: roundtrip mensaje→bytes→mensaje (proptest); máscaras de FDK (caso 060→C,D,F,G); FIT decode/encode; MCN; matriz de modos operativos; cada tipo de estado con fixtures sintéticos | `cargo test`, `proptest` |
| Unit engine | Transiciones por tipo de estado, Time-Out, default close, rejects de §10-87 (MCN, >40 billetes, function ID ilegal…), verificación local de PIN con claves de prueba | eventos inyectados, sin red ni reloj |
| Integración | Flujos completos contra `oxi-host-mock` en localhost: power-up → customización → in-service → transacción aprobada/rechazada → close; timeout de TRQ; reconexión | `tests/` en `oxi-ui` (glue real + tokio) y `oxi-net` |
| E2E manual | `oxi-host-mock` binario vs `oxi` binario, con tracing legible | scripts documentados |

- **Fixtures 100 % sintéticos**: state tables, pantallas, FITs y tarjetas de ejemplo se
  construyen con un builder de fixtures (código nuestro), sin datos reales ni extractos del
  manual (regla de AGENTS.md).
- CI local obligatoria por cambio: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`
  (AGENTS.md).

## 8. Errores y observabilidad

- Errores tipados por crate con `thiserror`: `ParseError` (core), `EngineError` (engine),
  `NetError` (net). Sin `unwrap()` fuera de tests.
- Distinción clave: **error de transporte** (reconectar) vs. **error de protocolo** (se convierte
  en reject/solicited status según el manual) vs. **error interno** (bug: se loguea y se
  degrada, nunca pánico).
- `tracing` con spans por mensaje; dump hex de frames en ambos sentidos (era lo más útil del
  simulador previo y de electron-atm); nivel por `RUST_LOG`.

## 9. Riesgos técnicos

| Riesgo | Mitigación |
|---|---|
| ~~Encuadre del host real distinto al asumido~~ (Q1 resuelto: 2 bytes BE) | Codec en `oxi-net` configurable por robustez ante otros hosts |
| Comandos "clásicos" NDC+ no documentados en el manual (Q2/Q3 resueltos: solo NDC) | Parser tolerant-reader: `Unknown` preservado + log; reject según manual |
| Comportamiento del host ante unsolicited/faults (Q9 resuelto: simular todo) | Hardware virtual con catálogo completo de fallas inyectables por perfil (§9-63…§9-91) |
| Complejidad de security (Q4/Q8 resueltos: por defecto simple) | v1 ISO-0 + 3DES de prueba, sin MAC; local/ISO-3/MAC documentados como opciones del perfil |
| El dueño aprende Rust mientras avanzamos | Fases pequeñas con revisión; notas de diseño en `DESIGN.md` explicando cada patrón nuevo |

## 10. Qué falta para cerrar esta arquitectura

Resueltas en nov-2026: Q1 (encuadre 2B BE), Q2/Q3 (host solo Advance NDC), Q4/Q8 (seguridad por
defecto: ISO-0 + 3DES de prueba, sin MAC; opciones documentadas) y Q9 (simular todas las fallas
de los dispositivos modelados). Quedan abiertas Q11 (fixture "Download" real local), Q5 (tipos
P/Q/i/Z), Q6 (teclados), Q7 (supervisor) y Q10 (default close) — ninguna cambia la forma de los
crates ni el modelo de eventos/acciones; solo ajustan detalles de `oxi-engine`.
