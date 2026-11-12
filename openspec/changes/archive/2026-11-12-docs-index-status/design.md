# docs-index-status — Diseño

Las decisiones de fondo están en la [propuesta](proposal.md#decisiones-tomadas) y no se reabren. Acá van las de implementación, con lo que se descartó. Las citas son `ruta:línea` de `develop` en `0716645`.

## D1. El estado vive en `Progress`, dentro de `DocIndex`

`Progress` (`crates/clawg-core/src/memory/project_docs/mod.rs:219`) ya es el estado por proyecto, bajo el mutex de `DocIndex`, y ya decide si un refresco corre. Suma dos campos:

- `current`: el refresco tomado. Lo pone `begin` o `begin_if_due`, y lo saca el `Drop` de `RefreshGuard`. Reemplaza al booleano `running`: «corre» es que haya uno.
- `last`: el último que terminó. Lo escribe `RefreshGuard::run` bajo el mismo candado y antes de soltar el refresco, donde hoy escribe `last_error`. Reemplaza a `last_error`, que pasa a leerse de ahí.

`last_started` (un `Instant`) queda: decide cuándo toca el próximo refresco y no se muestra. Las horas que se muestran son del reloj de pared (`TimestampMs`), en milisegundos, como las del bloque `import`.

**Descartado:** un registro aparte en `AppCtx`, del lado de `clawg-cli`. Habría dos lugares que saben si un refresco corre, y el que lo sabe de verdad es el `RefreshGuard`: suelta el refresco aunque su tarea se corte.

**Descartado:** marcar como fallido un refresco que se suelta sin terminar. Un test toma un refresco y lo suelta sin correrlo, y `last_error` llega al agente como `warning` de la búsqueda: cambiaría lo que el agente lee.

## D2. El refresco avisa por una función, no conoce a `DocIndex`

`refresh` (`mod.rs:497`) recibe un observador, `&(dyn Fn(RefreshStep) + Sync)`, y le avisa dos cosas: el plan, una vez (`RefreshStep::Planned`), y cada archivo terminado (`RefreshStep::FileDone`). `RefreshGuard::run` le pasa uno que escribe en `Progress`; `clawg-eval`, que llama a `refresh` sin daemon, le pasa uno que no hace nada.

Un archivo termina en tres lugares, los mismos donde `refresh` y `write_queued` ya anotan su `DocTiming`: no se pudo leer, un vector suyo no sirvió, o se escribió. Los archivos esperan en la cola hasta que su tanda se embebe, así que `files_done` avanza de a tandas: es el avance real de lo que ya se puede buscar.

**Descartado:** que `refresh` reciba el `DocIndex`. La función es la misma para el daemon y para la evaluación, y hoy no sabe que existe un estado compartido.

**Descartado:** un canal. Nadie espera los avisos: son dos escrituras bajo un mutex que ya existe.

## D3. El disparador sale de quién tomó el refresco

Hay dos formas de tomar un refresco y cada una tiene un solo uso en el daemon: `begin` (`mod.rs:285`) desde el registro (`spawn_doc_refresh`, `crates/clawg-cli/src/daemon/api.rs:3193`) y `begin_if_due` (`mod.rs:299`) desde la búsqueda (`api.rs:3364`). Cada una anota su disparador (`RefreshTrigger::Registration`, `RefreshTrigger::Search`), sin un argumento nuevo.

**Descartado:** un argumento `trigger` en las dos. Cambiaría una veintena de llamadas de test para decir algo que la función ya sabe.

## D4. La línea de log la escribe el daemon

El log es del daemon, y `project_docs` no escribe ninguna línea hoy. `RefreshGuard::run` recibe una función que llama una vez con el plan; `refresh_docs` (`api.rs:3243`) le pasa la que escribe la línea, armada por una función pura (`refresh_start_line`) al lado de `refresh_line` y `refresh_timing_line`. El guard dice su disparador (`RefreshGuard::trigger`).

## D5. Un tipo para `/health`; el cliente pasa cada entrada como llegó

`IndexStatus`, con `Serialize`, vive en `project_docs` y lo usa `HealthBody`. `DocIndex::statuses` arma la lista con una sola toma del mutex, ordenada por `project_id`.

El `Health` del cliente lleva `docs_index` como `Vec<serde_json::Value>`: `clawg status` sólo lo imprime, y nadie más lo lee.

`RefreshReport` no se serializa tal cual: lleva los archivos dejados como estaban con su ruta y su causa, y en `/health` va sólo cuántos son (`RefreshCounts`). Las rutas ya están en el log.

`docs_index` va siempre, vacío si no hay nada: una lista vacía se lee sin preguntar si la clave existe. En el cliente tiene `default`, por un daemon anterior.

**Descartado:** el mismo tipo en el cliente, con `Deserialize`, como `OutboxSummary` para `ingest`. Todos los clientes leen `/health` con ese `Health` (el puente y los hooks también), y si un daemon más nuevo cambiara la forma de una entrada, un cliente anterior no podría leer `/health` y daría al daemon por caído. Con cada entrada sin interpretar, eso no puede pasar por este campo.

## D6. `/health` no toca el store por esto

`health` (`api.rs:1279`) espera hasta 250 ms por `ingest` y lo deja afuera si el store no contesta. `docs_index` se arma después, de memoria, así que está en los dos casos. No se suma una lectura de `project_doc_file`: una consulta que vence queda igual en la cola de la única conexión ([read-connections](../../read-connections/exploration.md)), y `/health` ya deja una.
