# project-identity — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

Orden: las tareas 1–4 no cambian ningún id en uso. La 5 es la que empieza a usar la raíz resuelta, y llega con el re-key ya disponible.

## 1. Decisión pura de la raíz (R1, parte pura)

- [ ] Tests unit en `identity.rs`: `normalize_lexical` (separador final, `.`, `..`), `strip_verbatim` (`\\?\C:\x`, `\\?\UNC\srv\share`), `parse_gitdir` (absoluta, relativa, con espacios y `\r\n`), `root_from_marker` (directorio, worktree con `commondir` relativo y absoluto, submódulo).
- [ ] Las cuatro funciones puras.
- Commit: `feat(core): decide a project root from its git marker`

## 2. Resolución con filesystem e id (R1, R2)

- [ ] Tests en `crates/clawg-core/tests/project_root.rs`: misma raíz por varios caminos, worktree, submódulo, sin git, ruta inexistente, y `the_id_of_a_canonical_root_is_the_id_it_always_had`. El de Windows con `#[cfg(windows)]`.
- [ ] `resolve_project_root(&Path) -> PathBuf` y `project_id_for(&Path) -> String`, exportadas junto a `derive_project_id`.
- Commit: `feat(core): resolve a project root from any path inside it`

## 3. Tablas `project` y `project_rekey` (R5)

- [ ] Test `a_database_from_the_previous_version_opens_with_its_project_ids_untouched`: siembra con el runner hasta la última migración publicada y abre con el nuevo.
- [ ] Migración nueva (D2, número según D5) y su fila en `PINNED`.
- Commit: `feat(core): record project roots and re-keyed rows`

## 4. Re-key en el store (R4, R7)

- [ ] Tests en `crates/clawg-core/tests/project_rekey.rs`: filas en las tres tablas (incluida una borrada lógicamente) pasan al id nuevo, la otra no cambia, idempotencia, rollback con el SQL de `design.md`, búsqueda léxica después del re-key, colisión de raíz.
- [ ] Test `a_rekey_copies_the_legacy_workspace_and_keeps_it` en `workspace.rs`.
- [ ] `register_project(store, root, legacy_ids) -> Result<RegisterOutcome, _>` con error tipado para la colisión; copia de `workspace.md` después del commit (D3).
- Commit: `feat(core): move rows from a legacy project id to its resolved root`

## 5. Endpoint y puente (R3, R4, R6)

- [ ] Tests HTTP en `crates/clawg-cli/tests/api.rs`: 200 con `rekeyed`, 400 por id viejo inválido, 409 por colisión, 401 sin token.
- [ ] Tests unit: `resolve_project_id` y `Scope::project_id` con la resolución nueva; caché de registro del puente con cliente falso; un registro fallido no bloquea el save.
- [ ] `POST /v1/projects`, `DaemonClient::register_project`, el puente y `clawg memories --here` pasan a `project_id_for`.
- Commit: `feat(cli): scope memories by the resolved project root`

## 6. Docs

- [ ] `.claude/CLAUDE.md`: `POST /v1/projects` en la lista de la API y el baseline de tests.
- Commit: `docs: describe project registration`

## Verify

- [ ] `cargo test --workspace` contra la base (ver `verify.md`).
- [ ] En Windows (máquina de Marcos): el test de `#[cfg(windows)]` y un puente arrancado en la raíz y en un subdirectorio del repo real, con un perfil `dev` de prueba.
