# PENDING — Pendientes y limitaciones de oxi

> **Corte**: T8.4b (T/b/m), `main` en `8055f8e`.
> Este documento lista **lo que falta, lo que está parcial y lo que quedó
> fuera de alcance**. Las decisiones de protocolo con duda abierta viven en
> [`SPEC.md` §14](SPEC.md#14-preguntas-estado-de-resolución) (Q1–Q19); acá se
> agrupan por área y se agrega el trabajo necesario para cerrarlas.

---

## 1. Fase 8 — lo que queda

Nada de esto bloquea probar oxi contra un host que use el set del driver real
(A, B, D, F, H, I, J, K, W, X, Y + Z).

### 1.1 T8.1b — demo de ITR en el host falso (opcional)

**Qué falta**: el motor ya soporta ITR (clase 3/2) de punta a punta (T8.1),
pero `oxi-host-mock` **nunca manda un ITR**. El flujo normal contra el mock
usa sólo state tables, así que la GUI/CLI no tienen forma de mostrar una
pantalla dinámica ni el TRQ abreviado.

**Para cerrarlo**:

1. Agregar a `fixtures` un ITR sintético (pantalla, display flag, teclas
   activas, screen timer) y un guion opcional en `ScriptedHost` que lo mande
   después de un TRQ y espere el TRQ abreviado.
2. Exponerlo en el binario del mock (por ejemplo `--itr`), sin cambiar el
   guion por defecto.
3. Test de integración: TRQ → ITR → teclas → TRQ abreviado con buffer B.
4. Verificar en la GUI que la pantalla dinámica se dibuja y que el eco de
   teclas respeta el display flag.

**Supuestos ya tomados (Q14)**: FDK = letra `A`–`I`, dígitos tal cual,
relleno con ceros a la izquierda hasta 3 caracteres, `T` por timeout y `E`
por Cancel; TVN/MAC vacíos. Se ajustan si aparece una traza ITR real.

### 1.2 T8.4c — teclado completo (Q6)

**Qué falta**: hoy el teclado numérico, Enter/Clear/Cancel y las FDK A–I
están siempre cableados al motor. No se modela:

- la **asociación pantalla ↔ teclado** de la descarga (Cap. 4), ni
- los **teclados locales estándar** de 2.06+ (que el terminal real configura
  fuera del download), ni
- **áreas táctiles** (coordenadas, ≤64 áreas).

**Impacto**: bajo para el driver real (usa el layout de transacción), alto si
se quiere simular un terminal con teclados por estado o touch.

**Para cerrarlo**: definir con el dueño el modelo (Q6), guardar el mapa
tecla → return code por estado y hacer que `Terminal::handle` consulte ese
mapa en vez de aceptar siempre las teclas. Tests por estado y por teclado
inválido → close state.

### 1.3 Refinamientos de T/b/m (Q18, Q19)

Ya funcionan los flujos principales, pero quedan partes sin modelar:

| Estado | Pendiente | Referencia |
|---|---|---|
| T | Componer la pantalla/overlay de PIN temprano; hoy se muestra la pantalla de T y, si el PIN está incompleto, la de B | §2-52, Q18 |
| T | Lectura **DASH/chip**: el lector sólo entrega pistas magnéticas; la condición de chip (bit 3) no se evalúa | §2-4, §2-52 |
| b | Verificación **local** (entrada 3 de la extensión) y buffers U/V | §2-66, Q18 |
| b | Semántica exacta de `attempts = 000` (sin límite) y de la pantalla de mismatch con 1 intento | §2-66, Q18 |
| m | Verificación local, **operation codes**, buffer positions y multi-language screen bases | §2-71, Q19 |
| m | Composición de la pantalla de PIN con la selección de idioma | §2-71, Q19 |

### 1.4 Otras preguntas abiertas de protocolo

| # | Pendiente | Impacto |
|---|---|---|
| Q5 | Tipos `P`, `Q`, `i` legales en la descarga pero sin sección en el Cap. 2. Hoy no se interpretan (sólo `Z` como extensión) | Bajo |
| Q7 | Funciones de supervisor que usa el host real (replenish, settlement, test cash) | Alcance supervisor |
| Q10 | Número concreto del default close state en la customización real | Bajo |
| Q13 | ECPL opción 01 = `015` incluye el bit `004`, no documentado en el campo del CPL | Config |
| Q15 | Mapping `D0000`–`D0010` → pantalla reservada no publicado; el override se aplica sólo cuando la pantalla referencia esos IDs | Pantallas |
| Q16 | G condición 000: se usa la entrada 6 como posición decimal; el manual no es explícito | Amount check |
| Q17 | Overlay y language group de R/S no compuestos; S sólo localiza el código con PINDX track 2 start-sentinel forward | Idiomas |

**Cómo cerrarlas**: conseguir una traza real (o del driver) de cada caso,
comparar con el comportamiento actual y actualizar SPEC + tests. No se
inventa comportamiento mientras no haya evidencia.

---

## 2. Fuera de alcance v1 (por decisión de alcance)

Estas cosas **no están implementadas a propósito** y no son bugs. Están
documentadas en [`SPEC.md` §0, §12 y §13](SPEC.md).

### 2.1 EMV / chip / DASH

- No hay modelo de chip: la lectura de tarjeta sólo usa pistas 1/2/3
  sintéticas. La condición de lectura de chip se ignora.
- No se implementan los estados `k` (Smart FIT check), `L` (escritura de
  banda) ni la pantalla DASH de T.
- El TRQ no lleva datos de chip/EMV.

**Para agregarlo**: modelar un tag store EMV mínimo, la condición de chip en
A/T, el estado `k` y los campos EMV del TRQ. Es un proyecto aparte.

### 2.2 Seguridad

- **MAC/TVN desactivados** (Q4): el parser acepta el load de selección de
  campos MAC, pero no se calcula ni valida MAC, ni se genera/verifica TVN.
- **EPP/RSA/Base 94** fuera de alcance: oxi se presenta como terminal sin EPP.
- **PIN local** (PEKEY/POFDX/PDCTB) no está cableado a los estados B/M; el PIN
  siempre va al host (verificación remota, PCKLN=0 del driver real).
- **ISO-3 y BANKSYS** están documentados como opciones, pero sólo se forma
  **ISO-0**. KVV y EKC sí están implementados.
- `security.local_b_key` existe en el perfil, pero no hay flujo de
  verificación local que lo use.

### 2.3 Estados y hardware fuera de alcance o parciales

| Tipo | Motivo |
|---|---|
| `C` | No hay hardware de sobres: se toma el next state inmediato |
| `E` | Implementado; la opción 7a (keycode alternativo) no está modelada |
| `G` | Condición 003 exige cassette mapping y hace default close |
| `M` | PIN mejorado con reintentos vía Track 3: no implementado |
| `V` | Selección de grupo de pantallas por código de tarjeta: no implementado |
| `d`–`g` | Exit states programables: fuera de alcance v1 |
| `>` / `w` | Cash Accept (BNA) y Cheque Accept (CPM): hardware no configurado |
| Supervisor | Modo supervisor (Cap. 6): sin funciones replenish/settlement/test cash |
| EJ | Comandos EJ (clase 6): se declinan/ignoran, no hay journal upload |

Tampoco hay **depository de sobres** real: el contador existe en supplies,
pero no hay evento de depósito ni estado que lo use.

### 2.4 Impresión y pantallas

- La impresión es un **spool estructurado** (receipt/journal/statement), no
  un render fiel: no reproduce fuentes, gráficos, logos ni anchos reales.
- El render de pantallas (CLI y GUI) es **aproximado**: descarta SI/SO y las
  secuencias ESC (color, voz, logos, charsets) y convierte FF/CR/LF en saltos
  de línea. No posiciona el cursor ni compone pantallas anidadas.
- El override de reserved screens se aplica sólo al rango `D0000`–`D0010`
  (Q15); el resto de las reserved screens se trata como pantalla normal.

### 2.5 Perfil y configuración

- `ParseOptions::tolerant_driver_quirks` (Q12) existe en `oxi-core` pero **no
  está expuesto** en el TOML; un host que mande fecha/hora de 14 caracteres
  recibe `C/B08`.
- No hay configuración de MAC/TVN, ISO-3/BANKSYS ni verificación local.
- El perfil no permite definir un guion de host: el mock tiene fixtures
  canónicas y sólo alterna aprobar/rechazar.

---

## 3. Host falso (`oxi-host-mock`)

| Limitación | Detalle |
|---|---|
| Una conexión | `run_once` atiende **un** ATM por proceso; no es un host multipuerto |
| Fixtures fijas | La descarga es la canónica de `fixtures`; no se puede cargar una customización propia |
| Sin ITR | No manda mensajes 3/2 (ver §1.1) |
| Aprobar/rechazar global | `--reject` rechaza todos los TRQ; no hay reglas por importe/tarjeta |
| Sin MAC/TVN | No valida ni genera seguridad del host |
| Idle timeout | Cierra la sesión tras silencio (30 min por defecto, `--idle`) |
| Sin supervisor/EJ | No implementa comandos de supervisor ni journal upload |

**Para probar contra tu host real** no hace falta el mock (ver README,
“Probar contra tu host real”).

---

## 4. Interfaz (CLI y GUI)

- **Una sesión por proceso**: no hay pestañas ni varios ATM en la misma
  ventana.
- **Sin persistencia**: cada conexión vuelve a bajar la customización; no se
  guardan pantallas, state tables ni logs en disco.
- **GUI**: el render es aproximado (§2.4); no hay animación de lector,
  dispensador ni chip; no hay beep; el teclado touch no está modelado.
- **GUI en Windows**: con Smart App Control activo los binarios locales se
  bloquean; usar WSLg o desactivar SAC (ver
  [`WINDOWS-SAC.md`](WINDOWS-SAC.md)).
- **CLI**: los comandos son de demo (`card`, `pin`, `amount`, `fdk`, `cash`);
  no hay modo script ni captura de trazas a archivo.
- El eco local de PIN/importe es del motor y no viaja al host, pero la GUI
  dibuja `X`/`*` según el estado; no hay editor de pantallas.

---

## 5. Calidad y pruebas

- **Cobertura**: 255 tests verdes (nativos y WSL). No hay tests de propiedad
  para el intérprete de estados ni *fuzzing* del parser más allá de los
  `proptest` de `oxi-core`.
- **Sin CI**: el trío se corre localmente (`scripts/check-wsl.sh`); no hay
  workflow de GitHub Actions.
- **Sin benchmark**: no se mide latencia ni throughput del motor.
- **Datos**: todos los tests usan fixtures sintéticas; no hay trazas reales
  versionadas (por diseño).

---

## 6. Orden sugerido para retomar

1. **T8.1b** — demo ITR en el mock (cierra la fase 8 visible en la GUI).
2. **T8.4c / Q6** — teclado por estado, si el dueño define el modelo.
3. **Q18/Q19** — refinar T/b/m cuando haya traza real.
4. **Q12** — exponer `tolerant_driver_quirks` en el perfil (mejora chica).
5. **EMV/chip** — sólo si aparece un host que lo requiera; es un proyecto
   aparte.

Cada ítem se planifica como tarea del ROADMAP antes de codificarlo (regla de
oro de `AGENTS.md`).
