# client-setup — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S–M | memory-protocol (22), agent-lifecycle-hooks (25). El registro MCP con `--agent` fijo no depende de ninguno de los dos y puede ir primero | exploración (borrador) |

**Tesis:** conectar clawg a Claude Code hoy son tres pasos manuales en PowerShell y uno solo de ellos, `--agent`, decide de quién son las memorias. Si el usuario se lo salta, el agente es lo que el cliente diga de sí mismo, y un cambio de nombre deja lo guardado a nombre de otro agente: se encuentra, pero ya no se puede listar, cambiar, borrar ni resolver como propio. Un comando que haga el registro entero, con la ruta absoluta y la identidad fijada, y que se pueda repetir y deshacer, cierra ese hueco para el objetivo 4 (varios agentes, el usuario controla qué ve cada uno): sin identidad estable, el control de `sharing.json` apunta a nombres que se mueven.

## Problema (verificado contra el código)

### El alta es manual

- No hay comando de alta. `Command` tiene `Serve`, `Mcp`, `Status`, `Stop`, `Backup`, `Restore`, `Secrets`, `Purge`, `Sharing`, `Doctor`, `Memories` y `Path` (`crates/clawg-cli/src/cli.rs:32-126`). Nada registra el servidor ni toca la configuración de un cliente.
- `scripts/install.ps1` instala el binario y arregla el PATH; no menciona MCP ni Claude Code (`rg` por `mcp|claude` en el script sólo da la línea 181, un mensaje sobre el daemon).
- El registro lo documenta el README, a mano: `claude mcp add --scope user clawg -- "$env:LOCALAPPDATA\Programs\clawg\clawg.exe" mcp --profile prod` (`README.md:221`), y aparte la variante con `--agent claude-code` (`README.md:246`). El de desarrollo, con otra sintaxis (`README.md:367`).
- El README ya explica el riesgo de no fijar el agente (`README.md:241`), pero como una sección **posterior** y opcional al paso principal. Quien copia el primer comando queda sin `--agent`.
- El protocolo de uso (cuándo guardar, buscar, cerrar la sesión) no está en ningún archivo que Claude Code cargue de por sí. Hoy sólo viaja en `INSTRUCTIONS` del servidor MCP (`crates/clawg-cli/src/mcp/server.rs:108`). Qué debería contener y dónde vivir lo decide memory-protocol (22); los hooks, agent-lifecycle-hooks (25). Este cambio sólo los **instala**.

### Qué rompe exactamente un nombre de agente que cambia

La identidad es, en orden, `--agent`, `clientInfo.name` tal cual, y `"mcp"` (`server.rs:84-92`; contrato en `specs/visibility` R1). El nombre del cliente se usa sin normalizar a propósito (`server.rs:82-83`).

| Operación | Con filas guardadas bajo el nombre viejo | Fuente |
|---|---|---|
| `search_memory`, `recall_context` | **Las encuentra.** Salen marcadas `by <viejo>` y de sólo lectura, si `sharing.json` no las oculta | `crates/clawg-cli/src/mcp/tools.rs:92`; `specs/sharing` |
| `list_memories` | No las lista: sólo filas del agente | `specs/visibility` R3 |
| `delete_memory`, `update_memory`, `resolve_conflict`, `read_memory` | Responden como si el id no existiera; no tocan nada | `specs/visibility` R3; `crates/clawg-core/src/memory/admin.rs:324-328` (delete con `owner`) |
| Dedup | Guardar el mismo texto otra vez crea una fila nueva: el índice es por `(agent_id, proyecto, hash)` | `crates/clawg-core/migrations/V6__content_hash.sql:15` |
| `workspace.md` | La sección USER escrita a mano queda en la carpeta del nombre viejo | `specs/visibility` R5 |
| Revisión de síntesis | Es por agente: el nuevo parte sin historial de revisión | `crates/clawg-core/migrations/V10__synthesis.sql:22-27` |
| `sharing.json` | Las reglas `hide` nombran agentes por texto: las del nombre viejo ya no se aplican al nuevo, y a la inversa | `crates/clawg-core/src/memory/sharing.rs:5` |
| Contadores de `clawg memories stats` | Dos agentes en vez de uno | `admin.rs:839`, `:952` |

Nada se pierde: las filas siguen en la base (`README.md:249`). Lo que se rompe es el dominio sobre ellas.

### Premisas del encargo que son falsas o sólo parcialmente ciertas

- **"Claude Code reportó `Claude Code` y más tarde `claude-code`": no verificado en el repo.** `"Claude Code"` aparece sólo como entrada inventada de un test (`server.rs:1366`). La única observación real archivada es `claude-code` con Claude Code 2.1.296 (`openspec/changes/archive/2026-10-13-mcp-protocol-upgrade/verify.md:57`). El riesgo es **plausible** (el cliente puede cambiarlo, o dejar de mandarlo: `client_name` y `remembered`, `server.rs:102`, `:353-374`), pero no hay un caso documentado de renombre. La justificación del cambio no debe apoyarse en que ya pasó.
- **"Setup manual": cierto**, y además ya hay una mitigación escrita (`README.md:239-249`). El cambio no descubre el problema: lo vuelve imposible de olvidar.
- **"Delete/update/resolve quedan limitados al dueño": cierto** (tabla de arriba). Pero **leer no se rompe**: la búsqueda sigue encontrando todo. El daño es menor que "perder las memorias" y mayor que cosmético.
- **Hay un precedente de pinning que no pierde datos:** fijar `--agent` con el mismo nombre que el cliente venía reportando conserva filas y `workspace.md` (`specs/visibility` R6, test `rows_and_workspace_of_a_client_name_survive_pinning_it`). El problema sólo existe si el nombre fijado **difiere** del anterior.
- **No existe ningún mecanismo de renombre ni de alias de agente.** `rg` por `rename_agent`, `agents rename` y `alias` en `crates` no da nada relacionado.

### Hechos externos de Claude Code

| Hecho | Estado | Fuente |
|---|---|---|
| `claude mcp add [opciones] <nombre> -- <comando> [args]`; lo que va después de `--` se pasa al servidor sin tocar | verificado | https://code.claude.com/docs/en/mcp |
| `--scope` acepta `local` (por defecto), `project` y `user` | verificado | idem |
| Los servidores `local` y `user` se guardan en `~/.claude.json`; los `project` en `.mcp.json` de la raíz. **No** en `settings.json` | verificado | idem |
| Existen `claude mcp list`, `get`, `remove` y `add-json <nombre> '<json>'` (con `--scope user`) | verificado | idem |
| Ante el mismo nombre en varios lugares gana local, luego project, luego user, sin mezclar campos | verificado | idem |
| Los hooks se configuran en `~/.claude/settings.json` (usuario), `.claude/settings.json` y `.claude/settings.local.json`; forma `hooks.<Evento>[] -> {matcher, hooks[] -> {type: "command", command}}` | verificado | https://code.claude.com/docs/en/hooks |
| Las entradas de hooks se **suman** entre niveles; un mismo handler definido en varios archivos corre una vez | verificado | idem |
| `SessionStart` admite `command` y su stdout entra al contexto | verificado | idem |
| `~/.claude/CLAUDE.md` es el archivo de instrucciones de usuario, para todos los proyectos, y se suma a los del proyecto; se lee al iniciar la sesión | verificado (vía búsqueda sobre la página oficial) | https://code.claude.com/docs/en/memory |
| Si `claude` está en el PATH de quien corre `clawg setup` | sin verificar: depende de cada máquina | — |
| Que el formato interno de `~/.claude.json` sea estable entre versiones, o que escribirlo a mano esté soportado | sin verificar: la doc describe el formato pero no promete estabilidad ni dice que se edite a mano | — |

Esta exploración **no leyó** `~/.claude.json` ni `~/.claude/settings.json` del usuario; lo que se sabe de ellos viene sólo de la documentación.

## Lo que ya existe y se reutiliza

- Un precedente de "editar la configuración del usuario" con la lógica pura aparte del I/O: `crates/clawg-cli/src/user_path.rs` (`with_entry` y `without_entry` puras; el registro de Windows en un módulo `cfg(windows)`). Conserva lo ajeno byte a byte y el tipo del valor. Es el molde.
- `Profile::resolve` y el reparto en `cli.rs` entre parseo puro y ejecución.
- `clawg sharing hide|unhide` escribe `sharing.json` de forma atómica (temporal más `rename`, `sharing.rs:166-171`) y sin abrir el store.
- `clawg doctor` ya sabe cuál es el `clawg.exe` en ejecución y cuál resuelve el PATH. De ahí sale la ruta absoluta.
- Re-key de proyectos como molde de un cambio de identificador almacenado: una transacción, registro fila por fila en `project_rekey` para poder deshacer, tablas en una lista literal (`crates/clawg-core/src/memory/project.rs:15-17`, `:79-106`; `V9__project.sql:17`).
- Snapshots: `snapshot_into(c, dir, Reason)` (`crates/clawg-core/src/memory/backup.rs:279`) y `backup_offline(base_dir)` (`:310`), que toma el lock del store.
- `clawg purge` y `clawg restore` como modelo de comandos que exigen el daemon parado y piden confirmación.

## Enfoques posibles

### A. Registro: invocar `claude mcp add --scope user`

`clawg setup claude-code` ejecuta `claude mcp add --scope user clawg -- <ruta> mcp --profile P --agent claude-code`.

- A favor: el formato de `~/.claude.json` es asunto de Claude Code; si cambia, `clawg` no se entera. Es el camino documentado.
- En contra: exige `claude` en el PATH del proceso. No hay un "inspeccionar sin escribir" limpio: `claude mcp get` sirve para detectar una entrada previa, pero `--dry-run` sólo puede **imprimir** el comando. Una entrada ya existente con otro contenido hay que quitarla antes (`remove`) o `add` falla; eso es pisar, y el requisito es no pisar. Hay un proceso hijo cuyo código de salida y salida de texto hay que interpretar.

### B. Registro: editar el JSON directamente

Leer `~/.claude.json`, agregar `mcpServers.clawg`, escribir de vuelta.

- A favor: no depende de `claude` en el PATH; el plan es una función pura sobre un `serde_json::Value`; `--dry-run` muestra el diff exacto.
- En contra: `~/.claude.json` guarda mucho más que servidores (proyectos, historial, estado) y Claude Code lo reescribe mientras corre. Escribir un archivo que otro proceso también escribe puede perder cambios. Sin verificar que el formato sea estable. Es el único enfoque donde un error del setup puede dañar la configuración **completa** del usuario, no sólo la parte de clawg.

### C. Híbrido (recomendado)

Registro por `claude mcp add-json` / `claude mcp add` (A), y **fallback a imprimir el comando exacto** si `claude` no está en el PATH, sin editar el JSON. Hooks y bloque de `CLAUDE.md` por edición directa: `~/.claude/settings.json` y `~/.claude/CLAUDE.md` son archivos pensados para editarse a mano, y los hooks se suman entre archivos (verificado), así que añadir una entrada no desplaza las ajenas.

- A favor: lo frágil (`~/.claude.json`) lo hace quien lo posee; lo editable (`settings.json`, `CLAUDE.md`) lo hacemos con lógica pura y marcadores.
- En contra: dos mecanismos de escritura. A cambio, el fallo de cada uno es acotado.

### Qué es "no pisar" en cada superficie

| Superficie | Detección de conflicto | Idempotencia |
|---|---|---|
| Servidor MCP `clawg` | `claude mcp get clawg`: si existe con otro comando o argumentos, **conflicto**, se reporta y no se reemplaza | si coincide exactamente con el plan, no hace nada |
| Hooks en `settings.json` | una entrada cuyo `command` empieza por la ruta de nuestro binario y `hook` es nuestra | reconocer lo propio por el comando; lo ajeno nunca se toca |
| `CLAUDE.md` | bloque entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->` | reemplaza sólo el bloque; si falta un marcador de los dos, error sin escribir |

### Plan puro y aplicación

`plan(estado_actual, opciones) -> Vec<Edit>` pura, con el estado leído por un módulo de I/O fino. `--dry-run` imprime el plan y no escribe. `--remove` es otro plan sobre el mismo estado. Es el patrón de `user_path.rs`. No hay abstracción de "cliente" hasta que exista un segundo: una tabla declarativa de un solo elemento y nada más.

### Identidad de filas ya guardadas con otro nombre

| Opción | Qué es | A favor | En contra |
|---|---|---|---|
| **(a) Tabla de alias** | `old -> new`, resuelta en cada lectura y escritura | No toca datos almacenados | Toca **todas** las consultas con `agent_id`, el scope de delete/update/resolve, el dedup, el workspace y `sharing.json`. Mucha superficie, y cualquier camino que se olvide reabre el bug. Descartada como primer paso. |
| **(b) `clawg agents rename <old> <new>`** | Una transacción que reescribe `agent_id` en las tres tablas y `synthesis_review`, mueve la carpeta del workspace y reescribe `sharing.json` | Una vez hecho, no queda deuda: el dominio vuelve a ser uno. Molde probado (`project.rs`) | Cambia un identificador almacenado: seed de datos obligatorio (`config.yaml`), snapshot previo, daemon parado, y decidir qué pasa si `<new>` ya tiene filas (fusionar dedups, workspaces con USER escrito a mano en ambos lados). Es irreversible sin tabla de deshacer como `project_rekey`. |
| **(c) Nada** | Documentar que fijar `--agent` con el nombre ya usado no pierde nada (`specs/visibility` R6) | Cero código | Quien ya se separó queda separado. Aceptable si hoy nadie lo está |

Recomendación: **(c) para lo ya existente y fijar la identidad en el setup**, con (b) solo si `clawg memories stats` del usuario real muestra dos nombres del mismo cliente. Eso no se midió (no se tocó `~/.clawg`); lo decide el usuario mirando su propia base. Si hay que hacer (b), `memory_status` ya dice de dónde sale el nombre (`server.rs:68-76`), y `doctor-checks` (31) podría avisar de memorias repartidas entre dos nombres.

## Descartado

- **Tabla de alias como mecanismo primario (a).** Cada consulta con `agent_id` pasaría a resolver un alias; el scope del daemon (`admin.rs:324`, `specs/visibility` R4) es seguridad, no comodidad: una regla de equivalencia ahí es un lugar donde un agente podría actuar sobre lo de otro por un alias mal armado.
- **El mismo texto del protocolo copiado en varios archivos o superficies.** Dos copias divergen; la fuente única es la de memory-protocol (22), compilada en el binario, y el bloque de `CLAUDE.md` es una proyección generada, no un texto editado a mano.
- **Escribir una ruta relativa o dependiente del PATH.** Claude Code lanza el comando con su propio PATH (el README ya lo dice: `README.md:224`). Una ruta relativa se rompe según el directorio, y una que dependa del PATH puede apuntar a otra copia. El setup usa la ruta absoluta del binario en ejecución; `doctor` ya detecta copias duplicadas.
- **Editar `~/.claude.json` a mano como camino principal (B).** El riesgo de dañar la configuración entera no lo compensa lo que se gana.
- **Reemplazar una entrada ajena distinta.** Es un conflicto: se reporta y el usuario decide.
- **Soportar otros clientes ahora.** No hay un segundo cliente verificado (`README.md:286` lo dice).

## Alcance candidato

1. `clawg setup claude-code [--profile P] [--dry-run] [--remove]`.
2. Plan puro: `plan(estado, opciones)`, con tests sin disco ni procesos.
3. Registro MCP con la ruta absoluta del binario en ejecución, `--agent claude-code` y `--profile` cuando no es el predeterminado; sin `claude` en el PATH imprime el comando.
4. Hooks de 25 en `~/.claude/settings.json`, sin tocar entradas ajenas.
5. Bloque del protocolo de 22 entre marcadores en `~/.claude/CLAUDE.md`.
6. Idempotencia y conflicto reportado, nunca reemplazado.
7. Escritura atómica (temporal más `rename`, como `sharing.rs:166-171`) y copia previa del archivo editado.
8. Docs: README (el paso de conectar pasa a ser el comando), `CLAUDE.md` y `AGENTS.md` del repo.
9. Una tabla declarativa de un solo cliente, con el lugar para agregar otro.

### No-alcance

- Otros clientes (Cursor, Codex, OpenCode).
- Renombre o alias de agentes (según la decisión abierta).
- El texto del protocolo y los hooks en sí (22 y 25).
- Editar `~/.claude.json` a mano.
- Tocar el perfil `prod` desde un build debug: un debug no puede registrar `prod` (`CLAUDE.md` del repo, perfiles).
- Cambios de ranking, esquema o contrato de embedding.

## Decisiones abiertas

Son del usuario.

- [ ] **Cómo registrar.** ¿Híbrido C (recomendado), sólo `claude mcp add` (A) o JSON directo (B)?
- [ ] **Sin `claude` en el PATH.** ¿Se imprime el comando y se sale con error, o se ofrece editar el JSON con confirmación?
- [ ] **Nombre fijo.** ¿`claude-code` siempre, o se toma el que ya usan las filas (`clawg memories stats`)? Fijar un nombre distinto del histórico dispara el problema que el cambio quiere evitar.
- [ ] **Filas con otro nombre.** ¿(c) nada, (b) `clawg agents rename`, o (a) alias? Depende de lo que muestre `stats` en la base real.
- [ ] **Si hay (b):** ¿qué pasa si `<new>` ya tiene filas? ¿Se fusiona, se rechaza?
- [ ] **Hooks.** ¿Se instalan siempre, o con `--no-hooks`? ¿A nivel usuario (recomendado) o de proyecto?
- [ ] **Bloque de `CLAUDE.md`.** ¿Se escribe en `~/.claude/CLAUDE.md` (verificado como archivo de usuario) o se omite si 25 ya inyecta el protocolo al iniciar la sesión? Dos vías para el mismo texto duplican contexto.
- [ ] **`--remove`.** ¿Quita también el bloque y los hooks, y deja `~/.clawg` intacto? (Recomendado.)
- [ ] **Perfil dev.** ¿`setup` registra `clawg-dev` si el binario es debug?
- [ ] **División.** ¿Se parte el cambio y el registro con `--agent` sale antes que hooks y protocolo?

## Cómo se mide

Tests RED primero. Ninguno toca `~/.claude`, `~/.claude.json` ni `~/.clawg*`: todo corre sobre un directorio temporal.

1. Plan sobre un estado vacío: registro, hooks y bloque, con la ruta absoluta y `--agent claude-code`.
2. Idempotencia: aplicar el plan dos veces deja el mismo estado; la segunda corrida no escribe.
3. Conflicto: una entrada `clawg` distinta ya existente se reporta y el plan no la reemplaza.
4. Hooks ajenos en el mismo evento se conservan byte a byte; el orden se mantiene.
5. `CLAUDE.md`: el texto fuera de los marcadores no cambia; reemplazar sólo el bloque; un marcador solo (sin su par) es error sin escribir.
6. `--remove` deja el estado como antes del setup (ida y vuelta).
7. `--dry-run` no escribe nada.
8. Una ruta del binario relativa o sin resolver es rechazada.
9. `--profile prod` desde un debug es rechazado, como ya lo hace `Profile::resolve`.
10. Sin `claude` en el PATH: se imprime el comando y no se edita ningún JSON de cliente.
11. Si se hace (b): base sembrada en la versión anterior, con filas de `claude-code` y `Claude Code`, un `workspace.md` con sección USER, un `sharing.json` y un `synthesis_review`. Tras el renombre, `list_memories`, `delete_memory` y `search_memory` actúan sobre todas, y la carpeta y las reglas siguen. Es la regla de `config.yaml` para identificadores almacenados. Más: snapshot previo en `backups`, daemon parado y una tabla de deshacer.
12. Si se hace (b), `<new>` con filas propias: el test fija el comportamiento decidido, sin pérdida.

Sin cambio de ranking: `rag-eval --diff` no aplica.

## Riesgos

- **Escribir mal la configuración del usuario.** Es el riesgo mayor. Mitigación: atómico, copia previa, y no editar `~/.claude.json`.
- **Ruta absoluta que deja de existir** al reinstalar o mover el binario: el servidor queda apuntando a nada. `install.ps1` renombra el binario viejo (`README.md:309`), pero una reubicación rompe el registro. Candidato para `doctor-checks` (31), fuera de este cambio.
- **Elegir el nombre equivocado** y separar a un usuario que hoy no está separado. Mitigación: la decisión de "nombre fijo".
- **Doble inyección del protocolo** (hook más `CLAUDE.md`) que infla el contexto. Decisión abierta.
- **Formato de Claude Code que cambia:** sin verificar para `~/.claude.json`; por eso se prefiere su CLI.
- **Premisa del renombre no documentada:** la justificación se apoya en un riesgo plausible, no en un caso real (ver arriba). Si el usuario no tiene dos nombres en su base, el valor es de prevención y de comodidad, no de reparación.
- **Estado del repo:** el `CLAUDE.md` y el `AGENTS.md` del proyecto no listan `setup`; hay que sumarlo.
