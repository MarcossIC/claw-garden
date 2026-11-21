# conflicts-on-demand — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción (o porque lo que nombra todavía no existe). Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia. La suite completa y los dos clippy (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; en Windows, con `--target-dir target-test`) corren una vez, en la tarea 15. Todo corre en Linux con el embedder falso (`CLAWG_TEST_EMBEDDER=fake`), sin el modelo real y sin `~/.clawg*`; los tests que corren el binario usan `CLAWG_TEST_HOME` y `--port 0`, y en clawg-core el directorio sale de `crate::test_dir::test_dir()`. Ni el código, ni los comentarios, ni los nombres de test, ni los mensajes de error, ni los de commit nombran planes, changes ni ids CO: los ids van sólo aquí. No hay `design.md` (D1 a D7 cerradas en la [propuesta](proposal.md)).

Líneas citadas verificadas el 2026-11-20 sobre `develop`; los tests nuevos van junto a los que se nombran.

## 1. `context.cut` lleva `state` (CO11)

- [ ] Test primero: `a_context_records_its_state_its_size_and_the_blocks_it_cut` (`crates/clawg-cli/tests/api.rs:5167`): `gauge_of` (`:5148`) pide `block` y `state` de `context.cut`; un arranque cortado da `block=recent,state=recent` y ninguna serie `state=searched` para ese bloque.
- [ ] `record_context` (`crates/clawg-cli/src/daemon/api/memories.rs:1120-1126`): `context.cut` con `("block", ...)` y `("state", retrieval.name())`.
- Commit: `feat(metrics): label each context cut with the retrieval state`.

## 2. Los pares escalados, en el núcleo (CO7)

- [ ] Tests primero en `crates/clawg-core/src/memory/revision/dispute.rs` (mod tests, junto a `open_pairs_count_and_list_the_live_pending_ones`, `:1360`, y `open_pairs_count_by_project_and_reach`, `:1378`): el listado de escalados trae sólo `escalated`, los más viejos primero (`p.created_at, p.new_id, p.old_id`), con tope, con `new_line` y `old_line` (primera línea de cada lado, 500 caracteres) y las marcas de origen de `open_pairs_carry_each_sides_mark` (`:1403`); el alcance y los conteos `pending` y `escalated` no cambian; `open_pairs` sin pedirlo es el de hoy.
- [ ] `open_pairs` (`dispute.rs:524`; hoy filtra `p.state = 'pending'`, `:545`) acepta qué estado lista y el tope; con escalados suma las dos primeras líneas. Los límites (5 y `ESCALATED_LISTED` = 10) los pone el daemon (tarea 3), no el núcleo. `OpenPair`/`OpenPairs` (`:485`, `:500`) llevan los campos nuevos omitidos cuando no se piden.
- Commit: `feat(disputes): list the escalated pairs with the first line of each side`.

## 3. `GET /v1/disputes?escalated=true` (CO7)

- [ ] Tests primero en `crates/clawg-cli/tests/api.rs`, uno por escenario: «diez de doce» (12 escalados y 3 pendientes: `escalated` 12, `pending` 3, diez pares escalados, `new_line` y `old_line`), «sin el parámetro» (los pendientes, sin las líneas, byte a byte) y «alcance» (lado oculto por `sharing.json` y otro proyecto, con `project_id`).
- [ ] `DisputesQuery` (`crates/clawg-cli/src/daemon/api/disputes.rs:47-48`, `deny_unknown_fields`) gana `escalated: bool`; `ESCALATED_LISTED = 10` junto a `OPEN_PAIRS_SHOWN` (`:59`); el handler (`:104-110`) elige estado y tope.
- Commit: `feat(daemon): list the escalated pairs on request with the first line of each side`.

## 4. El otro lado de cada fila (base de CO2 y CO5)

- [ ] Tests primero en `crates/clawg-core/src/memory/retrieval/prompt_builder.rs` (mod tests; la regla de lectura de `a_conflict_side_cannot_open_a_new_or_old_line`, `:1536`): una función pura que da la marca (`conflict=#9`, varios `#4,#7,#9+2`, ordenados de menor a mayor, tres como máximo) y otra con el store que, dados ids de filas, devuelve los otros lados de los pares escalados con la regla de `fetch_active_conflicts` (`:1271-1325`: `m_new.status = 'disputed'`, sin `deleted_at`, `reader.admits` en los dos lados): no cuenta un par `pending`, un lado borrado ni un lado oculto; un solo lado pedido basta.
- [ ] La función con el store, con visibilidad la mínima que usen la tarea 5 (mismo módulo) y la 8 (daemon); una consulta por llamada, sobre los ids pedidos. La marca queda separada de la E/S.
- Commit: `feat(retrieval): find the other side of each escalated pair for a set of rows`.

## 5. Cada fila en disputa lleva `conflict=#N` en el prompt (CO2)

- [ ] Tests primero en `prompt_builder.rs` (mod tests): la línea de un hit `[<fecha> #12 <tipo> conflict=#9] <texto>` (después de tipo, origen y `piece=`, antes de ` by`; junto a `format_hit_line_names_the_piece_of_a_long_save`, `:1546`); la entrada de AUTO termina su primera línea con `(#9 conflict=#12)`; el lado oculto no deja marca ni id; más de tres (`conflict=#4,#7,#9+2`); siete pares con tope, cinco en el bloque, `(+2 more conflicts not shown)` y los siete hits marcados (junto a `conflict_block_keeps_five_pairs_and_counts_the_rest`, `:1737`); `format_hit_line_drops_the_old_labels` (`:1565`) y `hit_line_is_one_header_with_id_and_text` siguen pasando (sin marca, byte a byte). En `crates/clawg-core/src/memory/retrieval/workspace.rs` (mod tests): `workspace.md` no lleva la marca.
- [ ] `assemble` (`prompt_builder.rs:355-380`) pide las marcas sobre `shown` y las pasa a `format_hit_line` (`:828`) y a la sección AUTO sólo en el texto del prompt; LAST SESSION no se marca. No cambia orden, score ni presupuesto: los bytes entran en lo que mide `fit_to_budget`.
- [ ] El comentario de `crates/clawg-core/src/memory/retrieval/workspace.rs:390-393` da la razón nueva (la marca avisa de la disputa junto a cada fila, no el bloque). Sin test; `verify` lo mira en el diff.
- Commit: `feat(retrieval): mark each printed row that waits on a disputed pair`.

## 6. El presupuesto corta los pares antes que LAST SESSION (CO3)

- [ ] Tests primero en `prompt_builder.rs` (mod tests): cambian `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts` (`:1998`) y `conflicts_go_after_every_hit` (`:1841`, la expectativa del orden `Recent, LongTerm, LastSession, Conflicts` de `:1892-1905`); nuevo «los pares ceden al resumen» (consulta, resumen de 3.000 bytes, un hit, tres pares: LAST SESSION entero, el hit marcado, `(+N more conflicts not shown)`); `the_last_session_block_gives_way_after_the_hits` (`:1849`, sin pares) sigue. Un texto que entra entero es el mismo byte a byte, y `context.cut` cuenta `conflicts` por cada par quitado.
- [ ] `fit_to_budget` (`:468-540`): el brazo de `conflicts` (`:499-504`) pasa antes que el de `last_session` (`:495-498`); el orden pasa a ser AUTO sobre la mitad, hits sobre la reserva, pares desde el último, LAST SESSION, hits restantes, AUTO restante. La reserva y SC5 no cambian.
- [ ] El escenario «la reserva» de `openspec/specs/retrieval/spec.md` no se edita aquí: lo consolida el archivo.
- Commit: `fix(retrieval): cut the conflict pairs before the last session when a prompt overruns`.

## 7. El arranque sin ACTIVE CONFLICTS y con el resumen entero (CO1, CO4, CO12)

- [ ] Tests primero en `prompt_builder.rs` (mod tests, con el store): CO1 «una fila reciente y una preferencia en disputa» (sin `ACTIVE CONFLICTS`, `NEW `, `OLD ` ni NOTE; las dos filas marcadas) y «sin pares» (byte a byte el de antes); CO4 «un store maduro» (workspace con AUTO llena, resumen de 3.500 bytes, diez saves, cinco pares, 8.000 bytes: LAST SESSION entero, sin `ACTIVE CONFLICTS`, `cut` sin `LastSession`), junto a `a_context_without_a_query_keeps_the_last_session_over_its_saves` (`:4126`) y `a_stocked_workspace_leaves_room_for_the_last_session` (`:3331`); CO12 «una consulta con un par» (bloque, NOTE, `NEW #N`/`OLD […]` y el hit con `conflict=#M`); siguen `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest` (`:1629`), `conflict_block_has_one_note` (`:1672`), `the_conflict_note_names_the_tool` (`:1681`), `conflict_pairs_rank_by_best_hit_position_workspace_only_last` (`:1721`) y `e2e_full_4_block_output` (`:3383`), éste con la marca en la línea del hit.
- [ ] `build_session_prompt` (`:311-331`) arma sin pares: `assemble` no pide ni imprime el bloque cuando no hay consulta (`build_memory_prompt`, `:198`, no cambia); las marcas de la tarea 5 siguen.
- Commit: `fix(retrieval): leave the conflict block out of a context without a query`.

## 8. `POST /v1/search` manda `conflicts_with` (CO5, CO13)

- [ ] Tests primero: `crates/clawg-cli/tests/api.rs`, uno por escenario («los dos lados» con `[1]` y `[2]`; «lo que no cuenta»: pendiente, lado borrado, lado oculto, hit sin campo y cuerpo igual; «un solo lado devuelto»); `ranked_hit_serializes_to_json` (`crates/clawg-core/tests/public_api.rs:62`) guarda que el campo se omite vacío.
- [ ] `RankedHit` (`crates/clawg-core/src/memory/retrieval/search.rs:143`) gana `conflicts_with: Vec<i64>` con `skip_serializing_if` vacío, como `origin` (`:171-173`), en `[]` en cada constructor; el handler `search` (`crates/clawg-cli/src/daemon/api/memories.rs:938`) lo llena con la función de la tarea 4, una consulta por búsqueda, después de la poda de relevancia. Ni ranking, ni score, ni `search.found` cambian.
- Commit: `feat(search): tell each hit which rows its escalated pairs oppose`.

## 9. `search_memory` muestra ` conflict=#N` (CO6)

- [ ] Tests primero: `format_hits_shows_the_conflict_of_a_hit` (`crates/clawg-cli/src/mcp/tools.rs`, junto a `format_hits_shows_the_origin_of_a_preference`, `:1727`): `1. #12 [Memory/user_preference/t1 score=0.83 conflict=#9] <texto>`, antes de `topic=`, `project=` y ` by`, y sin el campo la línea de hoy; `a_hit_from_a_daemon_without_conflicts_has_none` (`crates/clawg-cli/src/client.rs`, junto a `:3149`); en `crates/clawg-cli/src/mcp/server.rs`, `search_memory` contra un daemon de prueba que marca.
- [ ] `Hit` (`client.rs:212-246`): `conflicts_with: Vec<i64>` con `#[serde(default)]`; `format_hits` (`tools.rs:232-267`, formato en `:252`) la imprime tras `score=` con la marca de la tarea 4 (`conflict=` del lado del puente, la lógica pura compartida o repetida sólo si no hay un lugar común sin dependencia nueva).
- Commit: `feat(mcp): show the conflict mark in each search hit`.

## 10. El cliente y el formato de la lista de pares (CO8, sin lo que ve el modelo)

- [ ] Tests primero en `crates/clawg-cli/src/mcp/tools.rs` (mod tests, junto a `format_status_ends_with_the_open_pairs`, `:2410`): el formato de la lista (`NEW #N <línea>`, `OLD #M <línea>`, marca de origen de cada lado, `(+N more waiting, ask again once these are answered)` y su equivalente en español cuando `escalated` pasa de lo que trae el daemon, línea final que pide preguntarle a la persona y registrar sólo su respuesta, `conflict candidates: none open` sin pares) y que la línea final no juzga ningún par (como `pending_pairs_are_not_put_to_the_agent`, `:2403`); en `client.rs`, que `open_disputes` (`:1497`) puede pedir `escalated=true`, y que un daemon que responde error de consulta se distingue.
- [ ] `open_disputes` con el parámetro; `OpenPairs`/`OpenPair` leen `new_line` y `old_line` con `#[serde(default)]`; el formateador en `tools.rs` y su versión en español (`tools_es.rs` si la tabla vive ahí).
- Commit: `feat(mcp): format the escalated pairs a daemon lists on request`.

## 11. `list_memories` con `conflicts` (CO8)

- [ ] Tests primero en `crates/clawg-cli/src/mcp/server.rs` (mod tests, junto a `memory_status_counts_only_the_pairs_that_wait_for_the_user`, `:4116`), uno por escenario: «los pares del proyecto» (dos del proyecto, uno de otro proyecto y uno pendiente: sólo los dos), «sin pares», «más de diez» (doce: diez y «quedan dos»), «con un filtro de filas» (`conflicts: true` con `tier`, `agent`, `memory_type`, `limit` u `offset`: se niega sin llamar al daemon) y «un daemon anterior» (rechaza `escalated=true`: pide `clawg stop` y no lista nada, como `server.rs:1599`); sin `conflicts` o con `false`, los de `list_memories` de hoy sin cambio.
- [ ] `ListMemoriesArgs` (`server.rs:313-333`) gana `conflicts: Option<bool>`, con una descripción de una oración en las dos lenguas (`tools_es.rs`); `list_memories_impl` (`:1396`) rama con `conflicts`: el proyecto abierto o todos según `one_project` (`:664`), `GET /v1/disputes?escalated=true`, el formato de la tarea 10. La frase de `list_memories` (`:1884`) nombra la opción.
- [ ] Regenerar los contratos: `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y `... the_spanish_agent_contract_matches_its_file`; el diff es sólo el parámetro y la frase de `list_memories`; correr los tests otra vez.
- Commit: `feat(mcp): list the pairs waiting for the user from list_memories`.

## 12. La apertura y las descripciones dicen dónde se ven los pares (CO9, CO10)

- [ ] Tests primero: `the_instructions_fit_older_clients` (`server.rs:2972`) y `the_instructions_carry_the_saving_habits` (`:3019`) y su versión en español con el texto nuevo (la frase de 205 caracteres que conserva «ask the user which memory holds», «record only their answer» y la regla de `is_name`, `:2935`; sin nombrar `list_memories`; las tres aperturas en 2.048 o menos); en `crates/clawg-cli/src/mcp/protocol.rs`, que ninguna apertura nombra `ACTIVE CONFLICTS` ni depende de `recall_context` y que `lean` no cambia; cambian `format_status_ends_with_the_open_pairs` (`tools.rs:2410`, fija «in ACTIVE CONFLICTS» en `:2413`), el de `format_resolution` (`tools.rs:2228`) y `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs:4156`, `:4220`); `pending_pairs_are_not_put_to_the_agent` sigue.
- [ ] `crates/clawg-cli/src/mcp/protocol.md:28-30` y `protocol.es.md:27-29`: la frase pide preguntar cuando una respuesta depende de una memoria marcada `conflict=` o la persona pide revisar, y registrar sólo su respuesta con `resolve_conflict`, `user_answered` y sus palabras en `reason`. Descripción de `resolve_conflict` (`server.rs:1973-1981`, en español `tools_es.rs`) y de `keep` (`server.rs:458`, `tools_es.rs:113`): marca `conflict=`, `list_memories` con `conflicts` o `save_memory`, sin `memory_status` como fuente. `format_resolution` (`tools.rs:270-282`) y `format_open_pairs` (`:286-293`) dejan de decir «in ACTIVE CONFLICTS» y dicen que `list_memories` con `conflicts` los lista, sin listar ni juzgar (VR4, VR5).
- [ ] Regenerar los contratos una vez (mismos comandos que la tarea 11); el diff es sólo la frase de las instrucciones, las descripciones de `resolve_conflict` y de su `keep`, y la de `list_memories` ya regenerada; correr los tests otra vez.
- Commit: `feat(mcp): ask about a conflict mark instead of a conflict block`.

## 13. Un par que ya esperaba no cambia (CO13)

- [ ] Test primero en `crates/clawg-cli/tests/api.rs`: un store con un par escalado de antes; se pide el contexto sin consulta (no lo imprime, marca sus filas), se busca con una consulta que lo toca (marca los dos lados) y se pide `GET /v1/disputes?escalated=true` (lo trae); `memory_conflict` y `memory` (estado, filas, `disputed_against`) idénticas antes y después, y las dos filas se siguen encontrando (`status IN ('active', 'disputed')`, `search.rs:287`).
- [ ] Sin código: si el test falla, el defecto está en 5 a 8 y se corrige allí.
- Commit: `test(daemon): pin that the waiting pairs keep their state while the context marks them`.

## 14. Documentación (CO1, CO5, CO7, CO8, CO11)

- [ ] `docs/reference/daemon.md`: el campo `conflicts_with` del hit de `POST /v1/search` (omitido vacío), el parámetro `escalated` de `GET /v1/disputes` (tope 10, `new_line` y `old_line`, sin él los pendientes) y dónde aparecen los pares (con consulta, el bloque, cortado antes que LAST SESSION; sin consulta, ninguno: la marca `conflict=` y `list_memories` con `conflicts`), y `context.cut` con `state` donde se listan las métricas del contexto. Las líneas que hoy dicen que el bloque llega a todo agente que alcanza los dos lados (línea 23) se ajustan.
- [ ] `docs/reference/testing.md` sólo si lista las etiquetas de `context.cut` (comprobar con `rg`).
- Commit: `docs(reference): document the conflict mark, the escalated listing and the state of a context cut`.

## 15. Verificación

- [ ] La suite y los dos clippy de `.claude/CLAUDE.md`, una vez, sobre el último commit; el árbol sin directorios de build extra.
- [ ] `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin regenerar; los dos diffs de los contratos sólo tocan lo que permite CO9.
- [ ] `verify.md`: cada requisito con su test, el diff de los contratos y la medida de la frase y de las tres aperturas, con la campaña local pendiente.
- Commit: `docs(openspec): verify conflicts-on-demand and leave its local measurement pending`.

## Después de publicar (local, perfil real, con autorización; sin casillas)

No se hace en la nube ni cuenta como implementación: necesita el modelo real, el golden de `docs/eval/` o el perfil real.

- **`rag-eval --real --agent-view`** con el golden contra la última línea base: `search.found` y `context.found` sin moverse («no distinguible» con menos de 6 casos en un sentido) e informar los tokens que suma la marca.
- **Una semana con la versión instalada:** `context.cut{block=conflicts,state=recent}` en 0 y `context.cut{block=last_session,state=recent}` en el 2 % o menos de los contextos con `context.retrieval_state{state=recent}` (hoy 134 de 250). Por encima, es otro cambio (SC2 o SC5). Si `context.cut{block=conflicts,state=searched}` sigue alto, queda B2.
- **Los bloques `full` instalados** quedan `stale` hasta `clawg setup claude-code`.

## Requisito → tarea

| Requisito | Tarea(s) |
|---|---|
| CO1 | 7 |
| CO2 | 4, 5 |
| CO3 | 6 |
| CO4 | 7 |
| CO5 | 4, 8 |
| CO6 | 9 |
| CO7 | 2, 3 |
| CO8 | 10, 11 |
| CO9 | 12 |
| CO10 | 12 |
| CO11 | 1 |
| CO12 | 7 |
| CO13 | 13 |
