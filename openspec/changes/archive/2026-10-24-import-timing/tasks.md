# import-timing — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. Las estructuras (IT3)

- [x] Tests primero (`timing.rs`): `a_step_reads_as_its_numbers` y `the_summary_names_every_step`, que no compilan sin las estructuras.
- [x] `Step`, `Timing` e `ImportTimings` en `crates/clawg-core/src/memory/portability/timing.rs`, con el `Display` de una fase y del resumen.

## 2. Los puntos de registro (IT1, IT2, IT4)

- [x] Tests primero (`import.rs`): `an_import_times_each_step_once_in_order`, `a_clawg_file_is_prepared_rather_than_translated`, `a_dry_run_times_only_what_it_runs` y `time_in_sqlite_never_exceeds_the_call`.
- [x] El ayudante de las llamadas, que devuelve con el resultado el tiempo dentro de la clausura.
- [x] `import` e `import_foreign` reciben `&mut ImportTimings`; `prepare_file`, `prepare_export`, `load` y `write` registran sus fases.
- [x] El handler de `POST /v1/import` mide `read` y registra el resumen.
- [x] Las pruebas de import que ya existen pasan sin cambiar ninguna aserción.

## 3. Docs

- [x] `.claude/CLAUDE.md` (lo que una importación deja en el log) y la fila 88 de `openspec/README.md`.

## 4. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [x] El CI en verde en Linux y Windows.
- [x] **[perfil dev, modelo real, con autorización]** Una importación real: la suma de las fases cierra contra el tiempo del cliente con un resto menor al 1 %. Resto de 0,03 % en la real y de 0,78 % en el dry run ([verify](verify.md#la-importación-real-en-el-perfil-dev-windows-2026-10-24)).
- [x] `verify.md`.

## 5. Archive

- [x] `specs/portability` con IT1 a IT4; `git mv` a `changes/archive/2026-10-24-import-timing`; `state.yaml` archivado; la fila 88 y el orden de ejecución del `README.md` de openspec; subir el minor. También se realinearon las citas de `import.rs` y `api.rs` en «Dónde está» ([verify](verify.md#al-archivar-2026-10-24)).
