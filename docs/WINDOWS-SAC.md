# Smart App Control y oxi en Windows

> Investigación local + documentación oficial, 24-nov-2026.
> Conclusión corta: **no es un bug de oxi ni de Rust**. Windows aplica una
> política de integridad de código (Smart App Control / WDAC) que bloquea
> ejecutables y DLLs sin reputación en la nube ni firma de una CA confiable.

## 1. Cómo decide Smart App Control

Según Microsoft, Smart App Control (SAC) combina inteligencia de aplicaciones
en la nube con *Code Integrity* de Windows:

1. Si el servicio puede predecir que el binario es seguro, lo deja ejecutar.
2. Si no puede predecir, deja ejecutar sólo si tiene una **firma válida de una
   CA del Microsoft Trusted Root Program**.
3. Si es desconocido y no está firmado (o la firma es inválida), lo bloquea.

SAC corre en modo **evaluación** (observa) o **enforcement** (bloquea). No
tiene lista de permitidos por aplicación: la FAQ oficial dice que no hay forma
de exceptuar una app concreta; la alternativa que sugiere Microsoft es firmarla
o desactivar SAC.

Referencias: [Smart App Control (Microsoft
Learn)](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/smart-app-control),
[FAQ de soporte](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions),
[Code signing para
SAC](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control).

## 2. Evidencia en esta máquina

- Windows 11 25H2, build **26200.7462**.
- `HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy`:
  `VerifiedAndReputablePolicyState = 1` (enforcement) y
  `SAC_EnforcementReason = 1`.
- El error `4551` es `0x11C7` =
  **`ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION`** (`winerror.h`); el mensaje
  localizado es “Una directiva de Control de aplicaciones bloqueó este
  archivo”.
- `Microsoft-Windows-CodeIntegrity/Operational` registra eventos **3077** y
  **3033** con la política
  `{0283ac0f-fff1-49ae-ada1-8a933130cad6}` (el issue de Microsoft Docs la
  identifica como `VerifiedAndReputableDesktop`, la política de SAC).

Binarios bloqueados observados:

| Binario | Cuándo | Efecto |
|---|---|---|
| `build_script_build.exe` (num-traits) | Cargo lo compila y lo ejecuta | `failed to run custom build command` / “never executed” |
| `tracing_attributes-*.dll` (proc-macro) | `rustc` lo carga al compilar | `can't find crate for tracing` (síntoma engañoso) |
| `rustfmt.exe` | `cargo fmt` | `os error 4551` |

`Get-AuthenticodeSignature` sobre `cargo.exe`, `rustc.exe`, `rustfmt.exe`, el
build script y el proc-macro devuelve **`NotSigned`**. En esta máquina `cargo`
y `rustc` llegaron a arrancar (la nube puede tener reputación para esas
versiones), pero cada artefacto nuevo generado por el build no tiene reputación
y cae en la política. La reputación es por archivo/hash y puede cambiar con el
tiempo; el issue de Microsoft Docs documenta el mismo `cargo.exe` funcionando
un día y bloqueado al siguiente.

El issue con el repro completo y la confirmación de Microsoft:
[MicrosoftDocs/windows-dev-docs#6139](https://github.com/MicrosoftDocs/windows-dev-docs/issues/6139).

## 3. Por qué Rust sufre esto más que otros stacks

Un build de Rust no es un solo `.exe`:

- Cargo compila y **ejecuta** pequeños `build_script_build.exe` de las
  dependencias.
- Los **proc-macros** son DLLs que `rustc` carga durante la compilación.
- `cargo test` genera un `.exe` de test por target; `cargo fmt`/`clippy` usan
  `rustfmt`/`clippy-driver`, también sin firma.
- El binario final (`oxi-gui.exe`) es otro PE nuevo, sin reputación.

SAC evalúa **cada archivo** cuando se carga/ejecuta. Por eso no alcanza con
firmar el `.exe` final: el build nativo falla antes, en los artefactos
intermedios. Mover `CARGO_TARGET_DIR` tampoco ayuda (la decisión es por
archivo, no por ruta).

## 4. Qué NO destraba el bloqueo

- **Modo desarrollador**: no es una excepción. La FAQ menciona que un perfil de
  desarrollador puede hacer que SAC se apague automáticamente, pero no es un
  bypass garantizado; el issue de Microsoft Docs lo confirma.
- **Certificado autofirmado**: `Get-AuthenticodeSignature` puede decir `Valid`
  si el certificado está en `LocalMachine\Root`, pero SAC no lo acepta. Hace
  falta una CA del Microsoft Trusted Root Program (o Artifact Signing).
- **Firmar sólo el `.exe` final**: no arregla build scripts, proc-macros,
  `rustfmt` ni el arnés de tests.
- **Exclusiones de Defender / “Desbloquear” el archivo**: no aplican a la
  política de Code Integrity.
- **Renombrar la extensión o mover el archivo**: el chequeo es sobre la imagen
  PE, no sobre el nombre.

## 5. Opciones reales

### A. WSLg (la que usa oxi hoy)

Los binarios Linux (ELF) corren dentro de la VM de WSL y no pasan por Code
Integrity de Windows. La ventana la dibuja WSLg con componentes de Microsoft.
Es la vía recomendada si no querés tocar la seguridad de Windows:

```powershell
wsl.exe -e bash -lc "cd /mnt/c/Users/<tu-usuario>/Desktop/oxi-atm && cargo run -p oxi-ui --features gui --bin oxi-gui -- --config perfil.toml"
```

### B. Desactivar Smart App Control

Seguridad de Windows → Control de aplicaciones y navegador → Smart App Control
→ **Desactivado**. Después hay que reconstruir desde cero (los artefactos
viejos pueden quedar bloqueados por hash):

```powershell
Remove-Item -Recurse -Force .\target
cargo run -p oxi-ui --features gui --bin oxi-gui -- --config perfil.toml
```

Según la FAQ vigente, en builds recientes se puede **volver a activar sin
reinstalar**; en builds viejas el interruptor era unidireccional. Para volver a
activarlo, los datos de diagnóstico opcionales deben estar habilitados. Es una
decisión de seguridad del dueño de la máquina: SAC deja de bloquear apps
desconocidas mientras esté apagado.

### C. Otra máquina o VM sin SAC

Windows Sandbox/una VM de desarrollo sin la política, o un equipo que no tenga
SAC en enforcement. Es la opción si no se quiere desactivar SAC en la máquina
principal.

### D. Compilación cruzada + firma (avanzado, no recomendado para iterar)

Compilar desde Linux/WSL para Windows evita ejecutar build scripts de Windows,
pero el `.exe` final sigue siendo un PE nuevo: para que SAC lo deje correr
necesitaría una firma de una CA confiable (o reputación). No resuelve el ciclo
de desarrollo nativo (`cargo test`, `rustfmt`, etc.) sin SAC apagado.

## 6. Diagnóstico rápido

```powershell
# Estado de la política (1 = enforcement)
Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' |
  Select-Object VerifiedAndReputablePolicyState, SAC_EnforcementReason

# Eventos de bloqueo (requiere permisos de lectura del log)
Get-WinEvent -LogName 'Microsoft-Windows-CodeIntegrity/Operational' |
  Where-Object { $_.Id -in 3033, 3077 } |
  Select-Object -First 10 TimeCreated, Id, Message

# Nombre de la política (requiere consola elevada)
CiTool.exe -lp
```

## 7. Fuentes

- [Smart App Control — Microsoft Learn](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/smart-app-control)
- [Smart App Control FAQ — Microsoft Support](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions)
- [Sign your app for Smart App Control compliance](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control)
- [Rust development environment on Windows is unusable with Smart App Control — MicrosoftDocs/windows-dev-docs#6139](https://github.com/MicrosoftDocs/windows-dev-docs/issues/6139)
- `winerror.h` (Windows SDK): `ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION = 4551L`.
