# agent-language — Especificación

Suma requisitos a [`specs/mcp`](../../specs/mcp/spec.md), [`specs/retrieval`](../../specs/retrieval/spec.md), [`specs/hooks`](../../specs/hooks/spec.md), [`specs/quality`](../../specs/quality/spec.md) y [`specs/eval`](../../specs/eval/spec.md). Lo que no se nombra aquí sigue como está. Las definiciones de [language-preference](../language-preference/spec.md) (preferencia, idioma preferido) valen aquí.

## Definiciones

- **Idioma de la sesión:** el idioma preferido que el puente leyó al arrancar, `en` sin preferencia.
- **Marcador:** un texto fijo que el agente, el protocolo o `rag-eval` reconocen por su forma: los encabezados `=== … ===`, `NEW`, `OLD`, `disputed`, `[truncated]`, `piece=K/N`, `by <agent>`, `topic=`, `score=`, `imp=`, `acc=`, `by=#N`, `from=`, `live=`, `lost=`, `origin=` y las marcas `inferred`, `conclusion`, `imported`; también los nombres de herramientas, de argumentos, los tipos de memoria y los valores de `status`.

## AL1. Sin preferencia o con `en`, lo de hoy

Sin preferencia o con `en`, las instrucciones, `tools/list`, cada respuesta del puente, el prompt de `POST /v1/context` sin `lang` o con `lang: en` y lo que imprimen los hooks MUST ser idénticos a los de la versión anterior, salvo lo que 78 ya cambiaba con `en`.

**Test:** `the_agent_contract_matches_its_file`, sin cambios en `agent_contract.json`; los tests actuales de `tools.rs`, `server.rs`, `prompt_builder.rs` y `hook.rs` pasan con `Lang::En` sin cambiar sus textos.

## AL2. El protocolo en español

Con `es`, `protocol::text` MUST ser `protocol.es.md`, y las instrucciones del servidor su apertura, de 2.048 caracteres o menos. `protocol.es.md` MUST tener las mismas secciones `##`, en el mismo orden, y el mismo conjunto de textos entre comillas invertidas que `protocol.md`, y cada texto entre comillas que sea un nombre MUST ser una herramienta registrada, un argumento suyo o un tipo de memoria. Su apertura MUST pedir guardar y buscar en español, responder a la persona en español salvo que escriba en otro idioma, y nombrar `save_memory`, `search_memory`, `recall_context`, `search_project`, `resolve_conflict` y `synthesize`.

#### Escenario: setup escribe el protocolo en español
- GIVEN la preferencia `es`
- WHEN corre `clawg setup claude-code`
- THEN el bloque entre los marcadores es `protocol.es.md`, y `doctor` da `protocol_block` `ok`

**Test:** `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one` (`protocol.rs`), `the_instructions_fit_older_clients` y `the_protocol_names_only_what_exists` sobre cada variante, `the_spanish_instructions_carry_the_saving_habits` (`server.rs`), `the_block_follows_the_preferred_language` (`setup.rs`).

## AL3. Las herramientas en español

Con `es`, la descripción de cada herramienta y de cada argumento MUST ser la de `tools_es.rs`, y `tools/list` MUST coincidir con `agent_contract.es.json`. La tabla MUST tener una entrada por herramienta registrada y por argumento suyo, y ninguna más; cada descripción en español MUST nombrar entre comillas invertidas los mismos textos que la inglesa. Los nombres, los tipos y los `enum` de los esquemas MUST NOT cambiar.

**Test:** `the_spanish_table_covers_every_tool_and_argument`, `the_spanish_descriptions_name_what_the_english_ones_name`, `only_descriptions_change_with_the_language`, `the_spanish_agent_contract_matches_its_file` (`server.rs`).

## AL4. Las respuestas del puente

Con el idioma de la sesión `es`, la prosa de cada respuesta del puente MUST estar en español y los marcadores MUST NOT cambiar. Una frase que el daemon devuelve en un error MUST ir tal cual dentro de la frase del puente.

#### Escenario: una búsqueda sin nada cerca
- GIVEN una sesión en `es` y una búsqueda cuyos hits puntúan 0
- THEN `search_memory` dice `no hay memorias relevantes para esta consulta: 3 buscadas, ninguna lo bastante cerca para mostrarla`

#### Escenario: un hit
- THEN la línea del hit es `1. #4 [memory/semantic_fact/t3 score=0.91 topic=api/port by cursor] …`, igual que en inglés

**Test:** un test por `format_*` en español (`tools.rs`) y `a_spanish_session_answers_in_spanish` (`server.rs`).

## AL5. El contexto

`POST /v1/context` con `lang: es` MUST escribir en español `SHARED_NOTE`, `CONFLICT_NOTE`, la nota del presupuesto y la de los conflictos que no se muestran, con los encabezados y los marcadores intactos. Sin `lang` o con `en` MUST ser el de hoy. El puente MUST mandar el idioma de la sesión cuando hay preferencia, y `session-start` el idioma preferido.

**Test:** `the_notes_of_the_prompt_follow_its_language` (`prompt_builder.rs`), `a_context_in_spanish_keeps_its_markers` (`tests/api.rs`).

## AL6. Los hooks

Con preferencia `es`, `language_line`, el texto de `session-start` y el recordatorio de `user-prompt-submit` MUST estar en español; `user-prompt-submit` MUST leer la preferencia en cada evento.

**Test:** `session_start_speaks_the_preferred_language`, `the_save_reminder_speaks_the_preferred_language` (`hook.rs`).

## AL7. El medidor

`rag-eval` MUST leer como vacía una respuesta de `search_memory` que diga `no memories found`, `no relevant memories for this query`, `no se encontraron memorias` o `no hay memorias relevantes para esta consulta`.

**Test:** `an_empty_answer_reads_as_empty_in_both_languages` (`agent_view/parse.rs`).
