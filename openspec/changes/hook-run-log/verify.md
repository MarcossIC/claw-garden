# hook-run-log — Verificación

## Suite y clippy

En Windows x86_64, build debug, toolchain 1.99.1, en `target-test` (limpiado con `cargo clean --workspace --target-dir target-test` antes de la primera corrida), sobre `cd446330`, el último commit que toca código es `e4bd99bd` (los que siguen sólo tocan docs):

- `cargo test --locked --workspace --no-fail-fast --target-dir target-test`: sale con 0, ningún binario con fallas; los ignorados son los de antes (los que piden el modelo real o un ensayo a mano). `session_start_in_a_declared_subfolder_reads_its_project`, que falló de forma intermitente con la CPU cargada (el plazo de 3 s del hook) y es anterior a este cambio, no falló.
- `cargo clippy --locked --workspace --all-targets --target-dir target-test -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -D warnings` también.
- Los tests de `clawg hook` que ya existían pasan sin cambiar lo que esperan: sólo se tocó el ayudante `broken_daemon`, que ahora delega en `daemon_answering` con el mismo mensaje de error.
- El CI de Windows no corrió: no hay pull request todavía.

## Los tests en rojo

Escritos antes del código de cada commit.

| Commit | Antes | Después |
|---|---|---|
| `feat(hook): record each session start` | `cargo test -p clawg-cli --lib hook` no compilaba: 73 errores (`cannot find type Outcome`, `Run` y `FILE`, `cannot find function respond_with_run`, `record`, `line`, `input_keys`, `read_runs`, `parse_runs`, `append`, `daemon_answering` y `start_run`) | los 64 tests de ese filtro pasan |
| `feat(doctor): show the hook's recent session starts` | `cargo test -p clawg-cli --lib hook` no compilaba: `unresolved import crate::hook_log::{Outcome, Run}` en `checks.rs` (un error de resolución, que tapa los de `hook_runs`, `age`, `folder_name`, `installed_in` y `hook_installed`, que tampoco existían) | pasan los tests nuevos de `checks.rs` y `setup/cursor.rs`, y los de `tests/binary.rs` |

Los tests de `tests/binary.rs` de los dos commits se escribieron con los unitarios y no se corrieron en rojo por separado: el binario no compila sin el módulo, así que su rojo es el de arriba.

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
| HR1 | `respond_with_run` arma `Run` (`crates/clawg-cli/src/hook.rs`, junto a `respond`), `hook_log::line` y `input_keys` (`crates/clawg-cli/src/hook_log.rs`); `main.rs` registra con `hook_log::record` |
| HR2 | `answer` devuelve el motivo de cada salida, `missing_folder` y `daemon_failure`; `respond_with_run` suma `bad_input`, `other_client` y `timed_out` |
| HR3 | `Run` sólo tiene los campos de la spec; `version_text` corta el texto de `cursor_version` a 32 caracteres; `folder` es `None` con varias raíces (lo decide `folder`, `hook.rs`) |
| HR4 | `hook_log::append` (una `write_all`, `rename` a 64 KiB, `create(true)` sin `create_dir`), `record` traga el resultado, `main.rs` registra antes de `println!` |
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

## Lo que no se vio

- Windows en CI: no hay pull request. Lo de Windows de este cambio (`rename` sobre un archivo que existe, `OpenOptions::append`) corrió sólo en la suite local.
- Dos hooks que rotan a la vez: cada uno decide por el tamaño que vio, y el segundo puede renombrar el archivo nuevo de un primero que ya rotó, y reemplazar `hook-runs.1.jsonl` por un archivo casi vacío. Pierde la generación vieja, nunca una línea del archivo en uso. No hay test de esa carrera.
- Un `sessionStart` de Cursor real: nada de este cambio prueba qué manda Cursor ni que el contexto llegue al agente. Es lo que `clawg doctor` va a mostrar después, en prod.

## Veredicto

Sin CRITICAL en Windows local. Falta el CI de Windows del pull request, que queda como WARNING.
