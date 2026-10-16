# project-identity — Verificación

**Estado:** Linux completo. Se archivó con dos pasos pendientes que necesitan Windows (abajo). Si alguno falla, el arreglo va en un cambio nuevo. El suite de Windows ya corrió y dejó 5 fallas en `project_root.rs`, que eran del helper del test y ya están arregladas; la prueba con el binario sigue pendiente.

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

### Windows (2026-10-12) — fallas del test, arregladas

- [x] `cargo test --workspace --target-dir <otro>` en Windows. Se corrió durante la verificación de visibility-policy (su [`verify.md`](../2026-10-12-visibility-policy/verify.md#windows-2026-10-12)), sobre `main` (`c45b18a`, con este cambio ya mergeado): 1.648 pasan y 5 fallan. Con ese número se actualizó el baseline de `.claude/CLAUDE.md` (`d5a00e6`).
- [x] **Las 5 fallas son de este cambio**, todas en `crates/clawg-core/tests/project_root.rs`. El helper `canonical_tempdir` arma las rutas esperadas con `fs::canonicalize`, que en Windows deja el prefijo `\\?\`, y `resolve_project_root` lo quita: las rutas no coinciden. Son `every_way_of_naming_a_repository_resolves_to_its_root`, `a_submodule_is_its_own_project`, `the_id_of_a_canonical_root_is_the_id_it_always_had`, `a_subdirectory_keeps_its_old_id_as_the_legacy_one` y `windows_spellings_of_one_directory_share_a_root`. El problema era sólo del helper: ahora pasa la ruta por `strip_verbatim`, como `resolve_project_root`, y en Windows pasan los 8 (1.655 en total, sin fallas).
- `a_worktree_resolves_to_the_repository_it_was_added_from` pasaba también con el helper viejo: la raíz de un worktree sale del texto de su `gitdir:` (`root_from_marker`), sin canonicalizar, y el test escribía ahí la ruta con `\\?\`. Con git real no hay diferencia: `git worktree add` escribe la ruta con la caja real del disco aunque se lo llame con otra (probado en Windows), y `normalize_lexical` pasa las `/` a `\`.

## Prueba con el binario — pendiente

Necesita un daemon real y un perfil `dev` de prueba en Windows, que no se puede tocar desde acá.

- [ ] Arrancar `clawg mcp` en la raíz de un repo: `memory_status` funciona y no se mueve ninguna fila (el id es el mismo que antes).
- [ ] Guardar algo con el puente arrancado en un subdirectorio con el binario de `main`, y repetir con este binario: la fila aparece bajo el id de la raíz (`clawg memories --here` desde la raíz) y `project_rekey` tiene una entrada.
- [ ] Si había un `workspace.md` bajo el id del subdirectorio, aparece copiado bajo el id de la raíz y el original sigue.

## rag-eval

No aplica: el cambio no toca el ranking ni el golden set, que no usa `project_id`.

## Dependencia

El cambio de storage (backups, partes B + C de data-preservation, rama `claude/storage-sdd-xv8bd2`) no agrega migraciones, así que V9 no choca. Con storage, `MemoryStore::open` toma un snapshot antes de aplicar migraciones pendientes, y eso cubre el backup antes de V9. También rechaza una base con checksums distintos de `PINNED`; V9 ya tiene su fila ahí. Orden de merge: storage primero, después este cambio, rebaseado sobre él. El re-key no depende del snapshot para deshacerse: `project_rekey` guarda cada fila movida.

Cuando storage esté en `main`, queda una mejora opcional para un cambio aparte: tomar un snapshot (`MemoryRuntime::snapshot`, con una razón nueva `PreRekey`) antes de un registro que vaya a mover filas. Hoy el re-key ya se puede deshacer con `project_rekey`; el snapshot sería una segunda red.
