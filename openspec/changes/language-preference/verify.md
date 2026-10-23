# language-preference — Verificación

## Windows (2026-10-22)

Sobre `d79fff1`, en `target-test` (limpiado con `cargo clean --workspace` antes de empezar), con el compilador de `rust-toolchain.toml`.

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: salen sin un aviso.
- Cada commit de código corrió los tests del crate que tocaba (`clawg-core --lib`, `clawg-cli --lib`, `--test api`, `--test binary` o `rag-eval`) y, cuando cambió una firma que usa otro crate, `cargo test --workspace --no-run`. La suite entera corrió una vez, al final, no antes de cada commit.
- `the_agent_contract_matches_its_file` se bendijo con `CLAWG_BLESS=1` y el diff se revisó: cambia sólo la descripción de `search_memory` (LP10). Las instrucciones sin preferencia quedan iguales.
- Las instrucciones miden 2.028 caracteres sin preferencia y 2.045 con `es` o con `en` (`the_instructions_fit_older_clients` corre sobre las tres).

## RED antes de cada cambio

- **Se vieron fallar antes del código:** los tests de `preferences.rs` y del detector en `language.rs` (no compilaban), los de la migración y el export (`MemoryLine.lang` no existía; después `published_migration_checksums_are_pinned` por la fila de V22 y `the_pinned_example_reads_and_writes_back_unchanged` por el campo nuevo), el de las notas escuetas del detector (`a_terse_note_with_evidence_of_one_language_alone_is_that_language`, que dio `None`) y `the_agent_contract_matches_its_file`.
- **Se escribieron antes que el código y no compilaban contra el anterior**, por una firma o un campo que no existía, sin correrlos aparte: `count_saves_in`, `SearchArgs.lang` y `Found.other_language` (cliente), `Command::Preferences`, `protocol::text`, `ClawgMcp::with_language`, `format_language` y la línea del otro idioma, `session_start_text` con idioma, `State.language`, `checks::preferences` y `ParseOutcome::LangDetect`.
- **Se escribieron antes que el código, compilaban contra el anterior y no se corrieron contra él:** los del relleno en `maintenance.rs`, los de `tests/api.rs` (`other_language`, `lang` inválido), `search_memory_sends_the_bridges_language`, `memory_status_names_the_language_of_the_session` y los tres de `tests/binary.rs`. Lo que los ata al cambio: piden una columna, un campo de la respuesta, un comando o una línea que la versión anterior no tiene.

## El binario real

Con `CLAWG_TEST_HOME` en un directorio temporal (`crates/clawg-cli/tests/binary.rs`):

- `preferences_set_and_unset_write_the_profiles_file`: `clawg preferences` sin archivo, `set language es` (salida en español, el archivo con `{"version": 1, "language": "es"}`), `unset language` (salida en inglés, `{"version": 1}`) y `set language fr` rechazado.
- `session_start_without_a_daemon_still_names_the_language`: con `es` y sin daemon, `clawg hook session-start` imprime el sobre con la línea del idioma y sale con 0; `user-prompt-submit` no imprime nada.
- `doctor_names_a_broken_preferences_file`: sin archivo no hay hallazgo `preferences`; con `"language": "fr"`, `warn` `invalid` y ningún `protocol_block`, y `set` no lo pisa; con `es` válido, `preferences` `ok` y `protocol_block` `info` `not_installed`.
- La migración por `clawg update`: los escenarios que ya existían sobre un store con migraciones pendientes (`an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data`, desde V15) migran ahora hasta V22, y `an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon` usa un candidato que conoce hasta V21. No se agregó un escenario propio desde V21.

## El detector (`rag-eval --lang-detect`)

Sin modelo, sobre los golden de `docs/eval/` (fuera de git):

| Golden | Filas | Aciertan | Al otro idioma | Indecisas |
|---|---|---|---|---|
| `golden-expanded.json` | 1.280 (657 en, 623 es) | 99,8 % | 0 | 0,2 % (3 en) |
| `golden.json` | 48 (26 en, 22 es) | 93,75 % | 0 | 6,25 % (3 en) |
| `golden-cross.json` | 48 | 93,75 % | 0 | 6,25 % (3 en) |

Todas las filas tienen ocho palabras o más, así que el corte largo da lo mismo. La barra de la propuesta (al menos 95 % de aciertos y a lo sumo 1 % al otro idioma sobre el golden expandido) se cumple.

La primera medida dio 98,4 % en el expandido y 83,3 % en `golden.json`: 8 de las 26 filas en inglés de este quedaban indecisas. Son notas técnicas escuetas, con 0 a 2 palabras funcionales y ninguna marca del español, y `MIN_EVIDENCE` pedía 3 puntos. Se agregó `TERSE_PROSE` (8): con evidencia de un idioma solo y ocho palabras de prosa o más, alcanza un punto (`cbd0311`). Las 3 que siguen indecisas no tienen ninguna palabra funcional de ningún idioma; una fila `und` no cuenta para ningún idioma, así que el conteo del otro idioma puede quedarse corto en notas así.

## Lo que no se midió

- Linux: lo corre el CI del pull request.
- M6, a mano sobre el perfil `dev`, que pide autorización y una sesión nueva de Claude Code: `clawg preferences set language es --profile dev`, comprobar que el agente responde y guarda en español, que una búsqueda pobre muestra la línea del otro idioma y una que trae lo suficiente no, que `memory_status` nombra el idioma y que `clawg setup claude-code --profile dev` reescribe el bloque.
- El relleno sobre un store real (unas 880 filas en inglés): corre en la primera pasada del mantenimiento, cinco minutos después de arrancar el daemon, y hasta entonces el conteo del otro idioma es cero.
- M1, M2 y M3 (paridad español contra inglés, el store mezclado con el umbral `FEW_SHOWN` y los tokens por idioma) son del cambio 80, como dice la propuesta. `FEW_SHOWN` (3) no está medido.
- La exploración pedía también pares entre idiomas en `conflict-eval` (M5); quedó en la exploración.
