# save-bounds — Verificación

## Linux (2026-10-20)

Sobre `886e780`, en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) (`f9ff25a`, `eff4b57`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa después de bendecir; el único cambio del archivo es la descripción de `content` de `save_memory`.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`, un daemon `dev` en `--port 0`:

- `POST /v1/memories` con 65 537 bytes responde `400 {"error":"content is 65537 bytes; at most 65536 are accepted: split it into several saves, one fact each"}`;
- con 65 536 bytes responde `201 saved` con 228 memorias;
- `clawg status` queda con `ingest` en `pending: 0`, `failed: 0`, y `clawg stop` lo para.

## Medición del progreso

Un test temporal, no commiteado, en `worker.rs` (mod tests): un store de prueba, un trigger `AFTER UPDATE OF progress ON ingest_outbox` que suma `length(NEW.progress)` de cada `UPDATE`, una fila del outbox con `content` de N KiB de palabras distintas (`w00000 w00001 …`, el peor caso: nada se colapsa ni se absorbe), y `replay_pending` con `FakeEmbedder` y el tokenizador de prueba (`ChatChunker::with_fallback`). El tiempo es el de todo el save en un store vacío, con el embedder falso: sirve para comparar tamaños, no como latencia con el modelo real.

| `content` | Pedazos | `UPDATE`s | Bytes de `progress` | Tiempo |
|---|---|---|---|---|
| 8 KiB | 29 | 29 | 132 756 | 52 ms |
| 16 KiB | 57 | 57 | 515 418 | 93 ms |
| 32 KiB | 114 | 114 | 2 067 411 | 186 ms |
| 64 KiB | 228 | 228 | 8 296 656 | 515 ms |
| 128 KiB | 455 | 455 | 33 124 327 | 1,6 s |
| 256 KiB | 910 | 910 | 132 693 797 | 5,3 s |

Una entrada con cabecera ocupa unos 320 bytes (8 296 656 / (228²/2)), lo que estimó la exploración. Cada `UPDATE` reescribe además la fila entera, `content` incluido, que el trigger no cuenta: en el tope, unos 228 × 64 KiB ≈ 15 MB más.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `content_is_checked_in_bytes` y los dos tests de `tests/api.rs` no compilaban sin `MAX_CONTENT_BYTES` y `check_content`. El de 65 537 bytes describe un save que antes se aceptaba: la medición de arriba guarda saves más grandes por el mismo camino |
| R2 | `the_agent_contract_matches_its_file` falló con la descripción nueva antes de bendecir. `save_memory_says_a_save_over_the_limit_was_not_saved` pasa desde R1, porque el puente ya mostraba la frase del daemon de cualquier `400` |

## Lo que no se midió

- La latencia de un save en el tope con el modelo real y un store grande, que requiere el modelo y un perfil de datos. El tiempo de los KNN del detector queda para knn-prefilter (37), como dice el [diseño](design.md), D1.
- Ningún ranking ni recuperación cambia, así que no hay `rag-eval`.
- Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma, y el CI del pull request cubre la suite y clippy allí.
