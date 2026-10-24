# import-linear — Verificación

## Windows (2026-10-24)

Sobre `f967979`, en `target-test`, con el compilador de `rust-toolchain.toml` (1.99.1):

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.

## RED antes del cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `the_import_lookups_seek_an_index`: `PRESENT_SQL in a project should seek idx_memory_dedup: ["SCAN memory"], 16 full scan steps`, y lo mismo sin proyecto, sobre el store de prueba de 17 filas. Las demás sentencias ya buscaban por su índice. |
| R2 | `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows`: `the index holds every row: CREATE INDEX idx_memory_dedup ON memory (COALESCE(project_id, ''), content_hash, created_at) WHERE deleted_at IS NULL AND content_hash IS NOT NULL` |
| R3 | `reinforce_lookup_uses_dedup_index` y `the_scope_lookup_uses_the_topic_index` pasaban antes y pasan después sin tocarlas. `TOPIC_HOLDERS_SQL` sigue en `idx_memory_topic` con el índice de dedup ya no parcial. |
| R4 | Las pruebas de `import.rs` pasan sin cambios, y el banco da el mismo informe antes y después (abajo). |
| R5 | El banco con el índice parcial: 3,80 veces (abajo). |

La primera corrida de la prueba de planes también marcó `LINKED_SQL`, porque su plan empieza por `SCAN CONSTANT ROW`. Esa es la fila única de un `SELECT EXISTS (…)` sin `FROM`, y la subconsulta busca por `idx_memory_piece_memory` con 0 pasos de recorrido completo. La regla quedó en «ninguna línea `SCAN` que recorra una tabla», y la spec dice lo mismo.

## Banco de escala

`twice_the_notes_import_in_at_most_three_times_the_time` en una compilación de depuración en Windows: 10 000 notas de una fila cada una y después 20 000, cada tanda en un store de prueba vacío con el embedder falso. Para correr la versión de antes del cambio, V25 se dejó un momento, sin commitearla, con la forma parcial de V20, y después se restauró.

| Índice | 10 000 notas | 20 000 notas | Cociente |
|---|---|---|---|
| Parcial (V20) | 15,97 s | 60,78 s | 3,80 |
| V25 | 2,11 s · 2,13 s · 2,08 s | 4,45 s · 4,17 s · 4,94 s | 2,11 · 1,96 · 2,38 |
| V25 y `prepare_cached` | 1,79 s · 1,77 s · 1,79 s | 3,55 s · 3,56 s · 3,57 s | 1,99 · 2,01 · 1,99 |

En todas las corridas el informe (sin la ruta del snapshot) tiene el mismo SHA-256: `cf766787…` con 10 000 notas y `800e55a3…` con 20 000. Sólo con el índice, las 20 000 notas tardan ~14 veces menos; sumando `prepare_cached`, ~17 veces menos.

**`prepare_cached`.** Con el índice puesto, preparar `PRESENT_SQL` y `TWIN_SQL` en cada fila costaba ~16 % del banco (2,12 s contra 1,78 s con 10 000 notas, repetido en tres corridas). Con la regla de la propuesta, eso pidió el cambio, que se hizo en un commit propio.

Estos tiempos son de depuración y no se trasladan a release; lo que vale es la comparación entre corridas de la misma compilación. El banco no mide la parte de la importación real que este cambio no toca: la traducción de notas largas y los embeddings del modelo.

## Importación real en el perfil de desarrollo (2026-10-24)

Se hizo con autorización explícita, con el binario de depuración de `f967979` (`target-test`, que lleva el feature `testing`, como el de la medición de origen) y el modelo real. El perfil `dev` corrió en un home temporal (`CLAWG_TEST_HOME`) sobre una **copia** del snapshot `pre-import` que había dejado la medición de origen, con una copia de su modelo. Así el punto de partida fue el mismo y `~/.clawg-dev`, que ya tiene esa importación, no se tocó. La exportación nueva (`engram export --all`) dio las mismas 3 814 notas y 52 080 filas que la de origen. Se guardó fuera del repositorio y, como tiene el texto sin enmascarar, se borró al terminar, junto con el home temporal. Por la longitud de esa ruta el daemon sirvió sólo por TCP.

| | Origen (0.56.0, índice parcial) | Con este cambio |
|---|---|---|
| `--dry-run` | 910 s | 70 s |
| Importación | 3 078 s | 1 218 s |
| Filas importadas | 51 996 de 52 080 | 51 996 de 52 080 |
| Peticiones al store durante la importación (`/health`, listado de una fila, búsqueda; una ronda cada 20 s, con tope de 15 s) | ~800 s en que todas vencían; búsquedas de más de 15 s mientras escribía | ninguna venció: el listado tardó como mucho 1,7 s y la búsqueda 1,3 s |

El informe de la importación es idéntico al de su dry run. Las 51 996 filas importadas son las mismas que dejó la medición de origen (comparadas con su snapshot `pre-move`, sin ids): agente, proyecto, hash, fecha, estado, tipo, clave, borrado, causa, origen, proyecto de origen, idioma e importancia dan el mismo SHA-256, y coinciden las piezas (51 068) y los pares (141). Sólo 70 filas difieren, y sólo en los accesos: 63 las tocaron las búsquedas que se hicieron después de la importación de origen, y 7 las tocó la sonda de esta corrida.

Lo que queda de los 1 218 s es casi todo el embedding de las filas con el modelo real (~19 min) y la traducción de las notas (~1 min), que este cambio no toca (embed-throughput, 85). El pase de decisión sigue reteniendo la única conexión mientras corre: en el dry run, un listado esperó 8,8 s detrás de él. Antes eran unos 800 s. Quitar esa espera es de import-decision-pass (82).

## Pendiente

- El CI del pull request en Linux y Windows.
