# agent-language — Verificación

## Linux (2026-10-23)

En `target`, con el compilador de `rust-toolchain.toml`, sobre cli-language. ONNX Runtime 1.24.2 enlazado de forma dinámica, como en la verificación de cli-language.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy -p clawg-cli -- -A clippy::all -D warnings`: salen sin un aviso.
- `the_agent_contract_matches_its_file` pasa sin tocar `agent_contract.json`: sin preferencia nada de lo que ve el modelo cambió. `agent_contract.es.json` se creó con `CLAWG_BLESS=1` y se revisó: las instrucciones son la apertura de `protocol.es.md` y cada esquema es el inglés con otras `description` (`only_descriptions_change_with_the_language`).

## RED antes de cada cambio

- **Se vieron fallar antes del código:** `spanish_is_the_spanish_protocol` y `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one` (no compilaban sin `PROTOCOL_ES`); `the_spanish_agent_contract_matches_its_file` (sin el archivo); los seis tests de 78 que fijaban en inglés las frases de una sesión en español (la línea del otro idioma y la de `memory_status`), que se reescribieron con el texto en español; `session_start_names_the_users_language`, por la frase nueva.
- **Se escribieron antes que el código y no compilaban contra el anterior:** los de la tabla (`the_spanish_table_covers_every_tool_and_argument`, `the_spanish_descriptions_name_what_the_english_ones_name`), los de `tools.rs` en español, `the_notes_of_the_prompt_follow_its_language` (`prompt_builder.rs`), `the_save_reminder_speaks_the_preferred_language` y `an_empty_answer_reads_as_empty_in_both_languages`.
- **Se escribieron antes que el código, compilaban contra el anterior y no se corrieron contra él:** `a_spanish_session_answers_in_spanish` (`server.rs`), `a_context_in_spanish_keeps_its_markers` (`tests/api.rs`) y `session_start_speaks_the_preferred_language`.

## Caracteres por idioma (M3, sin el tokenizador)

Medidos sobre los dos contratos y los dos protocolos. El conteo en tokens necesita el tokenizador del modelo que los lee, que no está en el repositorio; los caracteres son la medida que sí se puede repetir.

| Texto | Inglés | Español | Cambio |
|---|---|---|---|
| Instrucciones (apertura) | 2.028 | 1.925 | −5,1 % |
| Protocolo entero (bloque de `CLAUDE.md`) | 8.610 | 9.279 | +7,8 % |
| Descripciones de las herramientas y los argumentos | 10.801 | 11.485 | +6,3 % |
| `tools/list` (JSON de las 12 herramientas) | 16.670 | 17.307 | +3,8 % |
| Línea del otro idioma (12 memorias) | 174 | 179 | +2,9 % |

La apertura en español es más corta porque las frases sobre el tipo y la importancia pasaron a su sección («Tipo e importancia»), que llega por el bloque de `CLAUDE.md`.

## Lo que no se midió

- **M1, la paridad español contra inglés**, y **M2, el store mezclado y el umbral de la segunda búsqueda** (`FEW_SHOWN`, 3, sigue sin calibrar): piden el golden pareado de `docs/eval/`, fuera de git, y el modelo real. Con ellos: `rag-eval --real` sobre el golden en inglés y su versión en español, `rag-eval --diff` apareando por caso, y `--agent-view` con las tres políticas de la exploración de 78 (P1, P2, P3).
- **La línea de `topic_key_warning` en español:** `revision-eval` sigue fijando la inglesa (`cost.rs`, `WARNING_LINE`); la española no se midió en tokens.
- **M6, a mano sobre el perfil `dev`, con autorización:** una sesión nueva de Claude Code con `es`, que el agente siga llamando a las herramientas con los mismos argumentos y tipos, y que `clawg setup claude-code` escriba el protocolo en español.
- Windows: lo corre el CI del pull request.
