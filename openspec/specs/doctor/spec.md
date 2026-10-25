# doctor — Especificación

Fuente de verdad de `clawg doctor`: qué `clawg` resuelve el PATH y qué está mal en el directorio de datos, en el store y en el registro de Claude Code, con la evidencia y el siguiente paso seguro, sin escribir nada. Cómo `clawg setup claude-code` escribe ese registro está en [`specs/setup`](../setup/spec.md); el enmascarado y `clawg purge`, en [`specs/secrets`](../secrets/spec.md); los snapshots, en [`specs/storage`](../storage/spec.md).

**Origen:**
- [doctor-checks](../../changes/archive/2026-10-20-doctor-checks/proposal.md), archivado el 2026-10-20. Los ids de requisito son los de esa spec. El análisis del PATH (`analyze`) es anterior y queda como estaba.
- [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22: `memory.topic` y `memory.content_hash` agrupan por proyecto y nombran a los autores; el texto de `fallback_agent`.
- [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23: los chequeos `preferences` y `protocol_block` (LP14).
- [cli-language](../../changes/archive/2026-10-23-cli-language/proposal.md), archivado el 2026-10-23: `message` en el idioma del comando (R1) y el resto del hallazgo igual en los dos idiomas (CL4).
- [project-registry](../../changes/archive/2026-10-25-project-registry/proposal.md), archivado el 2026-10-25: el chequeo `project_remotes` (R7, su RG5).
- [project-locators](../../changes/project-locators/proposal.md), su entrega 1, que sale en 0.64.0: los chequeos `projects` y `project_remotes` leen las carpetas de `project_path` (V27), con los mismos hallazgos. Un store anterior a V27 se lee con la tabla que tiene. Su entrega 2a, en la misma versión: los chequeos `project_duplicates`, `project_orphans` y `project_merges` (R7).

**Dónde vive:** las decisiones en `crates/clawg-cli/src/doctor/` (`finding.rs`, el sobre y el resumen; `checks.rs`, los chequeos puros; `mcp.rs`, el registro; `mod.rs`, el PATH y la lectura en `run`); los hechos del store en `crates/clawg-core/src/memory/inspect.rs`. Un test citado como «(`doctor`)» está en el `mod tests` de alguno de esos archivos.

## R1. Un sobre uniforme y el JSON de antes

`clawg doctor` MUST imprimir un objeto JSON que conserve `exe`, `version`, `profile`, `installs`, `winner`, `shadowed` y `duplicates` con los tipos de hoy, y sume `data_dir` (la ruta del perfil), `checks` y `summary`.

`checks` MUST ser una lista de hallazgos, cada uno con:

| Campo | Contenido |
|---|---|
| `check_id` | Estable, en inglés (`path`, `store.schema`, `memory.topic`, …) |
| `severity` | `ok`, `info`, `warn` o `error` |
| `reason_code` | Estable por causa (`shadowed`, `stale`, `dangling`, …); `healthy` para `ok` |
| `message` | Una frase para una persona, en el idioma del comando ([`specs/preferences`](../preferences/spec.md), CL1): en inglés sin preferencia |
| `evidence` | Un objeto con hechos medidos: rutas, conteos, ids, fechas, nombres de agente o de proyecto |
| `safe_next_step` | Un comando que se puede copiar, o `null` |
| `requires_confirmation` | `true` cuando ese paso cambia datos |

`summary` MUST contar los hallazgos por severidad (`ok`, `info`, `warn`, `error`). Cada chequeo que corre MUST aportar al menos un hallazgo: `ok` cuando no encontró nada, uno por causa en otro caso. Un chequeo que no aplica (el store no existe, el esquema no se puede leer) no aporta ninguno.

Ni `evidence` ni `message` ni `safe_next_step` MUST contener el texto de una fila de `memory`, una razón de veredicto, un `topic_key` o el contenido de una fila del outbox: sólo ids, tipos, conteos, rutas y nombres.

#### Escenario: compatibilidad
- GIVEN un PATH con dos copias de `clawg`
- WHEN se serializa el informe
- THEN tiene las claves de hoy con los mismos valores y además `checks` y `summary`

#### Escenario: sin contenido
- GIVEN un store con una fila que contiene un secreto y viola una invariante
- WHEN corre `clawg doctor`
- THEN el hallazgo nombra su id y la salida no contiene el secreto

**Tests:** `the_report_serialises_the_documented_fields`, `a_finding_serialises_its_envelope`, `the_summary_counts_findings_by_severity` (`doctor`, mod tests); `doctor_prints_no_row_content` (`crates/clawg-cli/tests/binary.rs`).

## R2. Código de salida

`clawg doctor` MUST salir 1 cuando algún hallazgo es `error` y 0 en otro caso. Un PATH donde el primer `clawg` no es el binario que corre, o con una segunda copia, MUST dar un hallazgo `error`, así que sale 1 como hoy.

#### Escenario: un aviso no falla
- GIVEN un store sin ningún snapshot
- WHEN corre `clawg doctor`
- THEN hay un hallazgo `backup.age` `warn` y sale 0

**Tests:** `the_exit_code_fails_only_on_an_error`, `path_findings` (`doctor`); `doctor_reads_a_running_daemons_store_without_changing_it` y `doctor_prints_no_row_content`, que salen 0 con avisos (`binary.rs`).

## R3. Se lee el store sin lock y sin escribir

`clawg doctor` MUST leer `memory.db` sin tomar el lock del store y sin escribir (sólo `store.owner` lo prueba un instante, cuando `memory.owner` nombra un titular, como hace `clawg status`): ni el archivo ni su WAL MUST cambiar de contenido, con el daemon vivo o parado, y sin `-wal` ni `-shm` nuevos junto a un store parado. Sin `memory.db` MUST dar `store.schema` `info` `missing` y no correr los chequeos que leen filas.

El esquema se compara con el del binario como en `clawg status`:

| Estado | Severidad | `reason_code` | Siguiente paso |
|---|---|---|---|
| al día | `ok` | `healthy` | — |
| sin migraciones aplicadas | `info` | `fresh` | — |
| migraciones pendientes | `info` | `pending` | `clawg serve --profile P` (el arranque escribe un snapshot `pre-migration` y migra) |
| de un clawg más nuevo | `error` | `newer` | — |
| divergente | `error` | `diverged` | — |
| ilegible | `error` | `unreadable` | — |

Un store más nuevo, divergente, ilegible o sin migraciones MUST NOT leerse más allá de su historia. Uno con migraciones pendientes MUST leerse con las tablas y columnas que tiene: un chequeo cuya tabla o columna falta no aporta hallazgos.

`store.owner` MUST decir quién tiene el store: `ok` sin titular, `info` `held` con el titular (`daemon`, `maintenance`, `eval`) y su pid, confirmado con el try-lock de `StoreLock::current_owner`.

#### Escenario: daemon vivo
- GIVEN un daemon que tiene el store y escribe con WAL
- WHEN corre `clawg doctor`
- THEN los hallazgos se leen y `memory.db`, `memory.db-wal` no cambian

#### Escenario: versión anterior
- GIVEN un store migrado hasta V10 con filas
- WHEN se inspecciona
- THEN `store.schema` es `pending`, los chequeos que necesitan V11 o posteriores no aportan nada y el archivo no cambia

**Tests:** `inspecting_leaves_the_store_unchanged`, `a_store_at_an_older_schema_is_read_with_what_it_has`, `a_newer_store_is_not_read_past_its_history`, `a_missing_store_has_no_facts` (`crates/clawg-core/src/memory/inspect.rs`); `schema_findings_follow_the_plan`, `owner_findings` (`doctor`); `doctor_reads_a_running_daemons_store_without_changing_it` (`binary.rs`).

## R4. Contrato de embedding sin cargar el modelo

`store.contract` MUST comparar la fila de `embedding_contract` con el contrato que `clawg serve` verificaría (`bootstrap::serving_contract`), sin cargar el modelo: el del embedder de producción (`E5_MODEL_ID`, dimensión 384 y sus dos prefijos), o en un build `testing` el del modelo falso cuando `CLAWG_TEST_EMBEDDER` lo nombra, como el daemon de ese build:

| Estado | Severidad | `reason_code` |
|---|---|---|
| igual | `ok` | `healthy` |
| sin fila (el modelo nunca cargó) | `info` | `not_recorded` |
| la misma red con pooling CLS (`is_pooling_upgrade`) | `info` | `pooling_upgrade`; el arranque re-embebe con un snapshot `pre-reembed` |
| cualquier otra diferencia | `error` | `mismatch`; la evidencia nombra lo guardado y lo esperado |

**Tests:** `contract_findings` (`doctor`); `the_expected_contract_is_what_the_embedder_records` (`embedder.rs`); el contrato `ok` del modelo falso en `doctor_reads_a_running_daemons_store_without_changing_it` (`binary.rs`).

## R5. Archivos del directorio de datos

| `check_id` | Hallazgo | Severidad | Siguiente paso |
|---|---|---|---|
| `data_dir` | el directorio no existe | `info` `missing` | — |
| `data_dir` | (Unix) tiene bits de grupo u otros | `warn` `open_mode`, con el modo | `chmod 700 <dir>` |
| `socket` | la ruta de `clawg.sock` no entra en `sun_path` (o, en Windows, no es Unicode) | `error` `path_unusable`, con la ruta, el largo y el tope | — |
| `backup.age` | un store con filas sin ningún snapshot en `<data>/backups` | `warn` `none` | `clawg backup --profile P` |
| `backup.age` | el snapshot más nuevo tiene más de 30 días según su nombre | `warn` `stale`, con el nombre y los días | `clawg backup --profile P` |
| `spill` | `ingest-pending.jsonl` presente | `info` `pending` (el próximo arranque lo pasa al outbox) | — |
| `spill` | `ingest-pending.rejected.jsonl` presente | `warn` `rejected`, con la ruta y la cantidad de líneas | — |
| `sharing` | `sharing.json` ilegible | `warn` `invalid`, con el error (no se comparte nada) | `clawg sharing --profile P` |

La edad de un snapshot MUST salir de su nombre (`parse_snapshot_time`, pura), no del reloj del sistema de archivos. Un nombre que no es de un snapshot no cuenta.

**Tests:** `data_dir_findings`, `socket_findings`, `backup_findings`, `spill_findings`, `sharing_findings` (`doctor`); `snapshot_times_come_from_their_names` (`backup.rs`).

## LP14. La preferencia y el bloque del protocolo

Cuando `preferences.json` existe, `doctor` MUST sumar el chequeo `preferences` (`checks::preferences`, `crates/clawg-cli/src/doctor/checks.rs:322`): `ok` legible, o `warn` `invalid` con la causa en `message`, sin `safe_next_step` y con el path en `evidence`. Con preferencia, MUST sumar el chequeo `protocol_block` (`checks.rs:350`) sobre el `CLAUDE.md` del usuario que lee `setup`: `ok` cuando el bloque es el que `setup` escribiría, `warn` `stale` cuando es otro e `info` `not_installed` sin bloque, las dos con `safe_next_step` `clawg setup claude-code --profile P`, y `warn` `unreadable` cuando el archivo no se puede leer o tiene un marcador suelto. Sin el archivo MUST NOT sumar ninguno de los dos.

**Test:** `preferences_findings` y `protocol_block_findings` (`doctor/checks.rs`); `doctor_names_a_broken_preferences_file` (`tests/binary.rs`).

## CL4. Sólo `message` cambia de idioma

Cada chequeo de `checks.rs` y `mcp.rs` MUST recibir el idioma del comando, que `doctor::run` lee de `preferences.json`, y componer su `message` con un `match` sobre `Lang`. Los hallazgos de unos mismos hechos MUST tener, en `en` y en `es`, el mismo `check_id`, `severity`, `reason_code`, `evidence`, `safe_next_step` y `requires_confirmation`; sólo `message` cambia. Una causa que va en `evidence` (la de `config_unreadable`, por ejemplo) MUST quedar en inglés. Las claves y los demás valores del JSON de `doctor`, y su código de salida, MUST NOT cambiar.

#### Escenario: mismos hechos, dos idiomas
- GIVEN los hechos que hacen saltar cada `reason_code` de `checks.rs` y `mcp.rs`
- WHEN se corren los chequeos con `en` y con `es`
- THEN las dos listas de hallazgos son iguales salvo en `message`, y cada `message` de `es` es distinto del de `en`

**Test:** `the_findings_are_the_same_in_both_languages_but_their_message` y `a_finding_in_spanish_agrees_with_its_number` (`doctor/checks.rs`), `the_mcp_findings_are_the_same_in_both_languages_but_their_message` (`doctor/mcp.rs`); `doctor_speaks_the_preferred_language` (`tests/binary.rs`).

## R6. Invariantes del estado de `memory`

Una fila está viva cuando `deleted_at IS NULL AND status <> 'superseded'`. Un save es el conjunto de filas de un mismo `save_id` de `memory_piece`, o la fila sola cuando no tiene vínculo. Cada invariante MUST ser una consulta de sólo lectura, y cada violación MUST nombrar ids:

| # | `check_id` | Violación | Severidad | `reason_code` |
|---|---|---|---|---|
| 1 | `memory.topic` | más de un save vivo por (proyecto, `topic_key`), de cualquier autor | `warn` | `several_live_saves`, con los autores (`agents`, `null` por una fila sin autor), el proyecto y la primera fila de cada save |
| 2 | `memory.superseded_by` | una fila no borrada `superseded` cuyo `superseded_by` es nulo o no existe | `warn` (`dangling`) si su causa es `revision` o nula; `info` (`dangling_verdict`) si es `verdict`, que `purge` deja a propósito | |
| 3 | `memory.revived_from` | `revived_from` que no existe | `info` | `dangling` (sólo impide volver a ocultarla al restaurar una fila purgada) |
| 4 | `memory.content_hash` | dos filas vivas con el mismo `content_hash` en el mismo proyecto, de cualquier autor | `warn` | `live_duplicates`, con los autores (`agents`), el proyecto y los ids |
| 5 | `memory.pieces` | un save cuyo primer pedazo ya no existe y algún otro vínculo guarda `context` | `warn` | `purged_header`, con el `save_id`: el texto purgado sigue en `memory.db` |
| 6 | `memory.disputed` | una fila viva `disputed` sin ninguna fila viva en `disputed_against` | `info` | `no_live_rival` (la limpieza diaria la devuelve a `active`) |

Un vínculo de `memory_piece` hacia una fila que no existe sin `context` no es una violación: `purge` lo deja a propósito para decir qué pedazo falta.

Las reglas 1 y 4 agrupan por proyecto, como la clave de tema y el dedup ([`specs/ingest`](../ingest/spec.md)). Una base de una versión anterior, cuando esas reglas eran por agente y proyecto, puede tener dos agentes con un titular o un mismo texto vivo cada uno en un proyecto: el doctor los marca, sin `safe_next_step`, y el próximo save con esa clave reemplaza a los dos. Dos agentes que `sharing.json` oculta entre sí también pueden dejar un titular cada uno a propósito, y el doctor, que no lee la política, los marca igual.

#### Escenario: pedazos de un save
- GIVEN un save largo de tres pedazos vivos bajo un mismo `topic_key`
- WHEN se inspecciona
- THEN `memory.topic` no da violación

#### Escenario: cada invariante
- GIVEN una base que viola una sola de las seis
- WHEN se inspecciona
- THEN aparece sólo esa violación, con sus ids; la base sana no da ninguna

**Tests:** `a_healthy_store_breaks_no_invariant`, `two_live_saves_of_one_topic_are_found`, `the_pieces_of_one_save_are_one_version`, `a_superseded_row_whose_winner_is_gone_is_found`, `a_dangling_revived_from_is_found`, `live_rows_with_one_hash_are_found`, `a_purged_header_left_in_the_links_is_found`, `a_dispute_without_a_live_rival_is_found` (`inspect.rs`); `invariant_findings` (`doctor`).

## R7. Outbox, agentes y proyectos

| `check_id` | Hallazgo | Severidad | Siguiente paso |
|---|---|---|---|
| `outbox` | filas `pending` sin daemon | `info` `pending`, con la cantidad | `clawg serve --profile P` |
| `outbox` | una fila `failed` | `warn` `failed`, una por fila, con el `operation_id`, el agente y los intentos | `clawg outbox retry <OPERATION_ID> --agent <AGENT> --profile P` |
| `agents` | siempre que haya filas | `ok`, con cada agente y sus filas vivas y totales | — |
| `agents` | filas vivas del agente de respaldo `mcp` | `warn` `fallback_agent`, con la cantidad: un cliente no dio `--agent` ni `clientInfo.name`; todo agente las lee y las cambia, pero el workspace y RECENT del agente real del cliente no las incluyen | — |
| `projects` | una carpeta de `project_path` cuyo id no es ninguno de los que su raíz puede recibir (`candidate_id(root_path, 0..64)`, `is_candidate_for`) | `warn` `id_mismatch`, con el id y la ruta | — |
| `project_remotes` | un proyecto con dos o más remotos atados en `project_remote`: lo que deja un repositorio clonado en el camino de otro con un clawg anterior a 0.63.0, o un remoto renombrado | `warn` `several_remotes`, con el id, sus remotos y cada raíz que lo usa (las carpetas con ese id y las raíces cuya clave actual, leída con `origin_key`, es uno de ellos) con su clave actual | — |
| `project_remotes` | ninguno | `ok`, con cuántos remotos hay | — |
| `project_duplicates` | dos proyectos vivos con el mismo nombre, sin distinguir mayúsculas | `warn` `same_name`, con los dos ids y sus nombres | `clawg projects merge <el más nuevo> --into <el más viejo> --dry-run --profile P` |
| `project_duplicates` | carpetas de dos proyectos vivos con el mismo primer commit que ningún par de remotos distingue (alguna sin remoto, o el mismo) | `warn` `same_first_commit`, con los dos ids, sus nombres y el commit | el mismo |
| `project_duplicates` | ninguno, o sólo pares con una sugerencia respondida (`apart`, `merged`) | `ok`, con cuántos proyectos vivos hay | — |
| `project_orphans` | un id con filas en `memory`, `episodic_trajectory` o `procedural_skill`, borradas incluidas, que `project` no tiene | `warn` `no_project`, con el id y las filas de cada tabla | — |
| `project_orphans` | ninguno | `ok`, con cuántos ids tienen filas | — |
| `project_merges` | filas bajo una lápida | `warn` `rows_under_merged`, con el id, su proyecto vivo y las filas de cada tabla | `clawg projects merge <la lápida> --into <su proyecto vivo> --dry-run --profile P` |
| `project_merges` | una lápida cuya cadena no termina en un proyecto vivo: un ciclo, más de `MAX_MERGE_HOPS` uniones o un id que `project` no tiene | `warn` `broken_merge`, con el id y su `merged_into` | — |
| `project_merges` | ninguno | `ok`, con cuántas lápidas hay | — |

Los tres últimos chequeos corren desde V27, cuando `project` es la tabla de los proyectos (`StoreFacts::registry`); en un store anterior no aportan nada. Ningún paso pide confirmación: una unión en seco no escribe nada. La cadena de una lápida se sigue como la sigue `canonical` ([`specs/project`](../project/spec.md), PL12).

Con filas `pending` y un daemon vivo no hay hallazgo: las está ingiriendo.

**Tests:** `the_facts_name_agents_projects_the_outbox_and_the_contract`, `the_facts_name_the_remotes_of_each_project`, `the_facts_name_the_projects_their_folders_answers_and_rows` (`inspect.rs`); `outbox_findings`, `agent_findings`, `project_findings`, `a_project_id_assigned_to_its_root_is_ok`, `project_remote_findings`, `shell_words_are_quoted_only_when_a_shell_would_read_them`, `the_duplicates_are_named_by_name_and_by_first_commit`, `a_pair_kept_apart_is_no_duplicate`, `an_id_without_a_project_is_named`, `rows_under_a_merged_project_and_a_broken_chain_are_named` (`doctor`).

## R8. El registro MCP de Claude Code

`mcp` MUST leer, sin escribirlo, el `.claude.json` que lee `clawg setup claude-code` (`$CLAUDE_CONFIG_DIR/.claude.json` o `~/.claude.json`) y buscar el servidor del perfil (`clawg`, `clawg-dev`) en `mcpServers`:

| Estado | Severidad | `reason_code` | Siguiente paso |
|---|---|---|---|
| registrado con este binario y `--agent` | `ok` | `healthy` | — |
| sin archivo o sin el servidor | `info` | `not_registered` | `clawg setup claude-code --profile P` |
| archivo ilegible o JSON inválido | `warn` | `config_unreadable` | — |
| `command` absoluto que no existe | `error` | `binary_missing` | `clawg setup claude-code --profile P` |
| `command` absoluto que existe y no es este binario | `warn` | `other_binary` | — |
| `args` sin `--agent` | `warn` | `no_agent` | `clawg setup claude-code --profile P` |

La decisión MUST ser una función pura del texto del archivo, el perfil, el binario que corre y un predicado `exists`.

**Tests:** `mcp_findings` (`doctor`).
