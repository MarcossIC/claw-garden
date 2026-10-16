# project-root-guard — Verificación

**Estado:** verificado en Linux, commit por commit, y en Windows sobre `main` ([Windows](#windows)). En Windows fallaba un test de este cambio por un error del propio test, arreglado en `8312e42`. La base real aplica `V13` con la primera instalación posterior a 0.8.0 ([base real](#base-real)).

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

### Windows

`cargo test --workspace --no-fail-fast --target-dir target-test`:

| Commit | Pasan | Fallan | Ignorados | Warnings del workspace |
|---|---|---|---|---|
| `main` en `210c692` (este cambio y eval-rigor ya mergeados) | 1.983 | 1 | 0 | 0 |
| `8312e42` (el test arreglado) | 1.984 | 0 | 0 | 0 |

1.984 es la base de 1.918 que se esperaba para este cambio, más los 50 de eval-rigor y los 16 de este. Los 7 de `doctor::tests` que fallan en Linux pasan en Windows.

La falla de `210c692` era `projects::tests::adding_a_project_lets_its_docs_be_searched`, y el error estaba en el test, no en `add`:

- el test registraba la etiqueta con `std::fs::canonicalize`, que en Windows deja el prefijo `\\?\`;
- `add` resuelve la raíz con `ProjectIdentity::resolve` (`crates/clawg-cli/src/projects.rs:24`), que lo saca (`strip_verbatim`, `crates/clawg-core/src/memory/identity.rs:133`);
- las dos formas de escribir la misma carpeta daban ids distintos, así que el registro y el `add` caían en dos proyectos.

En Linux no hay prefijo, y por eso pasaba. `8312e42` arma la raíz del test con `ProjectIdentity::resolve(repo.path()).root` (`crates/clawg-cli/src/projects.rs:47`), igual que `repo_with_src` y `elsewhere` en `crates/clawg-cli/src/mcp/server.rs`. Producción no cambia.

`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` sobre `8312e42`: **código 0**, sin ningún warning.

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

## Base real

Al archivar, el daemon `prod` corre 0.7.0 (`memory_status`: `"version": "0.7.0"`, perfil `prod`, embedder `ready`). Ese bump (`153dc6c`) es anterior al merge de este cambio (`210c692`), así que la base real sigue en `V12` y `search_project` responde en este repo con el índice de siempre. No se instaló un release ni se tocó `~/.clawg` ni `~/.clawg-dev`.

La migración queda cubierta por `a_database_from_the_previous_version_keeps_its_projects_indexed`, que siembra hasta `V12` y abre con `V13`. La primera instalación posterior a 0.8.0 escribe el snapshot `pre-migration` antes de aplicarla. Hay que comprobar entonces que `clawg memories stats` muestre lo mismo que antes y que `search_project` siga respondiendo en este repo: cada fila de `project` queda aceptada, también la de esta raíz.

## Diferencias entre el spec y el código

Al consolidar en [`specs/project-docs`](../../../specs/project-docs/spec.md), [`specs/project`](../../../specs/project/spec.md), [`specs/mcp`](../../../specs/mcp/spec.md) y [`specs/storage`](../../../specs/storage/spec.md) se revisó contra `main` cada `ruta:línea` y cada test citado. Todos los tests existen, y lo que afirman R1 a R8 coincide con el código. Las diferencias:

1. **Al backlog.** El texto de la colisión (`crates/clawg-cli/src/mcp/server.rs:465-468`) dice siempre que `all_projects` sigue buscando en todo, también en `recall_context` y `search_project`, que no tienen ese argumento (`server.rs:676`, `:831` pasan `false`). R6 lo pedía para las cinco herramientas, así que el código cumple el spec, pero en esas dos el consejo no se puede seguir.
2. D5 dice que `project_for` devuelve `Result<Option<String>, String>`. Devuelve `Result<Option<String>, ClientError>` (`server.rs:563-568`), y el texto se arma en `explain` (`server.rs:465-470`).
3. D4 dice que `try_refresh` empieza por `root_of`. Primero toma la guarda `begin` y después llama a `root_of` (`crates/clawg-core/src/memory/project_docs/mod.rs:222-225`). Las citas de D4 (`mod.rs:426`, `:221`) hoy son 433 y 225.
4. Las Definiciones citan `default_project` en `server.rs:325-327`; hoy está en `331-333`.
5. El test de R8 llama a `run_migrations` directo, así que no prueba el snapshot `pre-migration` al abrir. Lo cubre la regla general de [`specs/storage`](../../../specs/storage/spec.md).
6. La tabla de requisitos pone `registering_a_root_whose_id_is_taken_by_another_is_a_conflict` (`crates/clawg-cli/tests/api.rs`) bajo R6. Ese test es de project-identity y no mira la marca; el de este cambio para la marca en una colisión es `a_collision_leaves_the_other_roots_mark_alone`.

Citas viejas que no vienen de este cambio y quedan como estaban: R3 de `specs/project` cita `server.rs:165-167` para el fallback de `current_dir` (hoy `331-333`, y ya estaba mal en `f8a361e`); «Lo compartido» de `specs/mcp` cita `tools.rs:317`, `server.rs:756` y `server.rs:871-912` (hoy `format_sharing` en `tools.rs:338` y `memory_status_impl` en `server.rs:878`); y `specs/mcp` y `specs/project-docs` cuentan once herramientas sin `restore_memory`, que el puente registra (doce, `server.rs:1011`) y que llega con recoverable-delete, todavía sin archivar.
