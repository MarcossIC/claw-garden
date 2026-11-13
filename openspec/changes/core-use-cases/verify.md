# core-use-cases — Verificación

**Resultado:** la parte 1, el guardado completo, aplicada y verificada en Linux. La suite entera y los dos clippy pasan sobre el último commit de código. Ningún test de respuestas ni ninguno de los dos contratos del agente cambió. Queda lo que se lista en [«Pendiente»](#pendiente). Las partes 2 a 4 están en [`tasks.md`](tasks.md#partes-siguientes).

## Qué se corrió

Sobre el último commit de código (`refactor(core): parse and admit a save in the core, not in the daemon`), en Linux, con un único directorio de build (`target`):

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo; incluye los doctests |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos; es el núcleo sin `testing` ni `eval`, como lo construye el release |

Además, `cargo check -p clawg-cli` y `cargo check --workspace --all-targets` no dejan ningún warning.

## Los tests en rojo

Todos en `clawg-core`, sin router ni HTTP:

| Test | Qué se vio antes del código |
|---|---|
| Los diez de `crates/clawg-core/src/memory/ingest/save.rs` (R1, R2) | no compilaban: `SaveInput`, `NewSave`, `SaveRejected`, `check_operation_id` y los topes no existían |
| Los siete de `mod tests::admit` en `crates/clawg-core/src/runtime.rs` (R2 a R6) | no compilaban: `MemoryRuntime::admit` no existía, ni las variantes `MissingSources`, `ReplacedSources`, `EmbedderFailed` y `Store` |

`replaced_sources_name_what_replaced_them` y `missing_sources_are_the_ones_that_were_not_found` se escribieron con el primer grupo y se sacaron hasta tener `admit`, para que el formato compilara solo; volvieron con el segundo grupo, todavía en rojo.

## La caracterización

`each_refused_save_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`) se commiteó antes de tocar el handler (`test(daemon): pin the status and body of every refused save`) y pasó sobre el código de `develop`. Fija, con el cuerpo JSON entero:

- las siete reglas de formato de R1, cada una sola, y seis pares que fijan el orden;
- las fuentes que faltan (inexistente, borrada y oculta por `sharing.json`) y las reemplazadas, y que una que falta va antes que una reemplazada;
- el `503 embedder_failed` con `retryable` `false` y `true`, y que las fuentes se comprueban antes que el embedder;
- los tres `500` de un store cerrado: con `derived_from` (`memory backend channel error: source lookup: ConnectionClosed`), con un proyecto (`Memory DB connection error: ConnectionClosed`) y sin ninguno de los dos, en el outbox (`memory backend channel error: outbox_record: ConnectionClosed`).

Los tres textos del `500` son los que dio `develop`; el borrador del test tenía otros dos y se corrigieron con lo que respondió el código de ese momento, antes del commit. Después del cambio pasa sin haberse tocado.

## Las comprobaciones de D8

1. **Los tests de respuestas.** `git diff origin/develop -- crates/clawg-cli/tests crates/clawg-cli/src/mcp` no muestra nada: ni `tests/api.rs`, ni los tests del puente, ni `agent_contract.json` ni `agent_contract.es.json`. Los dos tests de contrato pasan sin `CLAWG_BLESS`.
2. **Los tests puros que se mudaron.** `content_is_checked_in_bytes`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `replaced_sources_name_what_replaced_them`, `missing_sources_are_the_ones_that_were_not_found` y `operation_ids_are_checked` pasan de `crates/clawg-cli/src/daemon/api.rs` a `save.rs` con el mismo nombre y los mismos literales. Donde comparaban `ApiError::status` con `400` comparan la variante, y donde comparaban `.message`, el `Display`. `replaced_sources_name_what_replaced_them` compara ahora el mensaje entero de `ReplacedSources` en lugar de la lista sola.
3. **El diff de `api.rs`.** Ningún `StatusCode` cambia. Los `ApiError` que se van son los de las funciones que se mudaron y los del cuerpo de `save_memory`; los que llegan son los tres brazos de `save_refused` (`bad_request`, `embedder_failed`, `internal`), con los mismos estados, y el `bad_request` de `requested_operation`, que antes venía de `check_operation_id`.

## El handler

| | Antes | Después |
|---|---:|---:|
| `save_memory` | 133 líneas (`api.rs:1420-1552`) | 65 líneas |
| líneas de producción de `api.rs` | 3.412 | 3.305 |
| funciones de producción de `api.rs` | 126 | 123 |
| usos de `bad_request` en producción | 68 | 61 |

`save_memory` ya no nombra `EmbedderStatus`, `existing_memory_ids`, `replaced_memory_ids`, `live_project`, `validate_topic_key` ni `MemoryEvent`. De `MemoryType` sólo usa `as_str`, para la etiqueta de la métrica de la sesión. `existing_memory_ids` y `replaced_memory_ids` pasaron a `pub(crate)`, porque ya no los usa otro crate.

## Lo que se resolvió distinto de lo escrito

- **Las tareas 3 y 4 son un commit.** Con el formato solo en el núcleo, el handler seguía comprobando las fuentes y armando el evento, y `NewSave` habría tenido que mostrar getters y dejar cambiar su proyecto desde otro crate hasta el commit siguiente ([tarea 3](tasks.md#3-el-guardado-en-el-núcleo-r1-a-r7)).
- **`Admitted` está en `save.rs`**, como dice D1, y no en `runtime.rs`; `admit` lo construye con sus campos `pub(crate)`.
- **`EmbedderFailed` tiene un `Display` propio** (`the embedder failed: <causa>`) que ningún cliente ve: el handler responde con `ApiError::embedder_failed`, que usa los campos.

## Pendiente

Lo que esta verificación no cubre:

- **Windows.** La suite y los dos clippy no se corrieron en Windows; el job `test (windows-latest)` del CI del PR lo hace. El cambio no agrega código `cfg(windows)` ni toca rutas de archivos, salvo `sharing.json`, que `admit` lee con `sharing::reader_in` igual que antes lo leía el handler.
- **El modelo real.** Ningún test corrió con el modelo real: el guardado se probó con `FakeEmbedder`. Un save con el embedder `loading` mientras el modelo se descarga de verdad (el `202 embedder_loading`) sólo se comprueba con un daemon de perfil `dev` que todavía no tiene el modelo; la decisión de no esperar es la misma de antes, ahora en `Admitted::waits`.
- **`rag-eval`.** No corre: el cambio no toca la recuperación ni `eval_api` ([D6](design.md#d6-eval_api-no-cambia)).
- **Al archivar:** consolidar R1 a R6 en `specs/ingest`, `specs/synthesis` y `specs/provenance` con ids nuevos (R7 es de conservación y no se consolida), y corregir las citas que nombran las funciones que se mudaron: `check_content` (`specs/ingest`, «El contenido: vacío o sobre el tope») y los tests puros de `crates/clawg-cli/src/daemon/api.rs` que cita `specs/synthesis`, que ahora están en `save.rs`. Subir la versión y `openspec/ROADMAP.md`.
