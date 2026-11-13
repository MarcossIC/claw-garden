# verdict-on-replace — Especificación

Cambia lo que el puente MCP le pide al agente sobre los pares que el detector anota. El detector, el daemon, su API, `resolve_conflict` y los datos guardados no cambian (R6).

## R1. La respuesta del save pide la llamada sólo para un reemplazo

Cuando un save confirmado trae candidatas, `save_memory` MUST seguir sumando la línea `close to earlier memories: #id "<línea>"; …`, con `, asks the user` en las que escalan, igual que hoy. Lo que sigue a la lista MUST pedir `resolve_conflict` con `supersedes` (y `keep` = el id nuevo) si el save reemplaza una, y con `conflicts` si sólo la persona puede decir cuál vale, y MUST decir que si valen las dos no hay que hacer nada. La línea MUST NOT nombrar `compatible` ni `scoped`. Un save sin candidatas responde como hoy.

En inglés:

```
close to earlier memories: #412 "Build tool". If this save replaces one, call resolve_conflict with verdict supersedes (keep = the new id), or conflicts if only the user can tell; if both hold, do nothing
```

En español:

```
cerca de memorias anteriores: #412 "Build tool". Si este save reemplaza una, llama a resolve_conflict con verdict supersedes (keep = el id nuevo), o conflicts si sólo la persona puede decirlo; si valen las dos, no hagas nada
```

#### Escenario: un save con dos cercanas
- GIVEN un save confirmado como `#57` con `related` `#412 "Build tool"` y `#9 "claim 9"`, la segunda escalada
- WHEN el puente arma la respuesta
- THEN la línea nombra las dos, la segunda con `, asks the user`
- AND termina en `if both hold, do nothing`
- AND no contiene `compatible` ni `scoped`

**Test:** `save_outcome_text_names_related_memories` (`crates/clawg-cli/src/mcp/server.rs`), en los dos idiomas.

## R2. La respuesta de `restore_memory` pide lo mismo

La línea `saved while it was deleted, close to it: …` MUST pedir `resolve_conflict` con `supersedes` (y `keep` = el id más nuevo) si una más nueva reemplaza a la restaurada, y con `conflicts` si sólo la persona puede decir cuál vale, y MUST decir que si valen las dos no hay que hacer nada. MUST NOT nombrar `compatible` ni `scoped`.

En inglés: `saved while it was deleted, close to it: {named}. If a newer one replaces it, call resolve_conflict with verdict supersedes (keep = the newer id), or conflicts if only the user can tell; if both hold, do nothing`.

En español: `guardadas mientras estaba borrada, cerca de ella: {named}. Si una más nueva la reemplaza, llama a resolve_conflict con verdict supersedes (keep = el id más nuevo), o conflicts si sólo la persona puede decirlo; si valen las dos, no hagas nada`.

**Test:** `format_restore_names_key_replacements_and_pairs` (`crates/clawg-cli/src/mcp/tools.rs`).

## R3. El protocolo ya no manda juzgar cada cercana

La frase de la apertura sobre las cercanas MUST decir, en `protocol.md` y en `protocol.es.md`, que el agente llama a `resolve_conflict` sólo cuando la memoria nueva reemplaza a una de las que nombra `save_memory` (`supersedes`), mientras todavía tiene la conversación, o cuando sólo la persona puede decir cuál vale (`conflicts`), y que si valen las dos no hace nada. MUST NOT decir `judge each` ni `juzga cada una`, ni nombrar `compatible` o `scoped` en esa frase. La frase sobre ACTIVE CONFLICTS no cambia. La apertura MUST seguir entrando en 2.048 caracteres en los dos idiomas: en inglés la frase nueva la pasaba por tres, y la apertura cede la palabra `own` de «the project's own documentation and code».

En inglés:

```
When `save_memory` names earlier memories close to the new one and the new one
replaces one of them, call `resolve_conflict` with supersedes while you still
have the conversation, or with conflicts if only the user can tell; when both
hold, do nothing.
```

En español:

```
Cuando `save_memory` nombre memorias cercanas a la nueva y la nueva reemplace a
una, llama a `resolve_conflict` con supersedes mientras tengas la conversación,
o con conflicts si sólo la persona puede decirlo; si valen las dos, no hagas
nada.
```

**Tests:** `the_instructions_carry_the_saving_habits` y `the_spanish_instructions_carry_the_saving_habits`, con la frase nueva en lugar de `judge each with` y `juzga cada una con`; el test del tope de 2.048 caracteres, que ya existe (`server.rs`).

## R4. `memory_status` no le plantea los pares pendientes al agente

La línea de conflictos de `memory_status` MUST decir sólo cuántos pares esperan a la persona:

- sin pares escalados, `conflict candidates: none open` (`candidatos a conflicto: ninguno abierto`), haya o no pares pendientes;
- con `n` escalados, `conflict candidates: n waiting for the user in ACTIVE CONFLICTS` (`candidatos a conflicto: n esperando a la persona en ACTIVE CONFLICTS`).

MUST NOT contar los pares pendientes, listar ninguno, ni decir cómo juzgarlos. Contra un daemon sin `GET /v1/disputes` MUST omitir la línea, como hoy. El puente MUST NOT conservar código que ya no usa. La respuesta de `GET /v1/disputes` se sigue leyendo con el tipo que el daemon serializa (`OpenPairs`, de `clawg-core`), del que el puente imprime sólo `escalated`.

#### Escenario: pendientes y ninguno escalado
- GIVEN un store con dos pares `pending` y ninguno `escalated`
- WHEN el agente llama a `memory_status`
- THEN la línea es `conflict candidates: none open`
- AND la respuesta no contiene `resolve_conflict` ni `close to`

#### Escenario: uno escalado
- GIVEN un store con dos pares `pending` y uno `escalated`
- WHEN el agente llama a `memory_status`
- THEN la línea es `conflict candidates: 1 waiting for the user in ACTIVE CONFLICTS`

**Tests:** `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs`, en lugar de `memory_status_counts_pending_candidates`); `pending_pairs_are_not_put_to_the_agent` y `format_status_ends_with_the_open_pairs` (`tools.rs`). `the_unmarked_side_of_a_pair_goes_first` se retira con la lista que probaba.

## R5. Las descripciones de las herramientas

- La de `resolve_conflict` MUST decir que el agente juzga un par que nombra `save_memory` cuando el save nuevo reemplaza a la memoria vieja o cuando sólo la persona puede decir cuál vale, y uno bajo ACTIVE CONFLICTS una vez que la persona contestó. MUST NOT nombrar `memory_status` como fuente de pares para juzgar. La regla de que una preferencia o una decisión sólo se reemplaza por palabra de la persona no cambia.
- La de `restore_memory` MUST decir que nombra las memorias cercanas guardadas mientras tanto por si una la reemplaza, y MUST NOT pedir que se juzguen todas.
- La de `keep` MUST decir que el id es el que muestran ACTIVE CONFLICTS o `save_memory`, y ya no `memory_status`, que dejó de mostrar ids.
- Las descripciones de `verdict` y de los demás parámetros no cambian: la herramienta acepta los cuatro veredictos.

Lo mismo en `tools_es.rs`. `agent_contract.json` y `agent_contract.es.json` MUST coincidir con lo que el servidor declara.

**Tests:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs`), con los dos archivos regenerados.

## R6. Lo que no cambia

- El detector anota los mismos pares, `pending` o `escalated`, con su distancia ([`specs/disputes`](../../../specs/disputes/spec.md), «El detector anota candidatas»).
- `resolve_conflict` acepta `supersedes`, `compatible`, `scoped` y `conflicts` con el mismo efecto (R1, R2 y R3 de `specs/disputes`). Un agente con el protocolo anterior sigue funcionando.
- `GET /v1/disputes` devuelve lo mismo, así que un puente anterior sigue listando los pendientes.
- La nota de ACTIVE CONFLICTS sigue ofreciendo `compatible` y `scoped`: sobre un par escalado ese veredicto es la respuesta de la persona.

**Tests que ya lo guardan:** `a_confirmed_save_names_its_candidates` (`worker.rs`); `compatible_and_scoped_close_without_touching_the_ranking` y `open_pairs_carry_each_sides_mark` (`dispute.rs`); el test de la nota de conflictos en `prompt_builder.rs`.

## Specs que se consolidan al archivar

- [`specs/disputes`](../../../specs/disputes/spec.md), «Las candidatas en la respuesta del save»: lo que pide la línea.
- [`specs/mcp`](../../../specs/mcp/spec.md), «Los conflictos en las herramientas» y «El protocolo de memoria»: R1 a R5.
- [`specs/provenance`](../../../specs/provenance/spec.md), P4: el escenario de la marca en la línea de `memory_status` deja de existir en el puente; la marca sigue en `GET /v1/disputes` y en ACTIVE CONFLICTS.
