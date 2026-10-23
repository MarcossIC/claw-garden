# agent-language — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | L (la parte C de la [exploración de 78](../2026-10-23-language-preference/exploration.md#alcance-candidato)) | language-preference (78): `Lang`, `preferences.json`, `lang` en el contexto; cli-language (79) no | ver [`state.yaml`](state.yaml) |

## Problema

Con 78, una persona que eligió `es` tiene un agente al que se le pide guardar, buscar y responder en español, pero todo lo que ese agente lee de clawg sigue en inglés:

1. **Las instrucciones y el protocolo.** `protocol.md` (`crates/clawg-cli/src/mcp/protocol.md`, unos 8.600 caracteres) llega entero al `CLAUDE.md` del usuario por `clawg setup claude-code`, y su apertura como instrucciones del servidor (`get_info`, `server.rs`). Con `es`, `protocol::text` sólo reescribe dos frases (`OPENING_RULE` y `SAVE_RULE`, `protocol.rs`) y deja el resto en inglés.
2. **Las herramientas.** Las 12 descripciones (`#[tool(description = …)]`, `server.rs`) y los 44 argumentos (comentarios `///` de los `*Args`, que `schemars` vuelve `description`) están en inglés; `agent_contract.json` los fija.
3. **Las respuestas del puente.** Cada `format_*` de `crates/clawg-cli/src/mcp/tools.rs` (`format_search`, `format_recall`, `format_save`, `format_update`, `format_delete`, `format_restore`, `format_rows`, `format_synthesis`, `format_docs`, `format_open_pairs`, `format_resolution`, `format_sharing`, `format_language`, `format_status`, `topic_key_warning`, `synthesis_nudge`, `review_nudge`, `other_language_line`) y las frases que compone `server.rs` (lo que responde un save, «not saved: …», los rechazos del puente, `AgentSource::describe`).
4. **El contexto.** Las notas que arma el daemon dentro del prompt: `SHARED_NOTE`, `CONFLICT_NOTE`, `format_cut` y «(+N more conflicts not shown)» (`crates/clawg-core/src/memory/prompt_builder.rs`).
5. **Los hooks.** `language_line`, `session_start_text` y `save_reminder` (`crates/clawg-cli/src/hook.rs`). `user-prompt-submit` ni siquiera lee la preferencia (`respond`: `HookEvent::UserPromptSubmit => None`).

El agente lee entonces una mezcla: una instrucción en inglés que le dice que guarde en español, y respuestas en inglés a búsquedas en español. La exploración de 78 decidió que también se localiza lo que lee el agente, aunque cueste (decisiones 5 y 7).

## Decisiones

Las decisiones 5 y 7 de la exploración están cerradas con su opción recomendada; el [diseño](design.md) registra cómo se aplican:

1. **Marcadores estables, prosa traducida (5.a).** No cambian los encabezados `=== … ===` del contexto, los de `workspace.md`, `NEW`, `OLD`, `disputed`, `[truncated]`, `piece=K/N`, `by`, `topic=`, `score=`, `imp=`, `acc=`, `by=#N`, `from=`, `live=`, `lost=`, `origin=`, las marcas de procedencia (`inferred`, `conclusion`, `imported`), los nombres de herramientas, argumentos y tipos de memoria, los valores de `status`, el JSON de `memory_status` ni las claves que lee `rag-eval`. Se traduce la prosa alrededor.
2. **Un archivo por idioma para el protocolo (7.a):** `protocol.es.md`, escrito para el español y no traducido frase a frase; su apertura entra en 2.048 caracteres. Con `es`, `protocol::text` lo devuelve entero; con `en` o sin preferencia, lo de hoy.
3. **Una tabla por idioma para las herramientas (7.a):** `crates/clawg-cli/src/mcp/tools_es.rs`, `(herramienta, argumento) → texto`, que el puente aplica a su `ToolRouter` al fijar el idioma. Un segundo contrato, `agent_contract.es.json`, lo fija con el mismo `CLAWG_BLESS=1`.
4. **Paridad, por tests.** Los dos protocolos tienen las mismas secciones y los mismos textos entre comillas invertidas; la tabla cubre exactamente las herramientas y los argumentos registrados; cada descripción en español nombra entre comillas invertidas lo mismo que la inglesa.
5. **Las respuestas del puente siguen el idioma de la sesión (8.a de 78):** el que el puente fijó al arrancar. Un `match` sobre `Lang` por frase, como en 79.
6. **El contexto sigue el `lang` del pedido.** El daemon ya recibe `lang` en `POST /v1/context` (78); con `es` escribe sus notas en español. El puente lo manda con una preferencia, y ahora también `session-start`. Sin `lang` el prompt es el de hoy, byte a byte.
7. **Las frases que vienen del daemon siguen en inglés (6.a):** el `message` de un 400, 404 o 409 (`api.rs`) mezcla códigos con texto y lo lee también quien llama la API sin puente; el puente las enmarca («no se guardó: {causa}»).
8. **La línea de `topic_key_warning` en español pierde la medida de 34 tokens** que fijó `revision-eval` (`cost.rs`, `WARNING_LINE`), que sigue fijando la inglesa; la española queda sin medir (ver Éxito).

## Alcance

1. `protocol.es.md` y `protocol::text(Some(Lang::Es))`; `setup` escribe ese bloque y `doctor` (`protocol_block`) lo compara.
2. `tools_es.rs` y `ClawgMcp::with_language`, que reescribe las descripciones; `agent_contract.es.json` y `the_spanish_agent_contract_matches_its_file`.
3. Las respuestas de `tools.rs` y `server.rs` con `Lang`.
4. Las notas de `prompt_builder.rs` con `Lang` (`build_memory_prompt`, `build_session_prompt`); el daemon las toma del `lang` del pedido; `DaemonClient::recent_context` manda `lang`.
5. Los hooks: `session_start_text` y `save_reminder` en el idioma preferido; `user-prompt-submit` lee la preferencia.
6. `rag-eval`: el medidor de agent-view acepta las respuestas vacías en los dos idiomas.
7. La medición de caracteres de las instrucciones, de `tools/list` y de la línea del otro idioma en cada idioma (M3, sin el tokenizador del modelo que las lee).
8. `.claude/CLAUDE.md`; las specs `mcp`, `retrieval`, `hooks`, `quality` y `eval` se reescriben al archivar, junto con las de 78 y 79.

## Fuera de alcance

- **M1 y M2 con el modelo real** (la paridad español contra inglés sobre el golden pareado, y el store mezclado con el umbral `FEW_SHOWN`): piden el golden de `docs/eval/`, que está fuera de git, y el modelo. Quedan anotadas en la verificación con lo que hace falta para correrlas; `FEW_SHOWN` sigue sin calibrar.
- Las frases del daemon que llegan al agente (decisión 7 de arriba) y los encabezados del contexto (decisión 1).
- Traducir lo guardado, más idiomas, el idioma de `search_project`.

## Datos guardados

Ninguno. El cambio no toca `memory.db`, `preferences.json` ni el formato de export. El bloque del `CLAUDE.md` del usuario cambia sólo cuando la persona vuelve a correr `clawg setup claude-code`, que copia el archivo anterior a `<data>/backups/setup/` como siempre; `clawg doctor` avisa que el bloque instalado no es el del idioma preferido (`protocol_block`, `stale`).

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- sin preferencia y con `en`, las instrucciones, `tools/list`, cada respuesta del puente, el contexto y los hooks son los de hoy; `agent_contract.json` no cambia;
- con `es`, las instrucciones son la apertura de `protocol.es.md`, de 2.048 caracteres o menos, y `tools/list` coincide con `agent_contract.es.json`;
- `protocol.es.md` tiene las mismas secciones y los mismos textos entre comillas invertidas que `protocol.md`, y nombra sólo herramientas, argumentos y tipos que existen;
- la tabla cubre cada herramienta y cada argumento, ni uno más, y cada descripción nombra lo mismo que la inglesa;
- con `es`, cada respuesta del puente, las notas del contexto con `lang: es` y los hooks están en español, con los marcadores intactos;
- `rag-eval` lee una respuesta vacía en español como vacía.

Medidas, sin modelo: los caracteres de las instrucciones, de `tools/list` y de la línea del otro idioma en cada idioma.
