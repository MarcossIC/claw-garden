# visibility-policy — Verificación

**Estado:** completa. Partes A (identidad) y B (scope de lo que modifica). Los tests pasan en Linux y en Windows, y la prueba en el perfil `dev` real con Claude Code pasó (abajo). La verificación en Windows encontró un hueco en R5: los nombres que Windows reescribe. Se corrigió en `d875b20`.

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 1127b50) | 1.675 | 10 |
| Con el cambio (7f04101) | 1.696 | 10 |

Son 21 tests nuevos y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno, como en los cambios anteriores:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore`.

Cada commit del cambio se probó por separado, con el mismo comando: todos compilan y ninguno agrega fallas (1.678, 1.681, 1.684, 1.692, 1.696 y 1.696 pasan; 10 fallan en todos, las mismas).

Los tests nuevos fallaban antes de su código: los de firma nueva (`owner`, `--agent`, `workspace_segment`, el agente en los `*_impl`) no compilaban, y los HTTP de la tarea 2 se corrieron en rojo (404 esperado, la fila se borraba o cambiaba).

### Windows (2026-10-12)

`cargo test --workspace --no-fail-fast --target-dir ../clawg-target-verify` en Windows 11, sobre `main` (`c45b18a`, el cambio ya mergeado con #15 y los cambios posteriores).

| Corrida | Pasan | Fallan |
|---|---|---|
| `main` (`c45b18a`) | 1.648 | 5 |
| Con el fix de R5 (`d875b20`) | 1.650 | 5 |

- **Las 5 fallas son la base de Windows**, todas en `crates/clawg-core/tests/project_root.rs` (project-identity): comparan contra rutas sin el prefijo `\\?\` que `canonicalize` agrega. No son de este cambio.
- **Los tests de este cambio pasan todos**, incluidos `workspace_segment` y los de `workspace_path`, que eran lo que más podía diferir.
- La base de `CLAUDE.md` (1.691) quedó vieja: `675f574` sacó 133 tests junto con el pipeline de extracción LLM. Se actualiza a 1.655 / 5.
- Hay dos warnings de código muerto que no son de este cambio: `insert_row_with_embedding` en `worker.rs:1247` (viene de `57119d3`) y `production_lexical_budget` en `rag-eval`.

### Hueco encontrado: nombres que Windows reescribe

`workspace_segment` codificaba separadores, `:`, comodines, `%`, caracteres de control, `.` y `..`. Pero Windows además:

- **descarta los `.` y espacios del final:** `agent.` y `agent ` caían en la carpeta de `agent`. Dos agentes compartían workspace, justo lo que R5 prohíbe;
- **toma los nombres de dispositivo** (`con`, `nul.txt`, `com1`, …) **como el dispositivo:** un agente `con` no podía escribir su workspace.

El id lo manda el cliente MCP (o `--agent`), así que cualquiera de los dos casos se puede dar.

- **Rojo:** `a_name_windows_would_rewrite_is_encoded` (`"con"` daba `"con"`) y `windows_naming_rules_keep_agents_apart_on_disk` (`Os { code: 3, … "The system cannot find the path specified." }` al escribir el workspace de `con`).
- **Fix (`d875b20`):** se codifican cada `.` o espacio del final y la primera letra de un nombre de dispositivo (`con` → `%63on`, `agent.` → `agent%2E`). Es la misma regla en todas las plataformas, así un store le da a cada agente la misma carpeta donde corra.
  - El caso especial de `.`/`..` desaparece: lo cubre la regla del final y da los mismos segmentos que antes (`%2E`, `%2E%2E`).
  - `encoding_never_makes_two_agents_share_a_folder` suma `a`, `a.`, `a%2E`, `con` y `%63on`.
- **Datos existentes:** sólo cambia la ruta de un agente cuyo nombre termina en `.` o espacio, o es un nombre de dispositivo. En Windows esas carpetas nunca existieron por separado. En Linux, un agente así perdería de vista una sección USER escrita a mano en su carpeta vieja. No hay ninguno en el perfil dev, y los clientes conocidos (`claude-code`, `cursor`, `Claude Code`) no cambian.
- **Queda afuera:** NTFS no distingue mayúsculas, así que `Cursor` y `cursor` comparten carpeta en Windows. Codificar las mayúsculas movería carpetas que existen hoy (`Claude Code`); se anota para la parte C, que decide la identidad compartida.

## Prueba con el binario

Binario debug de 7f04101, con `HOME` apuntando a un directorio temporal (perfil `dev` vacío, sin tocar `~/.clawg*` de nadie). Un script habló MCP por stdio con `clawg mcp --profile dev --agent claude-code`, declarándose como `clientInfo.name = "Claude Code"`:

- el puente arrancó el daemon solo;
- `memory_status` terminó con `agent: claude-code (from --agent)`: el flag ganó sobre el nombre del cliente;
- `list_memories` respondió `no memories stored`;
- `clawg stop --profile dev` paró el daemon.

## Perfil dev real con Claude Code (2026-10-12)

Binario debug de `main` en `target\debug\clawg.exe` (la ruta del registro `clawg-dev`), sobre el `~/.clawg-dev` real, con autorización. Antes de empezar se tomó `memory-2026-10-13T00-24-07.120Z-manual.db`. Las sesiones fueron de Claude Code real (`claude -p`), no un script.

- **Por qué `clawg-dev` no conectaba:** el registro apuntaba a `target\debug\clawg.exe` y ese archivo no existía (se había limpiado `target/`). El puente no arrancaba y Claude Code mostraba `CONNECTION_CLOSED`. Se resolvió compilando el binario debug en esa ruta.

| Paso | Resultado |
|---|---|
| `clawg memories stats --profile dev` | 2 filas vivas: `claude-code` 1, `smoke-test` 1 |
| Sesión con el registro viejo (sin `--agent`), `memory_status` | `agent: claude-code (reported by the client; pin it with --agent)` |
| `claude mcp add -s local clawg-dev -- …\clawg.exe mcp --profile dev --agent claude-code` | `✔ Connected` |
| Sesión nueva: `memory_status` | `agent: claude-code (from --agent)` |
| Sesión nueva: `list_memories` y `search_memory` | ven la #2, guardada hace 29 días con el nombre que reportaba el cliente |
| Sesión `claude-code`: `save_memory` de una fila descartable | `saved #11` |
| Sesión con `--agent otro` (`--mcp-config`): `memory_status` | `agent: otro (from --agent)` |
| `otro`: `delete_memory` sobre la #11 | `daemon returned 404: no Memory row with id 11`, el mismo error que un id inexistente (R4) |
| `otro`: `list_memories` | `No memories stored`: sólo lista lo propio (R3) |
| `clawg memories list --profile dev` | la #11 sigue viva |
| Sesión `claude-code`: `delete_memory` sobre la #11 | `deleted 1`: el dueño sí puede |
| `clawg memories list`, después `clawg stop --profile dev` | quedan la #2 y la #1, como al empezar; daemon parado |

El borrado de prueba se hizo sobre una fila descartable creada para eso, nunca sobre una memoria real del perfil.

## rag-eval

No aplica. El cambio no toca la búsqueda: ni `search.rs` (salvo un argumento `None` en dos tests) ni el predicado de search/recall ni el ranking. `rag-eval` llama a la búsqueda directamente, sin las operaciones que ahora tienen scope. El fix de R5 sólo toca la ruta del workspace.

## Solapamiento con otros cambios

- **mcp-protocol-upgrade (#14, mergeado):** entró después de este, como estaba acordado. Cambió de dónde sale el nombre del cliente (cada request, no sólo el handshake; su R4). `resolve_agent` y la precedencia `--agent` > cliente > `"mcp"` quedaron como están.
- **secret-hygiene (#16, mergeado):** tiene que cerrar su verify antes de la parte C (compartir, ahora agent-sharing), como pide el mapa. Este cambio no comparte nada.
- **dispute-resolution:** `resolve_conflict` ahora exige que las dos filas sean del agente. El detector ya compara sólo dentro del mismo agente, así que una disputa real nunca cruza agentes.
- **knowledge-synthesis:** `POST /v1/synthesis` usa el mismo `agent_id` opcional como scope.
- **Migraciones:** ninguna.
