# self-update — Especificación

Fuente de verdad de `clawg update [--check]`: cómo se compara una versión, cuándo un binario puede actualizarse, cómo se elige el release y el asset, cómo se verifica la firma y el hash antes de tocar nada, el plan que intercambia el binario y vuelve a levantar el daemon, la vuelta atrás cuando el nuevo no arranca, las garantías sobre los datos, el informe y lo que sólo existe en un build de test. Cómo se publican los releases y sus claves está en [`specs/release`](../release/spec.md). No tiene que ver con [`specs/update`](../update/spec.md), que es la de `update_memory`.

**Origen:** [release-and-self-update](../../changes/archive/2026-10-18-release-and-self-update/proposal.md), archivado el 2026-10-18. Los ids de requisito son los de esa spec. [shutdown-budget](../../changes/archive/2026-10-20-shutdown-budget/proposal.md), archivado el 2026-10-20, sumó a S10 «Un `stop` que vence» y cambió los tests de S11. [local-transport](../../changes/archive/2026-10-20-local-transport/proposal.md), archivado el 2026-10-20, sumó a S8 el arranque sin `--port` en Unix y los dos escenarios de un actualizador anterior y del actual.

## Definiciones

- **Binario instalado:** el `clawg` que corre `clawg update`.
- **Marca de distribución** y **lista de claves:** las de [`specs/release`](../release/spec.md).
- **Sello de prueba:** un JSON al final de una copia del binario de prueba (versión, triple de distribución, migraciones que conoce, falla simulada del daemon y `transport: "tcp"`, que la hace comportarse como un clawg anterior al socket), que sólo lee un build con la feature `testing`.
- **Daemon:** el proceso `clawg serve` del perfil `prod`.
- **Candidato:** el binario nuevo bajado y preparado junto al destino, todavía sin instalar.
- **Intercambio:** el paso que renombra el binario instalado a `.old-*` y pone el candidato en su lugar.

Todo lo siguiente es el comando `clawg update [--check]`.

## S1. Versiones

Una versión MUST ser exactamente `X.Y.Z` de tres enteros sin signo, con un `v` inicial opcional sólo en el tag. `v0.19`, `0.19.0-rc1` y la cadena vacía MUST rechazarse. `update` MUST ofrecer sólo una versión estrictamente mayor que la instalada; una igual o menor MUST tratarse como «al día». La comparación MUST ser numérica (`0.9.0 < 0.18.0`). Sin dependencia nueva.

#### Escenario: orden
- GIVEN `0.18.0` instalada
- WHEN se compara con `0.19.0`, con `0.18.0` y con `0.17.9`
- THEN sólo `0.19.0` es una actualización

**Test:** `version_parse_and_order_cases` (módulo puro, `#[cfg(test)]`).

## S2. Elegibilidad del binario

La decisión de si este binario puede actualizarse MUST ser una función pura, y MUST negarse, con un mensaje que diga por qué, cuando el binario: es un build debug, no lleva la marca de distribución, está dentro de un directorio de target de cargo, o corre en el perfil `dev`. Un release marcado en `prod` MUST pasar. `--check` MUST obedecer la misma decisión.

#### Escenario: cada negativa
- GIVEN un build debug, otro sin marca, uno bajo `target/release/` y uno en `dev`
- WHEN se evalúa la elegibilidad
- THEN cada uno se niega con un motivo distinto

#### Escenario: release marcado
- GIVEN un build release con marca, fuera de un target, en `prod`
- WHEN se evalúa
- THEN pasa

**Test:** `update_eligibility_cases` (puro).

## S3. Elección del release y del asset

Entre los releases que devuelve la API de GitHub, `update` MUST ignorar borradores y prereleases y elegir el más nuevo. El asset MUST elegirse por triple de compilación (`clawg-<triple>[.exe]`). Si el release no trae el asset propio, el error MUST nombrar el triple.

#### Escenario: salto de borradores
- GIVEN un JSON con un borrador y un prerelease más nuevos que el último release
- WHEN se elige
- THEN se elige el último release publicado

#### Escenario: asset ausente
- GIVEN un release sin el asset del triple propio
- WHEN se resuelve el asset
- THEN falla y el mensaje contiene el triple

**Test:** `release_choice_and_asset_cases` (puro, sobre JSON fijo).

## S4. `SHA256SUMS`

El analizador de `SHA256SUMS` MUST rechazar una línea faltante para el asset, un hash que no tiene 64 dígitos hexadecimales y un nombre duplicado.

#### Escenario: líneas inválidas
- GIVEN archivos con la línea del asset ausente, con un hash corto y con el nombre repetido
- WHEN se analizan
- THEN los tres fallan con un error distinto

**Test:** `sha256sums_parse_cases` (puro).

## S5. Firma

`update` MUST verificar la firma minisign de `SHA256SUMS` con alguna clave de la lista (`minisign-verify`, con `allow_legacy = false`) antes de confiar en un hash. `minisign-verify` es la única dependencia directa nueva de runtime; el cambio de TLS (R2) suma `rustls-native-certs`, ausente hoy de `Cargo.lock`, y deja rustls, ring, hyper-rustls y tokio-rustls dentro del binario, y las pruebas suman la dev-dependency `minisign` (con ct-codecs, scrypt y rpassword). MUST fallar para una clave fuera de la lista, un `SHA256SUMS` alterado, un `.minisig` de otro archivo y una firma legada.

#### Escenario: firma válida con la segunda clave
- GIVEN una lista de dos claves y un archivo firmado con la segunda
- WHEN se verifica
- THEN pasa

#### Escenario: firma prehashed del firmante de pruebas
- GIVEN un archivo firmado con la dev-dependency `minisign` en modo *prehashed*
- WHEN se verifica con `allow_legacy = false`
- THEN pasa; y una firma legada del mismo archivo falla

#### Escenario: firma que no vale
- GIVEN un archivo firmado con una clave fuera de la lista, un archivo alterado, o el `.minisig` de otro archivo
- WHEN se verifica
- THEN falla y `update` no continúa

**Test:** `signature_cases` (puro, con claves de prueba generadas para la suite).

## S6. Sólo a pedido

El daemon MUST NOT consultar nada por su cuenta. Sólo `clawg update` y `clawg update --check` MUST tocar la red. Las peticiones MUST mandar `User-Agent`. Van sin token, así que la API de GitHub cuenta un límite por hora y por dirección: un 403 o un 429 con `x-ratelimit-remaining: 0` MUST informarse como ese límite agotado, con los minutos que faltan para `x-ratelimit-reset` si viene, y no sólo como el código de estado. `--check` MUST NOT descargar el binario ni modificar nada, y sus códigos de salida MUST ser `0` si está al día, `100` si hay una versión nueva (que nombra en stdout) y `1` si hay un error o una negativa (S2). El `2` queda para los errores de uso de clap.

#### Escenario: `--check` con y sin versión nueva
- GIVEN un servidor de release falso con claves de prueba
- WHEN corre `clawg update --check` primero con una versión mayor, luego con la misma y luego con el servidor caído
- THEN los códigos de salida son `100`, `0` y `1`, y el binario instalado no cambia

**Test:** `update_check_reports_a_newer_version_and_an_up_to_date_one` en `crates/clawg-cli/tests/binary.rs`; que las peticiones llevan `User-Agent`, en un test del cliente contra el servidor falso; el límite agotado, `a_refusal_with_no_requests_left_is_the_rate_limit` y `a_refusal_with_requests_left_or_another_status_is_only_a_status` (puros) y `the_rate_limit_is_read_from_the_answer` (contra un servidor falso).

## S7. Verificar antes de tocar

Antes del intercambio `update` MUST: bajar el candidato junto al destino, verificar la firma de `SHA256SUMS` (S5), comprobar el sha256 del candidato contra esa lista y ejecutar su `--version` y comprobar que da la versión elegida. Un `--version` que no termina en 30 s MUST contar como falla, con el proceso terminado y esperado, para que `update` no quede colgado con `update.lock` tomado. Si cualquiera falla, MUST dejar todo como estaba (binario instalado, daemon y datos), sin dejar el candidato. En Linux, sin permiso de escritura donde está el binario, MUST fallar antes de bajar nada con un mensaje claro, y MUST NOT usar `sudo`.

#### Escenario: hash que no coincide
- GIVEN un candidato cuyo sha256 difiere del de `SHA256SUMS`
- WHEN corre `update`
- THEN falla, el binario instalado y su daemon siguen como estaban y no queda el candidato

#### Escenario: sin permiso en Linux
- GIVEN un binario instalado en un directorio no escribible
- WHEN corre `update`
- THEN falla antes de la primera petición de descarga, con un mensaje que lo dice

**Test:** `a_hash_mismatch_never_touches_the_installed_binary` (plan, en proceso) y `a_failed_download_or_signature_leaves_everything_as_it_was` (plan); el permiso de Linux, `an_unwritable_install_dir_fails_before_downloading` (puro, sobre una función que decide con el resultado de un chequeo de escritura); el plazo del `--version`, `a_process_still_running_at_its_deadline_is_ended` y `a_process_that_ends_in_time_gives_its_output` (en `effects.rs`, con procesos del sistema).

## S8. Plan e intercambio

El plan de `update` MUST ser una lista de pasos como datos, ejecutada aparte, de modo que el orden y la deshecha de cada falla se prueben sin disco ni red. Con daemon, el orden MUST ser: parar el daemon con el binario instalado (`stop_in`, `crates/clawg-cli/src/stop.rs:23`); renombrar el binario a `.old-*`; poner el candidato; arrancar el daemon nuevo por su ruta explícita (no por `ProcessSpawner`, que lanza `current_exe()`, `client.rs:1152-1154`); esperar `/health`. En Unix el daemon nuevo, y el viejo cuando un deshacer lo vuelve a arrancar, MUST lanzarse sin `--port` y vigilarse por `from_data_dir`, que va por el socket ([`specs/daemon-access`](../daemon-access/spec.md), R9): así TCP queda encendido sólo donde alguien lo vuelve a pedir y no se arrastra de actualización en actualización. En Windows, único transporte TCP, MUST recibir el puerto del daemon viejo (`serve_port`, `crates/clawg-cli/src/update/effects.rs`). Un actualizador anterior al socket pasa siempre `--port`, así que el daemon nuevo enciende TCP y ese actualizador lo ve contestar. Sin daemon, `update` MUST NOT arrancar uno. Al empezar, MUST borrar los `.old-*` de corridas anteriores. Si un editor abre un puente entre la parada y el arranque, `update` MUST confirmar la versión del daemon que contesta e informarla. En Linux el binario instalado también se aparta a `.old-*` antes de poner el candidato: no se reemplaza en el lugar. Dos `update` a la vez MUST NOT intercambiar a la vez: un cerrojo exclusivo del sistema sobre `<data>/update.lock` hace que el segundo falle con un error claro.

#### Escenario: actualización completa con daemon
- GIVEN un daemon corriendo y un servidor de release falso con una versión mayor
- WHEN corre `clawg update`
- THEN `/health` contesta la versión nueva, el binario viejo queda como `.old-*` y un puente abierto llega al daemon nuevo

#### Escenario: sin daemon
- GIVEN ningún daemon corriendo
- WHEN corre `clawg update`
- THEN se intercambia el binario y no se arranca ningún daemon

#### Escenario: segunda actualización a la vez
- GIVEN un `update` en curso
- WHEN arranca otro
- THEN el segundo falla con un error claro y no toca el binario

#### Escenario: un actualizador anterior al socket
- GIVEN un clawg anterior al socket (sello `transport: "tcp"`) con su daemon en un puerto
- WHEN instala la versión actual
- THEN termina con 0, y el daemon nuevo contesta por el socket y por el mismo puerto

#### Escenario: el actualizador actual
- GIVEN un daemon con TCP en un puerto efímero
- WHEN la versión actual instala la siguiente, en Unix
- THEN el daemon nuevo no tiene `port` en `daemon.json`

**Pruebas:** `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` en `binary.rs` (parte del escenario de `binary.rs:564` con el binario reemplazado); `update_without_a_daemon_does_not_start_one` (plan); `plan_order_with_and_without_daemon` (plan); `two_updates_do_not_swap_at_once` (sobre el cerrojo, en un test de la unidad que lo toma); `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` (`update/effects.rs`); `an_older_updater_installs_the_socket_version_and_sees_it_answer`, `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (`tests/binary.rs`, Unix).

## S9. Vuelta atrás tras el intercambio

Si el daemon nuevo no contesta `/health` con la versión nueva en el plazo, `update` MUST devolver el binario viejo a su nombre y volver a levantar su daemon; el plazo MUST distinguir «arrancando» (migración o re-embed largos) de «no arrancó». Si el daemon nuevo alcanzó a migrar el store y el binario viejo se niega a abrirlo (`MemoryError::SchemaNewer` o `SchemaDiverged`, `crates/clawg-core/src/memory/error.rs:53-64`), `update` MUST restaurar en el acto el snapshot `pre-migration` que escribió ese arranque, con `clawg_core::memory::backup::restore` llamado desde el propio proceso de `update` (el binario viejo), y después devolver el binario viejo y arrancar su daemon. `restore` guarda antes el store migrado como `pre-restore`, así que nada se destruye. Si el restore falla, `update` MUST dejar el binario nuevo, el único que abre ese store, y el informe MUST dar la causa del arranque, la del restore, qué daemon contesta al terminar (o que no queda ninguno y cómo arrancarlo), dónde está cada archivo y los comandos para volver a mano, el arranque del daemon incluido (D2). Un candidato que no arranca y no migró MUST dejar el binario viejo y su daemon.

#### Escenario: el nuevo no arranca
- GIVEN un candidato que no levanta el daemon
- WHEN corre `update` con daemon previo
- THEN vuelven el binario viejo y su daemon, `/health` contesta la versión vieja y `update` sale con error

#### Escenario: el nuevo migró y el viejo se niega, el restore funciona
- GIVEN un daemon nuevo que migró el store y deja de contestar
- WHEN el binario viejo se niega a abrir el store migrado y el restore del `pre-migration` termina bien
- THEN vuelven el binario viejo y su daemon con los datos de antes de la migración, el store migrado queda como `pre-restore` y `update` sale con error explicando la causa

#### Escenario: el nuevo migró y el restore falla
- GIVEN el mismo caso, pero el restore falla
- WHEN `update` no puede devolver el store
- THEN queda el binario nuevo, el store migrado intacto, y el informe da la causa del arranque, la del restore, si contesta un daemon (su pid y su versión) o que no queda ninguno, las rutas del `.old-*`, del `pre-migration` y del `pre-restore`, y los comandos a mano, numerados, hasta volver a arrancar el daemon

**Pruebas:** `a_new_binary_that_does_not_start_brings_the_old_one_and_its_daemon_back` y `a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored` en `binary.rs`; `each_failed_step_undoes_what_it_did` y `a_failed_restore_leaves_the_new_binary_and_explains_the_way_back` (plan); `a_failed_restore_explains_the_way_back` y `a_failed_restore_names_the_daemon_that_answers_and_numbers_only_the_steps_left` (informe, puros); `restoring_over_a_store_migrated_by_a_newer_version_keeps_it_as_pre_restore` en `backup.rs` (D2).

## S10. Garantías de datos

`update` MUST NOT cambiar esquema, identificadores, estados ni el contrato de embedding por sí mismo. Se cumple siempre:

1. Antes del intercambio no se toca nada: una falla de descarga, firma, hash o humo deja binario, daemon y `memory.db` como estaban.
2. Después del intercambio, o contesta `/health` con la versión nueva el daemon nuevo, o vuelven el binario viejo y su daemon, o (S9) el usuario queda en una situación explicada con un camino de vuelta.
3. El usuario MUST NOT quedar sin daemon y sin saber por qué.
4. `update` MUST NOT borrar un `.old-*` ni un snapshot de la corrida en curso.
5. Un store con migraciones pendientes MUST dejar su `pre-migration` en `<data>/backups` (`crates/clawg-core/src/memory/db.rs:223-233`) y sus datos MUST seguir alcanzables. Un candidato cuyo daemon se niega a abrir el store (esquema más nuevo o migración divergente para el candidato, `db.rs:407-419`) MUST dejar el store sin tocar, y el usuario queda con el binario anterior y su daemon. Con releases ordenados por versión ese caso no ocurre de verdad (un store más nuevo que el candidato lo es también del binario viejo, y entonces no habría daemon viejo corriendo); se reproduce con el sello de prueba (`migrations_up_to`).

#### Escenario: migración pendiente
- GIVEN una base sembrada en la versión anterior con una migración pendiente
- WHEN el daemon nuevo la abre tras `update`
- THEN existe el snapshot `pre-migration` y los datos siguen alcanzables por búsqueda

#### Escenario: candidato cuyo daemon se niega a abrir el store
- GIVEN un store al día para el binario instalado y un candidato que, por su sello de prueba, conoce menos migraciones que las del store
- WHEN corre `update`
- THEN el daemon nuevo se niega sin tocar el store, no se escribe ningún snapshot nuevo, vuelven el binario viejo y su daemon respondiendo, y el informe dice por qué

#### Escenario: migración pendiente sobre filas sin causa de reemplazo
- GIVEN una base en V16 (`UNCAUSED_SCHEMA`) con una revisión por clave, una revisión deshecha por un veredicto, un veredicto entre filas sin clave y una vigente borrada
- WHEN `update` instala el binario nuevo y su daemon abre la base
- THEN el snapshot `pre-migration` es la base en V16 con todas sus filas, la base migrada tiene las mismas filas, la revisión queda `revision`, los dos veredictos `verdict` y ninguna fila tiene `revived_from`
- AND a través del daemon nuevo, borrar la revisión vigente revive a la anterior, y borrar el ganador de un veredicto (el de la revisión deshecha incluido) deja a la perdedora en `hidden`

**Pruebas:** `an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data`, `an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon` y `an_update_over_a_store_without_supersede_causes_keeps_its_rows_and_tells_revisions_from_verdicts` en `binary.rs`, con la base sembrada en la versión anterior (regla de `openspec/config.yaml` para cambios que tocan el esquema). Prerrequisito ya fusionado: `to_prune` conserva el `pre-migration` más nuevo (`crates/clawg-core/src/memory/backup.rs:243-269`, 0.18.1), con su test.

### Un `stop` que vence

`stop` espera el fin del proceso a lo sumo `STOP_TIMEOUT` (30 s, [`specs/storage`](../storage/spec.md), «Las fases caben en `STOP_TIMEOUT`»). Cuando vence con el apagado ya aceptado (`ClientError::StopTimeout`), el daemon termina por su cuenta poco después: el paso `StopDaemon` MUST contar entonces como hecho a medias (`StepError::under_way`, `crates/clawg-cli/src/update/plan.rs`) y su deshacer `StartOldDaemon` MUST correr. Ese deshacer MUST esperar primero el fin de ese pid, a lo sumo otro `STOP_TIMEOUT` (`wait_for_end`, `effects.rs`), y recién entonces arrancar el daemon con el binario instalado, que sigue en su lugar porque el plan se detuvo antes de apartarlo: arrancado antes, `wait_for` vería contestar al daemon que se apaga, con la misma versión, y daría `Ready` en falso. Si el pid sigue vivo, MUST NOT arrancar nada, y el informe MUST decir que se pidió el apagado y cómo arrancar el daemon cuando termine. Un `StopDaemon` que falla sin haber pedido el apagado (sin `/health`, o con el pedido rechazado) MUST NOT correr `StartOldDaemon`, y el informe sigue diciendo que el daemon no se tocó.

#### Escenario: el apagado vence
- GIVEN un daemon que acepta el apagado y no termina en `STOP_TIMEOUT`
- WHEN corre `update`
- THEN no se renombra nada, se borra el candidato y se corre `undo StartOldDaemon`

#### Escenario: el apagado no llegó
- GIVEN un `stop` que falla antes de pedir el apagado
- WHEN corre `update`
- THEN sólo se borra el candidato y el informe dice que el daemon no se tocó

#### Escenario: el daemon sigue cerrándose
- GIVEN un apagado vencido cuyo deshacer encontró el pid vivo al terminar su espera
- WHEN se arma el informe
- THEN dice que se pidió el apagado y cómo arrancar el daemon cuando termine

**Tests:** `a_daemon_that_does_not_stop_in_time_is_started_again_and_nothing_is_renamed` y `a_stop_that_never_reached_the_daemon_starts_nothing` (plan); `a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends` (informe, puro).

## S11. Informe

Al terminar bien, `update` MUST informar la versión anterior y la nueva, el snapshot `pre-migration` si se escribió (su ruta), que volver atrás pierde lo escrito después de él, y que los editores abiertos siguen con el puente viejo hasta reiniciarse. Un `update` fallido MUST decir en qué estado dejó la instalación y el daemon.

#### Escenario: informe de éxito
- GIVEN una actualización completa de una versión a otra mayor, con migración
- WHEN termina
- THEN la salida contiene ambas versiones, la ruta del `pre-migration` y el aviso de reinicio de los editores

**Tests:** `the_update_report_names_versions_snapshot_and_restart_notice` (puro, sobre la función que arma el texto); el estado del daemon tras un fallo, `a_failure_before_the_stop_says_the_daemon_was_not_touched`, `a_failure_after_the_stop_says_the_old_daemon_is_back`, `a_daemon_that_could_not_be_started_again_says_what_starts_it` y `a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends` (puros).

## S12. Sustitución sólo en builds de test

La URL base del servidor de releases y la lista de claves MUST poder sustituirse sólo en un build con la feature `testing`, con la regla de `CLAWG_TEST_HOME` (`crates/clawg-core/src/paths.rs:169`): un build sin `testing` MUST NOT leer esas variables, y la lista embebida de producción MUST NOT contener la clave de prueba. El sello de prueba (versión, triple de distribución, migraciones que conoce y falla simulada del daemon) MUST leerse sólo en un build con `testing`; en uno sin `testing` un sello al final del archivo MUST NOT cambiar nada. Ningún test MUST tocar `~/.clawg*` ni la red real.

#### Escenario: build de producción
- GIVEN un build sin `testing`
- WHEN las variables de sustitución están puestas
- THEN `update` usa la URL de GitHub y la lista embebida

#### Escenario: bytes finales en un ejecutable
- GIVEN una copia del binario de prueba con un sello al final, en Windows (PE) y en Linux (ELF)
- WHEN se ejecuta
- THEN se comporta igual que sin el sello, y un build con `testing` lee la versión del sello

**Pruebas:** `only_dev_dependencies_turn_on_the_test_stand_ins` (ya existe, `conventions.rs`) más `the_embedded_key_list_has_the_primary_and_backup_keys_and_not_the_test_key` (R6, medida 9); `update_overrides_are_read_only_in_a_test_build` (puro, con `cfg`). Entre los primeros tests en rojo: `trailing_bytes_after_the_executable_image_do_not_change_execution` en `binary.rs` (corre en las dos plataformas de CI) y el de firma *prehashed* con `allow_legacy = false` de S5 (`signature_cases`).
