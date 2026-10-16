# project-root-guard — Verificación

**Estado:** verificado en Linux. Falta la suite y el `clippy` en Windows ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, después de cada commit de la rama.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`main`, f8a361e) | 1.902 | 10 |
| `docs(openspec): propose project-root-guard` | 1.902 | 10 |
| `feat(core): record whether the daemon may read a project's files` | 1.903 | 10 |
| `feat(core): register a project with or without reading its files` | 1.905 | 10 |
| `feat(core): refuse to index a project root the user did not accept` | 1.906 | 10 |
| `feat(daemon): index a registered project only once it is accepted` | 1.910 | 10 |
| `feat(mcp): index only the project the bridge was started in` | 1.913 | 10 |
| `feat(mcp): refuse a project whose id already belongs to another root` | 1.915 | 10 |
| `feat(cli): add a project whose files the daemon may read` | 1.918 | 10 |
| `docs: describe which project roots the daemon reads` | 1.918 | 10 |

Las 10 fallas son las mismas en todos los commits y en `main`, y vienen del entorno: 7 de `doctor::tests` (modelan un PATH de Windows con `clawg.exe`) y 3 de `diff::tests` en `rag-eval` (leen `docs/eval/results/`, que está en `.gitignore`).

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina sin errores en el último commit.

**Conteo:** 17 tests nuevos y uno reemplazado (`a_refused_registration_does_not_block_the_save` pasa a ser `a_colliding_root_refuses_the_save_every_time`, que espera lo contrario). Neto: +16. En Windows se espera 1.902 + 16 = 1.918, sin fallas.

Cambiaron a propósito tres tests que contaban migraciones (`e2e_store_smoke`, `idempotent_migrations_second_open`, `a_v8_database_opens_unchanged_under_grouped_migrations`, de 12 a 13), el que fija los checksums de las migraciones, `registering_a_root_indexes_its_docs_in_the_background` (ahora manda `index_docs: true`) y `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (su raíz pasa a ser un directorio real, porque el daemon rechaza indexar uno que no existe).

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1. Sólo se indexa una raíz aceptada | `a_root_registered_without_its_docs_is_not_indexed`, `a_root_that_was_not_accepted_is_not_read` |
| R2. La marca sólo sube | `registering_to_index_raises_the_mark_and_never_lowers_it`, `a_collision_leaves_the_other_roots_mark_alone` |
| R3. 403 con el remedio | `searching_the_docs_of_a_root_that_was_not_accepted_is_forbidden`, `the_refusal_names_the_command_for_the_daemons_profile`, `search_project_does_not_read_a_root_the_user_did_not_add`, `search_project_lists_each_hit_under_its_breadcrumb` |
| R4. `clawg projects add` | `projects_add_takes_a_path_and_a_profile`, `adding_a_project_lets_its_docs_be_searched`, `adding_a_path_that_is_not_a_directory_is_refused`, `registering_a_missing_root_to_index_it_is_a_bad_request` |
| R5. El puente pide indexar sólo su raíz | `only_the_bridges_own_root_is_indexed`, `a_save_elsewhere_registers_the_root_without_reading_it` |
| R6. Colisión en cada llamada | `a_colliding_root_refuses_the_save_every_time`, `a_colliding_root_still_searches_all_projects`, `registering_a_root_whose_id_is_taken_by_another_is_a_conflict` |
| R7. Los demás fallos no bloquean | `a_registration_the_daemon_cannot_do_does_not_block_the_save`, `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` |
| R8. La base de la versión anterior | `a_database_from_the_previous_version_keeps_its_projects_indexed` |

## Contrato del agente

Cambió una sola línea de `agent_contract.json`: la descripción de `project_path` en `search_project`, que ahora dice que otro proyecto se busca sólo después de `clawg projects add`. Bendecido con `CLAWG_BLESS=1` y revisado.

## rag-eval

No aplica: el cambio no toca ranking ni recuperación, y `rag-eval --docs` usa la raíz del puente, que sigue indexándose sin consentimiento.

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --target-dir <otro-dir>`: se esperan 1.918 tests y 0 fallas.
2. `cargo clippy --workspace --all-targets --target-dir <otro-dir> -- -A clippy::all -D warnings`: código 0.
3. Al abrir la base real por primera vez con esta versión se escribe el snapshot `pre-migration` y se aplica `V13`; `clawg memories stats` debería mostrar lo mismo que antes y `search_project` en este repo seguir respondiendo.
