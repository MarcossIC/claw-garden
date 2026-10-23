# agent-language — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. El protocolo en español (AL2)

- [ ] Tests: secciones y textos entre comillas iguales a los de `protocol.md`, apertura de 2.048 caracteres o menos, nombres que existen, los hábitos de la apertura, `setup` con `es`.
- [ ] `protocol.es.md`; `protocol::text(Some(Lang::Es))` lo devuelve.
- Commit: `feat(mcp): write the memory protocol in Spanish`

## 2. Las herramientas en español (AL3)

- [ ] Tests: la tabla cubre cada herramienta y argumento, nombra lo mismo que el inglés, sólo cambian las descripciones; el contrato en español.
- [ ] `tools_es.rs`, `with_language` reescribe el router; `agent_contract.es.json` bendecido.
- Commit: `feat(mcp): describe the tools in Spanish`

## 3. Las respuestas del puente (AL4)

- [ ] Tests de cada `format_*` y de las frases de `server.rs` en español, con los marcadores intactos.
- [ ] `Lang` en `tools.rs` y `server.rs`.
- Commit: `feat(mcp): answer in the language of the session`

## 4. El contexto (AL5)

- [ ] Tests de las notas de `prompt_builder.rs` en español y de `POST /v1/context` con `lang: es`.
- [ ] `Lang` en `build_memory_prompt` y `build_session_prompt`; el daemon lo toma del pedido; `recent_context` manda `lang`.
- Commit: `feat(context): write the notes of the prompt in the requested language`

## 5. Los hooks (AL6)

- [ ] Tests de `session_start_text`, `save_reminder` y `language_line` en español; `user-prompt-submit` con preferencia.
- [ ] Los textos con `Lang`; `respond` lee la preferencia en los dos eventos y la manda al contexto.
- Commit: `feat(hooks): speak the preferred language`

## 6. El medidor (AL7)

- [ ] Test del parser con las respuestas vacías en español.
- [ ] `parse.rs` las acepta.
- Commit: `feat(eval): read an empty answer in either language`

## 7. Documentación

- [ ] `.claude/CLAUDE.md`, el mapa de `openspec/README.md`, la verificación con la medida de caracteres.
- Commit: `docs: describe what the agent reads in the preferred language`
