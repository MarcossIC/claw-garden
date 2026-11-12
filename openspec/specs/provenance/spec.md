# provenance — Especificación

Fuente de verdad del comportamiento actual del origen de una memoria: con qué `source` y `created_by_type` se sella cada save, cómo el agente declara que algo lo dijo el usuario, y qué marca muestra cada lector. El origen no cambia ningún par, estado ni puntaje: cómo se forman y se juzgan los pares está en [`specs/disputes`](../disputes/spec.md), y qué es una conclusión, en [`specs/synthesis`](../synthesis/spec.md).

**Origen:** [memory-provenance](../../changes/archive/2026-10-18-memory-provenance/proposal.md), archivado el 2026-10-18. Los ids de requisito son los de esa spec. [preference-model](../../changes/archive/2026-11-06-preference-model/proposal.md), archivado el 2026-11-06, que salió en 0.77.0, sumó PM5 y PM9, con los ids de su spec: la marca de un hit de preferencia, cuyo formato está en [`specs/retrieval`](../retrieval/spec.md), PM5, y el sello de un save declarado que repite una fila.

## Definiciones

- **Origen de una fila:** sus columnas `source` y `created_by_type` en `memory`. No hay columna nueva ni migración: los valores nuevos son `synthesis` en `source` y `agent` en un `explicit`.
- **Marca:** lo que un lector deriva del origen (`mark`, `crates/clawg-core/src/memory/provenance.rs:42-49`). Se marca lo que nunca fue ambiguo, no lo dicho: las filas guardadas antes del sello dicen `explicit` / `user` aunque el usuario no las haya dicho, y no se reescriben, así que una fila sin marca es «dicha por el usuario, o anterior a que se registrara el origen». Ningún lector afirma que una fila es del usuario.

## P1. Sello de un save

El insert de un evento `explicit` MUST sellar la fila así (`seal`, `crates/clawg-core/src/memory/provenance.rs:15-34`, que aplica `crates/clawg-core/src/memory/store_insert.rs:219`):

| `derived_from` | `user_stated` | `source` | `created_by_type` |
|---|---|---|---|
| no vacío | — | `synthesis` | `agent` |
| vacío | `true` | `explicit` | `user` |
| vacío | `false` o ausente | `explicit` | `agent` |

Un evento `chat`, `code_file` o `compaction` MUST sellarse como antes: por el rol del mensaje, y `system` la compactación. Cada pieza de un save largo MUST llevar el mismo origen. El evento de un save sigue siendo `MemorySource::Explicit`, el único que honra `memory_type`, `importance` y `derived_from`; `MemorySource::Synthesis` (`crates/clawg-core/src/memory/events.rs:78`) sólo lo produce el sello, nunca un evento.

#### Escenario: save sin declaración
- GIVEN un save por `POST /v1/memories` sin `user_stated`
- WHEN se guarda
- THEN la fila queda `explicit` / `agent`

#### Escenario: conclusión
- GIVEN dos memorias guardadas
- WHEN se guarda otra con `derived_from` = sus ids
- THEN la fila queda `synthesis` / `agent`, aunque las fuentes las haya dicho el usuario

**Test:** `seal_cases` en `crates/clawg-core/src/memory/provenance.rs`; `an_undeclared_save_is_the_agents`, `a_declared_save_is_the_users` y `a_conclusion_is_sealed_as_synthesis` en `crates/clawg-cli/tests/api.rs`; `explicit_overrides_reach_the_stored_row` en `crates/clawg-core/tests/public_api.rs`.

## P2. La declaración

`save_memory` MUST aceptar `user_stated`, un booleano opcional, y su esquema MUST describirlo (`crates/clawg-cli/src/mcp/server.rs:185-189`). El puente MUST mandarlo sólo cuando es `true` (`crates/clawg-cli/src/client.rs:417-419`). `POST /v1/memories` MUST aceptarlo (`crates/clawg-cli/src/daemon/api.rs:375-377`) y MUST responder 400 que nombra `user_stated`, sin escribir en el outbox, si llega `true` junto a un `derived_from` no vacío (`crates/clawg-cli/src/daemon/api.rs:1298-1302`): una conclusión la saca el agente, y guardarla como del agente después de esa declaración escondería el rechazo. Un evento serializado sin el campo, como una fila del outbox escrita por la versión anterior o el pedido de un puente viejo, MUST leerse como `false`, es decir, como del agente (`crates/clawg-core/src/memory/events.rs:50-55`).

`crates/clawg-cli/src/mcp/protocol.md` dice cuándo declararlo: sólo cuando la memoria es lo que dijo el usuario (sus palabras, una corrección o una instrucción explícita), nunca para lo inferido, encontrado o concluido, y nunca en una conclusión. También pide que un save declarado lleve sólo lo que dijo el usuario, y que el contexto que el agente le suma (una ruta, un motivo) vaya en otro save, sin declarar, y sólo cuando el código y la documentación no lo dicen ya: el sello es por save (`seal`, `crates/clawg-core/src/memory/provenance.rs:15`), así que una oración del agente dentro de un save declarado quedaría como del usuario. Esa regla está en «How to write a save» (y su par en `protocol.es.md`), fuera de la apertura, así que no entra en las instrucciones. El argumento queda fijado en `crates/clawg-cli/src/mcp/agent_contract.json`.

**Test:** `user_stated_with_derived_from_is_refused` en `crates/clawg-cli/tests/api.rs`; `an_event_without_user_stated_reads_as_undeclared` en `crates/clawg-core/src/memory/events.rs`; `the_save_schema_names_user_stated` en `crates/clawg-cli/src/mcp/server.rs`; `a_save_sends_user_stated_only_when_declared` en `crates/clawg-cli/src/client.rs`; `the_agent_contract_matches_its_file` en `server.rs`.

## P3. Marca

`mark(source, created_by_type)` MUST ser pura y devolver:

| `source` | `created_by_type` | Marca |
|---|---|---|
| `synthesis` | cualquiera | `conclusion` |
| `import` | cualquiera | `imported` |
| `explicit` | `agent` | `inferred` |
| cualquier otro | cualquier otro | ninguna |

`explicit` con `user` MUST NOT llevar marca. Ningún código escribe todavía `import`: es el valor que va a escribir una importación.

**Test:** `mark_cases` en `crates/clawg-core/src/memory/provenance.rs`.

## P4. Los pares

- `GET /v1/disputes` MUST traer la marca de cada lado de cada par pendiente que nombra (`new_mark`, `old_mark` de `OpenPair`, `crates/clawg-core/src/memory/dispute.rs:490-500`).
- `memory_status` MUST nombrar primero, en cada par, un lado sin marca cuando el otro la tiene, y MUST escribir la marca junto al id del lado que la tiene (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:277-283`): una inferencia es la que más probablemente cede, aunque el par lo sigue juzgando el agente.
- ACTIVE CONFLICTS MUST escribir la marca de cada lado que la tiene: en el rótulo cuando imprime su texto (`[AAAA-MM-DD #N disputed inferred] …`) y junto al id cuando lo cita (`format_conflict_block`, `crates/clawg-core/src/memory/prompt_builder.rs:817-853`). No reordena: agrupa por la fila `disputed`, y reordenar rompería el grupo.
- El origen MUST NOT cambiar el estado de un par ni el de sus filas, ni el orden de los pares listados.

#### Escenario: inferencia contra lo dicho
- GIVEN un par pendiente cuya fila nueva es `explicit` / `agent` y la vieja `explicit` / `user`
- WHEN el agente llama a `memory_status`
- THEN la línea es `#<vieja> close to #<nueva> inferred` y el par sigue `pending`

**Test:** `open_pairs_carry_each_sides_mark` en `crates/clawg-core/src/memory/dispute.rs`; `the_unmarked_side_of_a_pair_goes_first` en `crates/clawg-cli/src/mcp/tools.rs`; `the_conflict_block_marks_an_inferred_side` en `crates/clawg-core/src/memory/prompt_builder.rs`; `memory_status_counts_pending_candidates` en `crates/clawg-cli/src/mcp/server.rs`, que por HTTP ve dos saves sin declarar marcados `inferred`.

## P5. Síntesis

`synthesize` MUST escribir `origin=<marca>` después del tipo de cada memoria pendiente y de cada conclusión que tiene marca, y nada en las que no la tienen (`synthesis_line_with`, `crates/clawg-cli/src/mcp/tools.rs:596-608`).

**Test:** `the_synthesis_marks_each_rows_origin` en `crates/clawg-cli/src/mcp/tools.rs`.

## P6. Filas anteriores

Una base con filas `explicit` / `user` escritas por la versión anterior MUST abrir con todas sus filas, sin cambiar su origen, y ninguna MUST llevar marca. No hay migración: el sello es un valor más en columnas que ya existían. Un binario anterior abre la base sin cambios y muestra los valores nuevos tal cual, porque los lee como texto (`crates/clawg-core/src/memory/admin.rs:1076-1078`); un save que haga después vuelve a sellarse `explicit` / `user`.

**Test:** `rows_saved_before_provenance_keep_their_origin_and_no_mark` en `crates/clawg-core/src/memory/provenance.rs`.

## PM5. La marca en los hits de una preferencia

`search_memory` y `recall_context` MUST mostrar la marca de un hit sólo cuando es una `user_preference` (`preference_mark`, `crates/clawg-core/src/memory/provenance.rs:53`): el workspace imprime sólo las preferencias declaradas, y el modelo lee una preferencia como una regla de la persona. Cualquier otro hit MUST NOT llevarla. El formato de la línea, `POST /v1/search` y el cliente están en [`specs/retrieval`](../retrieval/spec.md), PM5; `synthesize` sigue marcando cada fila que tiene marca (P5).

**Tests:** los de PM5 en [`specs/retrieval`](../retrieval/spec.md).

## PM9. Un save declarado sella la fila que repite

Un save con `user_stated` cuyo texto ya está vivo en su alcance MUST dejar esa fila con `source = 'explicit'` y `created_by_type = 'user'`, además de reforzarla (`reinforce_in`, `crates/clawg-core/src/memory/store_insert.rs`), sea que la encuentre antes de calcular el embedding o después. También MUST subirla a la importancia del save, sin bajarla nunca, y recalcular su tier con ella: si no, lo que dijo la persona quedaría fuera del workspace por haberse inferido antes con menos importancia ([`specs/retrieval`](../retrieval/spec.md), PM2). Un save sin `user_stated` MUST dejar el sello y la importancia como estaban. Una fila `import` MUST conservar los dos, porque un segundo import y `clawg doctor` la encuentran por su `source`, y una `synthesis` también, porque una conclusión es del agente aunque se apoye en lo que dijo la persona.

#### Escenario: lo inferido, y después declarado
- GIVEN una preferencia guardada sin `user_stated`, que queda `explicit` / `agent` con importancia 5
- WHEN la persona dice lo mismo y el agente guarda el mismo texto con `user_stated` e importancia 9
- THEN sigue habiendo una fila, es `explicit` / `user` con importancia 9, y entra al workspace
- AND otro save declarado del mismo texto con importancia 2 la deja en 9

**Tests:** `a_stated_save_seals_the_row_it_repeats_as_the_users`, `an_inferred_save_leaves_the_seal_of_the_row_it_repeats`, `a_stated_save_leaves_an_import_and_a_conclusion_as_they_are`, `a_stated_save_raises_the_importance_of_the_row_it_repeats_and_never_lowers_it`, `an_inferred_save_leaves_the_importance_of_the_row_it_repeats`, `a_preference_stated_after_it_was_inferred_enters_the_workspace` y `a_twin_restored_while_the_save_is_embedded_is_reinforced_not_copied` (`crates/clawg-core/src/memory/worker.rs`).
