# secret-hygiene — Verificación

**Código:** rama `claude/secret-hygiene-sdd-feo162`, un commit por tarea.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Cada test se vio fallar antes de su código (por no compilar o por la aserción).
**Estado:** verificado. R1 a R6 pasan en Linux (commit por commit) y en Windows; `rag-eval --real` da diff vacío en el ancla y en agent-view; en el perfil dev, el save enmascara y `clawg secrets` lee la base real (ver [Windows](#windows-2026-10-14), [rag-eval](#rag-eval) y [Perfil dev](#perfil-dev-con-claude-code-2026-10-14)).

## Suite

`cargo test --workspace --no-fail-fast` en Linux, cada commit en un worktree limpio:

| Commit | Pasan | Fallan |
|---|---|---|
| `1127b50` (base, `main`) | 1675 | 10 |
| `c3b93ec` docs(openspec): propose secret-hygiene | 1675 | 10 |
| `caf0cff` feat(core): mask known secret formats in text | 1682 | 10 |
| `2dce41f` feat(core): mask secrets before a save is chunked, hashed or embedded | 1685 | 10 |
| `584b83f` feat(cli): tell the saver which secrets were masked | 1688 | 10 |
| `a2272c7` feat(core): find stored memories that hold a secret | 1691 | 10 |
| `4b417c8` feat(cli): add clawg secrets | 1693 | 10 |
| `bfdd4a8` feat(core): purge one memory for good, with its vector and index terms | 1700 | 10 |
| `6e2e05b` feat(cli): add clawg purge | 1704 | 10 |
| `80c4a0c` docs: document clawg secrets and clawg purge | 1704 | 10 |

- **+29 tests**, todos de este cambio. El conteo nunca baja.
- **Los 10 que fallan son los mismos 10 de `main`** en todos los commits (comparados por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **La base de 1691 de `.claude/CLAUDE.md`** es de Windows; en Linux la suite tiene esos 10 fallos de plataforma.
- **Warnings y clippy:** `cargo clippy -p clawg-core -p clawg-cli --all-targets` no da avisos en el código nuevo. Los warnings del build son los mismos de `main`.
- **`regex`:** pasa a dependencia directa de `clawg-core` con la versión que ya estaba en `Cargo.lock` (1.12.3); el lock sólo suma `regex` a las dependencias de `clawg-core`, sin paquetes nuevos.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Detector puro | `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `masking_twice_finds_nothing_new`, `an_anthropic_key_is_never_reported_as_an_openai_one`, `a_kind_found_twice_is_reported_once` | ✓ |
| R2. La ingesta enmascara | `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory`, `the_report_names_what_was_masked_even_when_the_save_fails` | ✓ |
| R3. El save avisa | `a_save_reports_the_secrets_it_masked` (`tests/api.rs`, por el router HTTP real), `a_confirmed_save_lists_the_kinds_it_masked`, `a_masked_save_tells_the_agent_what_was_masked` | ✓ |
| R4. `clawg secrets` | `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_reads_while_another_process_holds_the_store`, `the_scan_of_a_missing_store_creates_nothing`, `the_secrets_report_lists_rows_and_never_their_content`, `secrets_takes_only_a_profile` | ✓ |
| R5. `clawg purge` | `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched`, `purge_needs_yes_before_it_opens_anything`, `purge_takes_an_id_a_yes_and_a_profile`, `a_purge_refused_by_the_daemon_says_to_stop_it` | ✓ |
| R6. Backups que tienen la fila | `purge_names_the_backups_that_still_hold_the_row`, `the_purge_report_names_backups_to_delete_by_hand` | ✓ |

## Prueba de punta a punta (Linux, binario debug)

Con `HOME` en un directorio temporal del scratchpad, perfil dev. No se tocó ningún `~/.clawg*` real. El modelo no se puede descargar desde este entorno, así que el daemon quedó con el embedder en `loading`: el enmascarado de un save no se pudo probar con el binario (lo cubre `a_save_reports_the_secrets_it_masked` con el router HTTP real y un embedder de prueba). Las filas con secretos se escribieron con SQL directo sobre la base del daemon, como quedarían de una versión anterior: 200 filas comunes, una viva con una key AWS (#201) y una borrada con un JWT (#202).

| Paso | Resultado |
|---|---|
| `clawg secrets` con el daemon andando | `memory 201  aws_access_key  live` y `memory 202  jwt  deleted`, sin caracteres de los secretos; salida 0 |
| `clawg purge 201` | "clawg purge removes memory #201 for good … Run it again with --yes to do it", salida 1 |
| `clawg purge 201 --yes` con el daemon | "in use by the clawg daemon (pid 6888); run `clawg stop` first, then purge", salida 1 |
| `clawg backup` | snapshot `manual` por el daemon |
| `clawg stop` | ver «Lo que encontró la verificación»; el proceso salió unos segundos después |
| `grep` de la key en `memory.db` | 1 |
| `clawg purge 201 --yes` | `purged memory #201` y el backup manual como que todavía la tiene; salida 0 |
| `clawg purge 999 --yes` | `there is no memory #999`, salida 1 |
| `grep` de la key después | 0 en `memory.db` (no quedan `-wal` ni `-shm`), 1 en el backup, que no cambió |
| `clawg purge 202 --yes` (la borrada) | purgada, con el mismo backup listado; 0 ocurrencias del JWT en `memory.db` |
| `clawg secrets` | "no stored memory holds a secret of a known format" |
| Base después | 200 filas, `integrity_check` = `ok`, el FTS encuentra las 200 |
| `clawg serve` de nuevo y `clawg memories stats` | arranca; `200 live rows … 0 deleted` |

## Lo que encontró la verificación

- **`secure_delete` no alcanzaba para la purga.** El test de bytes sobre una base real encontró el secreto en una página de la lista libre, liberada por un merge del FTS mucho antes de purgar. La purga hace `VACUUM` (design D5). Sin `optimize` o sin `VACUUM`, los dos tests de bytes fallan.
- **`release_challengers` antes del `DELETE` no liberaba nada:** la fila todavía contaba como viva. Lo encontró `purging_releases_the_rows_that_disputed_it`; va después.
- **`clawg stop` con el modelo cargando** salió con 1 a los 15 s, y el proceso terminó unos segundos después. Es el problema conocido del backlog del README (la carga del embedder en `spawn_blocking`); no lo toca este cambio.
- **`rag-eval`:** el enmascarado sólo cambia textos con un patrón de secreto. El golden set no tiene ninguno, así que no debería moverse ninguna métrica. Medido en Windows: diff vacío (ver [rag-eval](#rag-eval)).

## Windows (2026-10-14)

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` en `35e448c`: **1.655 pasan, 0 fallan**. Desde `1569ea3` sólo cambió documentación, así que el código es el mismo que verificó knowledge-synthesis.

- Los 29 tests de este cambio aparecen uno por uno como `ok`, incluidos los de bytes de la purga (`purging_a_live_row_leaves_no_trace_of_its_text` y `purging_a_deleted_row_leaves_no_trace_of_its_text`), que dependen del `VACUUM` y del checkpoint sobre el sistema de archivos de Windows. También pasa `purging_drops_the_links_to_and_from_the_row`, que agregó knowledge-synthesis sobre esta purga.
- Las 10 fallas de Linux no aparecen: las 7 de `doctor::tests` modelan un PATH de Windows, y las 3 de `diff::tests` encuentran `docs/eval/results/` en esta máquina.
- La base de `.claude/CLAUDE.md` es 1.655 y no cambia. Es menor que los 1.704 de Linux porque knowledge-synthesis borró después 133 tests del pipeline de extracción.
- El único warning es el de `insert_row_with_embedding`, el mismo de `main` (está en el backlog del README).

## rag-eval

**Entorno:** Windows 11, binarios debug de `rag-eval`, modelo real (multilingual-e5-small) leído del caché de `~/.clawg-dev/models` (`--check-only` lo confirmó antes de correr, sin descargar ni escribir); los resultados van a un directorio temporal, fuera de `docs/eval/results/`.

**Binarios**, cada uno compilado en su worktree:
- Base: `c608c66` (`main` justo antes del merge de este cambio).
- Cambio: `262f839` (el merge de este cambio).

**Golden set:** `docs/eval/golden.json` (ancla, sha256 `d2d9d732…ed792a`, n = 46).

**Corridas**, las mismas con los dos binarios:
- `rag-eval --real --golden docs/eval/golden.json --tag anchor`
- `rag-eval --real --agent-view --golden docs/eval/golden.json`

El modo `--agent-view` es el que importa para este cambio: ingiere el corpus por el pipeline real de save, que es donde actúa el enmascarado.

| Métrica | Base | Cambio |
|---|---|---|
| R@1 | 0,4348 | 0,4348 |
| R@5 | 0,7826 | 0,7826 |
| R@10 | 0,8913 | 0,8913 |
| MRR | 0,5979 | 0,5979 |
| nDCG@10 | 0,6693 | 0,6693 |

- `rag-eval --diff` del ancla, base contra cambio: 0 mejoran y 0 empeoran en todas las métricas, en retrieval y en similarity.
- `rag-eval --diff` no compara el bloque `agent_view`, así que los dos JSON se compararon campo por campo, sin fecha, tag, sha ni latencias: **0 diferencias**, en el ancla y en agent-view.
- Agent-view, en las dos: 48 filas guardadas para un corpus de 48; `search` con hit rate 0,8913 y tokens media 851,5 y máximo 1239; `context` con hit rate 0,8913 y tokens media 3355,0 y máximo 4900.

Es lo esperado: el golden set no tiene ningún texto con forma de secreto, así que el enmascarado no cambia ni un texto.

## Perfil dev con Claude Code (2026-10-14)

**Entorno:** el MCP `clawg-dev` de Claude Code contra el daemon dev (pid 17944), que corre `target\debug\clawg.exe` compilado el 2026-10-12 a las 21:24, después del merge de knowledge-synthesis (incluye éste). Con autorización del usuario. Antes de tocar la base: `clawg backup --profile dev` por el daemon (`memory-2026-10-14T22-24-37.854Z-manual.db`). La fila de prueba se guardó en un proyecto aparte (un directorio temporal), rotulada como fixture.

| Paso | Resultado |
|---|---|
| `save_memory` con una key `ghp_` falsa de 36 caracteres | `saved #33; masked github_token before storing it` (R3) |
| `search_memory` de esa fila | `… authenticates with [REDACTED:github_token] as its GitHub token.` (R2) |
| `delete_memory` de #33 | `deleted 1`; la fila borrada sólo tiene el placeholder |
| `clawg secrets --profile dev` con el daemon andando | `no stored memory holds a secret of a known format`, salida 0 (R4) |
| Búsqueda de los bytes de la key falsa en `memory.db` y `memory.db-wal` | 0 en los dos |

**Purga:** la base dev no tiene ninguna fila con un secreto de formato conocido, así que no hubo nada que purgar y el daemon no se paró. La purga sobre una base real con el daemon parado ya está probada de punta a punta en Linux (arriba) y, en Windows, por los tests de bytes.
