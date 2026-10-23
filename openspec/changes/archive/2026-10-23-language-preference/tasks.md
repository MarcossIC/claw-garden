# language-preference — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La preferencia (LP1)

- [x] Tests de `preferences.rs`: ausente, ida y vuelta, versión, campo e idioma desconocidos, inválido como «sin preferencia», escritura por archivo temporal.
- [x] `clawg_core::preferences` (`Lang`, `Preferences`, `PreferencesError`, `load`, `save`, `language_in`) y `paths::preferences_file`.
- Commit: `feat(core): read and write the user's preferences file`

## 2. El detector (LP4)

- [x] Tests de `language.rs`: prosa en cada idioma, código, rutas, identificadores sueltos y en prosa, textos cortos y mezclas.
- [x] `clawg_core::memory::language::detect` y `column_value`.
- Commit: `feat(core): tell English from Spanish prose`

## 3. `memory.lang` al insertar (LP5, LP6)

- [x] Test `a_store_from_before_the_language_column_keeps_its_rows_and_gains_it` (`db.rs`), sembrando V21; tests del save, del export y del import.
- [x] `V22__memory_lang.sql`, el checksum fijado, los conteos de migraciones; `PreparedRow` y `place` escriben `lang`; `MemoryLine.lang` en el export.
- Commit: `feat(core): record the language of every memory row`

## 4. El relleno (LP5)

- [x] Tests de `maintenance.rs`: clasifica las filas en NULL, borradas incluidas, por tandas, sin tocar `content`, `content_hash`, `last_modified_at`, `update_count` ni `memory_fts`.
- [x] La tanda de `run_maintenance_once_at` sobre `idx_memory_lang_unset`.
- Commit: `feat(core): classify the language of rows saved before it was recorded`

## 5. El conteo y la API (LP7)

- [x] Tests de `language.rs` (el alcance del conteo y su plan de consulta), `api.rs` y `client.rs`.
- [x] `count_other_language`; `lang` en `SearchRequest` y `ContextRequest`; `other_language` en `SearchBody` y `ContextBody`; `SearchArgs.lang`, `context(.., lang)` y los campos del cliente.
- Commit: `feat(daemon): count the memories of a search's scope in the other language`

## 6. `clawg preferences` (LP3)

- [x] Tests de `cli.rs` y de las frases de `preferences.rs` (`clawg-cli`).
- [x] `Command::Preferences`, el comando y su salida en el idioma que fija.
- Commit: `feat(cli): set the language preference with clawg preferences`

## 7. Las instrucciones (LP2, LP9, LP10)

- [x] Tests de `protocol.rs` y `server.rs`: sin preferencia la apertura de hoy; con cada idioma, las frases y los 2.048 caracteres; cada frase de hoy una sola vez; sólo nombres que existen; el idioma fijo al arrancar; la descripción de `search_memory`.
- [x] `mcp/protocol.rs` (`PROTOCOL`, `opening`, `text`), usado por `server.rs` y `setup.rs`; `ClawgMcp::with_language` desde `mcp/mod.rs`; la descripción; el contrato bendecido y su diff revisado.
- Commit: `feat(mcp): tell the agent the preferred language of the user and of memories`

## 8. La línea del otro idioma y `memory_status` (LP8, LP11)

- [x] Tests puros de `tools.rs` (`other_language_line`, `format_language`) y de `server.rs`.
- [x] `format_search` y `format_recall` con la línea; `lang` en las búsquedas del puente; `memory_status` con el idioma.
- Commit: `feat(mcp): ask for a second search while memories remain in the other language`

## 9. `session-start` (LP12)

- [x] Tests de `hook.rs`: la línea con preferencia, sin daemon, con el contexto fallido y al vencer el plazo; menos de 1.000 bytes; sin preferencia lo de hoy.
- [x] `session_start_text` con el idioma; `answer` y `respond` con la línea de respaldo.
- Commit: `feat(hooks): name the user's language at session start, with or without a daemon`

## 10. `setup` (LP13)

- [x] Tests de `setup.rs` sobre `plan`: el bloque de la preferencia, otro idioma reescribe, `--dry-run` lo dice.
- [x] `State.language`, `read_state`, `block(eol, protocol)`.
- Commit: `feat(setup): write the protocol block for the preferred language`

## 11. `doctor` (LP14)

- [x] Tests puros de `doctor/checks.rs`: `preferences` y `protocol_block`.
- [x] Los dos chequeos y su lectura en `doctor/mod.rs`.
- Commit: `feat(doctor): check the preferences file and the installed protocol block`

## 12. `rag-eval --lang-detect` (LP15)

- [x] Tests de `rag-eval`: conteos por etiqueta, el corte de ocho palabras, ningún texto en la salida.
- [x] El modo `--lang-detect`.
- Commit: `feat(eval): report how the language detector labels the golden rows`

## 13. El binario real (LP3, LP5, LP12, LP14)

- [x] `preferences_set_and_unset_write_the_profiles_file`, `session_start_without_a_daemon_still_names_the_language`, `doctor_names_a_broken_preferences_file` y un `clawg update` sobre una base V21 (`tests/binary.rs`).
- Commit: `test(cli): the language preference through the real binary`

## 14. Docs

- [x] `.claude/CLAUDE.md`: `preferences.json`, `clawg preferences`, `memory.lang`, `lang` y `other_language` en la API, `lang` en el export, lo que hace `session-start` sin daemon.
- [x] `openspec/README.md`: las filas de 79 (B) y 80 (C) y el orden de ejecución, con los artefactos de planificación. La fila de 78 se reescribe al archivar.
- Commit: `docs: describe the language preference`

## Verificación

- [x] La suite y clippy en Linux y en Windows (`target-test`).
- [x] `rag-eval --lang-detect --golden docs/eval/golden-expanded.json`: la barra de la propuesta.
- [ ] Con autorización, el recorrido manual sobre el perfil `dev` (M6): no se corrió.
