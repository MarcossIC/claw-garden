# conclusion-upkeep — Verificación

## Linux

- `cargo test --workspace`: pasa entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- El contrato del agente se bendijo con `CLAWG_BLESS=1`; el diff cambia sólo la descripción de `synthesize` (lista también las conclusiones cuyas fuentes se borraron o reemplazaron, y se usa cuando `recall_context` pide revisarlas). Las instrucciones del servidor no cambian: la sección `## Conclusions` de `protocol.md` va después del primer `##`.
- Cada requisito de la [spec](spec.md) tiene su test, escrito antes que el código: los de C1, C2, C4 y C5 fallaron primero porque no compilaban; `purging_keeps_the_links_that_name_the_row_as_a_source` (C6) y `context_reports_conclusions_to_review` y `a_save_from_a_replaced_source_is_refused` (C4, C5) fallaron por la aserción.

## Windows

`cargo test --workspace --target-dir target-test` pasa entera y el clippy de `.claude/CLAUDE.md` sale con 0, con `target-test` limpio de los artefactos del workspace antes de compilar. Pasan otra vez con el arreglo del aviso (abajo).

## Binario real, home temporal

`clawg serve --profile dev --port 0` desde la build de prueba, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`, sin tocar `~/.clawg*`:

1. #1 con `topic_key` `deploy/day`, #2, y la conclusión #3 con `derived_from = [1, 2]`. Un segundo save con la clave (#4) reemplaza a #1.
2. Un save con `derived_from = [1]` responde 400: `derived_from names memories that were replaced: 1 (by 4); draw the conclusion from the current ones`.
3. `POST /v1/context` responde `conclusions_to_review: 1`; `POST /v1/synthesis` lista #3 con `live = 1` y `lost = [#1 superseded by 4]`; el contexto siguiente responde 0.
4. Con el daemon parado, `clawg purge 2 --yes`; de vuelta arriba, `synthesize` por el puente MCP real (`clawg mcp --no-autostart`) muestra `#3 semantic_fact origin=conclusion from=#1,#2 live=0/2 lost=#1 replaced by #4,#2 purged …` bajo «Conclusions to review», con la guía.

## Perfil dev, modelo real y Claude Code

La base dev estaba en V17, sin migraciones pendientes. `clawg serve --profile dev` de la build de prueba, con el modelo real. Por el puente real (`clawg mcp --profile dev --no-autostart --agent conclusion-check`), en una carpeta vacía:

1. #101 con `topic_key` `deploy/day` (los releases salen los martes), #102 (`scripts/smoke.sh` tiene que pasar antes de promover) y la conclusión #103 con `derived_from = [101, 102]`. #104, con la misma clave (ahora salen los jueves), reemplaza a #101.
2. `clawg hook session-start --agent conclusion-check --profile dev` en esa carpeta inyecta el contexto y cierra con `1 conclusion here rests on memories that were deleted or replaced since the last review. When the current task is done, call `synthesize` to check it.`
3. Una sesión `claude -p` (Claude Code 2.1.303, `claude-opus-5-5`) en esa carpeta, con sólo ese server (`--strict-mcp-config`), los dos hooks dev y sin los settings de usuario. Se le pregunta «¿Qué día salen los releases del servicio de pagos y qué tiene que pasar antes de promoverlos?», sin nombrar `synthesize`. Busca, llama a `synthesize` por su cuenta y lee `#103 … live=1/2 lost=#101 replaced by #104`. Guarda la conclusión corregida #105 con `derived_from = [104, 102]` y borra #103. Responde «los jueves, después de que pase el smoke».
4. Después, `memory_status` no tiene pares abiertos, `synthesize` lista #105 entre las ya sacadas sin nada que revisar y `recall_context` no avisa.
5. En otro alcance ya revisado, `recall_context` no avisa. Al borrar una fuente de su conclusión, avisa con la misma línea, porque es una pérdida posterior al `reviewed_at`.

Las filas de prueba se borraron al terminar (borrar la vigente de `deploy/day` revivió la anterior, que también se borró) y el daemon se detuvo con `clawg stop`.

## Lo que encontró la verificación

- El aviso elegía el verbo por la cantidad de conclusiones, pero su sujeto son las memorias: con una, decía «rests on memories that was deleted or replaced». Ahora dice siempre «were» (`769acc7`), y `the_review_nudge_appears_from_one` lo fija.
- El agente guardó la corregida antes de borrar la vieja. Las dos son `architectural_decision` y muy cercanas, así que el detector escaló el par (`asks the user`) y #105 quedó `disputed` hasta que el borrado de #103 lo cerró. La guía de `synthesize` dice «delete it and save the corrected conclusion», en ese orden, que no deja el par. En los dos órdenes el resultado final es el mismo, así que se deja así.

## No aplica

- `rag-eval`: no cambia el ranking ni el prompt de `build_memory_prompt`; las líneas nuevas van fuera de él.

## Diferencias entre la spec y el código

Al consolidar en `specs/synthesis`, `specs/secrets` y `specs/hooks` se leyó cada requisito contra el código y se comprobó que cada test nombrado existe. No hay diferencias de comportamiento. Quedan dos de redacción, en las que manda el código:

- **Cabecera del bloque.** El diseño (D6) escribía «Conclusions to review (2 of 2, newest first): they rest on…». `format_synthesis` escribe `Conclusions to review (K of N, newest first).` seguido de `REVIEW_GUIDE` (`crates/clawg-cli/src/mcp/tools.rs:388`).
- **Fuente reemplazada.** El escenario de C2 dice `lost = [A superseded by A2]`, que es el estado en la respuesta HTTP. El texto de la herramienta dice `#A replaced by #A2` (`lost_source`, `tools.rs:358-364`). La spec principal recoge las dos formas.

## Citas realineadas al consolidar

El archive de revision-aware-delete (`3b792cb`) se hizo en una rama paralela al código de este cambio, así que sus citas no vieron lo que este cambio movió en `client.rs`, `daemon/api.rs`, `hook.rs`, `mcp/server.rs`, `mcp/tools.rs`, `memory/secrets.rs` y `memory/synthesis.rs`. Además, entre `769acc7` y la base del archive entraron cinco arreglos de seguridad que movieron `api.rs` y `hook.rs`.

- **En los 20 specs que citan esos archivos y en el backlog del README**, cada cita se siguió desde el commit que la escribió hasta el árbol actual. Sólo se reescribió cuando un bloque sin cambios cae sobre el mismo texto. Después se comparó cada una, línea citada antes contra línea citada ahora.
- **A mano:** las de `memory/secrets.rs` (la abreviada `secrets.rs` se resolvía al de la CLI), `api.rs:600-639` → `:612-654` y `client.rs:264-296` → `:264-300` en `mcp`, y `client.rs:1068-1070` → `:1098-1100` en `self-update`.
- **Una cambia de contenido a propósito:** en `provenance`, la marca de origen pasó de `synthesis_line` a `synthesis_line_with` (`tools.rs:343-355`).
- **No se tocaron las citas a archivos que sólo movieron esos arreglos de seguridad** (`memory/search.rs`, `memory/project.rs` y `tests/binary.rs`), ni las que ya estaban desalineadas antes en archivos que este cambio no toca. Son del cierre de esos arreglos.
