# hook-run-log — Verificación

## Suite y clippy

En Windows x86_64, build debug, toolchain 1.99.1, en `target-test` (limpiado con `cargo clean --workspace --target-dir target-test` antes de la corrida), sobre `9bb55b33` (`chore: bump version to 0.76.0`), el último commit antes de los de documentación:

- `cargo test --locked --workspace --no-fail-fast --target-dir target-test`: sale con 0, ningún binario con fallas; los ignorados son los de antes (los que piden el modelo real o un ensayo a mano). `session_start_in_a_declared_subfolder_reads_its_project`, que falló de forma intermitente con la CPU cargada (el plazo de 3 s del hook) y es anterior a este cambio, no falló.
- `cargo clippy --locked --workspace --all-targets --target-dir target-test -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -D warnings` también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que ve el modelo no cambió.
- Después de esa corrida sólo cambiaron documentos (`docs/reference/data-profiles.md` y dos `verify.md`); como hay tests que leen documentos, se volvieron a correr `tests/conventions.rs` (clawg-cli) y `tests/docs_mode.rs` (clawg-eval), que pasan.
- Los tests de `clawg hook` que ya existían pasan sin cambiar lo que esperan: sólo se tocó el ayudante `broken_daemon`, que ahora delega en `daemon_answering` con el mismo mensaje de error.
- El CI de #126 pasó en `test (windows-latest)` y `test (ubuntu-latest)` sobre `4f5f0e5d`, que trae este cambio hasta `d68d4c35`. El arreglo de [la escritura trabada](#una-escritura-trabada) es posterior: tiene su corrida local, en esa sección, y ningún CI.

## Los tests en rojo

Escritos antes del código de cada commit.

| Commit | Antes | Después |
|---|---|---|
| `feat(hook): record each session start` | `cargo test -p clawg-cli --lib hook` no compilaba: 73 errores (`cannot find type Outcome`, `Run` y `FILE`, `cannot find function respond_with_run`, `record`, `line`, `input_keys`, `read_runs`, `parse_runs`, `append`, `daemon_answering` y `start_run`) | los 64 tests de ese filtro pasan |
| `feat(doctor): show the hook's recent session starts` | `cargo test -p clawg-cli --lib hook` no compilaba: `unresolved import crate::hook_log::{Outcome, Run}` en `checks.rs` (un error de resolución, que tapa los de `hook_runs`, `age`, `folder_name`, `installed_in` y `hook_installed`, que tampoco existían) | pasan los tests nuevos de `checks.rs` y `setup/cursor.rs`, y los de `tests/binary.rs` |

Los tests de `tests/binary.rs` de los dos commits se escribieron con los unitarios y no se corrieron en rojo por separado: el binario no compila sin el módulo, así que su rojo es el de arriba.

Los de la revisión (ver más abajo), también escritos antes del código. Con las funciones que faltaban, `cargo test -p clawg-cli --lib hook` no compilaba (7 errores: `cut`, `KEYS_KEPT`, `KEY_CHARS` y los que los usan); para ver cada test fallar por su aserción se dejaron primero `cut` (sin cortar nada) y las constantes, y se corrieron: 8 tests fallan y 9 pasan.

| Test | Falla por |
|---|---|
| `what_a_run_keeps_is_bounded` (`hook.rs`) | la línea de una entrada de 900 KB tiene más de 4 KiB |
| `the_cursor_version_is_kept_only_as_a_cut_string` (`hook.rs`) | un objeto en `cursor_version` se guarda, con el correo adentro |
| `cutting_stops_on_a_character_boundary`, `input_keys_keep_the_first_32_names_cut_at_64_characters` (`hook_log.rs`) | no cortan |
| `a_damaged_file_costs_only_its_damaged_lines` (`hook_log.rs`) | un byte `\xc3` entre dos corridas deja el registro vacío |
| `a_failed_roll_still_records_the_run` (`hook_log.rs`) | el renombrado fallido aborta antes de agregar |
| `a_run_stamped_at_the_extremes_does_not_panic` (`checks.rs`) | desborda en `now_ms - run.at_ms` (`at_ms` en `i64::MIN`) |
| `bad_input_does_not_claim_the_input_was_not_json` (`checks.rs`) | el mensaje dice que la entrada no era JSON |

Un test se escribió para una mutación que había sobrevivido y pasa desde que se escribe: la aserción de `folder` `None` con `other_client` en `every_session_start_names_why` (`hook.rs`). Quitando el filtro `outcome != Outcome::OtherClient` de `respond_with_run`, ese test falla; con el filtro puesto, pasa.

## Una corrida a mano

Con `CLAWG_TEST_HOME` en un directorio temporal dentro del espacio de trabajo (borrado al terminar), `CLAWG_TEST_EMBEDDER=fake`, `target-test/debug/clawg.exe`, el perfil `dev` y un daemon propio (`clawg serve --profile dev --port 0`, parado después con `clawg stop`), con el hook corriendo en `<home>/.cursor` y una entrada de Cursor con `user_email` `fake.person@example.com`:

1. Una raíz, con el daemon vivo: salida 0 y nada impreso.
2. Dos raíces: salida 0 y nada impreso.
3. Sin daemon (después de `clawg stop`): salida 0 y nada impreso.

`hook-runs.jsonl` quedó así (la ruta del home temporal, recortada a `<home>`):

```
{"at_ms":1793800451004,"client":"cursor","agent":"cursor","source":null,"outcome":"empty_context","bytes":0,"elapsed_ms":20,"roots":1,"folder":"<home>/repo","cursor_version":"3.28.0","input_keys":["conversation_id","cursor_version","workspace_roots","user_email","transcript_path","session_id"]}
{"at_ms":1793800451084,"client":"cursor","agent":"cursor","source":null,"outcome":"several_folders","bytes":0,"elapsed_ms":0,"roots":2,"folder":null,"cursor_version":"3.28.0","input_keys":["conversation_id","cursor_version","workspace_roots","user_email","session_id"]}
{"at_ms":1793800451213,"client":"cursor","agent":"cursor","source":null,"outcome":"no_daemon","bytes":0,"elapsed_ms":0,"roots":1,"folder":"<home>/repo","cursor_version":"3.28.0","input_keys":["conversation_id","cursor_version","workspace_roots","user_email","transcript_path","session_id"]}
```

El correo no aparece ni en el archivo ni en la salida de `doctor` (0 coincidencias en los dos). La carpeta quedó con las barras `/` con que llegó, sin normalizar. Que la primera corrida diga `empty_context` y no `injected` es lo que debe: el agente `cursor` del home temporal no guardó nada.

La entrada `hooks.runs` de `clawg doctor --profile dev` en ese mismo home (la salida real, con la ruta recortada; el resto del JSON tiene los demás chequeos):

```json
{
  "check_id": "hooks.runs",
  "severity": "info",
  "reason_code": "recent",
  "message": "the last 3 session starts of the hook, newest first:\njust now  cursor  repo  nothing: the daemon did not answer\njust now  cursor  (2 folders)  nothing: several folders\njust now  cursor  repo  nothing: no memories to show",
  "evidence": {
    "cursor_hook_installed": false,
    "total": 3,
    "runs": [ ...las tres corridas de arriba, completas, la más nueva primero... ]
  },
  "safe_next_step": null,
  "requires_confirmation": false
}
```

En esa corrida `doctor` salió con 1 por tres hallazgos `error` que no son de este cambio (`path`: el binario de tests no está en el PATH; `socket`: la ruta del socket del directorio temporal no entra; `store.contract`: el contrato del modelo falso). `hooks.runs` es `info`. `clawg doctor` imprime un solo documento JSON y ningún otro texto, así que lo que se pega para mostrar una corrida es esa entrada de `checks`.

## El spec contra el código

| Requisito | Dónde |
|---|---|
| HR1 | `respond_with_run` arma `Run` (`crates/clawg-cli/src/hook.rs`, junto a `respond`), `hook_log::line` y `input_keys` (`crates/clawg-cli/src/hook_log.rs`); `respond_and_record` registra con `hook_log::record_by` |
| HR2 | `answer` devuelve el motivo de cada salida, `missing_folder` y `daemon_failure`; `respond_with_run` suma `bad_input`, `other_client` y `timed_out` |
| HR3 | `Run` sólo tiene los campos de la spec; `version_text` corta el texto de `cursor_version` a 32 caracteres; `folder` es `None` con varias raíces (lo decide `folder`, `hook.rs`) |
| HR4 | `hook_log::append` (una `write_all`, `rename` a 64 KiB, `create(true)` sin `create_dir`), `record` traga el resultado, `respond_and_record` devuelve la salida después de registrar, `record_by` espera hasta el plazo y no más, `RECORD_RESERVE`, y `shutdown_background` en `main.rs` no espera la escritura al salir |
| HR8 | `hook_log::cut` y las constantes de largo, `input_keys`; `respond_with_run` corta `agent`, `source` y `folder`, y `version_text` sólo toma un texto JSON |
| HR5 | `respond` delega en `respond_with_run` y devuelve `.0`; el ayudante de tests es lo único cambiado en los tests de antes |
| HR6 | `checks::hook_runs`, `age`, `folder_name`, `what_happened` (`crates/clawg-cli/src/doctor/checks.rs`); la lectura, `hook_log::read_runs`, en `doctor/mod.rs`, que no abre el store para esto |
| HR7 | `checks::hook_runs` sin corridas, y `setup::cursor::hook_installed` con `installed_in` (`crates/clawg-cli/src/setup/cursor.rs`), que reusa `locations` e `is_clawg_hook` |

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Test de privacidad | En `hook_log.rs` | En `hook.rs` (`a_run_never_holds_an_input_value`) | Lo que se prueba es lo que `respond_with_run` arma a partir de una entrada con `user_email`, no la forma de `Run`; la spec dice dónde está |
| Qué dice `no_daemon` | «no hay daemon» | También un `daemon.json` viejo con el daemon caído, que da `ClientError::NotRunning` (ningún test lo ejerce) | Para quien mira es lo mismo: nadie respondió |
| Antigüedad en el mensaje | `hoy 10:42` o `hace 5 min` | `hace 5 min`, `hace 2 h`, `hace 1 día` | Sin hora local el chequeo sigue puro sobre `now_ms` |
| El cliente en el mensaje | No estaba en la forma pedida | Cada renglón lo lleva | Los dos clientes escriben en el mismo archivo |
| Corridas de los dos archivos | Las últimas 20 | Se lee `hook-runs.1.jsonl` y después `hook-runs.jsonl` | Justo después de rotar, el actual tiene casi nada |

## La revisión

Una revisión independiente reprodujo siete problemas; cada uno se resolvió con un test en rojo primero (arriba) y su requisito en la [spec](spec.md).

| # | Hallazgo | Resolución |
|---|---|---|
| 1 | Una entrada de 900 KB (`source` de 500.000 caracteres y un nombre de campo de 400.000) dejaba 900.200 bytes en `hook-runs.jsonl` y todo en `evidence.runs` de doctor | Cada campo se corta por caracteres: `source` a 32, `folder` a 1.024, `cursor_version` a 32, `agent` a 64 y de `input_keys` los primeros 32 nombres a 64 (HR8). Con esa entrada, la línea queda en 305 bytes. El peor caso con caracteres que JSON escapa sigue siendo de decenas de KiB; ningún test lo pinta |
| 2 | Un `cursor_version` que era un objeto guardaba su contenido, correo incluido | Sólo se guarda un texto JSON; cualquier otro tipo, `null` (HR3) |
| 3 | Un byte que no es UTF-8 hacía que `read_to_string` descartara todo el registro, y con el hook instalado doctor decía falsamente que ningún chat lo corrió | `read_runs` lee bytes y los pasa por `from_utf8_lossy`: se pierde sólo la línea dañada (HR6) |
| 4 | `now_ms - run.at_ms` desbordaba en debug con `at_ms` en `i64::MIN` | `saturating_sub` (HR6) |
| 5 | Un renombrado de rotación que falla abortaba antes de agregar y se perdía la corrida | Se ignora el error del renombrado y se agrega igual; la próxima corrida vuelve a intentarlo (HR4). El test deja un directorio no vacío donde iría `hook-runs.1.jsonl`, que un archivo no puede reemplazar (comprobado en Windows; en Unix no se corrió) |
| 6 | `bad_input` incluye un JSON de forma equivocada (`{"cwd":5}`) y doctor decía «la entrada no era JSON» | El mensaje dice que la entrada no se pudo leer, en los dos idiomas; no se agregó un `outcome` (HR2, HR6) |
| 7 | Una mutación que quitaba el filtro de `folder` para `other_client` sobrevivía | La aserción faltante (arriba) |

En la corrida a mano con la entrada de 900.029 bytes, un `cursor_version` que era un objeto con `leak@example.com` y un `{"cwd":5}`, con el binario de tests `target-test/debug/clawg.exe` en un home temporal (borrado al terminar): las tres corridas salieron con 0; `hook-runs.jsonl` quedó con 3 líneas y 713 bytes en total (305, 206 y 199 bytes); `leak@example.com` aparece 0 veces en el archivo y 0 en la salida de `clawg doctor`, que ocupa 5.548 bytes, y `hooks.runs` dice `nothing: the input could not be read` para la de forma equivocada.

### Una escritura trabada

Una revisión posterior encontró que `record` corría sin límite de tiempo entre la respuesta y la impresión: un disco o un recurso de red que no responde demoraba la respuesta sin tope. Ahora la escritura corre en un hilo aparte y se la espera hasta el plazo (HR4). Esperar sólo lo que queda del plazo deja sin tiempo a una corrida `timed_out`, que antes siempre escribía su línea, así que `session-start` le reserva 50 ms.

Medido en Windows, con 16 núcleos; «cargada» son 32 procesos ocupados:

| Medida | Máquina libre | Máquina cargada |
|---|---|---|
| Una escritura, 400 veces en un directorio temporal, dos tandas | mediana 0,4 ms, máximo 1,3 ms | mediana 0,7 ms, máximo 15 ms |
| Líneas `timed_out` del binario con stdin abierto, sin reserva | 25 de 26 | 12 de 12 |
| Las mismas, con 50 ms de reserva | 20 de 20 | 12 de 12 |

Los conteos son pocos para distinguir las dos variantes: sin reserva la línea depende de una carrera con la salida del proceso, que se perdió una vez. Los 50 ms salen de la primera fila, el triple del peor caso.

En Windows, sobre 0.77.0 y con `target-test` recién limpiado, antes de commitear este arreglo: `cargo test --workspace` sin fallos y los dos clippy salen con 0.

### Mutaciones

| Mutación | Resultado |
|---|---|
| Quitar el filtro de `folder` para `other_client` | Sobrevivía; ahora la mata `every_session_start_names_why` |
| Imprimir antes de registrar | Ya no se puede desde `main.rs`, que recibe la salida de `respond_and_record` una vez registrada; `a_session_start_out_of_time_still_leaves_its_line_by_the_deadline` fija que la línea está cuando esa función vuelve |
| Esperar la escritura sin límite | La mata `a_write_held_up_is_not_waited_for_past_the_deadline`: la espera dura los 10 s que el test retiene la escritura |
| Dejar `RECORD_RESERVE` en cero | La mata `a_session_start_out_of_time_still_leaves_its_line_by_the_deadline`: la respuesta usa los 3 s enteros |

## Lo que no se vio

- El CI sobre `0ff19a7d`, el arreglo de la escritura trabada. El de #126, en verde en Linux y Windows, es anterior: ahí corrió lo de Windows de este cambio (`rename` sobre un archivo que existe, `OpenOptions::append`), que ese arreglo no tocó, pero la espera con plazo sólo corrió en la suite local de Windows.
- Dos hooks que rotan a la vez: cada uno decide por el tamaño que vio, y el segundo puede renombrar el archivo nuevo de un primero que ya rotó, y reemplazar `hook-runs.1.jsonl` por un archivo casi vacío. Pierde la generación vieja, nunca una línea del archivo en uso. No hay test de esa carrera.
- Un `sessionStart` de Cursor real: nada de este cambio prueba qué manda Cursor ni que el contexto llegue al agente. Es lo que `clawg doctor` va a mostrar después, en prod.
- Una escritura trabada de punta a punta: `a_record_that_cannot_be_written_does_not_hold_up_the_answer` usa un FIFO y es sólo de Unix, y el único CI con Linux es anterior a ese test, así que no corrió en ningún sistema. En Windows se compiló sin su `cfg(unix)` pero no corrió, y lo que corre es `a_write_held_up_is_not_waited_for_past_the_deadline`, que no pasa por `main`. Tampoco se midió la carrera de la línea `timed_out` en Unix.

## Veredicto

Sin CRITICAL. Un WARNING: el arreglo de la escritura trabada (`0ff19a7d`) no pasó por ningún CI, y `a_record_that_cannot_be_written_does_not_hold_up_the_answer` no corrió en ningún sistema. El resto del cambio pasó el CI de #126 en Linux y Windows.

## Al archivar (2026-11-04)

- Se archiva con ese WARNING abierto: la validación se hace después, con el CI de Linux y Windows del pull request que suba `0ff19a7d`, y lo que marque se arregla aparte.
- La versión no sube: el código de este cambio salió en 0.76.0, y `0ff19a7d` viene después del bump a 0.77.0 (`5f83185e`), que todavía no tiene tag, así que sale en esa.
- Hasta que salga 0.77.0, la 0.76.0 publicada tiene la escritura sin límite de tiempo que ese arreglo corrige.
