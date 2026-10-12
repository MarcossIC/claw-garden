# project-identity — Verificación

**Estado:** Linux completo. Quedan pendientes dos pasos que necesitan Windows y la máquina de Marcos.

## Tests

### Linux

`cargo test --workspace --no-fail-fast`, con el runtime de ONNX tomado del wheel `onnxruntime==1.24.2` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1`), como en las verificaciones anteriores.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 197186d) | 1.551 | 10 |
| Con el cambio (c64ce74) | 1.584 | 10 |

Son 33 tests nuevos y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que no existe en un clon limpio.

Tests que cambiaron de expectativa, no de propósito: los que fijaban la cantidad de migraciones aplicadas en 8 (`idempotent_migrations_second_open`, `e2e_store_smoke`, `a_v8_database_opens_unchanged_under_grouped_migrations`) ahora esperan 9. `v7_store_upgrades_to_v8_and_a_second_run_is_a_noop` migra con `Target::Version(8)` para seguir probando la subida a V8, y `v8_migration_creates_memory_fts_and_bumps_count_to_eight` pasó a llamarse `v8_migration_records_itself_and_creates_memory_fts`: comprueba que V8 figura en el historial en lugar de contar 8 filas.

### Windows — pendiente

- [ ] `cargo test --workspace --target-dir <otro>` en la máquina de Marcos. Además de confirmar el conteo, corre `windows_spellings_of_one_directory_share_a_root` (`crates/clawg-core/tests/project_root.rs`), que no corre en Linux: fija que `canonicalize` devuelve la caja real del disco y que se quita el prefijo `\\?\`. Es la suposición del diseño que no se pudo comprobar acá. Con ese número se actualiza el baseline de `.claude/CLAUDE.md`.

## Prueba con el binario — pendiente

Necesita un daemon real y un perfil `dev` de prueba en Windows, que no se puede tocar desde acá.

- [ ] Arrancar `clawg mcp` en la raíz de un repo: `memory_status` funciona y no se mueve ninguna fila (el id es el mismo que antes).
- [ ] Guardar algo con el puente arrancado en un subdirectorio con el binario de `main`, y repetir con este binario: la fila aparece bajo el id de la raíz (`clawg memories --here` desde la raíz) y `project_rekey` tiene una entrada.
- [ ] Si había un `workspace.md` bajo el id del subdirectorio, aparece copiado bajo el id de la raíz y el original sigue.

## rag-eval

No aplica: el cambio no toca el ranking ni el golden set, que no usa `project_id`.

## Dependencia

El cambio de storage (backups, partes B + C de data-preservation, rama `claude/storage-sdd-xv8bd2`) no agrega migraciones, así que V9 no choca. Con storage, `MemoryStore::open` toma un snapshot antes de aplicar migraciones pendientes, y eso cubre el backup antes de V9. También rechaza una base con checksums distintos de `PINNED`; V9 ya tiene su fila ahí. Orden de merge: storage primero, después este cambio, rebaseado sobre él. El re-key no depende del snapshot para deshacerse: `project_rekey` guarda cada fila movida.
