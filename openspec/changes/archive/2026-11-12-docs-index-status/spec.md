# docs-index-status — Especificación

El daemon muestra, por proyecto, el refresco del índice que corre y el último que terminó, en `GET /health` y en `clawg status`, y escribe una línea de log cuando un refresco conoce su plan. Lo que el índice guarda, lo que una búsqueda contesta y lo que el agente lee no cambian (IS7).

## Definiciones

- **Refresco:** una pasada de `refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs`) sobre un proyecto.
- **Plan:** qué archivos el refresco vuelve a indexar, cuáles deja y cuáles saca (`plan_refresh`). Se conoce después de recorrer y leer el árbol.
- **Disparador:** quién empezó el refresco. `registration`: un registro del proyecto que pide indexar (`DocIndex::begin`). `search`: una búsqueda sobre un índice que lleva más de `REFRESH_INTERVAL` sin refrescar (`DocIndex::begin_if_due`).
- **Archivo terminado:** un archivo del plan que el refresco ya escribió en el índice, o que dejó como estaba porque no pudo leerlo o porque uno de sus vectores no sirvió.

## IS1. El índice guarda en memoria el refresco que corre

Mientras un refresco de un proyecto está tomado, `DocIndex` MUST guardar su disparador y la hora en que empezó, en milisegundos Unix. Cuando el refresco conoce su plan MUST guardar cuántos archivos va a indexar, y MUST sumar uno por cada archivo terminado. Antes del plan, el total MUST NOT tener valor: el refresco todavía recorre el árbol.

Cuando el refresco se suelta, termine como termine, el índice MUST dejar de mostrarlo como en curso.

#### Escenario: un refresco a mitad
- GIVEN un proyecto con dos archivos por indexar y un embedder que deja pasar el primero y retiene el segundo
- WHEN se lee el estado del proyecto
- THEN dice que corre, con 1 archivo hecho de 2
- AND su disparador y su hora de comienzo

#### Escenario: los archivos que no se escriben también cuentan
- GIVEN un plan de dos archivos, uno de ellos con un vector que no sirve
- WHEN el refresco termina
- THEN avisó dos archivos terminados

**Tests:** `the_status_follows_a_refresh_from_its_plan_to_its_end`, `a_refresh_reports_its_plan_and_each_file_it_finishes` y `a_claimed_refresh_shows_as_running_until_it_is_released` (`crates/clawg-core/src/memory/project_docs/mod.rs`).

## IS2. El índice guarda en memoria el último refresco que terminó

Cuando un refresco termina, `DocIndex` MUST guardar, antes de soltarlo, su disparador, cuándo empezó y cuándo terminó, y

- si terminó bien, lo que hizo: archivos en disco, reindexados, sin cambios, sacados, salteados y dejados como estaban, y pasajes escritos y reutilizados;
- si falló, su causa, sin cuentas.

Un refresco que termina reemplaza al anterior: uno que termina bien borra la causa del que falló. `DocIndex::last_error`, que alimenta el `warning` de una búsqueda, MUST seguir diciendo lo mismo que hoy.

Un refresco tomado que se suelta sin haber corrido MUST NOT cambiar el último.

#### Escenario: termina bien
- GIVEN un refresco que indexó 2 archivos
- WHEN se lee el estado
- THEN no corre, y el último tiene `finished_at_ms` ≥ `started_at_ms` y `indexed` 2, sin `error`

#### Escenario: falla
- GIVEN un refresco cuyo embedder falla
- WHEN se lee el estado
- THEN el último tiene la causa en `error` y no tiene cuentas

**Tests:** `the_status_follows_a_refresh_from_its_plan_to_its_end`, `a_failed_refresh_leaves_its_cause_until_one_succeeds` y `a_claimed_refresh_shows_as_running_until_it_is_released` (`mod.rs`).

## IS3. `/health` lleva `docs_index`, de memoria

`GET /health` MUST llevar `docs_index`: una lista con una entrada por cada proyecto que tuvo un refresco tomado en esta ejecución del daemon, ordenada por `project_id`, y vacía si no hubo ninguno. La lista MUST salir de la memoria del proceso, sin usar la conexión del store: está aunque `ingest` falte.

Cada entrada:

| Campo | Qué es |
|---|---|
| `project_id` | el proyecto |
| `indexing` | si un refresco corre ahora |
| `current` | el refresco que corre; falta si no corre ninguno |
| `current.trigger` | `registration` o `search` |
| `current.started_at_ms` | cuándo empezó |
| `current.files_done` | archivos terminados del plan |
| `current.files_total` | archivos del plan; falta mientras se recorre el árbol |
| `last` | el último refresco que terminó; falta si ninguno terminó |
| `last.trigger`, `last.started_at_ms`, `last.finished_at_ms` | quién lo disparó, cuándo empezó y cuándo terminó |
| `last.report` | si terminó bien: `docs`, `indexed`, `unchanged`, `removed`, `skipped`, `failed`, `passages`, `reused` |
| `last.error` | si falló: la causa |

```json
"docs_index": [
  {
    "project_id": "claw-garden-3f9a",
    "indexing": true,
    "current": { "trigger": "search", "started_at_ms": 1794000000000, "files_done": 12, "files_total": 36 },
    "last": {
      "trigger": "registration",
      "started_at_ms": 1793999000000,
      "finished_at_ms": 1793999042400,
      "report": { "docs": 901, "indexed": 36, "unchanged": 865, "removed": 0, "skipped": 0, "failed": 0, "passages": 412, "reused": 120 }
    }
  }
]
```

#### Escenario: un refresco detenido a mitad
- GIVEN un proyecto con dos archivos por indexar, cuyo refresco escribió el primero y espera al embedder por el segundo
- WHEN se pide `/health`
- THEN `docs_index` tiene una entrada con ese `project_id`, `indexing` `true`, `current.files_done` 1 y `current.files_total` 2

#### Escenario: la conexión está ocupada
- GIVEN ese refresco, y la única conexión del store tomada por otra llamada
- WHEN se pide `/health`
- THEN contesta sin esperar la conexión, sin `ingest` y con la entrada del proyecto

#### Escenario: al terminar
- GIVEN ese refresco, al que el embedder deja terminar
- WHEN se pide `/health`
- THEN la entrada tiene `indexing` `false`, no tiene `current`, y `last` tiene `finished_at_ms` y `report.indexed` 2

#### Escenario: un fallo
- GIVEN un proyecto cuyo refresco falla en el embedder
- WHEN se pide `/health`
- THEN `last.error` tiene la causa y `last` no tiene `report`

#### Escenario: ningún refresco
- GIVEN un daemon que no refrescó ningún proyecto
- WHEN se pide `/health`
- THEN `docs_index` es una lista vacía

**Tests:** `an_index_status_serializes_what_health_shows` (`mod.rs`); `health_names_the_project_whose_refresh_runs_and_how_far_it_is`, `health_shows_a_running_refresh_without_the_connection`, `health_says_when_a_refresh_ended_and_what_it_did`, `health_keeps_the_cause_of_a_failed_refresh` y `health_lists_no_docs_index_before_any_refresh` (`crates/clawg-cli/tests/api.rs`).

## IS4. Una línea de log cuando el refresco conoce su plan

Cuando un refresco del daemon conoce su plan, y antes de indexar el primer archivo, el daemon MUST escribir en su log, a nivel `info`:

```
docs refresh of <project_id>: started by <registration|search>: <n> to index, <n> unchanged, <n> to remove
```

Las dos líneas del final (`indexed the docs of …` y `docs refresh of …: done in …`) no cambian. Un refresco que falla antes del plan (una raíz que ya no existe) no escribe la línea: su fallo ya se escribe.

#### Escenario: un registro que pide indexar
- GIVEN un daemon y una carpeta con un `README.md`
- WHEN se registra la carpeta con `index_docs`
- THEN el log tiene `docs refresh of <id>: started by registration: 1 to index, 0 unchanged, 0 to remove`

**Tests:** `the_docs_start_line_names_the_project_the_trigger_and_the_plan` (`crates/clawg-cli/src/daemon/api.rs`) y `status_shows_the_docs_index_and_the_log_says_when_a_refresh_starts` (`crates/clawg-cli/tests/binary.rs`).

## IS5. `clawg status` muestra `docs_index`

El bloque `health` de `clawg status` MUST llevar `docs_index` como lo manda el daemon. Contra un daemon que no lo manda, MUST leer su `/health` sin fallar y mostrar una lista vacía. El cliente MUST pasar cada entrada como llegó, sin interpretarla: una entrada con una forma que no conoce no le impide leer `/health`.

#### Escenario: después de un refresco
- GIVEN un daemon que indexó una carpeta registrada
- WHEN corre `clawg status`
- THEN `health.docs_index` tiene la entrada del proyecto, con `last.report.indexed` 1

**Tests:** `the_status_report_shows_the_docs_index` (`crates/clawg-cli/src/status.rs`), `a_health_without_a_docs_index_still_parses` (`crates/clawg-cli/src/client.rs`) y `status_shows_the_docs_index_and_the_log_says_when_a_refresh_starts` (`binary.rs`).

## IS6. Se cuentan archivos

El avance MUST contarse en archivos del plan, no en pasajes. `files_total` es la cantidad de archivos por reindexar; los que el plan deja como están no entran, y sacar los que ya no existen no suma.

**Tests:** los de IS1.

## IS7. Lo que no cambia

- El esquema: MUST NOT haber una migración nueva.
- `POST /v1/projects/docs/search`: sus campos `indexing` y `warning`, y cuándo dispara y cuánto espera un refresco.
- El aviso de `search_project` mientras se indexa, y el contrato fijado del agente (`agent_contract.json`, `agent_contract.es.json`), que MUST pasar sin regenerarse.
- Las métricas `docs.refresh.duration` y `docs.refresh.files`.
- Los otros campos de `/health`.

**Tests:** sin cambios: `search_project_warns_while_indexing` (`crates/clawg-cli/src/mcp/tools.rs`), `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs`), y los de la búsqueda de docs en `crates/clawg-cli/tests/api.rs`.

## Specs que se consolidan al archivar

- IS1, IS2, IS4 e IS6, en [`specs/project-docs`](../../specs/project-docs/spec.md), junto a «Refresco incremental» e «IC2. La línea del daemon».
- IS3 e IS5, en [`specs/daemon-access`](../../specs/daemon-access/spec.md), junto a IJ12 (el bloque `import` de `/health`).
