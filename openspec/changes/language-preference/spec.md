# language-preference — Especificación

Crea [`specs/preferences`](../../specs/preferences/spec.md) y suma requisitos a [`specs/storage`](../../specs/storage/spec.md), [`specs/portability`](../../specs/portability/spec.md), [`specs/retrieval`](../../specs/retrieval/spec.md), [`specs/mcp`](../../specs/mcp/spec.md), [`specs/hooks`](../../specs/hooks/spec.md), [`specs/setup`](../../specs/setup/spec.md), [`specs/doctor`](../../specs/doctor/spec.md) y [`specs/eval`](../../specs/eval/spec.md). Lo que no se nombra aquí sigue como está.

## Definiciones

- **Preferencia:** `<data>/preferences.json` de un perfil, leído con éxito y con `language` puesto. Un archivo ausente, inválido o sin `language` es **sin preferencia**.
- **Idioma preferido:** el `language` de la preferencia, `es` o `en`. El **otro idioma** es el que no es el preferido.
- **Idioma de una fila:** `memory.lang`: `en`, `es`, `und` (no se pudo decidir) o NULL (no se clasificó todavía).
- **Fila viva:** `deleted_at IS NULL` y `status IN ('active', 'disputed')`.
- **Alcance de una búsqueda:** las filas que la búsqueda puede devolver: las del proyecto pedido, las sin proyecto y toda `user_preference` (o todas, sin proyecto), que el lector alcanza según [`specs/sharing`](../../specs/sharing/spec.md).
- **Save:** una fila sin piezas, o todas las piezas de un save de `memory_piece`, que cuentan como uno.

## LP1. La preferencia y su archivo

`preferences.json` MUST tener la forma `{"version": 1, "language": "es"|"en"}`, con `language` opcional. Leerlo MUST rechazar una versión distinta de 1, un campo desconocido y un idioma que no sea `es` ni `en`. Un archivo ausente MUST dar «sin preferencia» sin error. Escribirlo MUST pasar por un archivo temporal y un `rename` dentro del directorio de datos, creado privado como el resto (`private_dir`). El archivo MUST NOT estar en `memory.db`, en un snapshot ni en un export.

#### Escenario: ida y vuelta
- WHEN se escribe la preferencia `es` y se vuelve a leer
- THEN da `es`, y el texto es `{"version": 1, "language": "es"}` con salto de línea final

#### Escenario: lo que se rechaza
- WHEN el archivo dice `"version": 2`, `"lenguaje": "es"` o `"language": "fr"`
- THEN la lectura falla con un error que nombra el archivo y la causa, y quien lo lea como idioma obtiene «sin preferencia»

**Test:** `a_missing_file_is_no_preference`, `a_preference_round_trips`, `an_unknown_version_field_or_language_is_refused`, `an_invalid_file_reads_as_no_preference` (`preferences.rs`, puros salvo el último); `saving_goes_through_a_temporary_file` (`preferences.rs`).

## LP2. Sin preferencia, lo de hoy

Sin preferencia, las instrucciones del servidor, la lista de herramientas (salvo la descripción de LP10), las respuestas de las herramientas, el contexto, lo que imprimen los hooks, el bloque que escribe `setup` y los hallazgos de `doctor` MUST ser idénticos a los de la versión anterior. Un archivo inválido MUST NOT hacer fallar ningún comando, herramienta ni hook.

#### Escenario: nada cambia
- GIVEN un perfil sin `preferences.json`
- THEN `get_info().instructions` es la apertura de `protocol.md`, `session_start_text` da lo mismo que antes, `setup` deja el bloque instalado `Unchanged` y `doctor` no tiene hallazgos `preferences` ni `protocol_block`

#### Escenario: un archivo roto
- GIVEN un `preferences.json` que no es JSON
- THEN el puente arranca y sus instrucciones son las de sin preferencia, `session-start` imprime lo de sin preferencia y `doctor` lo marca (LP14)

**Test:** `without_a_preference_the_instructions_are_the_opening`, `a_broken_preferences_file_leaves_the_bridge_as_without_one` (`server.rs`); `without_a_preference_session_start_says_what_it_said` (`hook.rs`); los tests actuales de `server.rs`, `hook.rs`, `setup.rs`, `prompt_builder.rs` y `the_agent_contract_matches_its_file` pasan sin tocar sus textos esperados.

## LP3. `clawg preferences`

`clawg preferences [--profile P]` MUST mostrar la preferencia o decir que no hay y que clawg usa el inglés. `clawg preferences set language es|en` MUST escribirla y `clawg preferences unset language` MUST quitarla, y las dos MUST decir que se aplica a las sesiones del cliente que se abran después y que `clawg setup claude-code` reescribe el bloque del protocolo. Un archivo inválido MUST hacer fallar `set` y `unset` con salida 1, nombrando la causa, sin tocarlo. Las frases del comando MUST salir en el idioma preferido después del cambio (en inglés sin preferencia). No MUST necesitar el daemon ni abrir el store.

#### Escenario: fijar y quitar
- WHEN se corre `clawg preferences set language es --profile dev`
- THEN `preferences.json` del perfil dev dice `es`, la salida está en español y nombra la sesión nueva y `clawg setup claude-code`
- WHEN se corre después `clawg preferences unset language --profile dev`
- THEN el archivo no tiene `language` y la salida está en inglés

**Test:** `preferences_parses_set_and_unset` (`cli.rs`, puro); `the_preferences_command_speaks_the_language_it_sets` (`preferences.rs` de `clawg-cli`, puro); `preferences_set_and_unset_write_the_profiles_file` (`tests/binary.rs`).

## LP4. El detector

`clawg_core::memory::language::detect` MUST ser una función pura de un texto que devuelve `en`, `es` o «indeciso». MUST ignorar los bloques y spans de código, las URL, las rutas y los identificadores (palabras con dígitos, `_`, `::`, `.` interno o mayúsculas internas), y decidir por palabras funcionales de cada idioma y por las marcas del español (`ñ`, `¿`, `¡`, vocales con tilde). Con poca evidencia MUST dar «indeciso».

#### Escenario: prosa y código
- WHEN el texto es un párrafo en español, uno en inglés, un bloque de código, un identificador suelto, una ruta o tres palabras
- THEN da `es`, `en`, indeciso, indeciso, indeciso e indeciso

#### Escenario: identificadores en prosa
- WHEN un párrafo en español nombra `content_hash`, `MemoryStore::open` y `crates/clawg-core/src/memory/db.rs`
- THEN da `es`

**Test:** `spanish_prose_is_spanish`, `english_prose_is_english`, `code_paths_and_identifiers_are_undecided`, `a_short_text_is_undecided`, `identifiers_in_spanish_prose_do_not_make_it_english`, `a_mixed_text_follows_its_prose` (`language.rs`, puros).

## LP5. `memory.lang`

V22 MUST agregar a `memory` la columna `lang` (NULL o `en`, `es`, `und`) y un índice parcial sobre las filas vivas, sin cambiar ninguna fila. Cada fila que inserten el save y la importación MUST llevar el idioma que `detect` da sobre su propio texto ya enmascarado (`und` si indeciso). El mantenimiento MUST clasificar las filas con `lang` NULL por tandas de a lo sumo 500, cada una en su transacción, sin cambiar `content`, `content_hash`, `last_modified_at`, `update_count` ni `memory_fts`, hasta que no quede ninguna. MUST NOT tocar `episodic_trajectory` ni `procedural_skill`.

#### Escenario: una base anterior
- GIVEN una base en V21 con filas en los dos idiomas, sin autor, con claves, borradas y reemplazadas
- WHEN la abre `MemoryStore::open`
- THEN escribe el snapshot `pre-migration`, cada fila sigue igual y alcanzable, y `lang` es NULL en todas

#### Escenario: el relleno
- WHEN corre el mantenimiento sobre esa base
- THEN cada fila tiene `lang`, y su `content`, `content_hash`, `last_modified_at`, `update_count` y el índice de texto son los de antes

#### Escenario: un save y sus piezas
- WHEN se guarda un texto en español de tres piezas
- THEN cada pieza tiene `lang = 'es'`

**Test:** `a_store_from_before_the_language_column_keeps_its_rows_and_gains_it` (`db.rs`); `published_migration_checksums_are_pinned` con V22 (`migrations.rs`); `a_save_records_the_language_of_each_row` (`store_insert.rs`); `the_maintenance_pass_classifies_rows_without_a_language`, `classifying_leaves_content_and_modification_alone` (`maintenance.rs`); `counting_by_language_uses_its_index` (plan de consulta sin `TEMP B-TREE` ni `SCAN memory`); un escenario de `clawg update` sobre una base V21 (`tests/binary.rs`).

## LP6. Export e import

El export MUST escribir `lang` en cada línea `memory` (`null` sin clasificar). El import MUST calcular el idioma de cada fila que inserta con `detect` sobre su texto enmascarado, sin leer el `lang` del archivo, y MUST aceptar un archivo sin ese campo. La versión del formato `clawg-memories` MUST seguir siendo 1.

**Test:** `an_export_carries_the_language_of_each_row` (`export.rs`); `an_import_computes_the_language_whatever_the_file_says`, `an_export_without_languages_imports` (`import.rs`).

## LP7. `other_language` en la búsqueda y el contexto

`POST /v1/search` y `POST /v1/context` MUST aceptar `lang` (`es` o `en`; otro valor es un 400 antes de buscar). Con `lang`, `POST /v1/search` y `POST /v1/context` con una consulta no vacía MUST responder `other_language: {"lang": <el otro idioma>, "memories": N}`, con N la cantidad de saves vivos del alcance de esa búsqueda cuyo idioma es el otro. Las filas `und` y NULL MUST NOT contar. Sin `lang`, o en un contexto sin consulta, el campo MUST NOT aparecer. El cliente MUST leer una respuesta sin el campo.

#### Escenario: lo que cuenta
- GIVEN en el proyecto A dos saves vivos en inglés, uno de tres piezas, uno borrado, uno reemplazado, uno en inglés de un autor que `sharing.json` oculta al lector, uno en inglés del proyecto B y una `user_preference` en inglés del proyecto B
- WHEN el lector busca en A con `lang: "es"`
- THEN `other_language` es `{"lang": "en", "memories": 3}`: los dos saves y la preferencia

#### Escenario: sin el campo
- WHEN la búsqueda no lleva `lang`, o el contexto no lleva consulta
- THEN la respuesta no tiene `other_language`

**Test:** `counting_the_other_language_follows_the_search_scope` (`language.rs` o `search.rs`); `a_search_with_a_language_counts_the_other_one`, `a_search_without_a_language_has_no_count`, `an_unknown_language_is_a_bad_request`, `a_context_with_a_query_counts_the_other_language` (`api.rs`); `a_search_answer_without_other_language_still_parses` (`client.rs`).

## LP8. La línea del otro idioma

Con preferencia, `search_memory` y `recall_context` con consulta MUST mandar `lang` y MUST agregar, al final de su respuesta, una línea que dice cuántas memorias del alcance están en el otro idioma y que, si lo mostrado no responde, se busque otra vez con la consulta en ese idioma, cuando `other_language.memories` es mayor que cero y la búsqueda mostró menos de tres memorias (ninguna incluida). En cualquier otro caso MUST NOT agregarla.

#### Escenario: una búsqueda pobre
- GIVEN la preferencia `es` y 12 memorias en inglés en el alcance
- WHEN `search_memory` muestra una memoria, o dice `no relevant memories for this query`
- THEN la respuesta termina con la línea, que nombra 12 y el inglés

#### Escenario: cuándo no sale
- WHEN la búsqueda muestra tres memorias o más, el conteo es cero, no hay preferencia, o el daemon no manda `other_language`
- THEN la respuesta es la de hoy

**Test:** `a_poor_search_names_the_memories_in_the_other_language`, `a_search_that_shows_enough_says_nothing_of_the_other_language`, `no_memories_in_the_other_language_means_no_line`, `without_a_preference_no_line`, `a_daemon_without_the_count_means_no_line` (`tools.rs`, puros); `recall_context_names_the_other_language_after_a_poor_context` (`tools.rs`); `search_memory_sends_the_bridges_language` (`server.rs`).

## LP9. Las instrucciones y el protocolo con preferencia

Con preferencia, el texto del protocolo MUST ser el de `protocol.md` con dos cambios: la frase de la apertura que pide guardar y buscar en inglés pide hacerlo en el idioma preferido y suma que se le responda a la persona en ese idioma salvo que escriba en otro; y la línea de «How to write a save» pide guardar y buscar en el idioma preferido y, cuando una búsqueda diga que quedan memorias en el otro idioma, buscar otra vez con la consulta en él. Las instrucciones del servidor MUST ser la apertura de ese texto y MUST caber en 2.048 caracteres con cada idioma. El protocolo MUST seguir nombrando sólo herramientas, argumentos y tipos que existen. El puente MUST leer la preferencia una vez, al arrancar.

#### Escenario: español
- GIVEN la preferencia `es`
- THEN las instrucciones dicen `in Spanish` donde hoy dicen `in English`, nombran cómo responderle a la persona, no dicen `in English` y miden 2.048 caracteres o menos

#### Escenario: la sesión no cambia sola
- GIVEN un puente arrancado con la preferencia `en`
- WHEN el archivo pasa a decir `es`
- THEN sus instrucciones y sus búsquedas siguen en `en`

**Test:** `the_instructions_follow_the_preferred_language`, `the_instructions_fit_older_clients` para cada idioma, `the_protocol_names_only_what_exists` para cada idioma, `the_language_rules_occur_once_in_the_protocol`, `the_bridge_keeps_the_language_it_started_with` (`server.rs`).

## LP10. La descripción de `search_memory`

La descripción de `search_memory` MUST NOT decir que encuentra memorias escritas en otro idioma, y MUST decir que una memoria en otro idioma que la consulta suele no encontrarse. Vale con preferencia y sin ella; `agent_contract.json` se vuelve a bendecir.

**Test:** `search_memory_does_not_promise_other_languages` (`server.rs`); `the_agent_contract_matches_its_file`.

## LP11. `memory_status`

Con preferencia al arrancar, `memory_status` MUST decir el idioma de la sesión. Cuando `preferences.json` dice otro idioma que el de la sesión (o una preferencia que la sesión no tenía, o ninguna) MUST decir el actual y que se aplica en una sesión nueva. Sin preferencia al arrancar ni ahora, MUST NOT agregar nada.

**Test:** `memory_status_names_the_sessions_language`, `memory_status_says_when_the_preference_changed`, `memory_status_without_a_preference_says_nothing_of_it` (`tools.rs` puros, `server.rs`).

## LP12. `session-start`

Con preferencia, `session-start` MUST imprimir una línea que dice el idioma de la persona, que se le responda en él salvo que escriba en otro, y que se guarde y se busque en él. MUST imprimirla aunque no haya daemon, falle o no responda a tiempo, y lo que el hook suma MUST seguir ocupando menos de 1.000 bytes. Cambia R3 de [`specs/hooks`](../../specs/hooks/spec.md): sin daemon, `session-start` imprime esa línea si hay preferencia, y nada si no. `user-prompt-submit` no cambia.

#### Escenario: sin daemon
- GIVEN la preferencia `es` y ningún daemon
- WHEN corre `clawg hook session-start`
- THEN imprime el sobre con la línea del idioma y sale con 0

**Test:** `session_start_names_the_users_language`, `what_the_hook_adds_stays_small` con la línea (`hook.rs`, puros); `session_start_without_a_daemon_still_names_the_language` (`hook.rs` o `tests/binary.rs`).

## LP13. `setup`

`clawg setup claude-code` MUST escribir entre los marcadores el protocolo de LP9 para la preferencia del perfil (el de hoy sin preferencia). Con un bloque instalado para otro idioma MUST reescribirlo, y `--dry-run` MUST decirlo sin escribir.

**Test:** `the_block_follows_the_preferred_language`, `another_language_rewrites_the_block` (`setup.rs`, puros sobre `plan`).

## LP14. `doctor`

Cuando `preferences.json` existe, `doctor` MUST sumar el chequeo `preferences`: `ok` legible, o `warn` `invalid` con la causa en `message`, sin `safe_next_step` y con el path en `evidence`. Con preferencia, MUST sumar el chequeo `protocol_block`: `ok` cuando el bloque del `CLAUDE.md` del usuario es el que `setup` escribiría, `warn` `stale` cuando es otro y `info` `not_installed` sin bloque, las dos con `safe_next_step` `clawg setup claude-code --profile P`, y `warn` `unreadable` cuando el archivo no se puede leer o tiene un marcador suelto. Sin el archivo MUST NOT sumar ninguno de los dos.

**Test:** `preferences_findings`, `protocol_block_findings` (`doctor/checks.rs`, puros); `doctor_names_a_broken_preferences_file` (`tests/binary.rs`).

## LP15. `rag-eval --lang-detect`

`rag-eval --lang-detect --golden FILE` MUST imprimir como JSON, sin cargar el modelo, la matriz de confusión de `detect` sobre las filas del corpus que traen `lang`: por idioma etiquetado, cuántas dio `en`, `es` e indeciso, en total y entre las de ocho palabras o más, y el porcentaje de aciertos, de cruces (el otro idioma) y de indecisos. No MUST imprimir ningún texto de fila.

**Test:** `lang_detect_counts_by_label`, `lang_detect_prints_no_text` (`rag-eval`, puros).
