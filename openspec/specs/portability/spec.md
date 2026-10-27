# portability — Especificación

Fuente de verdad de cómo las memorias salen de un store y entran en otro, fila por fila: el formato `clawg-memories`, `clawg export`, `clawg import` y `POST /v1/import`, también desde el export de otra herramienta de memoria (HI4 a HI18), cómo escala la importación (IL1 e IL5), cómo decide sin ocupar la conexión del store (ID1 a ID3), cómo se deshace un import sin perder lo guardado después (IU1 a IU5), qué deja en el log cada fase de una importación (IT1 a IT4) y en qué orden embebe sus filas sin cambiar lo que escribe (ET1 a ET3). Los snapshots de archivo entero (`backup` y `restore`) están en [`specs/restore`](../restore/spec.md); el enmascarado que el import aplica, en [`specs/secrets`](../secrets/spec.md).

**Origen:** [memory-portability](../../changes/archive/2026-10-20-memory-portability/proposal.md), archivado el 2026-10-20. Los ids de requisito son los de esa spec. [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22, llevó a proyecto el texto vivo y el titular de la clave de R5. [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23, sumó el idioma de cada fila (LP6). [cli-language](../../changes/archive/2026-10-23-cli-language/proposal.md), archivado el 2026-10-23, sumó el idioma de los informes de `export` e `import` (CL3). [history-import](../../changes/archive/2026-10-24-history-import/proposal.md), archivado el 2026-10-24, sumó el proyecto de origen al formato (HI2) y el import desde otra herramienta (HI4 a HI18); su V23 está en [`specs/storage`](../storage/spec.md) y el filtro `--origin-project` en [`specs/project`](../project/spec.md) (HI3). [import-linear](../../changes/archive/2026-10-24-import-linear/proposal.md), archivado el 2026-10-24, sumó que cada búsqueda de la importación llega a sus filas por índice (IL1) y que su tiempo crece con sus filas (IL5); su V25 está en [`specs/storage`](../storage/spec.md) (IL2 e IL3). [import-decision-pass](../../changes/archive/2026-10-24-import-decision-pass/proposal.md), archivado el 2026-10-24, sumó que la pasada de decisión no ocupa la conexión (ID1), que decide lo mismo que la pasada SQL (ID2) y que deja de planificar cuando se suelta el import (ID3), y llevó a la tabla de IL1 las consultas de su carga (ID4). [import-undo](../../changes/archive/2026-10-24-import-undo/proposal.md), archivado el 2026-10-24, sumó el número y el diario de cada import (IU1), `clawg import --undo N` (IU2 a IU4) y sus puertas en el daemon y el CLI (IU5); su V26 está en [`specs/storage`](../storage/spec.md) (IU6). [import-timing](../../changes/archive/2026-10-24-import-timing/proposal.md), archivado el 2026-10-24, sumó la línea de log de cada fase de una importación (IT1), el tiempo en SQLite y en la cola de las fases que usan la conexión (IT2), el resumen (IT3) y que medir no cambia la importación (IT4). [ram-footprint](../../changes/archive/2026-10-24-ram-footprint/proposal.md), archivado el 2026-10-24, sumó a IT1 la memoria privada con que termina cada fase en el daemon (RF5 de esa spec). [embed-throughput](../../changes/archive/2026-10-27-embed-throughput/proposal.md), archivado el 2026-10-27, sumó que una importación embebe primero las filas de más tokens (ET1), que el orden no cambia lo que se escribe (ET2) y la medición de la fase de embeddings (ET3). [project-locators](../../changes/archive/2026-10-27-project-locators/proposal.md), archivado el 2026-10-27, su entrega 4, que sale en 0.65.0, llevó a la línea `project` el nombre, los localizadores, la lápida y las declaraciones de un proyecto (R1), cambió adónde va cada proyecto del archivo y lo que el import graba y sugiere (R4), sumó la fila presente en otro proyecto (R5), el diario del registro y lo que retira su `--undo` (IU1, IU2), `--project-map NOMBRE=project:REF` y los caminos crudos (HI7, HI16); sus requisitos están en [`specs/project`](../project/spec.md) (PL53 a PL65) y su V31 en [`specs/storage`](../storage/spec.md).

## Dónde está

- El formato: `FORMAT` y `VERSION` (`crates/clawg-core/src/memory/portability/format.rs:14-16`), las líneas (`format.rs:19-140`), `parse` (`format.rs:176`), las comprobaciones de una fila y de un par (`check_memory`, `format.rs:229`; `check_conflict`, `format.rs:262`) y `render` (`format.rs:278`).
- El export: `export` y `read` (`crates/clawg-core/src/memory/portability/export.rs:68` y `export.rs:80`); la vista, `markdown::render` (`crates/clawg-core/src/memory/portability/markdown.rs:12`); el comando, `run` y `write_new` (`crates/clawg-cli/src/export.rs:16` y `export.rs:31`).
- El import: `MAX_IMPORT_BYTES` (`crates/clawg-core/src/memory/portability/import.rs:62`), `prepare` (`import.rs:325`), el asiento sobre el que se escriben las reglas que colocan cada fila (`Seat`, `import.rs:583`) y sus dos implementaciones, la transacción (`SqlSeat`, `import.rs:603`) y el planificador (`Planned`, `import.rs:881`), qué es la misma fila (`PRESENT_SQL`, `TWIN_SQL`, `TOPIC_HOLDERS_SQL`, `import.rs:1170-1185`), `record_projects` (`import.rs:1228`, con su parte pura, `taken_projects`, `import.rs:1206`), `place` (`import.rs:1277`), `write_links` (`import.rs:1401`), la carga de la pasada de decisión (`LOAD_BATCH`, `import.rs:801`; `ORIGIN_ROWS_SQL`, `TEXT_ROWS_SQL` y `KEY_ROWS_SQL`, `import.rs:814-819`; `PROJECT_ROOT_SQL`, `import.rs:1200`; `load`, `import.rs:1001`) y la decisión (`decide`, `import.rs:1080`), las tres pasadas (`import`, `import.rs:1525`, y `write`, `import.rs:1655`), el de otra fuente (`import_foreign`, `import.rs:1568`; las transacciones por nota, `note_batches`, `import.rs:1627`) y el reporte (`import.rs:1789`); la ruta, `import_memories` (`crates/clawg-cli/src/daemon/api.rs:1966`, con su tope de cuerpo en `api.rs:1938`); el comando, `run` y `render` (`crates/clawg-cli/src/import.rs:42` y `import.rs:76`).
- El orden de embebido: `longest_first` (`crates/clawg-core/src/memory/portability/import.rs:1783`), que ordena los índices de más a menos tokens con los empates en el orden del archivo; `embedding_order` (`import.rs:1766`), que cuenta con `ChatChunker::count_all` (`crates/clawg-core/src/memory/chunker/chat.rs:126`, sobre `TokenSplitter::count_all`, `crates/clawg-core/src/memory/chunker/token_splitter.rs:51`) en grupos de `COUNT_BATCH` (`import.rs:72`) dentro de un hilo bloqueante; `rows_to_embed` (`import.rs:1753`), que da las filas a embeber; las tandas de `EMBED_BATCH` (`import.rs:68`), dentro de la fase `embed` de `write`; y el chunker que `import_memories` arma y pasa a `import` y a `import_foreign` (`crates/clawg-cli/src/daemon/api.rs:2012`).
- Lo que registra: las fases (`Step`, `crates/clawg-core/src/memory/portability/timing.rs:13`), una fase con sus llamadas y su memoria (`Timing`, `timing.rs:64`), las de un import, que `record` registra al terminar cada una y `finish` cierra con el resumen (`ImportTimings`, `timing.rs:139`, con la sonda de memoria de `with_memory`, `timing.rs:152`; `Summary`, `timing.rs:179`), y el ayudante que mide cada llamada a la conexión dentro y fuera de su clausura (`timed`, `import.rs:252`); `read`, la sonda y el resumen, en `import_memories`.
- El diario: `Journal` (`import.rs:510`), que escribe cada transacción de `SqlSeat` y de `record_projects` y que `write` cierra con `finished_at` (`import.rs:1740`), y los imports en curso del proceso (`MemoryStore::importing`, `crates/clawg-core/src/memory/db.rs:171`).
- Los proyectos del archivo: las líneas de un id (`FileProject`), el registro leído en la misma llamada que `plan_ids_in` (`Registry`, `registry_in`) y la decisión pura de adónde va cada uno y por qué paso (`place` y `By`, `crates/clawg-core/src/memory/portability/projects.rs`); lo que se graba, en `record_projects` (`import.rs`), y lo que se agregó al registro, en `memory_import_registry` (V31).
- El deshacer, en `crates/clawg-core/src/memory/portability/undo.rs`: `undo_import`, la carga (`load`; `LOAD_BATCH` y `WRITE_BATCH`), la decisión pura (`decide`, que termina en `decide_registry` para lo que el import sumó al registro de proyectos), lo que borra una fila (`REMOVE_SQL` y `remove_in`), lo que devuelve en las demás (`restore_in` y `take_back_in`), lo que retira del registro donde sigue como lo encontró la decisión (`take_back_registered_in`) y la entrada de `memory_import_registry` que deja de pesar (`forget_in`), y el reporte (`UndoReport`); la ruta, `undo_import` (`crates/clawg-cli/src/daemon/api.rs:2085`); el comando, `undo` y `render_undo` (`crates/clawg-cli/src/import.rs:298` y `import.rs:308`).
- Otra fuente: el modelo común (`ForeignExport`, `ForeignNote`, `ForeignRelation`, `crates/clawg-core/src/memory/portability/foreign.rs:12-72`), el adaptador de engram (`read`, `crates/clawg-core/src/memory/portability/engram.rs:22`) y la traducción a filas (`translate` y `rewrite_key`, `crates/clawg-core/src/memory/portability/translate.rs:307` y `translate.rs:502`).
- Los snapshots: `Reason::PreImport`, `pre-import`, y `Reason::PreUndo`, `pre-undo`, podados con los demás automáticos (`crates/clawg-core/src/memory/backup.rs`).

## R1. El formato `clawg-memories` versión 1

Un export MUST ser JSON Lines en UTF-8: una línea por objeto, cada objeto con `kind`. La primera línea MUST ser el encabezado:

```json
{"kind":"header","format":"clawg-memories","version":1,"exported_at":1760000000000,"clawg":"0.36.0","embedding_model":"multilingual-e5-small","filter":{"project":null,"agent":null,"include_deleted":false}}
```

Después, en este orden:

- `project`: `{"kind":"project","id":"<id de origen>","root_path":"<raíz>","name":…,"folders":[{"root_path":…,"remote":…,"root_commit":…}],"remotes":[…],"merged_into":…,"subfolders":[{"subpath":…,"project_id":…}]}`, una por proyecto ([`specs/project`](../project/spec.md) PL54): `id` es el del proyecto; `root_path`, el camino de su carpeta más vieja, que un proyecto sin carpeta no lleva; `folders`, cada carpeta que lleva a él, la más vieja primero, con las huellas que grabó; `remotes`, los que `project_remote` le ata; `merged_into`, sólo en una lápida; `subfolders`, las declaraciones de las que es el repositorio. Un campo vacío no se escribe, así que la línea de un store anterior a V27 (una por carpeta, sin `name`) sale como antes. Salen la de cada proyecto de una fila exportada, la de cada lápida que lleva a uno de ellos, la de cada repositorio con una declaración que lleva a uno de ellos y la de cada subproyecto al que lleva una declaración de uno de ellos. `deleted_at`, `born`, `reviewed`, `created_at`, `git_stamp` e `index_docs` no viajan.
- `memory`: una por fila de `memory`, por id ascendente, con `id` (el de origen, que sólo vale dentro del archivo) y todas sus columnas salvo `content_hash`: `scope`, `agent_id`, `project_id`, `content`, `memory_type`, `source`, `created_at`, `created_by_type`, `created_by_ref`, `source_session_id`, `last_modified_at`, `last_modified_by`, `update_count`, `last_accessed_at`, `access_count`, `importance`, `tier`, `status`, `superseded_by`, `supersede_cause`, `dispute_count`, `disputed_against` (una lista de ids), `last_disputed_at`, `deleted_at`, `topic_key`, `revived_from`, `lang` (LP6) y `origin_project` (HI2).
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

El archivo MUST crearse nuevo (un `FILE` que existe es un error que no lo toca) y, en Unix, con modo `0600`. La salida MUST nombrar el archivo y cuántas filas, proyectos (y cuántos de ellos van sin carpeta) y vínculos tiene, avisar que guarda el texto de las memorias tal como está guardado (`clawg secrets` lista las que tienen un secreto), y decir cuántas saves esperan en el outbox, que no viajan. Una base de un esquema más nuevo o divergente MUST rechazarse; una de un esquema anterior se lee con las columnas que tenga, y las que le faltan salen `null`.

`--format markdown` MUST escribir una vista para leer (por proyecto y tipo, cada fila con su id, fecha, estado y texto), que `import` no lee.

#### Escenario: con el daemon en marcha
- GIVEN un daemon que tiene el store
- WHEN se exporta
- THEN el archivo sale completo y el daemon sigue sirviendo

#### Escenario: archivo existente
- GIVEN un `FILE` que ya existe
- WHEN se exporta a él
- THEN falla y el archivo queda como estaba

**Tests:** `export_filters_by_project_agent_and_deleted`, `every_column_travels`, `an_export_writes_one_line_per_project_with_its_locators`, `an_export_of_a_subproject_carries_its_repository_and_declaration`, `a_deleted_project_exports_without_its_mark`, `a_store_from_before_the_registry_exports_its_folders_as_before`, `links_leave_only_with_the_rows_that_hold_them`, `a_store_from_v12_or_v16_exports_with_null_for_what_it_lacks`, `a_newer_store_is_not_exported`, `a_missing_store_is_named` (`crates/clawg-core/src/memory/portability/export.rs`); `markdown_groups_rows_by_project_and_type` (`crates/clawg-core/src/memory/portability/markdown.rs`); `export_refuses_an_existing_file_and_writes_owner_only` y `an_export_counts_its_projects` (`crates/clawg-cli/src/export.rs`); `export_takes_its_filters_and_a_format` (`crates/clawg-cli/src/cli.rs`); `export_and_import_round_trip_through_the_binary` (`crates/clawg-cli/tests/binary.rs`), que exporta con el daemon en marcha.

## R3. `clawg import` por el daemon

`clawg import FILE [--dry-run] [--agent A] [--map-root OLD=NEW]... [--profile P]` (con `--from` y `--project-map` para otra fuente, HI16) MUST enviar el archivo a `POST /v1/import` de un daemon en marcha y nunca iniciar uno. Un encabezado ausente, de otro formato o de una `version` desconocida MUST rechazarse con `400` sin escribir nada ni tomar snapshot.

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

Las líneas `project` de un mismo `id` son un proyecto del archivo, con las carpetas de todas. Cada proyecto del archivo MUST ir al primero de estos ([`specs/project`](../project/spec.md) PL56): el mapa (`--map-root OLD=NEW` nombra una de sus carpetas: el proyecto que un registro le da a NEW, con su camino crudo), un remoto suyo que `project_remote` ata acá, su id si es un proyecto de acá al que no contradice, su nombre si es el de un solo proyecto vivo de acá y ninguno de los dos tiene huellas, el camino de una de sus carpetas si está grabado o existe acá y no lo contradice (`plan_ids_in`, `crates/clawg-core/src/memory/project.rs`, sin que dos raíces del archivo tomen el mismo id), y si no, uno nuevo. Cada paso que nombra un proyecto lo sigue por `canonical`, y la transacción que escribe lo sigue otra vez. Un proyecto nuevo lleva el id y el nombre del archivo cuando están libres, y si no `new_project_id` de su nombre, con `born = 'import'`. Una línea sin filas (una lápida, el repositorio de una declaración, un subproyecto declarado) no crea un proyecto, salvo el subproyecto de una declaración que el import escribe; la de una lápida sólo prueba su id.

Una carpeta que no está MUST registrarse en `project_path` sin `index_docs`: una que existe acá, con lo que grabaría un registro (su proyecto, su remoto, su primer commit y su marca; PL7), salvo que su checkout contradiga a su proyecto del archivo; una de otra máquina, con el `remote` y el `root_commit` del archivo y sin marca. Cada remoto del proyecto del archivo que nadie ata MUST atarse en `project_remote` al proyecto al que fue. Lo que el plan necesita del disco se lee en un hilo bloqueante. Un import con escritura MUST dejar las sugerencias que dejaría un registro de cada carpeta que graba y existe acá (`root_commit`, `remote_changed`, `folder_name`), `similar_name` con los proyectos de acá de nombre parecido y `merged_at_source` cuando la fuente tiene unidos dos proyectos que acá están separados (PL58); una prueba en seco las reporta sin escribirlas. Las declaraciones de un proyecto del archivo MUST escribirse entre los proyectos de acá a los que fueron sus dos lados, salvo que acá ya haya una con esa clave (PL60). Una fila insertada borrada MUST NOT hacer volver a su proyecto borrado; una viva lo hace volver, y el reporte lo dice (PL59). El import nunca une, separa ni renombra proyectos de acá. Si entre el plan y la escritura un registro le dio ese id a otra raíz, las filas de ese proyecto MUST reportarse inválidas y no importarse. Una fila con un `project_id` sin línea `project` lo conserva, y si `project` no lo tiene, el import lo crea con `born = 'import'`.

#### Escenario: otra raíz
- GIVEN un export de `/a/repo`
- WHEN se importa con `--map-root /a/repo=/b/repo`
- THEN las filas quedan con `derive_project_id("/b/repo")` y `project` lo registra con esa raíz

#### Escenario: una raíz cuyo id es de otra
- GIVEN `project` con el id de la raíz exportada para otra raíz
- WHEN se importa
- THEN las filas quedan con el siguiente candidato de la raíz y ninguna es inválida

**Tests:** `map_root_gives_the_destination_project_id`, `an_import_gives_a_colliding_root_its_own_id`, `an_import_into_a_clone_goes_to_the_project_of_its_remote` (`crates/clawg-core/src/memory/portability/import.rs`), y los de PL55 a PL60 de [`specs/project`](../project/spec.md).

`clawg export --project PATH` MUST tomar el id de proyecto de la raíz de PATH del store, sin escribir: el de su remoto, si no el de su raíz en `project`, si no el que recibiría (`project_id_of`, `crates/clawg-core/src/memory/portability/export.rs`); en un store sin `project`, `derive_project_id`. Elige con PATH como camino crudo, como un registro: dentro de una subcarpeta declarada, el proyecto al que lleva ([`specs/project`](../project/spec.md) PL47); en un store anterior a V30, como si no hubiera declaraciones. El import manda el camino crudo de cada raíz de sus mapas (HI16).

**Tests:** `export_reads_the_project_id_from_the_store`, `a_store_from_before_roots_were_recorded_gives_the_derived_id` (`export.rs`).

## R5. Un segundo import no duplica

Una fila del archivo cuyo agente, proyecto, hash del texto enmascarado y `created_at` coinciden con una fila del destino (viva, reemplazada o borrada) MUST contarse como ya presente: no se inserta ni se le suman accesos, y sus vínculos apuntan a esa fila. Los vínculos de una fila presente sólo MUST completarse donde están vacíos; una save en piezas se importa sólo si ninguna de sus filas tiene ya una pieza.

Una fila viva cuyo texto ya está vivo en su proyecto, de cualquier autor, con otra fecha MUST fundirse con esa fila (no se inserta; sus vínculos apuntan a ella), para que no haya dos vivas iguales. Una fila viva con una `topic_key` que en el destino tiene otra fila viva de su proyecto, de cualquier autor, que no vino de este import, MUST insertarse `superseded` por esa fila con causa `revision`. El import es del usuario: `sharing.json` no lo acota (`TWIN_SQL` y `TOPIC_HOLDERS_SQL`, `crates/clawg-core/src/memory/portability/import.rs`). La identidad de una fila ya importada sigue incluyendo su agente.

Una fila de un export de clawg que no está presente en su proyecto MUST contarse también como presente si acá hay, en cualquier proyecto y borrada o no, una fila con su agente, su hash enmascarado, su `created_at` y su `created_by_ref` (`PRESENT_ELSEWHERE_SQL`): una fila movida después del export no se duplica. No se inserta, sus vínculos van a esa fila, y el reporte la cuenta en `present_elsewhere` por el proyecto donde está. No vale para las notas de otra herramienta, que encuentra su origen (HI11).

**Test (alcance por proyecto):** `an_import_merges_into_another_agents_live_text_and_key` (`import.rs`). **Tests (en otro proyecto):** `an_export_imported_again_after_a_move_adds_nothing` y `two_rows_alike_from_two_origins_stay_two` (`import.rs`).

#### Escenario: dos veces
- GIVEN un archivo ya importado
- WHEN se importa de nuevo
- THEN no hay filas nuevas, `access_count` no cambia y el reporte cuenta todas como presentes

**Tests:** `a_second_import_adds_nothing`, `links_lost_between_transactions_are_filled_by_importing_again`, `a_live_twin_absorbs_the_imported_row`, `a_taken_topic_key_supersedes_the_imported_row` (`crates/clawg-core/src/memory/portability/import.rs`).

## R6. Filas inválidas

Una línea que no es JSON, de un `kind` desconocido, o una fila `memory` con un `memory_type` desconocido, un `status` fuera de `active`, `disputed` y `superseded`, un `scope` que no cuadra con su agente y proyecto, una `topic_key` inválida, una `supersede_cause` desconocida, una `importance` fuera de 0 a 10 o un texto vacío MUST reportarse con su número de línea y su motivo, y el resto MUST importarse.

**Tests:** `invalid_lines_are_reported_and_the_rest_imports` (`crates/clawg-core/src/memory/portability/format.rs` y `crates/clawg-core/src/memory/portability/import.rs`); `unknown_fields_are_ignored` (`crates/clawg-core/src/memory/portability/format.rs`).

## R7. `--dry-run` y el reporte

`--dry-run` MUST responder el mismo reporte sin escribir, sin embeber y sin snapshot. El reporte MUST traer: cuántas filas se insertan (o se insertarían), por tipo, por estado (`live`, `superseded`, `deleted`) y por proyecto de destino; cuántas estaban presentes; las fundidas con su fila; las reemplazadas por una clave tomada; cuántas se enmascararon; las inválidas con línea y motivo; los vínculos descartados; la ruta del snapshot cuando lo hubo; y, con `#[serde(default)]`, `present_elsewhere`, `projects` (cada proyecto del archivo con adónde fue, el paso que lo decidió, `map`, `remote`, `id`, `name`, `path` o `new`, y si lo creó o lo hizo volver), `suggestions` y `subfolders` (las declaraciones escritas y las que no, con su motivo). Con proyectos nuevos o sugerencias, la salida termina con `clawg projects list`. `clawg import` MUST imprimirlo legible y salir 0, también con inválidas.

**Tests:** `a_dry_run_reports_without_writing` (`crates/clawg-core/src/memory/portability/import.rs`); `import_report_lists_counts_and_invalid_lines`, `render_names_merged_taken_and_dropped_rows` y `a_missing_file_is_named_before_the_daemon_is_asked` (`crates/clawg-cli/src/import.rs`).

## LP6. El idioma en el export y el import

El export MUST escribir `lang` en cada línea `memory` (`MemoryLine.lang`, `null` sin clasificar). El import MUST calcular el idioma de cada fila que inserta con `detect` sobre su texto enmascarado ([`specs/storage`](../storage/spec.md), LP4 y LP5), sin leer el `lang` del archivo, y MUST aceptar un archivo sin ese campo. La versión del formato `clawg-memories` MUST seguir siendo 1: un lector ignora los campos que no conoce.

**Test:** `an_export_carries_the_language_of_each_row` (`export.rs`); `an_import_computes_the_language_whatever_the_file_says` y `an_export_without_languages_imports` (`import.rs`).

## CL3. Los informes en el idioma del comando

La salida de `clawg export` (`report`, `crates/clawg-cli/src/export.rs`) y la de `clawg import` (`render`, `crates/clawg-cli/src/import.rs`) MUST salir en el idioma del comando ([`specs/preferences`](../preferences/spec.md), CL1 a CL3), en inglés sin preferencia, con cada frase con número concordando con él en cada idioma (CL5). Los conteos, los tipos y estados de memoria, los números de línea, los motivos de una línea inválida que da el import, las rutas y los comandos MUST NOT cambiar. El formato `clawg-memories` y la vista `--format markdown` MUST NOT depender del idioma.

#### Escenario: una fila y varias
- WHEN `clawg export` escribe una memoria, un proyecto y un vínculo de cada clase, y después varias
- THEN cada idioma dice el singular con uno y el plural con varios, y los números son los mismos en los dos

**Tests:** `the_export_report_agrees_with_its_numbers_in_both_languages` (`export.rs`); `the_import_report_speaks_spanish` (`import.rs`).

## Definiciones de otra fuente

- **Fuente:** la herramienta de la que viene un export que no es `clawg-memories`. Este cambio lee una: `engram`.
- **Nota:** lo que la fuente guardó y clawg guarda como un save: una observación, un prompt o el resumen de una sesión.
- **Origen de una nota:** la fuente y el id estable de la nota en ella, escrito en `created_by_ref` como `engram:<sync_id>`; sin `sync_id`, `engram:observation:<id>` o `engram:prompt:<id>`; el resumen de una sesión, `engram:session:<id>`.
- **Grupo de origen:** el proyecto y el scope que la nota tenía en la fuente (`personal` o no), y «sin proyecto» como un grupo más.
- **Fecha del texto:** `updated_at` de la nota, o `created_at` sin él; en UTC.
- **Destino de una nota:** el id de proyecto que un registro le daría a la raíz que el mapa le asigna a su proyecto de la fuente (R15 y R16 de [`specs/project`](../project/spec.md), como a una raíz de `--map-root`), o «sin proyecto». Dos clones de un remoto son un mismo destino.
- **Fila viva:** `deleted_at IS NULL` y `status IN ('active', 'disputed')`.
- **Fila protegida:** `memory_type` `user_preference` o `architectural_decision` ([`specs/disputes`](../disputes/spec.md)).

## HI2. El origen en el formato `clawg-memories`

El export MUST escribir `origin_project` en cada línea `memory` (`null` cuando la fila no lo tiene, o el store es anterior a V23). El import de un archivo `clawg-memories` MUST guardar el `origin_project` de cada fila que inserta y MUST aceptar un archivo sin el campo. La versión del formato MUST seguir siendo 1.

#### Escenario: ida y vuelta
- GIVEN una fila importada de otra herramienta, con su proyecto de origen
- WHEN se exporta y se importa en otro store
- THEN la fila nueva tiene el mismo `origin_project`, `source` y `created_by_ref`

**Test:** `an_export_carries_the_origin_project` (`export.rs`); `an_import_keeps_the_origin_project` y `an_export_without_an_origin_project_imports` (`import.rs`).

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

Cada fila importada de una fuente MUST tener `scope = 'agent'`, `agent_id` `migrated` (o el de `--agent`), `source = 'import'`, `created_by_type` `user` para un prompt y `agent` para lo demás, su origen en `created_by_ref`, el nombre de su proyecto en la fuente en `origin_project` (NULL sin proyecto), y la sesión de la fuente en `source_session_id` cuando es de 1 a 128 caracteres ASCII visibles (otra se reporta). Su `project_id` MUST ser su destino: el de su nombre en el mapa (el proyecto que un registro da al camino crudo, o el proyecto que nombra `project:REF`); sin proyecto si el nombre no está en el mapa, si la nota no tiene proyecto o si es `personal`. Los ids de las raíces del mapa MUST planearse antes de traducir (`plan_ids_in`), porque las claves se distinguen por destino (HI8). Cada raíz del mapa MUST registrarse en `project` sin `index_docs`, y su remoto en `project_remote`, como las de `--map-root`; un camino dentro de una subcarpeta declarada lleva sus notas al subproyecto, y graba la carpeta del repositorio. Un destino `project:REF` no registra ninguna carpeta, y la transacción que escribe comprueba otra vez que no esté borrado: las notas de un proyecto borrado mientras tanto se reportan inválidas. Si entre el plan y la escritura un registro le dio ese id a otra raíz, las notas de ese nombre MUST reportarse por su proyecto y no importarse.

#### Escenario: el mapa
- GIVEN notas de `a`, de `b` y sin proyecto, y una `personal` de `a`
- WHEN se importa con `--project-map a=RUTA`
- THEN las de `a` quedan en el proyecto de `RUTA`, las demás sin proyecto, y `origin_project` es `a`, `b`, NULL y `a`

#### Escenario: el id de la raíz ya es de otra
- GIVEN `project` con el id que `derive_project_id` da a `RUTA` para otra raíz
- WHEN se importa con `--project-map a=RUTA`
- THEN las notas de `a` quedan en el siguiente id libre de `RUTA`, y el reporte nombra ese proyecto

**Tests:** `the_project_map_places_notes_and_the_rest_have_none`, `every_row_carries_its_origin_and_author` (`translate.rs`); `a_mapped_root_whose_id_is_taken_gets_its_own`, `a_project_taken_while_importing_is_told_by_its_notes`, `a_project_map_to_a_project_places_its_notes_there`, `a_project_map_path_in_a_declared_subfolder_goes_to_its_project` (`import.rs`).

## HI8. Claves de tema

Una `topic_key` válida MUST conservarse. Una inválida MUST reescribirse en minúsculas, con cada carácter fuera de `[a-z0-9._-]` y `/` como `-` y sin segmentos vacíos; si eso la cambió o pasa de 80 caracteres, MUST terminar con `-` y los primeros 10 hex del SHA-256 de la original, cortando lo anterior para caber en 80. La misma original MUST dar siempre la misma clave.

Si notas de más de un grupo de origen caen en un mismo destino con la misma clave, cada nota de ese grupo de claves MUST calificarse: su clave pasa a `<grupo>/<clave>`, reescrita con la misma regla si no entra. `<grupo>` es el nombre del proyecto de la fuente (`no-project` sin él) seguido de `.personal` para una `personal`; un nombre que no es un segmento válido, que es `no-project` o que termina en `.personal` MUST reescribirse como un segmento seguido de `-` y el hash de la original, así dos grupos nunca dan el mismo. Una clave calificada que cae sobre la de una nota de otro grupo en el mismo destino MUST calificarse otra vez, hasta que ningún destino tenga una clave de dos grupos. Dos notas vivas de un mismo grupo de origen y destino con una clave MUST dejar viva la de fecha más nueva y la otra `superseded` por ella con causa `revision`. Una clave que en el destino tiene una fila viva que no vino de este import MUST seguir R5: la nota entra `superseded` por esa fila.

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

Una nota cuyo origen tiene filas no borradas, ninguna con su fecha del texto, MUST entrar como una revisión de la versión guardada más nueva: en el proyecto donde está, con la clave que tiene (la suya si no tiene). Si su fecha es más nueva, MUST entrar viva y dejar las filas vivas guardadas de su origen `superseded` por su primera fila, con causa `revision`, en la misma transacción, salvo las que comparte por ser del mismo texto. Si es más vieja, MUST entrar `superseded` por la primera fila viva guardada de su origen, con causa `revision`, o, sin filas vivas, con las reglas de siempre en ese proyecto. Una nota borrada en la fuente cuyo origen tiene filas MUST NOT cambiar nada, y MUST reportarse cuando alguna de esas filas no está borrada.

#### Escenario: reescrita
- GIVEN una nota importada, y el mismo origen con otro texto y una fecha posterior
- WHEN se importa el export nuevo
- THEN la nota nueva está viva en el proyecto de la vieja y con su clave, y la vieja queda `superseded` por ella

**Tests:** `a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is`, `an_older_version_enters_superseded`, `a_note_deleted_at_the_source_changes_nothing` (`import.rs`).

## HI13. Escritura por nota

Las filas de una nota, y lo que su revisión reemplaza, MUST escribirse en una sola transacción: las transacciones del import MUST cerrarse sólo entre dos notas. Los vínculos de piezas de una nota MUST escribirse salvo que una de sus filas que es de su origen ya tenga uno, o que el texto de una de sus piezas ya no esté; una fila fundida con otra nota, o compartida con la versión que una revisión reemplaza, MAY ser pieza de las dos, como la comparten dos saves.

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

El motivo MUST guardarse enmascarado, con la evidencia debajo cuando la hay. Una relación `orphaned`, una con un extremo que no da filas en el archivo, y una reemplazada por otra MUST contarse y no guardarse. Una relación entre dos notas que ya estaban MUST guardar su par sin cambiar el estado de sus filas.

#### Escenario: un veredicto protegido
- GIVEN una `decision` A y una nota B con la relación `B supersedes A` marcada por un agente
- WHEN se importa
- THEN A sigue viva, el par está `escalated` y B está `disputed`

**Tests:** `a_deleted_note_comes_in_deleted_without_a_vector`, `relations_become_pairs`, `supersedes_over_a_decision_without_the_user_is_escalated`, `what_has_no_place_is_counted` (`import.rs`); `relations_map_to_pair_states` (`translate.rs`).

## HI15. Prompts y resúmenes de sesión

Un prompt con texto MUST guardarse como `chat_message` del usuario, y el `summary` de una sesión como una `session_summary` con la fecha de fin de la sesión (la de inicio sin ella), su proyecto y su origen. Un prompt sin texto MUST contarse.

**Test:** `prompts_and_session_summaries_land_as_their_types` (`translate.rs`).

## HI16. La ruta y el comando

`POST /v1/import` MUST aceptar `from` (`clawg` por defecto, `engram`) y `project_map` (pares de nombre y destino). El destino de cada par de `project_map` y de `map_roots` es un texto (una raíz, como antes), `{"root", "path"}` (la raíz y el camino crudo) o, sólo en `project_map`, `{"project"}` (un id o el nombre de un solo proyecto vivo, que el daemon resuelve antes de leer el archivo: 404 si no es ninguno, 409 `project_ambiguous` si es de dos o más, 409 `project_deleted` si está borrado). `from` desconocido, `project_map` con `from` `clawg`, o `map_roots` con otra fuente MUST ser un `400` sin escribir. Con otra fuente, el daemon MUST leer el archivo con su adaptador antes de pedir el embedder, así un archivo que no lee es un `400` también mientras el modelo carga, y MUST responder `503` `tokenizer_not_ready` sin el tokenizer, también en un `--dry-run`. Un export de engram mandado sin `from` MUST ser el `400` de un archivo sin encabezado.

`clawg import [--from clawg|engram] FILE [--dry-run] [--agent A] [--map-root OLD=NEW]... [--project-map NOMBRE=RUTA|NOMBRE=project:REF]...` MUST mandar el archivo a un daemon en marcha y nunca iniciar uno. `--project-map` MUST separarse en el último `=project:`, y si no lo hay, en el último `=`. Por cada camino, el CLI MUST mandar un objeto con la raíz y el camino crudo, absoluto y canónico, así un daemon anterior rechaza el cuerpo en lugar de leer el camino crudo como una raíz. `--map-root` MUST valer sólo con `--from clawg` y `--project-map` sólo con otra fuente. El CLI MUST resolver cada `RUTA` como un puente y rechazar, antes de leer el archivo, una que no es un directorio. `--dry-run` MUST responder el reporte que daría el import, sin escribir, sin embeber y sin snapshot.

#### Escenario: un ensayo y el import
- GIVEN un export de engram con notas largas, claves, prompts y un proyecto mapeado
- WHEN se corre con `--dry-run` y después sin él
- THEN los dos reportes coinciden salvo `dry_run` y el snapshot, y el segundo dejó las filas

#### Escenario: un export sin notas
- GIVEN un export de engram sin notas, con `sessions` y `prompts` en `null`
- WHEN se importa
- THEN responde el reporte sin error y no inserta ninguna fila

**Tests:** `an_engram_import_stores_notes_and_a_dry_run_reports_the_same`, `an_engram_export_without_notes_imports_nothing`, `import_refuses_an_unknown_source_or_a_mixed_map`, `an_engram_export_without_from_is_a_400`, `a_foreign_import_without_the_tokenizer_is_a_503` (`crates/clawg-cli/tests/api.rs`); `a_project_map_to_a_deleted_ambiguous_or_unknown_project_is_refused`, `a_map_given_as_a_root_is_read_as_before` (`crates/clawg-cli/tests/api.rs`); `import_takes_a_source_and_project_maps` e `import_takes_a_project_map_to_a_project` (`crates/clawg-cli/src/cli.rs`); `a_project_map_must_name_a_directory` y `the_maps_are_sent_with_their_raw_paths` (`crates/clawg-cli/src/import.rs`); `an_engram_export_imports_through_the_binary` (`crates/clawg-cli/tests/binary.rs`).

## HI17. El reporte

El reporte MUST traer, además de lo de R7, las filas insertadas por idioma (`by_lang`, para los dos formatos) y, para otra fuente, en `foreign`: la fuente y su versión; notas por tipo de la fuente con su tipo de destino; notas por proyecto de la fuente con su destino; los nombres sin mapa y las entradas del mapa sin notas; cuántas filas y cuántas notas en más de una pieza; cuántas notas se enmascararon; las claves reescritas y las calificadas, de qué a qué; las notas reemplazadas por otra de su origen; las absorbidas por igual texto; las presentes por proyecto donde están; las revisadas y las versiones viejas; las fechas acotadas; las borradas; las relaciones guardadas por estado y las contadas por motivo; lo que no tiene lugar, por campo, también los que el adaptador no conoce; las notas que la fuente reescribió; las sesiones descartadas; y las notas sin texto. Lo que el reporte de R7 dice por línea del archivo (las filas fundidas con una de acá, las de clave tomada y las que no entraron) MUST ir, para otra fuente, en `foreign` por nota, nombrada por su origen, y las notas de un proyecto cuya raíz pertenece a otro, por ese proyecto: las líneas y los ids de la traducción no significan nada fuera de ella. `clawg import` MUST imprimirlo en el idioma del comando (CL3).

**Tests:** `the_foreign_report_counts_what_the_import_did` (`import.rs`); `render_names_what_a_foreign_import_did` y `the_foreign_report_speaks_spanish` (`crates/clawg-cli/src/import.rs`).

## HI18. El agente lee y cambia lo importado

Desde el agente del puente, una nota importada larga MUST leerse entera con `read_memory`, y `update_memory` y `delete_memory` MUST cambiarla, como cualquier fila que alcanza ([`specs/visibility`](../visibility/spec.md)).

**Test:** `an_imported_long_note_reads_whole_and_changes_for_the_bridge_agent` (`crates/clawg-cli/tests/api.rs`).

## IL1. Las búsquedas de la importación llegan a sus filas por índice

Toda sentencia que la importación (`crates/clawg-core/src/memory/portability/import.rs`) ejecuta por fila, por save o por proyecto, al cargar el store para la pasada de decisión o al colocar las filas en la que escribe, y que busca por una columna o por el rowid, MUST tener en `EXPLAIN QUERY PLAN` una línea `SEARCH` que nombre su índice, y MUST NOT tener ninguna línea `SCAN` que recorra una tabla. `SCAN CONSTANT ROW`, la fila única de un `SELECT EXISTS (…)` sin `FROM`, no recorre ninguna tabla. Esto vale sin estadísticas del planificador. Ejecutada contra un store con filas, ninguna de esas sentencias MUST dar un paso de recorrido completo (`StatementStatus::FullscanStep` igual a 0). El índice que corresponde a cada una:

| Sentencia | Índice |
|---|---|
| `ORIGIN_SQL` | `idx_memory_import_origin` |
| `PRESENT_SQL`, con proyecto y sin él | `idx_memory_dedup` |
| `TWIN_SQL`, con proyecto y sin él, con `id` en su orden (ID4) | `idx_memory_dedup` |
| `TOPIC_HOLDERS_SQL` | `idx_memory_topic` |
| `REVISED_SQL` | la clave primaria de `memory` (`INTEGER PRIMARY KEY`) |
| `LINKED_SQL`, si la fila ya es una pieza (`write_links`) | `idx_memory_piece_memory` |
| `NEXT_SAVE_SQL`, el próximo `save_id` (`write_links`) | la clave primaria de `memory_piece` |
| `ORIGIN_ROWS_SQL`, las filas de un origen, en la carga (ID4) | `idx_memory_import_origin` |
| `TEXT_ROWS_SQL`, las filas de un texto, en la carga (ID4) | `idx_memory_dedup` |
| `KEY_ROWS_SQL`, las filas vivas de una clave, en la carga (ID4) | `idx_memory_topic` |
| `PROJECT_ROOT_SQL`, la raíz de un proyecto, en la carga (ID4) | la clave primaria de `project` |
| `PRESENT_ELSEWHERE_SQL`, la fila de un export en cualquier proyecto (R5) | `idx_memory_created_at` |
| `DATE_ROWS_SQL`, las filas de una fecha, en la carga | `idx_memory_created_at` |

`PRESENT_SQL` MUST seguir encontrando la fila con el agente, el proyecto, el hash y la fecha dados, esté borrada o no: por eso `idx_memory_dedup` no tiene predicado ([`specs/storage`](../storage/spec.md) IL2).

#### Escenario: la fila presente, por índice
- GIVEN un store migrado y con filas
- WHEN se pide el plan de `PRESENT_SQL` con un agente, un proyecto (o `NULL`), un hash y una fecha
- THEN una línea es `SEARCH memory USING INDEX idx_memory_dedup (<expr>=? AND content_hash=? AND created_at=?)` y ninguna recorre una tabla
- AND al ejecutarla no da ningún paso de recorrido completo

#### Escenario: las demás búsquedas
- GIVEN el mismo store
- WHEN se pide el plan de cada una de las otras sentencias de la tabla y se la ejecuta
- THEN cada plan nombra el índice de su fila de la tabla, ninguna línea recorre una tabla y ninguna ejecución da un paso de recorrido completo

**Test:** `the_import_lookups_seek_an_index` (`import.rs`).

## IL5. El tiempo de una importación crece con sus filas, no con su cuadrado

Importar el doble de notas en un store vacío SHOULD tardar como mucho tres veces lo que tarda importar la mitad.

#### Escenario
- GIVEN dos stores vacíos
- WHEN se importan N notas de una fila en el primero y 2N en el segundo, con el embedder falso
- THEN las dos importaciones insertan todas sus notas
- AND la segunda tarda como mucho tres veces lo que la primera

**Test:** `twice_the_notes_import_in_at_most_three_times_the_time` (`import.rs`, `#[ignore]`). Depende de la máquina, así que queda fuera de CI, y sólo se comparan corridas de una misma compilación.

## ID1. La pasada de decisión no ocupa la conexión

La pasada que decide dónde va cada fila de un import (la que corre una corrida en seco entera y la real antes de embeber) MUST NOT correr en una llamada a la conexión del store que dure con el archivo. Lo que lee del store MUST cargarse en llamadas de como mucho `LOAD_BATCH` búsquedas por índice, cada una su propio `conn.call`, sin una transacción abierta entre una y otra. La decisión MUST correr fuera de la conexión y fuera de los hilos del runtime async (`spawn_blocking`). La pasada MUST NOT escribir nada en el store.

#### Escenario: una sonda durante la corrida en seco
- GIVEN un store vacío y un export de otra herramienta de 10.000 notas
- WHEN corre `--dry-run` y, al mismo tiempo, una sonda hace `SELECT 1` en la conexión cada 5 ms
- THEN ninguna sonda espera más de 300 ms

#### Escenario: nada escrito
- GIVEN el store de `a_dry_run_reports_without_writing`
- WHEN corre la corrida en seco
- THEN `memory`, `project`, `project_remote`, `memory_fts` y `sqlite_sequence` quedan iguales y no hay snapshot

**Tests:** `a_dry_run_leaves_the_connection_to_other_calls` y `a_dry_run_reports_without_writing` (`import.rs`).

## ID2. El planificador decide lo mismo que la pasada SQL

El `Ledger` que da el planificador MUST ser igual al que da la pasada SQL sobre el mismo store y el mismo archivo. Esa pasada es `place_rows` y `write_links` sobre `SqlSeat`, en una transacción que se suelta, y hoy sólo la corre la prueba diferencial. La igualdad incluye las ubicaciones con sus ids, los agentes de las filas insertadas, los proyectos tomados, las líneas inválidas, los enlaces perdidos, lo que se encontró de cada nota y las filas ausentes. Las reglas MUST estar escritas una vez, sobre un asiento que implementan la transacción y el planificador. Los ids que asigna MUST seguir a `sqlite_sequence`, como `AUTOINCREMENT`, y no a `MAX(id)`. Entre dos filas vivas del mismo texto y la misma fecha, el gemelo MUST ser la de menor id, en los dos.

#### Escenario: la tabla
- GIVEN cada uno de estos stores y archivos:
  - un export de clawg en un store con una fila;
  - el mismo, importado por segunda vez;
  - con un gemelo vivo de otro agente;
  - con una clave tomada;
  - con `--agent`;
  - las notas de muestra de otra herramienta, primero y por segunda vez;
  - una nota reescrita en la fuente, más nueva y más vieja;
  - una nota borrada acá y otra borrada en la fuente;
  - dos notas de un mismo origen;
  - un texto repetido en el archivo;
  - una clave repetida en el archivo;
  - un proyecto que tomó otra raíz;
  - dos filas vivas del mismo texto y la misma fecha;
  - un store con la última fila purgada;
  - un export cuyos enlaces se perdieron entre transacciones.
- WHEN se decide con la pasada SQL y con el planificador
- THEN los dos `Ledger` son iguales

#### Escenario: después de una purga
- GIVEN un store cuya fila de mayor id se borró, así que `sqlite_sequence` queda por encima de `MAX(id)`
- WHEN corre la corrida en seco y después la real
- THEN los dos reportes coinciden salvo `dry_run` y el snapshot

**Tests:** `the_planner_decides_as_the_sql_pass` y `a_dry_run_after_a_purge_reports_the_ids_of_the_import` (`import.rs`), y las pruebas de import de `import.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`.

## ID3. Un import soltado deja de planificar

Cuando se suelta el futuro de un import mientras el planificador corre (un apagado que corta el pedido), el planificador MUST terminar antes del próximo lote de filas sin decidir el resto.

**Test:** `a_stopped_planner_decides_nothing` (`import.rs`).

## IU1. Cada import que escribe tiene un número y un diario

Un import que escribe algo MUST quedar en `memory_import` con un número (`MAX + 1`), su inicio, su fuente (`clawg` o `engram`) y su snapshot `pre-import`, y su reporte MUST traer ese número en `import_id`. En la misma transacción que cada escritura MUST quedar en el diario:

- cada fila insertada (`memory_import_row`), con `update_count`, `deleted_at` y `supersede_cause` tal como se insertó;
- cada fila que `REVISED_SQL` reemplazó (`memory_import_supersede`), con su `status`, `superseded_by` y `supersede_cause` previos;
- en `memory_import_extra`, sólo si la escritura agregó la fila (`changes() = 1`): cada vínculo rellenado en una fila que el import no insertó (`fill`: fila, columna y valor), cada par (`pair`, con el `state` y el `verdict` escritos), cada derivación de una conclusión que no insertó (`derivation`), cada pieza de un save de una fila que no insertó (`piece`), cada raíz (`project`) y cada remoto (`remote`).

Y en `memory_import_registry` (V31), en la misma transacción: cada proyecto que creó (`project`), cada proyecto borrado que hizo volver con el `deleted_at` que tenía (`revived`), cada sugerencia que insertó (`suggestion`) y cada declaración que escribió (`subfolder`) ([`specs/project`](../project/spec.md) PL63).

`finished_at` MUST escribirse en la transacción de los vínculos. Un dry run, y un import que no escribe nada, MUST NOT tener número (`import_id: null`) ni cabecera. Las tablas del diario son V26 ([`specs/storage`](../storage/spec.md) IU6).

#### Escenario: número y cabecera
- GIVEN un store con una fila
- WHEN se importa un export de clawg y después el mismo archivo otra vez
- THEN el primero da `import_id = 1` con su cabecera terminada y una fila de diario por fila insertada, y el segundo da `null` y no agrega cabecera

**Tests:** `an_import_is_numbered_and_journaled` y `a_dry_run_and_an_import_that_adds_nothing_have_no_number` (`import.rs`). Las pruebas de import de `import.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs` que comparan un dry run con el real suman `import_id` a lo que difiere.

## IU2. Deshacer devuelve el store al estado anterior

`undo_import(N)` MUST borrar de verdad cada fila insertada por N que el criterio de IU3 retira (la fila, su vector, sus enlaces de `memory_piece`, sus derivaciones y sus pares), MUST revertir lo que N cambió en filas previas que sigue como N lo dejó y MUST sacar del diario lo que retiró o revirtió:

- una fila que `REVISED_SQL` reemplazó vuelve a su estado previo si sigue `superseded` por la misma fila y esa fila se retira o ya no está;
- un vínculo rellenado vuelve a `NULL` si conserva el valor escrito;
- un par, una derivación o un save agregados se quitan si ninguna fila que se queda los sostiene, y el par sólo si conserva su `state` y su `verdict`;
- una raíz agregada se quita si ninguna fila queda con su id, no tiene `index_docs` ni documentos y ningún remoto que se queda la nombra; un remoto agregado, si ninguna fila queda con su proyecto y ninguna raíz que se queda tiene ese id;
- lo anotado en `memory_import_registry`, en la última transacción y en este orden: cada declaración que sigue llevando al proyecto que escribió N, cada sugerencia que sigue sin respuesta, cada proyecto vuelto que sigue vivo, sin revisar y sin filas vivas, cuyo borrado sigue abierto (queda otra vez borrado con su `deleted_at`), y cada proyecto creado al que ya nada nombra y sigue sin revisar. Lo que queda sale en `registry.kept` con su motivo (`in_use`, `answered`, `reviewed`, `changed`), y vuelve a pesarse en el próximo `--undo` ([`specs/project`](../project/spec.md) PL63). Un import anterior a V31 no tiene estas anotaciones.

Antes de escribir, si hay algo que deshacer, MUST escribir un snapshot `pre-undo`. El snapshot `pre-import` MUST NOT tocarse.

Lo que lee del store MUST cargarse en llamadas de como mucho `LOAD_BATCH` búsquedas, y las filas que retira MUST borrarse en transacciones de como mucho `WRITE_BATCH` filas, así ninguna llamada a la conexión dura con el import. Cada lectura y cada borrado por fila MUST buscar por un índice: su `EXPLAIN QUERY PLAN` nombra el suyo en una línea `SEARCH` y no recorre ninguna tabla (el vector se borra por el módulo de vec0).

#### Escenario: un archivo de clawg ida y vuelta
- GIVEN un store con una fila y el export de `seed`
- WHEN se importa el export y se deshace el import
- THEN `memory`, `memory_vectors`, `memory_fts`, `memory_piece`, `memory_derivation`, `memory_conflict`, `project` y `project_remote` quedan como antes de importar, y un nuevo import del mismo archivo inserta las mismas filas con otro número

#### Escenario: un export de otra herramienta ida y vuelta
- GIVEN un store con una fila y notas de otra herramienta con relaciones entre ellas y una nota larga
- WHEN se importan y se deshace el import
- THEN el store queda como antes, pares incluidos

#### Escenario: una reimportación más nueva
- GIVEN notas importadas y una reimportación de una nota reescrita en la fuente, que reemplaza a la guardada
- WHEN se deshace la reimportación
- THEN la versión guardada vuelve a `active` sin `superseded_by` ni `supersede_cause`, y deshacer después el primer import retira todo

#### Escenario: un vínculo rellenado
- GIVEN un import cuyos vínculos se perdieron y un segundo import del mismo archivo que los rellena
- WHEN se deshace el segundo
- THEN los vínculos vuelven a vacíos; con un valor cambiado a mano después, ese queda

#### Escenario: lo guardado después
- GIVEN un import y un save posterior que el detector empareja con una fila previa al import
- WHEN se deshace el import
- THEN el save y su par siguen

**Tests:** `undoing_a_clawg_import_gives_the_store_back`, `undoing_a_foreign_import_gives_the_store_back`, `undoing_a_rewrite_brings_the_stored_version_back`, `undoing_fills_empties_the_links_it_filled`, `what_was_saved_after_stays`, `the_undo_lookups_seek_an_index`, `undoing_an_import_takes_back_what_it_added_to_the_registry`, `what_an_import_added_to_the_registry_stays_once_used` y `undoing_an_import_deletes_again_the_project_it_brought_back` (`crates/clawg-core/src/memory/portability/undo.rs`).

## IU3. Sólo se retira lo que sigue como el import lo dejó

Una fila insertada por N MUST quedarse, nombrada con su motivo en `kept`, si:

- `edited`: su `update_count` difiere del anotado;
- `deleted` o `restored`: su `deleted_at` difiere del anotado;
- `judged`: su `supersede_cause` pasó a `verdict`, o un par suyo difiere del que escribió N, o tiene un veredicto y N no lo escribió;
- `referenced`: la nombra una fila que no se retira, por `superseded_by`, `revived_from`, `disputed_against` o como fuente en `memory_derivation`, sin contar los vínculos que N escribió en filas previas y siguen como los dejó;
- `tied`: es pieza de un save que tiene una fila de N que se queda.

La decisión MUST ser una función pura sobre el diario y el estado leído, iterada hasta que no cambie. MUST NOT contar el proyecto, el `scope`, el `tier`, los accesos, `last_modified_at` ni un `supersede_cause = 'revision'`. Una fila que ya no está (purgada) MUST salir del diario sin fallar. Cada transacción de borrado MUST volver a leer `update_count`, `deleted_at` y `supersede_cause` de sus filas y conservar las que cambiaron desde la decisión.

#### Escenario: el criterio
- GIVEN un import y, después, una fila editada, una borrada, una restaurada, una con un par juzgado, una que es fuente de una conclusión, una movida de proyecto, una accedida, una bajada de `tier` y una reemplazada por un save con su clave
- WHEN se deshace el import
- THEN las cinco primeras se quedan con su motivo, y las otras se retiran

**Tests:** `decide_keeps_what_changed_and_what_is_named` (unidad, cada motivo y la iteración), `the_rows_someone_changed_stay` y `a_purged_row_does_not_stop_the_undo` (`undo.rs`).

## IU4. Deshacer es explícito, repetible y se puede ensayar

Un `import_id` sin cabecera MUST ser `UnknownImport` (`404`); un import que este proceso está escribiendo MUST ser `Running` (`409`). Deshacer dos veces MUST NOT cambiar nada la segunda vez. Con `dry_run` MUST reportar lo mismo que el real sin escribir nada, ni snapshot.

**Tests:** `an_undo_twice_changes_nothing_the_second_time`, `an_unknown_or_running_import_is_refused` y `a_dry_run_undo_reports_without_writing` (`undo.rs`).

## IU5. La puerta del daemon y la del CLI

`POST /v1/import/undo` (`import_id`, `dry_run`) MUST responder el `UndoReport` sin pedir el embedder. `clawg import --undo N [--dry-run]` MUST mandarlo y decir lo que hizo en el idioma de la preferencia; `--undo` MUST NOT aceptar un archivo, `--from`, `--agent`, `--map-root` ni `--project-map`, y `clawg import` sin archivo ni `--undo` MUST ser un error de uso. La salida de un import con número MUST nombrar el comando que lo deshace.

**Tests:** `import_takes_a_file_or_an_undo` (`crates/clawg-cli/src/cli.rs`); `render_undo_names_what_it_took_back_and_what_stayed`, `the_undo_report_speaks_spanish` e `import_report_lists_counts_and_invalid_lines`, con el deshacer contra un daemon de prueba (`crates/clawg-cli/src/import.rs`); `an_import_is_undone_by_its_number` (`crates/clawg-cli/tests/api.rs`), también sin embedder.

## IT1. Cada fase de una importación se registra al terminar

Una importación MUST registrar cada fase que corre al terminarla, una vez y en el orden en que corre, con una línea de `info` en el log del daemon (`import: <fase> …`). Las fases son:

| Fase | Qué mide | Detalle |
|---|---|---|
| `read` | el handler lee el archivo (`format::parse` o `engram::read`) | — |
| `plan` | los ids que daría el registro de cada raíz | llamadas |
| `prepare` | export de clawg: analizar, enmascarar y armar las filas | filas |
| `translate` | export de otra herramienta: traducir las notas y armar las filas | filas |
| `load` | lo que la pasada de decisión lee del store | filas leídas, llamadas |
| `decide` | la pasada de decisión | filas |
| `embed` | los vectores de las filas a insertar | filas, tandas, filas/s, la tanda más lenta |
| `snapshot` | el snapshot `pre-import` | — |
| `write` | los lotes de escritura | filas, llamadas |
| `links` | los vínculos y el cierre del diario | llamadas |

Un export de clawg se analiza antes de `plan` y se arma después; las dos partes suman en `prepare`, que se registra una vez, después de `plan`. Un dry run MUST terminar en `decide`. Ninguna línea MUST llevar texto de una fila, un id, una clave ni una ruta. Una importación que falla deja las líneas de las fases que terminó.

En el daemon, cada línea de fase MUST terminar con `private N MiB`: la memoria privada del proceso al terminar la fase, leída con la sonda que le da el handler (`Timing::private_bytes`; `ImportTimings::with_memory`, `timing.rs:152`; `import_memories`, `crates/clawg-cli/src/daemon/api.rs:1900`). Así se separa lo que retiene la importación de lo que ocupa el modelo cargado debajo. Sin sonda, como en las pruebas de `import.rs`, no se agrega nada. El resumen (IT3) no la lleva.

```
import: embed 1142.0 s (51068 rows, 1596 batches, 44.7 rows/s, slowest 2300 ms, private 812.0 MiB)
```

#### Escenario: una importación real de otra herramienta
- GIVEN un store y las notas de muestra de otra herramienta
- WHEN se importan con `FakeEmbedder`
- THEN quedan registradas `plan`, `translate`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`, una vez cada una y en ese orden
- AND las filas de `embed` son las insertadas no borradas del reporte
- AND las llamadas de `write` son los lotes de `note_batches`

#### Escenario: un export de clawg
- GIVEN el export del store de prueba
- WHEN se importa
- THEN las fases son `plan`, `prepare`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`

#### Escenario: un dry run
- GIVEN las mismas notas
- WHEN corre `--dry-run`
- THEN las fases son `plan`, `translate`, `load` y `decide`

#### Escenario: la memoria de cada fase
- GIVEN un `ImportTimings` con sonda
- WHEN se registran dos fases
- THEN las dos llevan la memoria y la línea de la primera termina en `(private 1.0 MiB)`
- AND sin sonda, la fase no la lleva

**Tests:** `an_import_times_each_step_once_in_order`, `a_clawg_file_is_prepared_rather_than_translated` y `a_dry_run_times_only_what_it_runs` (`crates/clawg-core/src/memory/portability/import.rs`); `a_step_reads_as_its_numbers` y `each_step_carries_the_memory_as_it_ended_when_there_is_a_probe` (`crates/clawg-core/src/memory/portability/timing.rs`).

## IT2. La conexión: SQLite y la cola por separado

En cada fase que usa la conexión del store (`plan`, `load`, `write`, `links`), la línea MUST contar las llamadas y separar el tiempo dentro de la clausura (`in sqlite`) de lo que falta hasta que vuelve la llamada (`queued`). También MUST nombrar la llamada que más tiempo tuvo la conexión. La suma de los dos tiempos MUST NOT superar la duración de la fase. `load` MUST hacer una llamada por cada `LOAD_BATCH` búsquedas de cada conjunto, más la de las raíces. `snapshot` abre una conexión propia en un hilo bloqueante, así que sólo lleva su duración.

#### Escenario: las llamadas de `load`
- GIVEN un store vacío y un export de clawg de 600 filas de textos distintos, sin clave y sin origen
- WHEN se importa
- THEN `load` hace 4 llamadas (3 de textos y 1 de las raíces)
- AND en `load` y en `write`, `in sqlite` + `queued` ≤ la duración

**Tests:** `time_in_sqlite_never_exceeds_the_call` (`import.rs`).

## IT3. El resumen

Al terminar una importación con éxito, el daemon MUST registrar una línea con el total desde que el handler la empezó y la duración de cada fase en el orden de IT1, en segundos con un decimal (`import: done in …`). Un dry run MUST decirlo en esa línea (`import: dry run done in …`). Una importación que falla no tiene resumen.

#### Escenario: el texto
- GIVEN una estructura armada a mano con las fases de una importación real
- WHEN se formatean una fase y el resumen
- THEN dan, por ejemplo, `load 3.1 s (812 rows, 5 calls, 2.6 s in sqlite, 0.5 s queued, slowest 41 ms)` e `import: done in 1362.4 s: read 1.8, …, links 0.4`

**Tests:** `a_step_reads_as_its_numbers` y `the_summary_names_every_step` (`crates/clawg-core/src/memory/portability/timing.rs`).

## IT4. Medir no cambia la importación

Ni lo que una importación escribe ni su reporte MUST cambiar. Los tiempos van al log y no al reporte, porque la paridad entre el dry run y el real (R7) no admite un valor que cambia en cada corrida.

**Tests:** las pruebas de import de `import.rs`, `undo.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin cambiar ninguna aserción.

## ET1. Una importación embebe primero las filas de más tokens

La importación MUST mandar al embedder las filas que embebe (las que inserta y no están borradas) ordenadas por los tokens del texto que embebe cada una (`embedding_text` de su cabecera y su contenido), de más a menos. Dos filas con los mismos tokens MUST quedar en el orden del archivo. Los tokens MUST contarse con el tokenizer del daemon, el mismo del embedder, en un hilo bloqueante y en grupos, no todas las filas de una vez. Sin tokenizer, la cuenta MUST ser la del chunker sin él (un cuarto de los bytes, redondeado hacia arriba). Las llamadas al embedder MUST seguir siendo de a lo sumo `EMBED_BATCH` (32) textos. Vale igual para un export de clawg y para el de otra herramienta.

#### Escenario: el orden
- GIVEN filas de 3, 9, 3, 1 y 9 tokens, en ese orden
- WHEN se ordenan para embeber
- THEN el orden es la segunda, la quinta, la primera, la tercera y la cuarta

#### Escenario: una importación de otra herramienta
- GIVEN notas de largos mezclados, más que las de una llamada, con dos del mismo largo
- WHEN se importan con un embedder que registra cada llamada
- THEN los textos que recibe, una llamada después de otra, van de más a menos tokens
- AND ninguna llamada pasa de 32 textos
- AND las dos del mismo largo llegan en el orden del archivo

#### Escenario: un export de clawg
- GIVEN el export del store de prueba
- WHEN se importa con el mismo embedder
- THEN los textos que recibe van de más a menos tokens

#### Escenario: contar en grupo
- GIVEN un chunker con tokenizer y uno sin él
- WHEN cuentan varios textos juntos
- THEN dan, para cada texto, lo mismo que contarlo solo

**Tests:** `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order`, `an_import_of_another_tool_embeds_its_rows_longest_first` y `a_clawg_import_embeds_its_rows_longest_first` (`crates/clawg-core/src/memory/portability/import.rs`); `counting_together_counts_each_text_as_alone` (`crates/clawg-core/src/memory/chunker/token_splitter.rs`).

## ET2. El orden no cambia lo que se escribe

Cada fila importada MUST guardar el vector de su propio texto, cualquiera sea su lugar en el orden de ET1. Ni las filas que se escriben, ni sus ids, ni el reporte, ni el diario MUST cambiar con el orden. La escritura sigue por notas enteras, en el orden del archivo (`note_batches`).

#### Escenario: cada fila con su vector
- GIVEN notas de largos mezclados, con piezas de un guardado largo
- WHEN se importan con el embedder falso
- THEN el vector guardado de cada fila insertada y no borrada es el del embedder falso para el texto de su cabecera y su contenido

**Tests:** `each_imported_row_keeps_the_vector_of_its_own_text` (`import.rs`), y las pruebas de import de `import.rs`, `undo.rs`, `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin cambiar ninguna aserción.

## ET3. La medición de la fase de embeddings

Un test ignorado MUST medir la fase de embeddings sobre las filas que el export de otra herramienta le da a un store vacío. Con sólo el tokenizer, MUST decir cuántas veces los tokens reales y la atención corre el modelo en el orden del archivo, por bytes, por caracteres y por tokens, en las llamadas de la importación y las corridas del embedder. Con el modelo, sobre las primeras N filas, MUST decir cuánto tarda cada orden (la mediana de varias corridas alternadas), qué parte es tokenizar y rellenar y qué parte el pooling, y cuántos vectores cambian de bits entre órdenes y contra un texto embebido solo. Cada salida MUST decir el perfil de compilación, el sistema, las CPUs y sus extensiones. MUST NOT imprimir texto, ids ni rutas de las filas.

#### Escenario: la cuenta del relleno
- GIVEN corridas de largos conocidos
- WHEN se cuenta el relleno
- THEN cada corrida del embedder se rellena hasta su propio texto más largo, no hasta el de la llamada

**Tests:** `padding_counts_each_run_of_the_embedder_apart` y `the_embedding_phase_of_a_real_export` (`import.rs`, `#[ignore]`: necesita `CLAWG_E5_MODEL_DIR` y `CLAWG_IMPORT_EXPORT`, y opcionalmente `CLAWG_EMBED_ROWS` y `CLAWG_EMBED_RUNS`). Depende de la máquina, así que queda fuera de CI.
