# agent-language — Verificación

## Linux (2026-10-23)

En `target`, con el compilador de `rust-toolchain.toml`, sobre cli-language. ONNX Runtime 1.24.2 enlazado de forma dinámica, como en la verificación de cli-language.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy -p clawg-cli -- -A clippy::all -D warnings`: salen sin un aviso.
- `the_agent_contract_matches_its_file` pasa sin tocar `agent_contract.json`: sin preferencia nada de lo que ve el modelo cambió. `agent_contract.es.json` se creó con `CLAWG_BLESS=1` y se revisó: las instrucciones son la apertura de `protocol.es.md` y cada esquema es el inglés con otras `description` (`only_descriptions_change_with_the_language`).

## Windows y Linux en el CI (2026-10-23)

El CI del pull request que trajo el cambio junto con cli-language (#82) corrió sobre `1116ac7`, el merge de `develop` en la rama, que contiene `a38075a`, el último commit con código, sin otra diferencia bajo `crates/`: el job `test` pasó en `windows-latest` y en `ubuntu-latest`. Ese job corre `cargo clippy --locked --workspace --all-targets -- -A clippy::all -D warnings`, `cargo test --locked --workspace` (con `tests/api.rs` y `tests/binary.rs`, así que `a_context_in_spanish_keeps_its_markers` y los contratos en los dos idiomas corrieron también en Windows), `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y, en Linux, `cargo test --locked -p clawg-core --lib build_info`.

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
- **`user-prompt-submit` con preferencia contra un daemon:** el recordatorio en español lo fija `save_reminder`, pura; el camino de `respond` con el idioma sólo se prueba sin daemon, donde no imprime nada.

## Al archivar (2026-10-23)

### Las aperturas, medidas otra vez

Sobre `protocol.md`, `protocol.es.md` y las dos reglas de `protocol.rs` en `25a6961`, con la misma unión de líneas que `opening`: sin preferencia 2.028 caracteres, con `en` 2.045 (3 de margen hasta 2.048) y con `es` 1.925. Las dos tienen las mismas nueve secciones `##` en el mismo orden.

### Diferencias entre la spec del cambio y el código

- **AL2** pide las mismas secciones «en el mismo orden»: `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one` compara cuántas son, y el orden se comprobó a mano. Compara también los nombres y marcadores entre comillas (los textos sin espacio, o con `<`), no los ejemplos de un save, que se escriben en cada idioma. La spec consolidada lo dice así.
- **AL4**: el escenario del hit no tenía el `piece=2/3` que pide `a_spanish_search_keeps_its_markers`; la spec consolidada trae la línea del test.
- **LP9** (de language-preference) seguía diciendo que con `es` las instrucciones dicen `in Spanish`: desde AL2 son la apertura de `protocol.es.md`. Se reescribió con un escenario por idioma y los tests de `protocol.rs`.
- **La NOTE de los conflictos en español** empieza con `NOTA:` y mide 245 bytes, por encima de los 228 que `specs/retrieval` le pone a la inglesa (227). Ningún test le pone tope; la spec dice que el tope es de la inglesa.
- **`specs/mcp`, «El protocolo de memoria»**, decía que sólo la apertura llega al agente; desde client-setup el texto entero llega al `CLAUDE.md` por `setup`. Se corrigió al reescribir el párrafo.

### Las citas `archivo:línea`

Se revisaron las citas a los 18 archivos que el cambio modificó (`server.rs`, `tools.rs`, `protocol.rs`, `mod.rs`, `hook.rs`, `setup.rs`, `client.rs`, `daemon/api.rs`, `prompt_builder.rs`, `worker.rs`, `relocate.rs`, `tests/api.rs`, `tests/binary.rs`, `parse.rs`, `runner.rs`, `cost.rs`, `run.rs` y `.claude/CLAUDE.md`) en todas las specs, en `openspec/README.md` y en los documentos de los cambios abiertos salvo history-import, que se está aplicando en la misma rama. Se compararon con `25a6961`, que suma a `develop` los dos commits de código de history-import.

La base de cada cita es el commit que escribió esa línea del documento. Se conservó la cita cuando el texto citado no se movió, se mapeó por los bloques que no cambiaron cuando la línea base tenía el identificador que la precede, y 124 se resolvieron leyendo el código. Cambiaron 574 citas en 32 documentos, sólo en sus números y rutas.

- **Mal desde antes:** las continuaciones de `specs/sharing` R4 (`:626`, `:29`, `:537-543`, `:35`) heredaban `tools.rs` por la gramática de la línea, pero nombran constantes de `prompt_builder.rs`; ahora lo dicen explícitamente. `specs/hooks` R3 y R7 citaban `respond` y `answer` con rangos distintos; ahora R3 cubre los dos y R7 sólo `respond`.
- **Apuntan al sucesor:** el texto de esos documentos sigue nombrando un identificador que ya no existe: `OwnerQuery`, hoy `ReaderQuery` (el backlog del README, «`sharing.json` no es una frontera de acceso»); `INSTRUCTIONS`, hoy `PROTOCOL` en `protocol.rs` (docs-search-quality); `try_refresh`, hoy `begin_if_due` y `spawn_doc_refresh` (pipeline-metrics, read-connections).
- **Sin tocar:** el ítem del backlog del README «Un save largo compartido no se puede leer entero» cita `an_agent_cannot_read_another_agents_save` (`server.rs:3305`), que ya no existe: agent-authorship hizo que cualquier agente lea entero lo que `sharing.json` le deja alcanzar, y el test es hoy `another_agent_reads_a_save_unless_hidden`. El ítem parece resuelto y queda para quien ordene el backlog. `:412` en read-connections es de un crate externo.
