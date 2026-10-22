# docs-refresh-background — Especificación

Cambia [`specs/project-docs`](../../specs/project-docs/spec.md): «Refresco incremental», «Búsqueda», «Registrar una raíz aceptada la indexa en segundo plano», «Invariantes» y «Límites conocidos».

## R1. La búsqueda no espera un refresco más que un plazo corto

`POST /v1/projects/docs/search`, cuando el último refresco del proyecto empezó hace `REFRESH_INTERVAL` (60 s) o más, o nunca empezó, y no hay uno en curso, MUST tomar la marca del refresco y lanzarlo en una tarea propia, que MUST seguir hasta el final aunque la petición se cancele. La búsqueda MUST esperarlo a lo sumo `REFRESH_GRACE` (3 s), o `FIRST_INDEX_WAIT` (20 s) si el índice del proyecto está vacío, y después MUST buscar en lo que haya indexado. `indexing` MUST ser `true` si el refresco sigue en curso al responder.

Una búsqueda que encuentra un refresco en curso que no lanzó ella MUST esperarlo sólo si el índice está vacío (`FIRST_INDEX_WAIT`, como hoy); con un índice armado contesta enseguida.

#### Escenario: muchos docs cambiados
- GIVEN un proyecto indexado cuyo `a.md` cambió y al que se le agregó `b.md`, y un embedder que no termina de embeber pasajes
- WHEN se buscan sus docs y el proyecto está vencido
- THEN la respuesta llega antes de `REFRESH_GRACE` más un margen, con `indexing: true` y el pasaje viejo de `a.md`
- AND al soltar el embedder el refresco termina, y la búsqueda siguiente trae el pasaje nuevo de `a.md`, `docs: 2` e `indexing: false`

#### Escenario: una edición chica
- GIVEN un proyecto indexado cuyo `a.md` cambió, con el embedder falso
- WHEN se buscan sus docs y el proyecto está vencido
- THEN la misma respuesta trae el pasaje nuevo e `indexing: false`

#### Escenario: la petición se va
- GIVEN el proyecto del primer escenario, con el embedder detenido
- WHEN el cliente corta la búsqueda que lanzó el refresco antes de la respuesta, y después se suelta el embedder
- THEN el refresco termina y el índice tiene los dos docs

**Tests:** `a_search_answers_from_the_index_while_a_long_refresh_runs`, `a_small_edit_is_found_by_the_search_that_refreshes` y `the_refresh_outlives_the_search_that_started_it` en `crates/clawg-cli/tests/api.rs`; `begin_if_due_claims_only_a_due_idle_project` en `project_docs/mod.rs`. Siguen pasando `docs_search_indexes_on_first_use_and_answers` y `a_search_during_the_first_index_waits_for_it`.

## R2. El error del último refresco llega al modelo

`DocIndex` MUST guardar, por proyecto, el error del último refresco que terminó, lanzado por una búsqueda o por un registro, y MUST borrarlo cuando un refresco termina bien. Un refresco que no corrió porque había otro en curso, o que se canceló, MUST NOT cambiarlo. Cada búsqueda MUST traerlo en `warning` mientras esté guardado. Un refresco fallido MUST NOT impedir la búsqueda sobre lo que ya está indexado, como hoy, y el `warning` MUST seguir siendo el texto del error.

#### Escenario: refresco de fondo fallido
- GIVEN una raíz con un `README.md` y un embedder que falla al embeber pasajes y no consultas
- WHEN se registra con `index_docs: true`, el refresco de fondo termina, y se buscan sus docs
- THEN la respuesta es 200 con `warning` que nombra el error del embedder

#### Escenario: un refresco bueno lo borra
- GIVEN un proyecto cuyo último refresco falló
- WHEN otro refresco termina bien
- THEN `last_error` del proyecto queda vacío

**Tests:** `a_background_refresh_that_failed_warns_the_next_search` en `crates/clawg-cli/tests/api.rs`; `the_last_error_lasts_until_a_refresh_succeeds` en `project_docs/mod.rs`. Sigue pasando `a_root_that_vanished_still_answers_from_its_index`.

## R3. Un doc con un vector inservible no frena a los demás

Si el embedder devuelve para un doc un vector que `to_unit` rechaza (otra dimensión, un valor no finito o norma cero), `refresh` MUST dejar ese doc como estaba, MUST anotarlo en `RefreshReport::failed` con su ruta y la causa, y MUST seguir con los demás docs del plan y con `remove_docs`. El siguiente refresco MUST reintentarlo: su huella vieja no coincide con el disco. El daemon MUST registrar cada doc salteado como `warn`.

Si el embedder falla (`DocIndexError::Embed`), devuelve otra cantidad de vectores que de pasajes (`DocIndexError::VectorCount`) o falla la tienda, el refresco MUST terminar con el error, como hoy.

`rag-eval --docs` (`EvalDocIndex::index`) MUST fallar si un doc quedó en `failed`, nombrándolo.

#### Escenario: un doc malo
- GIVEN `a.md` cuyo texto el embedder convierte en un vector con `NaN`, `b.md` sano y un `gone.md` indexado y borrado del disco
- WHEN se refresca
- THEN `b.md` queda indexado, `gone.md` pierde sus pasajes, `a.md` no tiene pasajes, y el reporte dice 1 reindexado, 1 quitado y `failed` con `a.md` y la causa

#### Escenario: cuenta distinta
- GIVEN un embedder que devuelve un vector menos que pasajes
- WHEN se refresca
- THEN el refresco falla con `VectorCount` y nada se indexa

**Tests:** `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_short_batch_still_aborts_the_refresh` en `project_docs/mod.rs`.
