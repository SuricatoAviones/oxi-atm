# SPEC — oxi: protocolo NDC (APTRA Advance NDC)

> **Estado**: borrador v0.1, pendiente de revisión del dueño.
>
> **Fuentes**:
>
> - **Fuente de verdad**: manual oficial de APTRA Advance NDC (NCR, Issue 1, jun-2006). Material
>   confidencial que NO se versiona en este repo; aquí solo se citan capítulo/sección.
> - **Referencias secundarias**: simulador previo del dueño (`reference/atm`, confidencial, no
>   versionado) y el emulador público `electron-atm` (LGPL, sin mantenimiento). Lo que no proviene
>   del manual se marca como **[REF]** (coincidencia entre referencias) o **[PEND]** (a confirmar).
>
> Este documento está escrito para poder publicarse: no reproduce texto del manual ni datos
> reales de ninguna empresa. Todos los ejemplos son sintéticos.

## 0. Objetivo y alcance

`oxi` simula el **lado terminal (ATM)** del protocolo NDC para probar hosts y drivers centrales
de cajeros. El peer a probar es la aplicación Central (host), que en NDC no la provee el
fabricante: es la parte que implementa el dueño (Cap. 1, §1-3).

**Dentro del alcance (v1)**:

- Conexión TCP iniciada por el ATM hacia el host, con re-envío de encuadre configurable.
- Máquina de estados de terminal con las tablas descargadas por el host (State Tables Load).
- Recepción de customización: pantallas, teclados, FITs, parámetros de configuración, Config ID.
- Emisión de Transaction Request y procesamiento de Transaction Reply (incl. dispensado virtual,
  impresión virtual, devolver/retener tarjeta, next state).
- Estados de terminal: solicited status (incl. Terminal State / supply counters) y unsolicited
  básico (power-up).
- Excepciones: command reject / specific command reject y validación MCN.
- Verificación de PIN con ISO-0 + 3DES de prueba **por defecto** (PIN block en el TRQ,
  verificación remota); local (FIT PEKEY), ISO-3/BANKSYS y MAC/TVN documentados como
  **opciones configurables**, desactivadas en v1 (decisión del dueño, Q4/Q8).

**Fuera del alcance (v1)** — Cap. 15 de ROADMAP para más detalle:

- Dial-up CCM VISA2 (Apéndice O): completo, salvo que el host lo requiera.
- EMV/chip a fondo (mensajes clase 8, DASH, ICC): se "passthrough" minimal.
- BNA (aceptador de billetes) y CPM (cheques): modelados como no configurados.
- Seguridad EPP real (RSA initial key loading, Base 94, double length restricted).
- Exits (DLLs de extensión) y mensajes clase 5/7 (Exit to Host / Host to Exit).

## 1. Transporte y encuadre — **RESUELTO (Q1)**

El manual declara header y trailer de cada mensaje como "dependientes del protocolo" (§9-2,
§10-2): no define el encuadre sobre TCP ni una longitud máxima global (solo límites puntuales,
p.ej. 2000 bytes por mensaje de Screen/Keyboard Data Load, §10-11).

**Decisión del dueño (nov-2026)** — confirmada contra su host/driver:

- Prefijo de longitud de **2 bytes big-endian** (byte alto primero), seguido del mensaje ASCII.
- La conexión la inicia el **ATM** hacia el host (rol cliente TCP).
- El cuerpo usa FS (0x1C) como separador de campo y GS (0x1D) como separador de grupo
  (valor hex de FS documentado solo en §O-4; GS inferido de uso, **[REF]**).

`oxi` implementa este encuadre como codec **por defecto** en `oxi-net`; el codec sigue siendo
configurable para tolerar variantes si algún día se prueba contra otro host.

## 2. Estructura general de un mensaje

Referencias: §9-2, §10-2, §10-7.

- Primer campo: **clase** (1 char) y, en mensajes del host, flag de respuesta opcional.
- Campos separados por **FS**; bloques repetidos dentro de un mismo campo, por **GS**.
- Campos transversales: **LUNO** (3 chars; 9 con data security activa — 3 de LUNO + 6 de
  machine number, §9-2), **número de secuencia**, **TVN + MAC** si hay autenticación (§9-13).
- En data commands del host, **sub-clase** (1 char) + **identificador** (1 char) van contiguos
  sin FS entre sí (p.ej. `3` + `1` + `1` = Screen/Keyboard Data Load, §10-11).
- **MCN** (message coordination number): lo asigna el ATM a cada Transaction Request; rango
  0x31–0x3F (o 0x31–0x7E con opción 34, §10-27), rotativo; el host debe devolverlo en el
  Transaction Reply o usar `0` para anular la verificación (§9-2). Desmatch → Command Reject.

## 3. Mensajes ATM→host (Cap. 9)

| Mensaje | Clase/Sub | Propósito | Ref. |
|---|---|---|---|
| Transaction Request | 1/1 | Pistas 2/3 (+1 opc.), op-code, importe, buffer PIN (A), buffers B/C, estado de la transacción previa, datos opcionales | §9-2 |
| Respuesta a Interactive TR | 1/1 (abreviado) | Buffer B con teclas del cardholder tras una ITR | §9-12 |
| Solicited Status | 2/2 | Respuesta a todo comando del host | §9-13 |
| Encryptor Initialisation Data | 2/3 | Respuesta a Extended EKC (serial EPP, claves, KVVs) | §9-55 |
| Unsolicited Status | 1/2 | Espontáneo: power-up, alarmas, fallas, suministros, supervisor | §9-58 |
| Exit to Host | 5/1 | Datos de Exits (fuera de alcance v1) | §9-60 |
| Upload EJ Data | 6/1 | Bloques del journal electrónico; solo sin transacción activa | §9-61 |

**Descriptors de Solicited Status** (§9-13): `8` device fault, `9` ready, `A` command reject,
`B` ready (variante de Transaction Reply, configurable), `C` specific command reject, `F`
terminal status (con sub-campos según el comando respondido: config ID, contadores, hardware,
suministros, fitness, software release, notas BNA, etc., §9-19).

**TRQ — bloques** (§9-2): identificación (clase, sub-clase, LUNO, TVN), bandera de recibo + MCN,
pistas, op-code, importe, buffer A (PIN), buffers B/C, estado de la última transacción (serial,
último estado, billetes dispensados), datos opcionales identificados por ID (CSP, EMV, BNA, CPM).

## 4. Mensajes host→ATM (Cap. 10)

### 4.1 Terminal Commands (clase 1, §10-2)

| Código | Comando | Ref. |
|---|---|---|
| `1` | Go In-Service | §10-2 |
| `2` | Go Out-of-Service (modificador 0/1: pantalla normal/temporal) | §10-6 |
| `3` | Send Configuration ID | §10-2 |
| `4` | Send Supply Counters | §10-2 |
| `5` | Send Tally Information (no soportado en Advance NDC) | §10-4 |
| `6` | Send Error Log (no soportado) | §10-2 |
| `7` | Send Configuration Information (modif.: 1 hardware, 2 supplies, 3 fitness, 4 tamper, 5 software release, 6 extendida, 7 config local, 8 notas BNA) | §10-5 |
| `8` | Send Date and Time | §10-2 |
| `F`/`G` | Disconnect / No-Op (solo dial-up VISA II, Ap. O) | §10-4 |

### 4.2 Customisation Data Commands (clase 3, §10-7…§10-55)

Formato: clase `3`, sub-clase `1`, identificador (§10-7, Tabla 10-2):

| Id | Comando | Ref. |
|---|---|---|
| `1` | Screen/Keyboard Data Load (≤2000 bytes/msg, repetible) | §10-11 |
| `2` | State Tables Load (pares nº de estado + datos, repetibles) | §10-9 |
| `3` | Configuration Parameters Load | §10-14 |
| `5` | FIT Data Load (≤1000 FITs) | §10-31 |
| `6` | Configuration ID Number Load (cierra la descarga) | §10-33 |
| `A` | Enhanced Configuration Parameters Load | §10-21 |
| `B` | MAC Field Selection Load | §10-35 |
| `C` | Date and Time Load | §10-41 |
| `E` | Dispenser Currency Cassette Mapping Table | §10-51 |
| `G` | Override Reserved Screens | §10-55 |

Sub-clases: `2` Interactive Transaction Response (§10-79), `3` Encryption Key Change (§10-42),
`4` Extended Encryption Key Change (§10-45). `F` (Initialise EKC) no soportado (§10-53).

- **Override Reserved Screens** (§10-55): pares `D0000`–`D0010` + pantalla de usuario
  `000`–`999`; si el valor se omite o es inválido, revierte (Tabla Note 34). Un `D` fuera de
  rango se rechaza con `C/B18`. El manual no publica el mapping `D`→pantalla reservada; el
  simulador aplica el override cuando una pantalla referencia `D0000`–`D0010` (Q15).

> Los comandos "clásicos" Set Teller Totals / Initialize Terminal / Display Supervisor Frame /
> SendMKDAT / Send PPMA / Send Third Party Data **no aparecen** en este manual. Decisión del
> dueño (Q2/Q3): su host es **solo Advance NDC**; si alguno llegara, se trata como `Unknown` +
> reject según el manual (§4.5), sin casos especiales.

### 4.3 Transaction Reply (clase 4, §10-58…§10-74)

Campos principales: next state, conteos de billetes (4 campos, máx. 40 notas; monedas vía GS),
serial de transacción, **function identifier**, pantalla a mostrar (+updates), MCN, bandera
devolver/retener tarjeta, hasta 13 **grupos de impresión** (flag 0 nada, 1 journal, 2 receipt,
3 ambos; +PPD/statement/sideways, separados por GS), escritura de pistas 1/3, datos VC/EMV/cheque.

**Function IDs** (§10-60): `1`/`7` depositar e imprimir · `2`/`8` dispensar e imprimir ·
`3`/`9` desplegar e imprimir · `4` print immediate · `5` set next state e imprimir · `6` bolsa
nocturna · `A` tarjeta antes del dispensado · `B`/`C` dispensado paralelo (fast cash) ·
`D`–`G`/`H`–`L`/`M`–`R`/`S`–`U` variantes con impresión/cheque (detalladas en la tabla) ·
`P`/`Q` statement · `*`/`-`/`'` BNA · `:` CPM.

Cada función define la **secuencia de finalización del terminal** (dispensar → timer de
presentación → retract con reporte unsolicited; devolver tarjeta → timer de retiro → captura;
close → pantalla final con timer propio) (§10-68…§10-74). Termina con estado ready/fault al host
y transición al next state.

**Bandera de tarjeta** (§10-59): `0` devolver en Close, `1` retener, `4` devolver durante el
procesamiento del reply.

### 4.4 EJ Commands (clase 6, §10-82…§10-86)

Opciones/timers de subida, acknowledge de bloque, acknowledge & stop; métodos continuo y batch.
Fuera de alcance v1 (se declina con reject o se ignora según modo operativo).

### 4.5 Excepciones y modos operativos (§10-87…§10-97)

- **Command Reject** (`A`): clase ilegal (legales: 1, 2, 3, 4, 6, 7 y 8-EMV), sub-clase o
  identificador ilegal, FS mal ubicado, campos insuficientes, MCN sin match, function ID ilegal
  o con hardware no configurado, >40 billetes, >13 grupos de impresión, etc. (§10-87).
- **Specific Command Reject** (`C`, con opción 12 activa, §10-88): par valor/calificador —
  `A` formato, `B` valor de campo (01 clase, 02 sub-clase, 06 function ID, 09 MCN, 11 demasiados
  billetes, 18 pantalla…), `C` tipo ilegal para el modo operativo, `D` hardware, `E` no soportado.
- **Mensajes según modo operativo** (§10-95): tablas Power-Up / Out-of-Service / Supply /
  In-Service que definen aceptar (`A`), rechazar (`R`), responder con Config ID (`C`), retener
  hasta cierto modo (`P`, `A1`, `A2`, `A3`). En servicio, los comandos se retienen hasta quedar
  inactivos; uno nuevo pisa al anterior sin respuesta (§10-97).

## 5. Máquina de estados del terminal (Cap. 2)

### 5.1 Estructura (§2-1)

Cada estado = **número** (000–254, 256–999) + **tipo** (1 char) + **datos de tabla**. Patrón
general: al entrar se muestra la pantalla, se ejecuta la acción del tipo y el flujo sigue al
next state. Next state inválido o no descargado → **default close state** (cierra entregando
recibo/statement y devolviendo o capturando tarjeta). `255` = desactivar (timer o tecla). La
descarga se hace por State Tables Load (§10-9); el primer char de los datos es el tipo.

### 5.2 Tipos de estado

| Tipo | Función | Ref. |
|---|---|---|
| A | Lectura de tarjeta (recomendado como estado 000); inicializa buffers; define pistas/chip a leer y condiciones de lectura | §2-4 |
| B | Entrada de PIN (4–16 dígitos), verificación local o remota, reintentos | §2-8 |
| C | Dispensador de sobres | §2-11 |
| D | Pre-carga del buffer de op-code (máscaras por tecla) | §2-12 |
| E | Selección de 4 FDK derechas (A–D) | §2-15 |
| F | Entrada de importe (8/12 dígitos) | §2-17 |
| G | Chequeo de importe (4 condiciones: decimal cero, límite, múltiplo, dispensable) | §2-19 |
| H | Entrada de información a buffer B/C | §2-23 |
| I | Transaction Request + ejecución del Reply | §2-25 |
| J | Close (entrega/captura de tarjeta, recibo, statement) | §2-30 |
| K / `_` | FIT switch (índices 0–7 / 7–14) | §2-35, §2-36 |
| L | Escritura de banda magnética | §2-38 |
| M | PIN entry mejorado (reintentos vía Track 3) | §2-40 |
| N | Cámara (**no soportado**) | §2-43 |
| R | Entrada de importe multilingüe | §2-48 |
| S | Selección de idioma por código de tarjeta | §2-50 |
| T | Lectura de tarjeta + habilitación temprana de teclado (DASH/EMV) | §2-52 |
| V | Selección de grupo de pantallas por código de tarjeta | §2-57 |
| W | Switch por keycode guardado en FDK buffer | §2-58 |
| X | FDK information entry (valor por FDK a buffer B/C/amount) | §2-59 |
| Y | Selección de 8 FDK (A–I) | §2-62 |
| b | PIN elegible por el cliente (CSP) | §2-66 |
| d…g | Exit states (programables; fuera de alcance v1) | §2-69 |
| k | Smart FIT check (match con datos de chip) | §2-70 |
| m | PIN + selección de idioma por FDK | §2-71 |
| `>` | Cash Accept (BNA) | §2-77 |
| w | Cheque Accept (CPM) | §2-82 |
| — | Time-Out State (fijo, pantalla C00) | §2-89 |

Extensiones de estado (tipo `Z`): añaden entradas 2–9 a D, I, J, `_`, R, S, T, X, Y, b, m, `>`, w
(§2-12…§2-86). Nota: en la descarga (§10-9) también figuran los tipos `P`, `Q`, `i`, `Z` como
legales, sin sección propia en el Cap. 2 — **[PEND: Q5]**.

### 5.3 FDKs y máscaras (§2-2, Nota 18 §2-61)

- FDKs 'A'–'I'; Enter = FDK 'A'; Cancel y Clear con return codes propios (Cap. 4 §4-2).
- **Máscara activa** (3 chars decimales, 000–255): bit 0 = FDK 'A', bit 1 = 'B', bit 2 = 'C',
  bit 3 = 'D', bit 4 = **'F'**, bit 5 = 'G', bit 6 = 'H', bit 7 = 'I'. **La tecla 'E' no participa
  de la máscara** (el ejemplo del manual: 060 → C, D, F, G). Trampa clásica de implementación.
- En la ITR, el campo de teclas activas es una **cadena binaria** donde el primer bit habilita el
  teclado numérico y los siguientes corresponden a las FDKs (§10-79) — formato distinto a la
  máscara decimal de las state tables.
- **ITR**: se acepta sólo In-Service y en estado I (Tabla 10-38 'A2'); el display flag es
  `0`=no mostrar, `1`=mostrar, `2`=`*` por tecla. El screen timer va en ticks de 800 ms
  (`000`=indefinido, `001`–`255`). La respuesta es un **TRQ abreviado** (Tabla 9-2) con buffer B;
  al vencer el timer el último char es `T` y con Cancel es `E` (Tabla 9-1 campo m). Ver Q14 para
  la convención de FDK/relleno adoptada por el simulador.

### 5.4 Time-Out State (§2-89)

Estado fijo al que se entra al expirar: timer 00 (teclado), timer 04 (sobre no insertado),
timer 08 (bolsa nocturna), screen timer de ITR, timers 94/77/78 (cheques/billetes BNA).
Al entrar: pantalla **C00** + timer 01 + beep. FDK 'A' → reintentar; FDK 'B'/timer 01 →
salida Time-Out del estado original; Cancel → salida Cancel. Los timers son configurables por
CPL/ECPL (Cap. 7).

## 6. Pantallas (Cap. 3, Ap. A)

- Una pantalla es una **cadena de texto con caracteres de control embebidos**, no un objeto
  gráfico (§3-1). Máx. 9999 customer screens (IDs 0000–9999); grupos de idioma ×1000 (§3-33).
- **Reserved screens**: letra de tipo + número — `C00–C07` cardholder display (C00 timeout,
  C01 Off-Line, C02 Out-of-Service, C04 tarjeta retenida…), `G00–G06` gráficos estándar,
  `K00–K10` controles extendidos, `L00–L15` logos, `M` menús de supervisor; Apéndice A agrega
  `A`/`E`/`I`/`M`/`P`/`S`/`T`/`U`/`i` (§3-3…§3-10, §A-1).
- Control characters en el texto: `FF` clear, `SO` anidar pantalla (≤5 niveles), `SI` posicionar
  cursor, `ESC` + extendidos (color, voz, logo, charsets…), `HT` nombre de Track 1 (§3-14…§3-31).
- **Eco local de captura** (presentación del terminal, no viaja al host): el estado B muestra una
  `X` por cada tecla de PIN (§2-8); el estado F muestra el importe digitado y Clear limpia buffer
  y eco (§2-17); el estado H muestra `X` con la entrada 9 `000`/`002` o los datos con `001`/`003`
  (§2-23). El motor expone este eco como `InputEcho` y la GUI lo dibuja en el área de pantalla.
- Carga por Screen/Keyboard Data Load (§10-11); anulación parcial por Override Reserved Screens
  (§10-55). `%` en reserved screens = placeholder dinámico (§3-10).
- El número del último estado ejecutado se muestra arriba-izquierda del CRT (diagnóstico, §2-1).

## 7. Teclados (Cap. 4)

- "Teclado" NDC = **mapa tecla → return code**, no hardware (§4-1). Físico o touch (áreas
  rectangulares por coordenadas, ≤64 áreas, §4-5).
- Layout de transacción: teclas 0–9 → 30–39 hex; FDK 'A'–'I' → F1–FA hex (Enter = FDK 'A' = F1,
  Cancel = F2, Clear = FF) (§4-2).
- Teclados habilitados por estado en: B/M, E, F/R, H, I, T, X, Y, b, exit states, Time-Out, `>`,
  ITR (§4-7). Sin asociación válida → layout por defecto; error → close state.
- En 2.06+ la asociación pantalla↔teclado de la descarga se ignora (se configura localmente en
  el terminal real, §4-8) — **[PEND: Q6]** para el simulador.

## 8. Impresión (Cap. 5)

Receipt, journal, statement y PPD vía CEN-XFS (§5-2). Datos llegan en los printer data fields
del Transaction Reply (hasta 13 grupos, §10-58). PPD default print: serial, importe, Track 2
(30 chars), fecha/hora, machine number (§5-4). Controles `LF`/`FF`/`SO`/`HT`/`ESC` extendidos
(§5-6, §5-7). El simulador implementará "spool virtual" (log estructurado) sin render real v1.

## 9. Modo supervisor (Cap. 6, Ap. L)

Mensajes de supervisor sobre pantallas reserved (`I`, `M`, `P`, `S`, `T`) e impresión; el terminal
reemplaza `%` por datos propios (contadores, fecha, KVVs — "automatic screen editing", §6-3).
Simulación mínima: entrada/salida de supervisor por función (opción ECPL 33, §10-25) y pantallas
de menú estándar; **[PEND: Q7]** qué menús necesita el dueño.

## 10. FITs (Cap. 8)

- Una FIT por institución y tipo de tarjeta; al leer la tarjeta se busca la FIT cuyo FIID
  coincida (§8-1). Carga: FIT Data Load (§10-31), pares nº de FIT + datos (≤1000).
- **Estructura plana** (§8-2): cada entrada = 3 chars decimales (000–255) que codifican un byte
  (dos nibbles); la Tabla 8-1 suma 41 bytes de datos. **Verificación con el download real
  (nov-2026)**: el driver del dueño envía **40 entradas** por FIT (el nº de FIT + 40 grupos, sin
  el último campo PBFMT) — oxi acepta 40 o 41 y aplica default cero a lo omitido; no seguimos
  ciegamente la nota "fewer than 41 → no values stored" de §10-31 porque contradice el
  comportamiento del driver real verificado. Campos omitidos = 0 (§8-2).
- **No hay "bloques" B1/B2/B3** en la FIT — ese concepto pertenece a las state tables
  (aclaración importante para no confundir modelos).
- Campos (§8-3…§8-11): PIDDX/PFIID (identificación de institución), PSTDX (indirect next state),
  PMXPN/PCKLN/PINPD (PIN local), PANDX/PANLN/PANPD (PAN), PRCNT (reintentos Track 3), POFDX/
  PDCTB/PEKEY (offset/decimalización/clave local), PLNDX (idioma), PBFMT (formato de PIN block:
  ISO-0/ISO-3/BANKSYS), PINDX (pista/delimitador/dirección por offset).
- Linked FITs solo para verificación local de PIN (§8-12).

## 11. Configuración del terminal (Cap. 7)

- **CPL** (3/1/3, §7-2): LUNO, bitmask supply/ready/amount-buffer (001 ready, 002 supply,
  008 buffer de 12 dígitos — se suman), timers en ticks de 800 ms (000–255).
- **ECPL** (3/1/A, §7-5, §10-22…§10-28): pares repetibles opción/código. Principales: 1
  ready/supply/buffer, 3 formato de fecha (MMDDYY/DDMMYY), 4/5 ancho de rollo y columna
  izquierda, 7 formato de Track 1, 12 specific command reject, 15 transaction status info en el
  TRQ, 33 supervisor door, 34 rango de MCN extendido, 37 print Track 2 al journal, 44/45 BNA,
  77 estado inicial cardless.
- **Timers** (§7-3, §10-17…§10-20; defaults 30 s si no se bajan, §10-29; en ECPL en segundos):
  00 teclado, 01 respuesta a C00, 02 pantalla de close, 03 respuesta de comunicaciones (TRQ),
  04 sobre, 05 retract de efectivo, 07 presentación de efectivo, 09 retiro de tarjeta, 72 DASH,
  77/78 BNA, 87 cheque capturado, 92 display de fallas, 94 retiro de cheque, 95/96 statement.
- **Config ID** (§10-33): cierra la tanda de customización; el terminal lo reporta en solicited
  status con descriptor `F`.

## 12. Seguridad (Cap. 11)

- PIN block **ISO-0** (control '0', pad ANSI X9.8), ISO-3 (control '3', pad aleatorio 10–15) y
  BANKSYS (control '1', pad aleatorio 0–15): 16 hex, XOR del PIN field con el PAN block (PAN sin
  dígito verificador, 12 menos significativos, relleno izquierdo) (§11-3…§11-6). Elección por
  PBFMT (FIT) + PINPD.
- Cifrado del PIN: DES/3DES con la clave de institución (PEKEY) bajo master, o envío para
  verificación remota (§11-6).
- **MAC**: ANSI X9.9 (simple) / X9.19 (triple), cadena CBC de bloques de 8 bytes, 4 bytes finales
  → 8 chars hex (§11-8). Full vs. selective con tabla descargable (MAC Field Selection Load,
  §10-35). **TVN**: derivado del reloj del terminal; el host lo devuelve; mismatch → reject (§11-9).
- **KVV**: cifrar 8 bytes cero con la clave, tomar 6 hex izquierdos (§11-18).
- EPP/RSA/Base 94/double-length-restricted (§11-19…§11-30): fuera de alcance v1; oxi se
  presentará como terminal "sin EPP" con ISO-0 + 3DES de prueba.
- **Decisión del dueño (Q8)**: v1 arranca con ISO-0 + 3DES con claves de prueba sintéticas y
  PIN remoto (PIN block en el TRQ). Opciones documentadas y configurables en el perfil, para
  activar después sin rediseño: verificación local vía FIT (PEKEY + POFDX/PDCTB), ISO-3/BANKSYS,
  MAC full/selective con tabla de selección (§10-35) + TVN, y Encryption Key Change.

## 13. Hardware virtual y estados de dispositivo

- Identificadores de dispositivo y datos de hardware/supplies/fitness: Ap. H (§H-2…§H-10);
  product classes en byte 34 de hardware configuration (§9-30).
- Estado por dispositivo en solicited/unsolicited status: reloj, energía, lector/grabador,
  cash handler, depository, receipt/journal/statement printer, encryptor, sensors, supervisor
  keys, BNA, envelope dispenser, CPM (§9-63…§9-91).
- oxi modelará: dispensador con 4 casetes virtuales + conteos, lector/grabador (3 pistas
  sintéticas), receipt/journal/statement como spool, depository de sobres, reloj, energía,
  encryptor (estado), sensores y modo supervisor como eventos. **Decisión del dueño (Q9): se
  simulan TODAS las fallas documentadas de los dispositivos modelados** (§9-63…§9-91), con
  inyección desde el perfil de terminal. BNA/CPM siguen sin configurar en v1 (hardware no
  configurado → specific reject C02/C18 según §10-88); sus fallas se agregarían junto con los
  dispositivos si el dueño los pidiera.

## 14. Preguntas: estado de resolución

**Resueltas (nov-2026, decisiones del dueño)**

| # | Pregunta | Decisión |
|---|---|---|
| Q1 | Encuadre del host | **2 bytes big-endian**, confirmado; codec configurable por robustez |
| Q2/Q3 | Modo del host | **Solo Advance NDC**; no se esperan comandos clásicos NDC+; si llegan → `Unknown` + reject según §4.5 |
| Q4 | MAC/TVN | **Desactivado en v1**; diseñado y documentado como opción del perfil (`security.*`) |
| Q8 | PIN | **Por defecto**: ISO-0 + 3DES con claves de prueba sintéticas, verificación remota; local (PEKEY), ISO-3/BANKSYS documentados como opciones |
| Q9 | Fallas/unsolicited | **Simular TODAS las fallas** de los dispositivos modelados (§9-63…§9-91), inyectables por perfil |
| Q11 | "Download" real del host | **Resuelto**: disponible en `reference/download/` (local, sin versionar) y verificado contra el manual. Tipos de estado usados por el driver real: **A, B, D, F, H, I, J, K, W, X, Y + Z** (extensiones). Sin E, G, C, M, R, S, T, V, b, k, m, `>`, w. ECPL con specific command reject (op. 12) y transaction status info en TRQ (op. 15) **activados**; FIT con verificación de PIN **remota** (PCKLN=0) |
| Q12 | Date/Time | **Resuelto con evidencia** (driver clonado local + trace de electron-atm): el manual exige 10 chars `YYMMDDHHMM` (§10-41); la versión del driver que generó el trace enviaba **10 chars conformes** (`2502221139` = 25-02-22 11:39); el **código actual** del driver inserta `yyyyMMddHHmmss` (**14 chars**) — una regresión posterior que un terminal real rechazaría (mes inválido al parsear posiciones). **Decisión aprobada por el dueño**: oxi es **estricto por defecto** (10 chars; 14/vacío/inválido → specific reject `C/B08` + warning que explica la divergencia). **Opción documentada (no expuesta en el perfil v1)**: `ParseOptions::tolerant_driver_quirks` en `oxi-core` acepta el formato de 14 chars con warning; exponerlo como `[protocol] tolerate_driver_quirks = true` en el TOML queda como mejora opcional para quien quiera probar un driver con esa regresión sin arreglarlo. Nota: el campo "vacío" del ECPL es el LUNO — el driver inserta `cajero.Codigo` en runtime (§10-21 campo 'h') |

**Abiertas**

| # | Pregunta | Impacto |
|---|---|---|
| Q13 | ECPL opción 01 = `015` incluye el bit `004`, no documentado en el campo 'm' del CPL (§10-14 solo documenta 001/002/008). El código del driver (local, `reference/atm-driver`) puede responderlo cuando lo analicemos a fondo. | Config |
| Q5 | ¿Qué hacer con los tipos de estado `P`, `Q`, `i`, `Z` legales en la descarga (§10-9) sin sección en el Cap. 2? ¿Los tratamos como inválidos → default close? (`Z` ya resuelto: estado de extensión, usado por el driver real) | Intérprete (fase 3) |
| Q6 | ¿La descarga real trae asociación pantalla↔teclado, o teclados locales estándar (2.06+)? | Modelo de teclados |
| Q7 | ¿Qué funciones de supervisor usa tu host realmente (replenish, settlement, test cash)? | Alcance supervisor |
| Q10 | ¿Default close state: tu customización usa un número concreto que debamos respetar? | Máquina de estados |
| Q14 | ITR: el manual no especifica cómo representar una FDK en buffer B ni el relleno (sólo mínimo 3 chars y `T`/`E` finales). Convención del simulador: FDK = letra A–I, dígitos tal cual, relleno con ceros a la izquierda; TVN/MAC vacíos. Se ajusta si aparece una traza ITR real. | ITR (fase 8) |
| Q15 | Override Reserved Screens: el manual limita `D0000`–`D0010` y el valor `000`–`999`, pero no publica el mapping `D`→pantalla reservada. El simulador aplica el override cuando una pantalla referencia `D0000`–`D0010`; validar con una traza real. | Pantallas reservadas (fase 8) |

---

*Documento generado como parte del análisis previo a la implementación. Las decisiones de
diseño están en `docs/ARCHITECTURE.md` y el plan de trabajo en `docs/ROADMAP.md`.*
