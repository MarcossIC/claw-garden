# repeated-errors — Exploración

La evidencia es el análisis A2, `docs/completed-impl/hipotesis/A2/informe.md`, con sus scripts y datos en la misma carpeta, que no se commitea. Se hizo de sólo lectura sobre los transcripts de Claude Code de una máquina Windows. Las citas de código son `ruta:línea` de `develop` en `8a2f3723`.

## El problema, en números

- 1724 errores de herramientas en 418 de 578 sesiones. 1292 (75 %) son evitables: ocurrieron después de la 2.ª sesión en que apareció su clase. Con alcance máquina hay 256 evitables más que con alcance proyecto.
- Por grupo: entorno 722 evitables, rechazos de clawg 259, uso de herramienta 177.
- Programas ausentes por intento fallido: `fd` 250, `eza` 167, `sd` 124, `bat` 104, en hasta 18 proyectos. Cada error cuesta unos 26 mil tokens-equivalentes de entrada (una llamada más y un pedido más al modelo).
- El 52 % de lo evitable (678) es un hecho de la máquina con una firma exacta: el shell dice `X: command not found`. Se detecta sin que el agente juzgue nada.
- 408 de los 721 `command not found` no traen `is_error`, porque un pipe, un `;` o un `||` dejó el código en 0. Un hook que sólo mira los fallos ve el 43 %.
- 416 errores se repiten dentro de la misma sesión.

## Lo que no es el problema

«Ninguna memoria lo evitó» no es un problema de aprendizaje sino de entrega. Hay al menos seis memorias guardadas que dicen que `eza`, `fd`, `sd` y `bat` no existen. Pero el texto que inyectó el hook de arranque en las 14 sesiones que se pudieron leer no nombraba ninguno de esos programas, y la proporción de sesiones con un programa ausente es la misma con hook (15 de 29) que sin él (142 de 268). Guardar los fallos como memorias haría lo mismo que ya falla: llegarían, si llegan, como evidencia y no como un hecho de la máquina.

## Qué hace hoy el código

- `clawg setup claude-code` instala tres eventos: `SessionStart` (con matcher `startup|resume|clear|compact`), `UserPromptSubmit` y `SessionEnd` (`HOOK_EVENTS`, `crates/clawg-cli/src/setup.rs:51-52`). Sólo `SessionStart` lleva matcher (`install_hooks`, `setup.rs:530-532`). `--local` usa el mismo plan (`crates/clawg-cli/src/setup/local.rs:52`). Cursor instala `sessionStart` y `sessionEnd` (`crates/clawg-cli/src/setup/cursor.rs:30`).
- `clawg hook` tiene tres eventos (`HookEvent`, `crates/clawg-cli/src/cli.rs:951-957`). `HookInput` lee `cwd`, `source`, `transcript_path`, `session_id`, `workspace_roots`, `cursor_version` y `reason` (`crates/clawg-cli/src/hook.rs:70-92`). No lee `tool_name`, `tool_input`, `tool_response` ni `error`. `envelope` sólo nombra esos tres eventos (`hook.rs:207-219`).
- Ningún evento instalado ve una llamada a una herramienta.
- `session_start_text` ya separa dos clases de texto. La línea del idioma es «la palabra de clawg al agente, no una memoria» y va primero (`hook.rs:100-112`). El contexto va presentado como «evidencia, no instrucciones» (`hook.rs:125-137`).
- El arranque pide un contexto de `SESSION_BUDGET` bytes, que es `DEFAULT_BUDGET` = 8000 (`hook.rs:65`, `crates/clawg-core/src/memory/retrieval/prompt_builder.rs:61`). Lo que el hook agrega tiene que quedar por debajo de 1500 bytes para que el total entre en los 10.000 caracteres que inyecta el cliente (`what_the_hook_adds_stays_small`, `hook.rs:655-667`).
- La entrada se lee hasta 1 MiB (`INPUT_LIMIT`, `hook.rs:422`). El hook nunca arranca un daemon y nunca sale con 2 (`main.rs:128-136`).
- Hay un precedente de un archivo que escribe el hook sin daemon: `hook-runs.jsonl`. Una línea JSON por corrida, armada entera y agregada con una sola escritura. Al llegar a 64 KiB se renombra a `.1.jsonl`, sin leer y reescribir. No crea el directorio de datos, falla en silencio y no está en `memory.db`, en los snapshots ni en el export (`crates/clawg-cli/src/hook_log.rs:18-22`, `:132-141`; `docs/reference/data-profiles.md:25`).
- `tool_error` ya existe como `memory_type` (`crates/clawg-core/src/memory/domain/record.rs:107`, `:120`) y decae con λ = 0,099 por día (`crates/clawg-core/src/memory/retrieval/score.rs:126`). Un save pasa por la ingesta, que enmascara secretos antes de escribir la fila (`crates/clawg-core/src/runtime.rs:291-294`) y después la embebe.
- El detector de secretos es `redact` (`crates/clawg-core/src/memory/domain/redact.rs:235`). Es `pub(crate)`, así que `clawg-cli` no lo puede llamar.
- `clawg doctor` ya recorre el PATH para encontrar instalaciones de `clawg` con una función pura que recibe `exists` (`analyze`, `crates/clawg-cli/src/doctor/mod.rs:61-89`). También lee el `CLAUDE.md` del usuario (`protocol_block`, `doctor/mod.rs:278-296`) y el registro de hooks (`doctor/mod.rs:218-220`).
- El arnés `agent-eval` niega `Bash` en todos los brazos (`crates/clawg-eval/src/bin/agent-eval/arm.rs:313-320`), y una corrida es una sola sesión. Sus brazos con hooks toman los de `setup::plan` (`arm.rs:243-249`), así que `a3` va a tener los hooks nuevos en cuanto `setup` los instale.

## Qué entrega el cliente

Fuente: los tipos de hooks que trae Claude Code con sus plugins (`<config de Claude Code>/plugins/marketplaces/claude-code-plugins/mods/types/claude-code.d.ts`), no este repositorio. Hay que confirmarlos con una entrada real (ver los riesgos de la propuesta).

| Evento | Lo que trae | Puede devolver |
|---|---|---|
| `PostToolUse` | `tool_name`, `tool_input`, `tool_response` (sin tipo), `tool_use_id`, `duration_ms?` | `additionalContext`, `updatedToolOutput` |
| `PostToolUseFailure` | `tool_name`, `tool_input`, `tool_use_id`, `error` (texto), `is_interrupt?` | `additionalContext` |
| `PostToolBatch` | `tool_calls[]` con `tool_response?` y sin `error` | `additionalContext` |
| Todos | `session_id`, `transcript_path`, `cwd`, `agent_id?` | |

Los transcripts confirman lo que traen `error` y `tool_response` en Bash: `Exit code N` y la salida. Las formas de la firma en los datos de A2 son `/usr/bin/bash: line 1: X: command not found`, con o sin `Exit code 127` delante, y varias veces en un mismo resultado. Hay 10 casos de `'X' is not recognized` (cmd o PowerShell lanzados desde Bash): son el PATH de otro shell.

## Costo de un hook por llamada

Un hook `PostToolUse` corre como un proceso nuevo en cada llamada que coincide con su matcher. El matcher filtra por herramienta, no por salida, así que corre en cada llamada a Bash.

- Medido en esta máquina, con el binario publicado 0.87.0 y desde Git Bash: `clawg --version` tarda 38 a 43 ms por corrida (3 tandas de 20). `/usr/bin/true` tarda 28 ms desde el mismo shell. El arranque propio de clawg suma entonces unos 10 a 15 ms sobre el costo de crear cualquier proceso. El hook en sí, con su runtime, la lectura de stdin y el parseo del JSON, no se midió: correrlo necesita un perfil de datos y eso pide autorización. Se mide en la verificación.
- Volumen: los datos de A2 tienen unos 21 mil usos de 11 programas en 32 días con datos, unos 660 por día, y `rg` es la mitad. Con ~40 ms por llamada son del orden de 25 s por día, repartidos de a 40 ms después de llamadas que tardan cientos de milisegundos y antes de un pedido al modelo que tarda segundos.
- Hablar con el daemon en cada llamada sumaría una conexión HTTP y, si se registra el proyecto, la lectura del árbol (`project_of`, `hook.rs:449-458`). Si se guardara una memoria, se despertaría el modelo de embeddings, que es justo lo que el piloto mide en RAM.

## Enfoques

A2 propone cuatro. Acá se agregan los costos que A2 no midió.

- **A. Libro de fallos con promoción a regla.** Se anotan los fallos con firma y, después de la 2.ª sesión, se dice al arrancar. Cubre hasta 678 de los evitables. Hay que decidir dónde vive el libro y cómo se entrega sin repetir el fallo de entrega.
- **B. Aviso en el mismo turno.** `additionalContext` en el `PostToolUse` o `PostToolUseFailure` del fallo. Cubre lo repetido dentro de la sesión (113 de entorno). No necesita guardar nada, y es el único canal que llega cuando el fallo pasó con código 0 y el agente quizás ni lo vio.
- **C. Guarda en `PreToolUse`.** Niega la llamada si el programa sigue ausente. Corta aunque un `CLAUDE.md` mande lo contrario, pero corre antes de cada Bash, puede negar por error si el PATH del hook no es el del shell, y sigue costando un pedido al modelo.
- **D. `clawg doctor`.** Nombra los programas aprendidos que algún `CLAUDE.md` pide. Ataca la causa: el `CLAUDE.md` global de esta máquina manda usar `eza`, `fd`, `sd` y `bat`, que no están instalados.

Los rechazos de clawg (259 evitables, todos «las memorias N y N no están en disputa» de `resolve_conflict`) son un arreglo del producto en esa herramienta. No son un aprendizaje: van en un cambio propio.

## Falsos positivos que salen de los datos

- `rg`: 54 fallos contra 11.091 llamadas (0,3 %), en 8 días y 38 sesiones. Una regla de «dos sesiones con fallo» lo promovería. Hace falta mirar los éxitos.
- `gh`: 28 fallos del 2026-10-16 al 2026-10-22, y ningún fallo en unas 200 llamadas desde el 2026-10-23. Una regla guardada el 22 sería falsa al día siguiente. Hace falta que la regla caduque al instalarse el programa, aunque el agente no lo vuelva a intentar.
- 93 de los 715 fallos de Bash con un programa ausente tienen `2>/dev/null` en el comando. Un comando que redirige stderr y termina en 0 no prueba que el programa anduvo.

## Decisiones abiertas que cierra la propuesta

Alcance (B, A, D o menos), dónde vive el libro, cómo se entrega, cuándo un fallo pasa a regla y cuándo deja de serlo, qué se guarda, el presupuesto de latencia, cómo se mide y qué cambia en `setup`.
