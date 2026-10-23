# portability — Especificación

Fuente de verdad de cómo las memorias salen de un store y entran en otro, fila por fila: el formato `clawg-memories`, `clawg export`, `clawg import` y `POST /v1/import`. Los snapshots de archivo entero (`backup` y `restore`) están en [`specs/restore`](../restore/spec.md); el enmascarado que el import aplica, en [`specs/secrets`](../secrets/spec.md).

**Origen:** [memory-portability](../../changes/archive/2026-10-20-memory-portability/proposal.md), archivado el 2026-10-20. Los ids de requisito son los de esa spec. [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22, llevó a proyecto el texto vivo y el titular de la clave de R5. [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23, sumó el idioma de cada fila (LP6).

## Dónde está

- El formato: `FORMAT` y `VERSION` (`crates/clawg-core/src/memory/portability/format.rs:14-16`), las líneas (`format.rs:19-140`), `parse` (`format.rs:176`), las comprobaciones de una fila y de un par (`check_memory`, `format.rs:229`; `check_conflict`, `format.rs:262`) y `render` (`format.rs:278`).
- El export: `export` y `read` (`crates/clawg-core/src/memory/portability/export.rs:68` y `export.rs:80`); la vista, `markdown::render` (`crates/clawg-core/src/memory/portability/markdown.rs:12`); el comando, `run` y `write_new` (`crates/clawg-cli/src/export.rs:16` y `export.rs:31`).
- El import: `MAX_IMPORT_BYTES` (`crates/clawg-core/src/memory/portability/import.rs:38`), `prepare` (`import.rs:149`), qué es la misma fila (`PRESENT_SQL`, `TWIN_SQL`, `TOPIC_HOLDERS_SQL`, `import.rs:236-247`), `record_projects` (`import.rs:262`), `place` (`import.rs:280`), `write_links` (`import.rs:398`), las tres pasadas (`import`, `import.rs:530`) y el reporte (`import.rs:614`); la ruta, `import_memories` (`crates/clawg-cli/src/daemon/api.rs:1619`, con su tope de cuerpo en `api.rs:1600`); el comando, `run` y `render` (`crates/clawg-cli/src/import.rs:28` y `import.rs:57`).
- El snapshot: `Reason::PreImport`, `pre-import`, podado con los demás automáticos (`crates/clawg-core/src/memory/backup.rs`).

## R1. El formato `clawg-memories` versión 1

Un export MUST ser JSON Lines en UTF-8: una línea por objeto, cada objeto con `kind`. La primera línea MUST ser el encabezado:

```json
{"kind":"header","format":"clawg-memories","version":1,"exported_at":1760000000000,"clawg":"0.36.0","embedding_model":"multilingual-e5-small","filter":{"project":null,"agent":null,"include_deleted":false}}
```

Después, en este orden:

- `project`: `{"kind":"project","id":"<id de origen>","root_path":"<raíz>"}`, una por proyecto registrado al que pertenece alguna fila exportada.
- `memory`: una por fila de `memory`, por id ascendente, con `id` (el de origen, que sólo vale dentro del archivo) y todas sus columnas salvo `content_hash`: `scope`, `agent_id`, `project_id`, `content`, `memory_type`, `source`, `created_at`, `created_by_type`, `created_by_ref`, `source_session_id`, `last_modified_at`, `last_modified_by`, `update_count`, `last_accessed_at`, `access_count`, `importance`, `tier`, `status`, `superseded_by`, `supersede_cause`, `dispute_count`, `disputed_against` (una lista de ids), `last_disputed_at`, `deleted_at`, `topic_key` y `revived_from`.
- `derivation`: `{"kind":"derivation","memory":…,"source":…,"created_at":…}`.
- `piece`: `{"kind":"piece","save":…,"piece":…,"memory":…,"context":"…"}`.
- `conflict`: `{"kind":"conflict","new_id":…,"old_id":…,"distance":…,"state":…,"verdict":…,"reason":…,"created_at":…,"judged_at":…}`.

Los ids que aparecen en `superseded_by`, `disputed_against`, `revived_from` y en las líneas de vínculo MUST ser ids de origen. Un lector MUST ignorar un campo que no conoce. El formato se versiona aparte del esquema: un cambio que un lector de la versión 1 leería mal MUST subir `version`.

#### Escenario: el ejemplo fijado
- GIVEN el archivo de ejemplo de los tests
- WHEN se lee y se vuelve a escribir
- THEN sale idéntico línea por línea

**Tests:** `the_pinned_example_reads_and_writes_back_unchanged`, `a_header_of_another_format_or_version_is_refused` (`crates/clawg-core/src/memory/portability/format.rs`).

## R2. `clawg export`

`clawg export --out FILE [--project PATH] [--agent A] [--include-deleted] [--format jsonl|markdown] [--profile P]` MUST leer `memory.db` sin el lock y sin escribirlo, con o sin daemon. `--project` MUST resolver la raíz como un puente y exportar sólo las filas de ese `project_id`; `--agent`, sólo las de ese agente. Sin `--include-deleted` MUST dejar fuera las filas borradas. Las líneas de vínculo MUST salir sólo cuando la fila que las tiene sale; un `conflict`, sólo cuando salen sus dos lados.

El archivo MUST crearse nuevo (un `FILE` que existe es un error que no lo toca) y, en Unix, con modo `0600`. La salida MUST nombrar el archivo y cuántas filas, proyectos y vínculos tiene, avisar que guarda el texto de las memorias tal como está guardado (`clawg secrets` lista las que tienen un secreto), y decir cuántas saves esperan en el outbox, que no viajan. Una base de un esquema más nuevo o divergente MUST rechazarse; una de un esquema anterior se lee con las columnas que tenga, y las que le faltan salen `null`.

`--format markdown` MUST escribir una vista para leer (por proyecto y tipo, cada fila con su id, fecha, estado y texto), que `import` no lee.

#### Escenario: con el daemon en marcha
- GIVEN un daemon que tiene el store
- WHEN se exporta
- THEN el archivo sale completo y el daemon sigue sirviendo

#### Escenario: archivo existente
- GIVEN un `FILE` que ya existe
- WHEN se exporta a él
- THEN falla y el archivo queda como estaba

**Tests:** `export_filters_by_project_agent_and_deleted`, `every_column_travels`, `links_leave_only_with_the_rows_that_hold_them`, `a_store_from_v12_or_v16_exports_with_null_for_what_it_lacks`, `a_newer_store_is_not_exported`, `a_missing_store_is_named` (`crates/clawg-core/src/memory/portability/export.rs`); `markdown_groups_rows_by_project_and_type` (`crates/clawg-core/src/memory/portability/markdown.rs`); `export_refuses_an_existing_file_and_writes_owner_only` (`crates/clawg-cli/src/export.rs`); `export_takes_its_filters_and_a_format` (`crates/clawg-cli/src/cli.rs`); `export_and_import_round_trip_through_the_binary` (`crates/clawg-cli/tests/binary.rs`), que exporta con el daemon en marcha.

## R3. `clawg import` por el daemon

`clawg import FILE [--dry-run] [--agent A] [--map-root OLD=NEW]... [--profile P]` MUST enviar el archivo a `POST /v1/import` de un daemon en marcha y nunca iniciar uno. Un encabezado ausente, de otro formato o de una `version` desconocida MUST rechazarse con `400` sin escribir nada ni tomar snapshot.

Antes de escribir, el daemon MUST tomar un snapshot `pre-import` (automático, podado como los demás). Cada fila insertada MUST:

- conservar todas sus columnas exportadas, con sus fechas, accesos, tier, importancia, estado y origen (`source`, `created_by_type`), salvo los ids, el `agent_id` cuando se pidió `--agent` y el `project_id` (R4);
- tener su `content` enmascarado como un save (`redact`) y su `content_hash` calculado sobre el texto enmascarado; lo mismo el `reason` de un par y el `context` de una pieza;
- quedar en el índice de texto si no está borrada, y fuera de él si lo está;
- tener un vector re-embebido con el modelo del daemon (detrás del `context` de su pieza, como el worker) si no está borrada; una borrada no lleva vector, como una que borró `delete`, y `undelete` la trae.

El detector de contradicciones MUST NOT correr sobre lo importado. Los vínculos (`superseded_by`, `disputed_against`, `revived_from`, derivaciones, piezas y pares) MUST traducirse a los ids nuevos; uno cuyo otro extremo no está en el archivo ni en el destino se descarta y se reporta. Una `disputed` sin ningún par que la sostenga queda `active`.

#### Escenario: ida y vuelta
- GIVEN una base sembrada con los siete tipos, una save en piezas, una conclusión, una cadena `superseded` por revisión y otra por veredicto, una fila revivida, un par `escalated`, uno juzgado y una fila borrada
- WHEN se exporta con `--include-deleted` y se importa en una base vacía
- THEN los conteos por estado, tipo y proyecto son los mismos, las cadenas, derivaciones, piezas y pares unen las filas equivalentes, accesos y tier son los del origen, y cada fila viva se alcanza por búsqueda

#### Escenario: versión desconocida
- GIVEN un archivo con `"version":2`
- WHEN se importa
- THEN responde `400`, no hay filas nuevas ni snapshot nuevo

**Tests:** `a_round_trip_keeps_rows_states_and_links` (también comprueba que los pares son sólo los del archivo, sin detector), `an_unknown_version_writes_nothing`, `without_an_embedder_nothing_is_written`, `imported_text_is_masked_and_indexed_once`, `a_deleted_row_comes_back_deleted_without_a_vector`, `agent_reassigns_every_row_that_has_one`, `links_to_rows_outside_the_file_are_dropped_and_reported`, `a_store_from_v16_travels_whole` (`crates/clawg-core/src/memory/portability/import.rs`); `an_import_stores_rows_with_their_state_and_a_dry_run_stores_none` e `import_answers_400_for_an_unknown_version_before_the_embedder` (`crates/clawg-cli/tests/api.rs`); `import_takes_a_file_and_repeated_root_maps` (`crates/clawg-cli/src/cli.rs`); `export_and_import_round_trip_through_the_binary` (`crates/clawg-cli/tests/binary.rs`), donde `import` sin daemon sale con 1 y no arranca ninguno.

## R4. Proyectos

Cada fila de un proyecto con línea `project` MUST quedar con el `project_id` que `derive_project_id` da para su raíz en el destino: la `root_path` exportada, o `NEW` si `--map-root OLD=NEW` nombra esa raíz (el CLI resuelve `NEW` como un puente). El proyecto MUST registrarse en `project` sin `index_docs`. Si ese id ya pertenece a otra raíz, las filas de ese proyecto MUST reportarse inválidas y no importarse. Una fila con un `project_id` sin línea `project` (un id de antes de que se registraran raíces) lo conserva.

#### Escenario: otra raíz
- GIVEN un export de `/a/repo`
- WHEN se importa con `--map-root /a/repo=/b/repo`
- THEN las filas quedan con `derive_project_id("/b/repo")` y `project` lo registra con esa raíz

**Tests:** `map_root_gives_the_destination_project_id`, `a_project_id_of_another_root_is_reported_and_skipped` (`crates/clawg-core/src/memory/portability/import.rs`).

## R5. Un segundo import no duplica

Una fila del archivo cuyo agente, proyecto, hash del texto enmascarado y `created_at` coinciden con una fila del destino (viva, reemplazada o borrada) MUST contarse como ya presente: no se inserta ni se le suman accesos, y sus vínculos apuntan a esa fila. Los vínculos de una fila presente sólo MUST completarse donde están vacíos; una save en piezas se importa sólo si ninguna de sus filas tiene ya una pieza.

Una fila viva cuyo texto ya está vivo en su proyecto, de cualquier autor, con otra fecha MUST fundirse con esa fila (no se inserta; sus vínculos apuntan a ella), para que no haya dos vivas iguales. Una fila viva con una `topic_key` que en el destino tiene otra fila viva de su proyecto, de cualquier autor, que no vino de este import, MUST insertarse `superseded` por esa fila con causa `revision`. El import es del usuario: `sharing.json` no lo acota (`TWIN_SQL` y `TOPIC_HOLDERS_SQL`, `crates/clawg-core/src/memory/portability/import.rs`). La identidad de una fila ya importada sigue incluyendo su agente.

**Test (alcance por proyecto):** `an_import_merges_into_another_agents_live_text_and_key` (`import.rs`).

#### Escenario: dos veces
- GIVEN un archivo ya importado
- WHEN se importa de nuevo
- THEN no hay filas nuevas, `access_count` no cambia y el reporte cuenta todas como presentes

**Tests:** `a_second_import_adds_nothing`, `links_lost_between_transactions_are_filled_by_importing_again`, `a_live_twin_absorbs_the_imported_row`, `a_taken_topic_key_supersedes_the_imported_row` (`crates/clawg-core/src/memory/portability/import.rs`).

## R6. Filas inválidas

Una línea que no es JSON, de un `kind` desconocido, o una fila `memory` con un `memory_type` desconocido, un `status` fuera de `active`, `disputed` y `superseded`, un `scope` que no cuadra con su agente y proyecto, una `topic_key` inválida, una `supersede_cause` desconocida, una `importance` fuera de 0 a 10 o un texto vacío MUST reportarse con su número de línea y su motivo, y el resto MUST importarse.

**Tests:** `invalid_lines_are_reported_and_the_rest_imports` (`crates/clawg-core/src/memory/portability/format.rs` y `crates/clawg-core/src/memory/portability/import.rs`); `unknown_fields_are_ignored` (`crates/clawg-core/src/memory/portability/format.rs`).

## R7. `--dry-run` y el reporte

`--dry-run` MUST responder el mismo reporte sin escribir, sin embeber y sin snapshot. El reporte MUST traer: cuántas filas se insertan (o se insertarían), por tipo, por estado (`live`, `superseded`, `deleted`) y por proyecto de destino; cuántas estaban presentes; las fundidas con su fila; las reemplazadas por una clave tomada; cuántas se enmascararon; las inválidas con línea y motivo; los vínculos descartados; y la ruta del snapshot cuando lo hubo. `clawg import` MUST imprimirlo legible y salir 0, también con inválidas.

**Tests:** `a_dry_run_reports_without_writing` (`crates/clawg-core/src/memory/portability/import.rs`); `import_report_lists_counts_and_invalid_lines`, `render_names_merged_taken_and_dropped_rows` y `a_missing_file_is_named_before_the_daemon_is_asked` (`crates/clawg-cli/src/import.rs`).

## LP6. El idioma en el export y el import

El export MUST escribir `lang` en cada línea `memory` (`MemoryLine.lang`, `null` sin clasificar). El import MUST calcular el idioma de cada fila que inserta con `detect` sobre su texto enmascarado ([`specs/storage`](../storage/spec.md), LP4 y LP5), sin leer el `lang` del archivo, y MUST aceptar un archivo sin ese campo. La versión del formato `clawg-memories` MUST seguir siendo 1: un lector ignora los campos que no conoce.

**Test:** `an_export_carries_the_language_of_each_row` (`export.rs`); `an_import_computes_the_language_whatever_the_file_says` y `an_export_without_languages_imports` (`import.rs`).
