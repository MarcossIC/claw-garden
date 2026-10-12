# context-density — Especificación

**Dominio destino:** `specs/retrieval/spec.md` (se fusiona al archivar; hoy `openspec/specs/` está vacío, así que es una spec completa, no un delta).

**Alcance:** el texto que arma `recall_context` (`build_memory_prompt`, `prompt_builder.rs:98-259`), su parser en el eval y la doc de `delete_memory`. Nada escribe en la base ni en el archivo del workspace.

## Definiciones

- **Hit mostrado:** hit de `kind = memory` presente en RECENT o LONG-TERM de este prompt. Los ids son por tabla (`prompt_builder.rs:198-203` sólo cuenta `TableKind::Memory`), así que un hit episodic o procedural nunca cuenta.
- **Fila del workspace:** id en `EmittedIds` (`prompt_builder.rs:202`, `workspace.rs:152-168`). Su bullet no tiene id (`workspace.rs:394`). **No** es un hit mostrado.
- **Posición de un hit:** índice 0-based en el orden impreso: RECENT y luego LONG-TERM.
- **Par:** fila nueva (`disputed`) contra una fila vieja.

## R1. Línea de hit

Cada hit MUST imprimirse como un encabezado de una línea seguido del texto en la misma línea:

- `kind = memory`: `[YYYY-MM-DD #id tipo] texto`
- otro kind: `[YYYY-MM-DD #id kind/tipo] texto`, con `kind` en minúsculas (`episodic`, `procedural`).

Si el texto tiene varias líneas, las siguientes MUST seguir tal cual (líneas en blanco y código incluidos) hasta el próximo encabezado de hit o separador `=== `. No se imprime `SOURCE:`, `TYPE:` ni `CONTENT:`.

#### Escenario: memory
- GIVEN un hit memory id 57, tipo `semantic_fact`, creado el 2026-10-10, texto `usar sqlite`
- WHEN se arma el prompt
- THEN contiene la línea `[2026-10-10 #57 semantic_fact] usar sqlite`

#### Escenario: otro kind
- GIVEN un hit procedural id 3, tipo `skill`
- THEN su línea empieza con `[<fecha> #3 procedural/skill] `

#### Escenario: texto multilínea
- GIVEN un hit con texto `fn main() {\n    run();\n\n    stop();\n}` seguido de otro hit
- WHEN se arma el prompt
- THEN las 5 líneas, la vacía incluida, siguen al encabezado y el segundo encabezado las cierra

**Test:** `hit_line_is_one_header_with_id_and_text` en `prompt_builder.rs` (mod tests). Falla hoy: emite `SOURCE:`/`CONTENT:` sin id (`:300-306`).

## R2. Referencia por id en conflictos

Un lado que es hit mostrado MUST imprimirse sólo como `#id`. Un lado que no es hit mostrado (fila del workspace, fila sólo en `EmittedIds`, o fila fuera del prompt) MUST imprimirse con su texto completo bajo el encabezado `[YYYY-MM-DD #id]`. Un par con ambos lados mostrados MUST NOT imprimir ningún texto.

Formato de un grupo (R3):

```
NEW #N                      | NEW [YYYY-MM-DD #N disputed] texto
OLD #M                      | OLD [YYYY-MM-DD #M] texto
```

`importance`, `tier` y `status` dejan de imprimirse.

#### Escenario: lado mostrado
- GIVEN el par (old 1, new 2), con 2 como hit en RECENT y 1 sin mostrar
- THEN el bloque contiene `NEW #2` y `OLD [<fecha> #1] <texto de 1>`, y el texto de 2 aparece una sola vez en todo el prompt

#### Escenario: ambos mostrados
- GIVEN old 1 y new 2 como hits
- THEN el bloque tiene `NEW #2` y `OLD #1` y ninguno de los dos textos se repite

#### Escenario: workspace
- GIVEN old 1 es una fila del workspace y new 2 un hit
- THEN `OLD [<fecha> #1] <texto>` lleva el texto

**Test:** `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest` en `prompt_builder.rs`. Falla hoy: copia ambos textos (`:309-320`).

## R3. Agrupación por fila nueva

Los pares que comparten `new_id` MUST formar un grupo: una línea `NEW`, y una línea `OLD` por cada vieja. El texto de la nueva MUST aparecer a lo sumo una vez en el bloque.

#### Escenario
- GIVEN new 9 disputa a las viejas 1, 2 y 3, y 9 no es hit
- THEN `[<fecha> #9 disputed] <texto>` aparece una vez, seguido de tres líneas `OLD`

**Test:** `a_new_row_disputing_several_old_rows_prints_its_text_once` en `prompt_builder.rs`. Falla hoy: join por par (`:394-395`).

## R4. NOTE una vez

La NOTE MUST imprimirse una sola vez por bloque, justo bajo el separador y antes de los grupos, con el texto vigente (`:315-316`):

```
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
```

#### Escenario
- GIVEN 3 grupos
- THEN `NOTE:` aparece exactamente una vez

**Test:** `conflict_block_has_one_note` en `prompt_builder.rs`. Falla hoy: una por par.

## R5. Orden determinista

Una función pura sobre los pares leídos MUST ordenarlos por clave `(rank, new_id desc, old_id asc)`:

- `rank` = menor posición de hit entre sus lados que son hits mostrados;
- los pares sin ningún hit mostrado (sólo workspace o sólo `EmittedIds`) MUST ir al final, con `rank` = infinito.

El resultado MUST NOT depender del orden de entrada. Los grupos salen en orden de su primer par.

#### Escenario: posición
- GIVEN un par cuyo hit está en posición 0 y otro en posición 4
- THEN el primero va antes, aunque su `new_id` sea menor

#### Escenario: sólo workspace
- GIVEN un par sólo de workspace con `new_id` 99 y otro con un hit en posición 9
- THEN el de workspace va último

#### Escenario: estable
- GIVEN los mismos pares en dos permutaciones de entrada
- THEN la salida es idéntica

**Test:** `conflict_pairs_rank_by_best_hit_position_workspace_only_last` y `conflict_pair_order_ignores_input_order` en `prompt_builder.rs`, sin base. Fallan hoy: la función no existe y la consulta no tiene `ORDER BY` (`:390-401`).

## R6. Tope de 5 pares

Tras ordenar, el bloque MUST conservar los primeros 5 pares (el tope cuenta pares, antes de agrupar). Si quedan N > 0 afuera, la última línea del bloque MUST ser `(+N conflictos más sin mostrar)`. Con N = 0 MUST NOT haber esa línea.

#### Escenario
- GIVEN 8 pares
- THEN hay 5 pares impresos y la línea `(+3 conflictos más sin mostrar)`

#### Escenario: sin exceso
- GIVEN 5 pares
- THEN no hay línea final de omitidos

**Test:** `conflict_block_keeps_five_pairs_and_counts_the_rest` en `prompt_builder.rs`. Falla hoy: sin tope.

## R7. Workspace en el prompt

El workspace entregado al modelo MUST NOT contener `<!-- `, ni sección USER vacía, ni encabezado `## ` sin bullets debajo. El contenido no vacío de USER MUST conservarse (sin marcadores, recortado). Un archivo sin `AUTO_START` MUST seguir entregándose entero (`:351-354`). Si no queda nada, no se emite la sección WORKSPACE. El archivo en disco MUST ser byte a byte idéntico antes y después de un recall por este camino de lectura/render. El comentario de `:109` MUST pasar a describir lo que hace el código.

#### Escenario: sin marcadores
- GIVEN un archivo con AUTO/USER, un bullet bajo `## User Preferences` y los otros dos encabezados vacíos
- THEN el prompt contiene el bullet y su encabezado, y ni `<!--` ni los otros encabezados

#### Escenario: USER con contenido
- GIVEN texto del usuario entre USER-START y USER-END
- THEN aparece en el prompt sin marcadores

#### Escenario: archivo plano
- GIVEN un archivo sin marcadores
- THEN se entrega tal cual

**Tests:** `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers`, `workspace_file_bytes_are_unchanged_by_rendering_the_prompt` en `prompt_builder.rs`. Los dos primeros fallan hoy (`:365-366` devuelve el archivo entero); el tercero fija la invariante.

## R8. Parser del eval

`context_segments` (`parse.rs:78-116`) MUST mantener `found`: los segmentos de hit salen sólo de las secciones RECENT CONTEXT y LONG-TERM KNOWLEDGE; los bullets, sólo de WORKSPACE; el texto de ACTIVE CONFLICTS MUST NOT contar. Encabezado de hit = línea que cumple `[` + `YYYY-MM-DD` + ` #` + dígitos + (` ` + palabra sin espacios)? + `]` + ` `; el texto es lo que sigue en la misma línea. `starts_with_date_header` (`:129-137`, exige `]` en el byte 11) deja de servir y se reemplaza. Se actualizan los fixtures de `parse.rs:222-264` y `agent_view/mod.rs:373`.

#### Escenario: multilínea
- GIVEN dos hits, el primero con código y línea en blanco (como el test de `parse.rs:228`)
- THEN hay 2 segmentos, el primero completo

#### Escenario: conflictos
- GIVEN una sección ACTIVE CONFLICTS con `NEW [..#2 disputed] x` y `OLD [..#1] y`
- THEN no se genera ningún segmento

**Tests:** `a_multi_line_hit_runs_until_the_next_header`, `conflict_text_yields_no_segments` en `parse.rs`; `read_context_finds_rows_in_hit_lines_and_workspace_bullets` en `agent_view/mod.rs`. Fallan hoy: no reconocen `#id`.

## R9. Doc de `delete_memory`

La doc de `DeleteMemoryArgs.id` (`server.rs:128-129`) MUST nombrar `recall_context` además de `search_memory` y `list_memories`, y decir que un hit sin kind explícito es `memory`. `kind` sigue siendo obligatorio (`:131-134`).

**Test:** `delete_memory_id_doc_names_recall_context_and_default_kind` en `server.rs` (mod tests): el schema de `DeleteMemoryArgs` contiene ambos textos. Falla hoy.

## R10. Sin cambios

- Prompt vacío: `format_context` devuelve `no context yet` (`tools.rs:138-144`).
- Orden y separadores de secciones, y `K_FINAL`, sin cambios.
- La salida de `search_memory` es idéntica.
- No se agregan escrituras. `fetch_active_conflicts` sigue siendo un `SELECT`.

#### Escenario
- GIVEN dos corridas iguales antes y después
- THEN `search_memory` imprime los mismos bytes, y el recall sin hits ni conflictos sigue vacío

**Test:** los tests existentes de `prompt_builder.rs`, `tools.rs` y la base de 1.304 siguen verdes. Se agrega `empty_prompt_still_yields_the_no_context_sentence` sólo si falta.

## R11. Medición

Con `rag-eval --real --agent-view --diff` contra `2026-10-12-real-lines-conflicts-shown`:

| Métrica | Umbral |
|---|---|
| `found` | 41/46 |
| `search_memory`, media | 852 sin cambios |
| ACTIVE CONFLICTS, media cuando aparece | ≤ 800 |
| `recall_context`, media / máximo | ≤ 2.100 / ≤ 3.000 |
| RECENT CONTEXT, media | ≤ 1.130 |

Si alguno falla, el cambio no se da por bueno. **Test:** verificación manual por `sdd-verify`, que reporta cada métrica.
