# hook-run-log — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Verificado contra `develop` (`4bc5a96f`):

- `clawg hook` no deja rastro. Ante cualquier problema no imprime nada y sale con 0 (`crates/clawg-cli/src/hook.rs:6-7`), y `main` imprime la salida o nada (`crates/clawg-cli/src/main.rs:126-133`). Un hook que no inyectó, uno que no corrió y uno que corrió y no tenía memoria se ven igual.
- `respond` descarta el motivo: `answer` devuelve `Option<String>` (`hook.rs:281-288`), y una carpeta que no se pudo elegir (`hook.rs:194`), un daemon ausente (`hook.rs:290`), una raíz rechazada (`hook.rs:275`) y un contexto vacío (`hook.rs:304`) terminan en el mismo `None`.
- `clawg doctor` no sabe nada de Cursor: lo único del cliente que mira es el registro de Claude Code (`crates/clawg-cli/src/doctor/mod.rs:215`).
- Cursor no ayuda. Su propio log de hooks sólo dice `Hook step requested: workspaceOpen` ([verify de client-setup-cursor](../2026-11-04-client-setup-cursor/verify.md#pendiente-en-prod), punto 4), y su documentación dice de `sessionStart`: «This hook runs as fire-and-forget; the agent loop does not wait for or enforce a blocking response» (mismo archivo, punto 2).
- La prueba con Cursor real se hace en prod, con el binario publicado. Si un chat empieza sin memoria, hoy nadie puede decir si el hook corrió, qué mandó Cursor ni por qué no se inyectó nada.

## Objetivo que sirve

4: varios agentes, con el usuario controlando qué ve cada uno. Que el hook de Cursor sea diagnosticable es parte de que Cursor sea un agente más.

## Decisiones

Cerradas antes de escribir esta propuesta; no se reabren:

1. **Un renglón por inicio de sesión.** Un objeto JSON por línea en `<data>/hook-runs.jsonl`, sólo de `session-start` y de los dos clientes. `user-prompt-submit` escribiría en cada mensaje, y Cursor nunca llega a él.
2. **Campos:** `at_ms`, `client`, `agent`, `source`, `outcome`, `bytes` (lo que se imprimió en stdout), `elapsed_ms`, `roots` (cuántas `workspace_roots`, si se mandaron), `folder` (la carpeta usada, tal como llegó, sin normalizar: así se sabe la forma real en Windows), `cursor_version` e `input_keys` (los NOMBRES de los campos de primer nivel de la entrada).
3. **Privacidad, regla dura.** Nunca el valor de `user_email`, nunca el texto del contexto, ningún otro valor de la entrada fuera de los campos de arriba. Con varias raíces, sólo la cuenta, no las rutas.
4. **`outcome` dice qué pasó con la memoria:** `injected` (se imprimió un contexto no vacío), `empty_context` (el daemon respondió y el contexto no tiene texto; puede haber bytes, por ejemplo la línea del idioma o un aviso), `several_folders`, `no_folder`, `no_daemon`, `project_refused`, `daemon_error`, `timed_out`, `bad_input`, `other_client` (un hook de Claude Code que corre Cursor). La línea del idioma impresa sola conserva su motivo como `outcome`, y `bytes` muestra que se imprimió.
5. **Acotado y sin pisar.** La línea se arma entera y se agrega con una sola escritura. Al llegar a 64 KiB el archivo pasa a `hook-runs.1.jsonl` (reemplaza al anterior) y se empieza de nuevo: ninguna lectura-modificación-escritura, así que dos ventanas no pierden una línea en una reescritura. Cualquier falla se traga sin salida en stdout ni stderr. Nunca se crea el directorio de datos. Se registra antes de imprimir, para que una impresión que falla deje la evidencia.
6. **La salida no cambia.** `respond` conserva su firma y lo que devuelve; una función nueva devuelve la salida y la corrida, `respond` le delega y `main.rs` llama a la nueva. Las decisiones son puras y el registro vive en su módulo, `crates/clawg-cli/src/hook_log.rs`.
7. **`clawg doctor`:** un chequeo `hooks.runs`, siempre `Info`, sin umbrales (`openspec/README.md` difiere las advertencias de degradación hasta tener datos, y varios `outcome` no son fallas). Nunca cambia el código de salida. El mensaje lista las últimas 5 corridas en palabras, con su antigüedad (`hace 5 min`), y la `evidence` trae las últimas 20 completas. Si el hook de Cursor del perfil está instalado y no hay corridas, lo dice: es el caso que más importa, porque significa que Cursor nunca corrió el hook. Sin hook instalado y sin corridas, ningún hallazgo. Doctor no abre el store para esto.

## Alcance

1. `clawg-cli`:
   - `hook_log.rs`, nuevo: `Run`, `Outcome`, la línea, la ruta y el agregado con rotación, y la lectura tolerante.
   - `hook.rs`: el motivo de cada `None` de `answer`, y una función que devuelve la salida y la corrida.
   - `main.rs`: registra antes de imprimir.
   - `doctor/checks.rs` y `doctor/mod.rs`: el chequeo `hooks.runs` y su lectura.
   - `setup/cursor.rs`: una función que dice si el hook del perfil está instalado, con `locations` e `is_clawg_hook`.
2. Docs: `docs/reference/commands.md`, `docs/reference/data-profiles.md` y un párrafo corto en «Conectarlo a Cursor» del README.
3. `openspec`: esta propuesta, la [spec](spec.md), las [tareas](tasks.md), `verify.md`, `state.yaml`, la fila 118 de `README.md` y la línea de `ROADMAP.md`. Al archivar, HR1 a HR5 se suman a `specs/hooks` y HR6 y HR7 a `specs/doctor`, que queda fuera de este cambio.

## Fuera de alcance

- **Ver si apareció una ventana o si el agente leyó el contexto.** El registro dice lo que el hook hizo, no lo que Cursor hizo con ello: `additional_context` puede imprimirse y no llegar nunca al agente (`sessionStart` es fire-and-forget). Eso sigue siendo una prueba a mano en prod.
- Cambiar qué imprime el hook, el autoarranque (el hook no arranca un daemon y el puente sí, así que `no_daemon` se va a ver en un arranque real; registrarlo es el punto), o el contrato del agente.
- Umbrales o advertencias en doctor, y un subcomando para leer el registro: `clawg doctor` ya lo muestra.
- Registrar `user-prompt-submit`.
- Subir la versión y archivar.

## Cómo se mide el éxito

Tests en rojo primero. Los nombres, en la [spec](spec.md):

- un inicio de sesión de cada cliente deja una línea con los campos de la decisión 2, y cada motivo de la decisión 4 sale de su hecho;
- una entrada con `user_email`, un contexto y varias raíces no deja ninguno de esos valores en el archivo;
- el archivo rota a los 64 KiB, nunca crea el directorio de datos, y una falla de escritura no imprime nada ni cambia la salida;
- los tests de `clawg hook` que ya existen pasan sin cambiar lo que esperan;
- `clawg doctor` muestra las últimas corridas, dice que el hook está instalado y nadie lo corrió, no dice nada sin hook ni corridas, y su código de salida no cambia.

La suite y los dos clippy pasan en Windows. Una corrida a mano con el binario de tests y un home temporal muestra el archivo con los tres motivos y sin el correo, y la salida real de doctor.

`rag-eval`: no aplica, no hay cambio de ranking.

## Datos y vuelta atrás

No toca el store, el esquema, los identificadores guardados ni el contrato de embedding. Escribe un archivo nuevo en el directorio de datos, que ningún snapshot, `restore`, `export` ni `purge` lee ni borra (no es `memory.db`; el único enumerado de ese directorio es `backups/`). Volver atrás es borrar `hook-runs.jsonl` y `hook-runs.1.jsonl`, o no hacer nada: el binario anterior los ignora.
