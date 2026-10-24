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

## Pendiente

- La importación real en el perfil de desarrollo, con una exportación completa y compilación de depuración, para comparar con los 3 078 s. Necesita autorización explícita, porque usa `~/.clawg-dev` y una exportación con el texto sin enmascarar, que se guarda fuera del repositorio y se borra al terminar.
- El CI del pull request en Linux y Windows.
