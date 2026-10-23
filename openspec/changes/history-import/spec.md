# history-import — Especificación

Suma requisitos a [`specs/portability`](../../specs/portability/spec.md) (HI2 y HI4 a HI17), [`specs/storage`](../../specs/storage/spec.md) (HI1) y [`specs/project`](../../specs/project/spec.md) (HI3). Lo que no se nombra aquí sigue como está; en particular R1 a R7 de `specs/portability` siguen valiendo para un archivo `clawg-memories`.

## Definiciones

- **Fuente:** la herramienta de la que viene un export que no es `clawg-memories`. Este cambio lee una: `engram`.
- **Nota:** lo que la fuente guardó y clawg guarda como un save: una observación, un prompt o el resumen de una sesión.
- **Origen de una nota:** la fuente y el id estable de la nota en ella, escrito en `created_by_ref` como `engram:<sync_id>`; sin `sync_id`, `engram:observation:<id>` o `engram:prompt:<id>`; el resumen de una sesión, `engram:session:<id>`.
- **Grupo de origen:** el proyecto y el scope que la nota tenía en la fuente (`personal` o no), y «sin proyecto» como un grupo más.
- **Fecha del texto:** `updated_at` de la nota, o `created_at` sin él; en UTC.
- **Destino de una nota:** el id de proyecto que un registro le daría a la raíz que el mapa le asigna a su proyecto de la fuente (R15 y R16 de [`specs/project`](../../specs/project/spec.md), como a una raíz de `--map-root`), o «sin proyecto». Dos clones de un remoto son un mismo destino.
- **Fila viva:** `deleted_at IS NULL` y `status IN ('active', 'disputed')`.
- **Fila protegida:** `memory_type` `user_preference` o `architectural_decision` ([`specs/disputes`](../../specs/disputes/spec.md)).

## HI1. V23: el proyecto de origen

V23 MUST agregar a `memory` la columna nullable `origin_project`, un índice parcial sobre ella para las filas que la tienen, y un índice parcial sobre `created_by_ref` para las filas con `source = 'import'`. Migrar MUST NOT cambiar ninguna fila: toda fila anterior queda con `origin_project` NULL. Su checksum MUST quedar fijado con los demás.

#### Escenario: un store anterior
- GIVEN un store en V22 con filas de dos agentes, con y sin proyecto
- WHEN se abre con este clawg
- THEN cada fila tiene las mismas columnas que antes, `origin_project` es NULL y los dos índices existen

**Test:** `a_database_from_the_previous_version_opens_with_no_origin_project` (`crates/clawg-core/src/memory/db.rs`); `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`).

## HI2. El origen en el formato `clawg-memories`

El export MUST escribir `origin_project` en cada línea `memory` (`null` cuando la fila no lo tiene, o el store es anterior a V23). El import de un archivo `clawg-memories` MUST guardar el `origin_project` de cada fila que inserta y MUST aceptar un archivo sin el campo. La versión del formato MUST seguir siendo 1.

#### Escenario: ida y vuelta
- GIVEN una fila importada de otra herramienta, con su proyecto de origen
- WHEN se exporta y se importa en otro store
- THEN la fila nueva tiene el mismo `origin_project`, `source` y `created_by_ref`

**Test:** `an_export_carries_the_origin_project` (`export.rs`); `an_import_keeps_the_origin_project` y `an_export_without_an_origin_project_imports` (`import.rs`).

## HI3. Elegir filas por su proyecto de origen

`clawg projects move` MUST aceptar `--origin-project NOMBRE`, que elige las filas cuyo `origin_project` es exactamente `NOMBRE`, y cuenta como filtro de selección (R12 de [`specs/project`](../../specs/project/spec.md)). `POST /v1/memories/move` MUST aceptar `origin_project` con el mismo efecto. `clawg memories list --origin-project NOMBRE` y `GET /v1/memories?origin_project=NOMBRE` MUST listar sólo esas filas, con los demás filtros.

#### Escenario: acomodar un proyecto de la fuente
- GIVEN notas importadas sin proyecto de dos proyectos de origen, `a` y `b`
- WHEN se corre `clawg projects move --origin-project a --to RUTA`
- THEN se mueven las de `a` con sus unidades y las de `b` quedan sin proyecto

**Tests:** `a_batch_move_selects_by_origin_project` (`crates/clawg-core/src/memory/relocate.rs`); `a_listing_filters_by_origin_project` (`crates/clawg-core/src/memory/admin.rs`); `move_and_list_take_an_origin_project` (`crates/clawg-cli/src/cli.rs`); `move_and_list_select_by_origin_project` (`crates/clawg-cli/tests/api.rs`).

## HI4. El adaptador de engram

`engram::read` MUST ser una función pura que lee el JSON de un export de engram y devuelve sus notas, sus relaciones y lo que no se traduce. MUST aceptar `"version"` `0.1.0` y `0.2.0`, y MUST rechazar sin devolver nada un texto que no es JSON, que no es un objeto, sin `version` o con otra. Una lista de nivel superior ausente o `null` MUST leerse como vacía. Las fechas MUST leerse en UTC, como `YYYY-MM-DD HH:MM:SS` con o sin fracción de segundo (hasta 9 dígitos, truncada a milisegundos) o RFC 3339.

De cada observación MUST tomar id, `sync_id`, sesión, tipo, título, contenido, proyecto (el de su sesión si la nota no tiene), scope, `topic_key`, `revision_count`, `duplicate_count`, `last_seen_at`, `created_at`, `updated_at`, `deleted_at` y, en `0.2.0`, `pinned`; de cada prompt, id, `sync_id`, sesión, contenido, proyecto y fecha; de cada sesión con `summary`, el resumen como una nota más. MUST contar por su nombre lo que no tiene lugar: `tool_name`, `review_after`, `superseded_by_sync_id`, `prompt_tombstones` y todo campo presente, no nulo ni vacío, de una observación, un prompt, una sesión o una relación que el adaptador no lee, también uno que no conoce. MUST contar las notas con `revision_count` mayor que 1, que la fuente reescribió y cuyas versiones anteriores el export no trae.

#### Escenario: versión desconocida
- GIVEN un export con `"version":"0.3.0"`
- WHEN se lee
- THEN devuelve un error que nombra la versión y las que lee

#### Escenario: un export sin notas
- GIVEN un export `0.2.0` con `"sessions": null`, `"observations": []` y `"prompts": null`
- WHEN se lee
- THEN no hay notas ni error

#### Escenario: un campo que el adaptador no conoce
- GIVEN una observación con un campo que el adaptador no lee
- WHEN se lee
- THEN la nota se lee igual y el campo se cuenta por su nombre

**Tests:** `an_unknown_or_missing_version_is_refused`, `a_0_1_0_export_reads_its_notes_prompts_and_session_summaries`, `a_0_2_0_export_reads_deletions_pins_and_relations`, `null_lists_read_as_empty`, `fields_without_a_place_are_counted_by_name`, `notes_rewritten_at_the_source_are_counted`, `a_note_without_a_project_takes_its_sessions`, `dates_read_as_utc` (`crates/clawg-core/src/memory/portability/engram.rs`).

## HI5. Tipos y texto

Una nota MUST guardarse con el tipo que le toca: `decision` y `architecture` → `architectural_decision`; `preference` → `user_preference`; `session_summary` → `session_summary`; cualquier otro tipo de observación, también uno desconocido, → `semantic_fact`; un prompt → `chat_message`; el resumen de una sesión → `session_summary`. Su texto MUST ser el título, un salto de línea y el contenido; el título solo si no hay contenido; el contenido solo si no hay título. Una nota sin título ni contenido MUST NOT escribir nada y MUST reportarse.

#### Escenario: sin contenido
- GIVEN una `session_summary` con título y contenido vacío
- WHEN se importa
- THEN la fila tiene el título como texto y tipo `session_summary`

**Tests:** `each_source_type_lands_on_its_memory_type`, `the_title_opens_the_text`, `a_note_without_text_writes_nothing_and_is_reported` (`crates/clawg-core/src/memory/portability/translate.rs`).

## HI6. Enmascarada entera y partida como un save

El texto de una nota MUST enmascararse entero (`redact`) antes de partirlo, y MUST partirse con `split_save` del chunker del daemon: las filas MUST ser las piezas de un save del mismo texto, en el mismo orden, sin repetir una pieza igual, y cada pieza después de la primera MUST embeberse detrás de la misma cabecera y guardarla en `memory_piece.context`. Una pieza MUST NOT volver a enmascararse sola.

#### Escenario: una nota larga
- GIVEN una nota de 5.000 caracteres
- WHEN se importa, y el mismo texto se guarda con `save_memory` en otro store
- THEN los dos stores tienen las mismas filas, en el mismo orden, con la misma cabecera

#### Escenario: un secreto en el corte
- GIVEN una nota larga con una clave de API justo donde se cortan dos piezas
- WHEN se importa
- THEN ninguna fila tiene un fragmento de la clave, y la nota cuenta como enmascarada

**Tests:** `a_long_note_splits_into_the_pieces_of_a_save`, `a_secret_across_a_piece_cut_is_masked_whole` (`translate.rs`); `an_imported_note_has_the_rows_of_a_save_of_its_text` (`crates/clawg-core/src/memory/portability/import.rs`).

## HI7. Autor, proyecto y origen

Cada fila importada de una fuente MUST tener `scope = 'agent'`, `agent_id` `migrated` (o el de `--agent`), `source = 'import'`, `created_by_type` `user` para un prompt y `agent` para lo demás, su origen en `created_by_ref`, el nombre de su proyecto en la fuente en `origin_project` (NULL sin proyecto), y la sesión de la fuente en `source_session_id` cuando es de 1 a 128 caracteres ASCII visibles (otra se reporta). Su `project_id` MUST ser su destino: el de su nombre en el mapa; sin proyecto si el nombre no está en el mapa, si la nota no tiene proyecto o si es `personal`. Los ids de las raíces del mapa MUST planearse antes de traducir (`plan_ids_in`), porque las claves se distinguen por destino (HI8). Cada raíz del mapa MUST registrarse en `project` sin `index_docs`, y su remoto en `project_remote`, como las de `--map-root`. Si entre el plan y la escritura un registro le dio ese id a otra raíz, las notas de ese nombre MUST reportarse por su proyecto y no importarse.

#### Escenario: el mapa
- GIVEN notas de `a`, de `b` y sin proyecto, y una `personal` de `a`
- WHEN se importa con `--project-map a=RUTA`
- THEN las de `a` quedan en el proyecto de `RUTA`, las demás sin proyecto, y `origin_project` es `a`, `b`, NULL y `a`

#### Escenario: el id de la raíz ya es de otra
- GIVEN `project` con el id que `derive_project_id` da a `RUTA` para otra raíz
- WHEN se importa con `--project-map a=RUTA`
- THEN las notas de `a` quedan en el siguiente id libre de `RUTA`, y el reporte nombra ese proyecto

**Tests:** `the_project_map_places_notes_and_the_rest_have_none`, `every_row_carries_its_origin_and_author` (`translate.rs`); `a_mapped_root_whose_id_is_taken_gets_its_own`, `a_project_taken_while_importing_is_told_by_its_notes` (`import.rs`).

## HI8. Claves de tema

Una `topic_key` válida MUST conservarse. Una inválida MUST reescribirse en minúsculas, con cada carácter fuera de `[a-z0-9._-]` y `/` como `-` y sin segmentos vacíos; si eso la cambió o pasa de 80 caracteres, MUST terminar con `-` y los primeros 10 hex del SHA-256 de la original, cortando lo anterior para caber en 80. La misma original MUST dar siempre la misma clave.

Si notas de más de un grupo de origen caen en un mismo destino con la misma clave, cada nota de ese grupo de claves MUST calificarse: su clave pasa a `<grupo>/<clave>`, reescrita con la misma regla si no entra. `<grupo>` es el nombre del proyecto de la fuente (`no-project` sin él) seguido de `.personal` para una `personal`; un nombre que no es un segmento válido, que es `no-project` o que termina en `.personal` MUST reescribirse como un segmento seguido de `-` y el hash de la original, así dos grupos nunca dan el mismo. Una clave calificada que cae sobre la de una nota de otro grupo en el mismo destino MUST calificarse otra vez, hasta que ningún destino tenga una clave de dos grupos. Dos notas vivas de un mismo grupo de origen y destino con una clave MUST dejar viva la de fecha más nueva y la otra `superseded` por ella con causa `revision`. Una clave que en el destino tiene una fila viva que no vino de este import MUST seguir R5 de `specs/portability`: la nota entra `superseded` por esa fila.

#### Escenario: dos proyectos de la fuente, un destino
- GIVEN una nota de `a` y una de `b` con la clave `sdd/x/proposal`, las dos mapeadas a la misma raíz
- WHEN se importa
- THEN quedan vivas las dos, con `a/sdd/x/proposal` y `b/sdd/x/proposal`

#### Escenario: dos clones de un remoto
- GIVEN una nota de `a` y una de `b` con la clave `sdd/x`, mapeadas a dos clones de un remoto que ya tiene proyecto
- WHEN se importa
- THEN quedan vivas las dos en el proyecto del remoto, con `a/sdd/x` y `b/sdd/x`

**Tests:** `a_valid_key_is_kept_and_an_invalid_one_rewritten_stably`, `keys_of_two_origins_in_one_destination_are_qualified`, `a_qualified_key_that_takes_another_projects_key_is_qualified_again`, `keys_of_two_roots_of_one_project_here_are_qualified`, `two_notes_of_one_origin_and_key_leave_the_newest_live` (`translate.rs`); `a_key_held_here_supersedes_the_imported_note`, `notes_mapped_to_clones_of_one_remote_share_its_project` (`import.rs`).

## HI9. Fechas, accesos, importancia y tier

`created_at` y `last_modified_at` MUST ser la fecha del texto; una fecha posterior al momento del import, o que no se puede leer, MUST pasar a ser ese momento y reportarse. `access_count` MUST ser `duplicate_count` (al menos 1), y `last_accessed_at` la más nueva entre la fecha del texto y la última vez que la fuente vio la nota (`last_seen_at`, acotada igual): escribir una nota también es verla, y una fila nunca se lee antes de escribirse. La importancia MUST ser 5, o 9 para una nota fijada (`pinned`). El tier MUST ser `compute_tier(3, importancia, accesos, tipo)`.

#### Escenario: una nota vista tres veces
- GIVEN una `decision` con `duplicate_count` 3
- WHEN se importa
- THEN tiene `access_count` 3 y tier 1, y su `created_at` es su `updated_at`

**Tests:** `dates_accesses_importance_and_tier_follow_the_note`, `a_date_after_the_import_is_clamped_and_reported` (`translate.rs`).

## HI10. Notas de igual texto

Notas vivas de un mismo grupo de origen cuyo texto enmascarado es igual MUST guardarse una vez: en la de fecha más vieja (y, a igual fecha, el origen menor), con `access_count` la suma de los de todas, `last_accessed_at` el más nuevo e importancia 9 si alguna estaba fijada. El reporte MUST nombrar cada nota absorbida con la que la absorbió, y una relación a una absorbida MUST ir a la que la absorbió.

**Test:** `notes_of_one_origin_with_the_same_text_store_once_with_every_access` (`translate.rs`).

## HI11. Un segundo import la reconoce por su origen

Una nota cuyo origen tiene en el store una fila con su misma fecha del texto (`created_at`), o cuya fecha se acotó y su origen tiene alguna fila, MUST contarse como presente: no se inserta ni se le suman accesos, sus vínculos van a esas filas, y el reporte la cuenta por el proyecto donde está, aunque el mapa sea otro o se haya movido con `projects move`. Una nota cuyo origen sólo tiene filas borradas MUST contarse como presente: el import nunca trae de vuelta lo que el usuario borró.

#### Escenario: otro mapa, después de mover
- GIVEN un export importado sin mapa, y sus notas de `a` movidas con `projects move --origin-project a`
- WHEN se importa de nuevo con `--project-map a=OTRA`
- THEN no hay filas nuevas ni accesos nuevos, y el reporte cuenta las de `a` en el proyecto adonde se movieron

**Tests:** `a_second_import_adds_nothing_even_with_another_map`, `a_moved_note_is_recognized_where_it_is`, `a_note_deleted_here_is_not_brought_back` (`import.rs`).

## HI12. Una nota que la fuente reescribió

Una nota cuyo origen tiene filas no borradas, ninguna con su fecha del texto, MUST entrar como una revisión de la versión guardada más nueva: en el proyecto donde está, con la clave que tiene (la suya si no tiene). Si su fecha es más nueva, MUST entrar viva y dejar las filas vivas guardadas de su origen `superseded` por su primera fila, con causa `revision`, en la misma transacción, salvo las que comparte por ser del mismo texto. Si es más vieja, MUST entrar `superseded` por la primera fila viva guardada de su origen, con causa `revision`, o, sin filas vivas, con las reglas de siempre en ese proyecto. Una nota borrada en la fuente cuyo origen tiene filas MUST NOT cambiar nada y MUST reportarse.

#### Escenario: reescrita
- GIVEN una nota importada, y el mismo origen con otro texto y una fecha posterior
- WHEN se importa el export nuevo
- THEN la nota nueva está viva en el proyecto de la vieja y con su clave, y la vieja queda `superseded` por ella

**Tests:** `a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is`, `an_older_version_enters_superseded`, `a_note_deleted_at_the_source_changes_nothing` (`import.rs`).

## HI13. Escritura por nota

Las filas de una nota, y lo que su revisión reemplaza, MUST escribirse en una sola transacción: las transacciones del import MUST cerrarse sólo entre dos notas. Los vínculos de piezas de una nota MUST escribirse salvo que una fila de su origen ya tenga uno; una fila fundida con otra nota MAY ser pieza de las dos, como la comparten dos saves.

**Tests:** `write_batches_never_split_a_note` (`import.rs`, puro); `links_lost_between_transactions_are_filled_by_importing_again` (`import.rs`, con una nota de otra fuente).

## HI14. Borradas y relaciones de `0.2.0`

Una nota con `deleted_at` MUST entrar borrada en esa fecha, sin vector y fuera del índice de texto. Cada relación vigente (no reemplazada por otra) entre dos notas importadas MUST entrar como un par entre sus primeras filas, `new_id` la de id mayor:

| Relación de la fuente | Par |
|---|---|
| `pending`, o `judgment_status` `pending` | `pending` |
| `related`, `compatible`, `not_conflict`, o `judgment_status` `ignored` | juzgado `compatible` |
| `scoped` | juzgado `scoped`, con su motivo |
| `conflicts_with` | `escalated`, nombrado en `disputed_against` de la fila nueva, que queda `disputed` |
| `supersedes` | juzgado `supersedes`: las filas vivas de la nota destino `superseded` por la primera fila de la nota fuente, con causa `verdict` |
| `supersedes` sobre una nota protegida, sin `marked_by_kind` `human` | `escalated`, como `conflicts_with` |

El motivo MUST guardarse enmascarado, con la evidencia debajo cuando la hay. Una relación `orphaned`, una con un extremo que no está en el archivo ni en el store, y una reemplazada por otra MUST contarse y no guardarse.

#### Escenario: un veredicto protegido
- GIVEN una `decision` A y una nota B con la relación `B supersedes A` marcada por un agente
- WHEN se importa
- THEN A sigue viva, el par está `escalated` y B está `disputed`

**Tests:** `a_deleted_note_comes_in_deleted_without_a_vector`, `relations_become_pairs`, `supersedes_over_a_decision_without_the_user_is_escalated`, `what_has_no_place_is_counted` (`import.rs`); `relations_map_to_pair_states` (`translate.rs`).

## HI15. Prompts y resúmenes de sesión

Un prompt con texto MUST guardarse como `chat_message` del usuario, y el `summary` de una sesión como una `session_summary` con la fecha de fin de la sesión (la de inicio sin ella), su proyecto y su origen. Un prompt sin texto MUST contarse.

**Test:** `prompts_and_session_summaries_land_as_their_types` (`translate.rs`).

## HI16. La ruta y el comando

`POST /v1/import` MUST aceptar `from` (`clawg` por defecto, `engram`) y `project_map` (pares de nombre y raíz). `from` desconocido, `project_map` con `from` `clawg`, o `map_roots` con otra fuente MUST ser un `400` sin escribir. Con otra fuente, el daemon MUST leer el archivo con su adaptador antes de pedir el embedder, así un archivo que no lee es un `400` también mientras el modelo carga, y MUST responder `503` `tokenizer_not_ready` sin el tokenizer, también en un `--dry-run`. Un export de engram mandado sin `from` MUST ser el `400` de un archivo sin encabezado.

`clawg import [--from clawg|engram] FILE [--dry-run] [--agent A] [--map-root OLD=NEW]... [--project-map NOMBRE=RUTA]...` MUST mandar el archivo a un daemon en marcha y nunca iniciar uno. `--map-root` MUST valer sólo con `--from clawg` y `--project-map` sólo con otra fuente. El CLI MUST resolver cada `RUTA` como un puente y rechazar, antes de leer el archivo, una que no es un directorio. `--dry-run` MUST responder el reporte que daría el import, sin escribir, sin embeber y sin snapshot.

#### Escenario: un ensayo y el import
- GIVEN un export de engram con notas largas, claves, prompts y un proyecto mapeado
- WHEN se corre con `--dry-run` y después sin él
- THEN los dos reportes coinciden salvo `dry_run` y el snapshot, y el segundo dejó las filas

#### Escenario: un export sin notas
- GIVEN un export de engram sin notas, con `sessions` y `prompts` en `null`
- WHEN se importa
- THEN responde el reporte sin error y no inserta ninguna fila

**Tests:** `an_engram_import_stores_notes_and_a_dry_run_reports_the_same`, `an_engram_export_without_notes_imports_nothing`, `import_refuses_an_unknown_source_or_a_mixed_map`, `an_engram_export_without_from_is_a_400`, `a_foreign_import_without_the_tokenizer_is_a_503` (`crates/clawg-cli/tests/api.rs`); `import_takes_a_source_and_project_maps` (`crates/clawg-cli/src/cli.rs`); `a_project_map_must_name_a_directory` (`crates/clawg-cli/src/import.rs`); `an_engram_export_imports_through_the_binary` (`crates/clawg-cli/tests/binary.rs`).

## HI17. El reporte

El reporte MUST traer, además de lo de R7 de `specs/portability`, las filas insertadas por idioma (`by_lang`, para los dos formatos) y, para otra fuente, en `foreign`: la fuente y su versión; notas por tipo de la fuente con su tipo de destino; notas por proyecto de la fuente con su destino; los nombres sin mapa y las entradas del mapa sin notas; cuántas filas y cuántas notas en más de una pieza; cuántas notas se enmascararon; las claves reescritas y las calificadas, de qué a qué; las notas reemplazadas por otra de su origen; las absorbidas por igual texto; las presentes por proyecto donde están; las revisadas y las versiones viejas; las fechas acotadas; las borradas; las relaciones guardadas por estado y las contadas por motivo; lo que no tiene lugar, por campo, también los que el adaptador no conoce; las notas que la fuente reescribió; las sesiones descartadas; y las notas sin texto. Lo que el reporte de R7 dice por línea del archivo (las filas fundidas con una de acá, las de clave tomada y las que no entraron) MUST ir, para otra fuente, en `foreign` por nota, nombrada por su origen, y las notas de un proyecto cuya raíz pertenece a otro, por ese proyecto: las líneas y los ids de la traducción no significan nada fuera de ella. `clawg import` MUST imprimirlo en el idioma del comando (CL3).

**Tests:** `the_foreign_report_counts_what_the_import_did` (`import.rs`); `render_names_what_a_foreign_import_did` y `the_foreign_report_speaks_spanish` (`crates/clawg-cli/src/import.rs`).

## HI18. El agente lee y cambia lo importado

Desde el agente del puente, una nota importada larga MUST leerse entera con `read_memory`, y `update_memory` y `delete_memory` MUST cambiarla, como cualquier fila que alcanza ([`specs/visibility`](../../specs/visibility/spec.md)).

**Test:** `an_imported_long_note_reads_whole_and_changes_for_the_bridge_agent` (`crates/clawg-cli/tests/api.rs`).
