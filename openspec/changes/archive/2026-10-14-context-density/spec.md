# context-density — Especificación

**Dominio destino:** `specs/retrieval/spec.md`. Es un delta: al archivar, sus requisitos se suman como secciones nuevas junto a las de retrieval-hygiene y lexical-fusion.

**Alcance:** el texto que arma `recall_context` (`build_memory_prompt`, `crates/clawg-core/src/memory/prompt_builder.rs:97-249`), su parser en el eval y la doc de `delete_memory`. Nada escribe en la base ni en el archivo del workspace.

## Definiciones

- **Hit mostrado:** hit de `kind = memory` presente en RECENT o LONG-TERM de este prompt. Los ids son por tabla (`shown`, `prompt_builder.rs:187-193`, sólo toma `TableKind::Memory`), así que un hit episodic o procedural nunca cuenta.
- **Fila del workspace:** id en `EmittedIds` (`crates/clawg-core/src/memory/workspace.rs:201-221`; se suma a `shown` en `prompt_builder.rs:192`). Su bullet no tiene id (`workspace.rs:451`, `:455`). **No** es un hit mostrado.
- **Posición de un hit:** índice 0-based en el orden impreso: RECENT y luego LONG-TERM.
- **Par:** fila nueva (`disputed`) contra una fila vieja. `fetch_active_conflicts` (`prompt_builder.rs:368-420`) sólo trae pares con al menos un lado que es hit mostrado o fila del workspace; el otro lado puede estar fuera del prompt.

## R1. Línea de hit

Cada hit MUST imprimirse como un encabezado de una línea seguido del texto en la misma línea:

- `kind = memory`: `[YYYY-MM-DD #id tipo] texto`
- otro kind: `[YYYY-MM-DD #id kind/tipo] texto`, con `kind` en minúsculas (`episodic`, `procedural`). Para esos kinds la búsqueda pone el tipo fijo `episode` o `procedure` (`crates/clawg-core/src/memory/search.rs:130`, `:145`).

Si el texto tiene varias líneas, las siguientes MUST seguir tal cual (líneas en blanco y código incluidos) hasta el próximo encabezado de hit o separador `=== `. No se imprime `SOURCE:`, `TYPE:` ni `CONTENT:`.

#### Escenario: memory
- GIVEN un hit memory id 57, tipo `semantic_fact`, creado el 2026-10-10, texto `usar sqlite`
- WHEN se arma el prompt
- THEN contiene la línea `[2026-10-10 #57 semantic_fact] usar sqlite`

#### Escenario: otro kind
- GIVEN un hit procedural id 3, tipo `procedure`
- THEN su línea empieza con `[<fecha> #3 procedural/procedure] `

#### Escenario: texto multilínea
- GIVEN un hit con texto `fn main() {\n    run();\n\n    stop();\n}` seguido de otro hit
- WHEN se arma el prompt
- THEN las 5 líneas, la vacía incluida, siguen al encabezado y el segundo encabezado las cierra

**Test:** `hit_line_is_one_header_with_id_and_text` en `prompt_builder.rs` (mod tests). Falla hoy: `format_hit_line` emite `SOURCE:`/`CONTENT:` sin id (`:288-296`).

## R2. Referencia por id en conflictos

Cada par MUST imprimir los ids de sus dos lados como `#id`: son los valores que `resolve_conflict` recibe como `keep` y `other` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:237-249`), y los dos son ids de `memory`.

Un lado que es hit mostrado MUST imprimirse sólo como `#id`. Un lado que no es hit mostrado (fila del workspace, o fila fuera del prompt) MUST imprimirse con su texto completo bajo el encabezado `[YYYY-MM-DD #id]` la primera vez que aparece en el bloque, y sólo como `#id` las siguientes. Así el texto de una fila aparece a lo sumo una vez en el bloque aunque esté en varios pares: en una cadena de disputas, la fila del medio es nueva en un par y vieja en otro. Un par con ambos lados mostrados MUST NOT imprimir ningún texto.

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

#### Escenario: cadena
- GIVEN los pares (old 2, new 3), (old 1, new 3) y (old 1, new 2), ninguno con hits
- THEN el bloque es `NEW [<fecha> #3 disputed] <texto de 3>`, `OLD [<fecha> #2] <texto de 2>`, `OLD [<fecha> #1] <texto de 1>`, `NEW #2`, `OLD #1`

**Test:** `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest` en `prompt_builder.rs`. Falla hoy: `format_conflict_pair` copia ambos textos (`:299-311`). La cadena, `a_row_in_several_pairs_prints_its_text_once`: se agregó en la verificación, cuando la prueba en el perfil dev imprimió dos veces el texto de la fila del medio.

## R3. Agrupación por fila nueva

Los pares que comparten `new_id` MUST formar un grupo: una línea `NEW`, y una línea `OLD` por cada vieja. El texto de la nueva MUST aparecer a lo sumo una vez en el bloque.

#### Escenario
- GIVEN new 9 disputa a las viejas 1, 2 y 3, y 9 no es hit
- THEN `[<fecha> #9 disputed] <texto>` aparece una vez, seguido de tres líneas `OLD`

**Test:** `a_new_row_disputing_several_old_rows_prints_its_text_once` en `prompt_builder.rs`. Falla hoy: join por par (`:385-386`).

## R4. NOTE una vez

La NOTE MUST imprimirse una sola vez por bloque, justo bajo el separador y antes de los grupos, con el texto vigente de `format_conflict_pair` (`:299-311`), las tres líneas:

```
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
Con su respuesta, llamá a resolve_conflict (keep = la que vale, other = la otra).
```

#### Escenario
- GIVEN 3 grupos
- THEN `NOTE:` aparece exactamente una vez, y `resolve_conflict` también

**Test:** `conflict_block_has_one_note` en `prompt_builder.rs`. Falla hoy: una por par. `the_conflict_note_names_the_tool` (`:557`) pasa a comprobar la NOTE del bloque y no la última línea de cada par.

## R5. Orden determinista

Una función pura sobre los pares leídos MUST ordenarlos por clave `(rank, new_id desc, old_id asc)`:

- `rank` = menor posición de hit entre sus lados que son hits mostrados;
- los pares sin ningún hit mostrado (sólo tocan una fila del workspace) MUST ir al final, con `rank` = infinito.

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

**Test:** `conflict_pairs_rank_by_best_hit_position_workspace_only_last` y `conflict_pair_order_ignores_input_order` en `prompt_builder.rs`, sin base. Fallan hoy: la función no existe y la consulta no tiene `ORDER BY` (`:381-393`).

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

El workspace entregado al modelo MUST NOT contener `<!-- `, ni sección USER vacía, ni un encabezado `## ` de la sección AUTO sin bullets debajo. El contenido no vacío de USER MUST conservarse tal cual lo escribió el usuario, encabezados incluidos, sin marcadores y sin líneas en blanco al principio ni al final: la poda de encabezados sólo mira la sección AUTO, que es la que genera `render_rows` (`workspace.rs:445-464`). Un archivo sin `AUTO_START` MUST seguir entregándose entero (`:342-345`). Si no queda nada, no se emite la sección WORKSPACE. El archivo en disco MUST ser byte a byte idéntico antes y después de un recall por este camino de lectura/render. El comentario de `:107-108` MUST pasar a describir lo que hace el código.

#### Escenario: sin marcadores
- GIVEN un archivo con AUTO/USER, un bullet bajo `## User Preferences` y los otros dos encabezados vacíos
- THEN el prompt contiene el bullet y su encabezado, y ni `<!--` ni los otros encabezados

#### Escenario: USER con contenido
- GIVEN texto del usuario entre USER-START y USER-END
- THEN aparece en el prompt sin marcadores

#### Escenario: archivo plano
- GIVEN un archivo sin marcadores
- THEN se entrega tal cual

**Tests:** `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers`, `workspace_file_bytes_are_unchanged_by_rendering_the_prompt` en `prompt_builder.rs`. Los dos primeros fallan hoy (`strip_empty_workspace_markers` devuelve el archivo entero en `:356-357`); el tercero fija la invariante.

## R8. Parser del eval

`context_segments` (`parse.rs:78-116`) MUST mantener `found`: los segmentos de hit salen sólo de las secciones RECENT CONTEXT y LONG-TERM KNOWLEDGE; los bullets, sólo de WORKSPACE; el texto de ACTIVE CONFLICTS MUST NOT contar. Hoy el parser recorre todo el texto sin mirar la sección y abre un segmento en cada `CONTENT: ` (`:99`); segmentar por sección es comportamiento nuevo. Encabezado de hit = línea que cumple `[` + `YYYY-MM-DD` + ` #` + dígitos + (` ` + palabra sin espacios)? + `]` + ` `; el texto es lo que sigue en la misma línea. `starts_with_date_header` (`:129-137`, exige `]` en el byte 11) deja de servir y se reemplaza. Se actualizan los fixtures de `parse.rs:222-264` (el de conflictos, `:260-264`, todavía tiene la NOTE de dos líneas) y `agent_view/mod.rs:373`.

#### Escenario: multilínea
- GIVEN dos hits, el primero con código y línea en blanco (como el test de `parse.rs:227`)
- THEN hay 2 segmentos, el primero completo

#### Escenario: conflictos
- GIVEN una sección ACTIVE CONFLICTS con `NEW [..#2 disputed] x` y `OLD [..#1] y`
- THEN no se genera ningún segmento

**Tests:** `a_multi_line_hit_runs_until_the_next_header`, `conflict_text_yields_no_segments` en `parse.rs`; `read_context_finds_rows_in_hit_lines_and_workspace_bullets` en `agent_view/mod.rs`, que reemplaza a `read_context_finds_rows_in_hit_blocks_and_workspace_bullets` (`:371`). Fallan hoy: no reconocen `#id`.

## R9. Doc de `delete_memory` y `update_memory`

La doc de `DeleteMemoryArgs.id` (`server.rs:210-211`), la de `UpdateMemoryArgs.id` (`:221-222`) y la descripción de `delete_memory` (`:764-766`) MUST nombrar `recall_context` además de `search_memory` y `list_memories`, y decir que un hit sin kind explícito es `memory`. `kind` sigue siendo obligatorio en las dos (`:212-216`, `:223-225`).

**Test:** `memory_id_docs_name_recall_context_and_the_default_kind` en `server.rs` (mod tests): los schemas de `DeleteMemoryArgs` y `UpdateMemoryArgs`, y la descripción de `delete_memory`, contienen ambos textos. Falla hoy.

## R10. Sin cambios

- Prompt vacío: `format_context` devuelve `no context yet` (`crates/clawg-cli/src/mcp/tools.rs:229-235`).
- El aviso de `synthesize` que `format_recall` agrega después del prompt (`tools.rs:216-222`) no cambia. Con prompt vacío y aviso, la salida sigue siendo `no context yet` seguido del aviso.
- Orden y separadores de secciones, y `K_FINAL`, sin cambios. El separador `=== ACTIVE CONFLICTS (review needed) ===` (`prompt_builder.rs:21`) no cambia: lo nombran las instrucciones del servidor (`server.rs:111`) y la doc de `resolve_conflict` (`:238`, `:794`).
- La salida de `search_memory` es idéntica.
- No se agregan escrituras. `fetch_active_conflicts` sigue siendo un `SELECT` y sigue trayendo sólo pares con un lado mostrado.

#### Escenario
- GIVEN dos corridas iguales antes y después
- THEN `search_memory` imprime los mismos bytes, y el recall sin hits ni conflictos sigue vacío

**Test:** los tests existentes de `prompt_builder.rs`, `tools.rs` y la base de 1.655 siguen verdes, salvo los que fijan el formato viejo, que se actualizan al nuevo en el mismo commit que lo cambia:

| Test | Qué fija hoy | Pasa a fijar |
|---|---|---|
| `format_hit_line_emits_canonical_format` (`prompt_builder.rs:507`) | `SOURCE:`/`CONTENT:` | R1 |
| `format_conflict_pair_renders_both_dates_in_sane_range` (`:528`) | dos líneas `[fecha]` por par | fechas de R2 |
| `the_conflict_note_names_the_tool` (`:557`) | NOTE al final de cada par | R4 |
| `conflicts_query_one_disputed_pair_renders_correctly` (`:726`) | formato `OLD:`/`NEW:` | R2 |
| `e2e_full_4_block_output` (`:1633`) | los cuatro bloques con el formato viejo | R1, R2, R4 y R7 |
| `conflict_blocks_and_the_no_context_sentence_yield_nothing` (`parse.rs:260`) | fixture `OLD:`/`NEW:` | R8 |

Se agrega `empty_prompt_still_yields_the_no_context_sentence` sólo si falta.

## R11. Medición

Con `rag-eval --real --agent-view --diff` contra `docs/eval/results/2026-10-13-real-lexical-fusion-main-agent-view.json` (`main` en `d28fe90`):

| Métrica | Hoy | Umbral |
|---|---|---|
| `found` | 41/46 | 41/46 |
| `search_memory`, media | 852 | sin cambios |
| ACTIVE CONFLICTS, media cuando aparece | 2.274 | ≤ 800 |
| `recall_context`, media / máximo | 3.355 / 4.900 | ≤ 2.100 / ≤ 3.000 |
| RECENT CONTEXT, media | 1.130 | ≤ 1.130 |

Si alguno falla, el cambio no se da por bueno. **Test:** verificación manual por `sdd-verify`, que reporta cada métrica.
