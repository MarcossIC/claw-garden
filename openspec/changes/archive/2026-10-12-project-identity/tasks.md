# project-identity — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

Orden: las tareas 1–4 no cambian ningún id en uso. La 5 es la que empieza a usar la raíz resuelta, y llega con el re-key ya disponible.

## 1. Decisión pura de la raíz (R1, parte pura)

- [x] Tests unit en `identity.rs`: `normalize_lexical` (separador final, `.`, `..`), `strip_verbatim` (`\\?\C:\x`, `\\?\UNC\srv\share`), `parse_gitdir` (absoluta, relativa, con espacios y `\r\n`), `root_from_marker` (directorio, worktree con `commondir` relativo y absoluto, submódulo).
- [x] Las cuatro funciones puras.
- Commit: `feat(core): decide a project root from its git marker` (`293d5ac`)

## 2. Resolución con filesystem e id (R1, R2)

- [x] Tests en `crates/clawg-core/tests/project_root.rs`: misma raíz por varios caminos, worktree, submódulo, sin git, ruta inexistente, y `the_id_of_a_canonical_root_is_the_id_it_always_had`. El de Windows con `#[cfg(windows)]`.
- [x] `resolve_project_root(&Path) -> PathBuf` y `ProjectIdentity::resolve(&Path)` (raíz, id e id viejo), exportadas junto a `derive_project_id`.
- Commit: `feat(core): resolve a project root from any path inside it` (`a556e90`)

## 3. Tablas `project` y `project_rekey` (R5)

- [x] Test `a_database_from_the_previous_version_opens_with_its_project_ids_untouched` en `db.rs`: siembra con el runner hasta la última migración publicada y abre con el nuevo.
- [x] `V9__project.sql` (D2) y su fila en `PINNED`. Los tests que fijaban 8 migraciones pasan a 9, o a `Target::Version(8)` donde prueban la subida a V8.
- Commit: `feat(core): record project roots and re-keyed rows` (`48114ca`)

## 4. Re-key en el store (R4, R7)

- [x] Tests en `crates/clawg-core/src/memory/project.rs`: filas en las tres tablas (incluida una borrada lógicamente) pasan al id nuevo, la otra no cambia, idempotencia, rollback con el SQL de `design.md`, búsqueda léxica después del re-key, colisión de raíz.
- [x] Test `a_rekey_copies_the_legacy_workspace_and_keeps_it` en `workspace.rs`.
- [x] `register_project(store, root, legacy_ids) -> Result<RegisterOutcome, _>` con error tipado para la colisión; copia de `workspace.md` después del commit (D3).
- Commit: `feat(core): move rows from a legacy project id to its resolved root` (`a06f3b3`)

## 5. Endpoint y puente (R3, R4, R6)

- [x] Tests HTTP en `crates/clawg-cli/tests/api.rs`: 200 con `rekeyed`, 400 por id viejo inválido, 409 por colisión, 401 sin token.
- [x] Tests: `resolve_project` y `Scope::project_id` con la resolución nueva; en el puente, contra un daemon en proceso, el re-key desde un subdirectorio, un solo registro por raíz y un registro rechazado que no bloquea el save.
- [x] `POST /v1/projects`, `DaemonClient::register_project`, el puente y `clawg memories --here` pasan a `ProjectIdentity::resolve`. El puente registra cada raíz una vez por proceso.
- Commit: `feat(cli): scope memories by the resolved project root` (`56c0f29`)

## 6. Docs

- [x] `.claude/CLAUDE.md`: `POST /v1/projects` en la lista de la API. El baseline de tests se actualiza con la corrida de Windows (ver `verify.md`).
- Commit: `docs: describe project registration` (`c64ce74`)

## Verify

- [x] `cargo test --workspace` en Linux contra la base (ver `verify.md`).
- [ ] En Windows: el test de `#[cfg(windows)]` y un puente arrancado en la raíz y en un subdirectorio del repo real, con un perfil `dev` de prueba.
