# import-timing — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace --locked`) y los dos clippy de `.claude/CLAUDE.md` pasan: el del workspace con `--all-targets` y el de `-p clawg-cli` sin `testing`. Se enlazó ONNX Runtime 1.29 en dinámico (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque la descarga de `ort-sys` no estaba disponible. Ningún test carga el modelo real.

- **IT1, las fases:** `an_import_times_each_step_once_in_order` (las notas de muestra de otra herramienta, con una nota borrada en la fuente: `embed` tiene las insertadas no borradas), `a_clawg_file_is_prepared_rather_than_translated` y `a_dry_run_times_only_what_it_runs`.
- **IT2, la conexión:** `time_in_sqlite_never_exceeds_the_call`. Con un export de clawg de 600 filas en un store vacío, `load` hace 4 llamadas (3 lotes de textos y las raíces) y `write` los 3 lotes de `note_batches`. En las dos fases, el tiempo en SQLite más la espera no supera la duración, y la llamada más lenta no supera el tiempo en SQLite.
- **IT3, el texto:** `a_step_reads_as_its_numbers` y `the_summary_names_every_step` (`timing.rs`), que no compilaban antes de las estructuras.
- **IT4, sin cambios:** las pruebas de import de `import.rs` y `undo.rs`, `tests/api.rs` y `tests/binary.rs` pasan. La única edición fue el argumento `&mut ImportTimings` en las llamadas a `import`, `import_foreign`, `prepare_file`, `prepare_export` y `load`. Ninguna aserción cambió.

## CI

El del pull request #91 (cabeza `98d13eb`) pasó en `ubuntu-latest` y en `windows-latest`.

## La importación real en el perfil dev (Windows, 2026-10-24)

Con autorización, como en import-linear e import-decision-pass. El binario es el build debug de `9e1624e` (0.61.0) en `target-test`, con la feature `testing` y `target-test` limpio de los artefactos del workspace. Su `crates/` es el de la cabeza del pull request. Usa el modelo real y el perfil dev en un `CLAWG_TEST_HOME` aislado, con una copia del snapshot `pre-import` del 2026-10-23 (`~/.clawg-dev/backups/memory-2026-10-24T01-08-10.782Z-pre-import.db`) y otra del modelo. `~/.clawg-dev` no se abrió. Al arrancar, el daemon migró la copia de V24 a V26. Sirvió sólo por TCP porque la ruta del socket en el directorio temporal mide 175 bytes. El export fue un `engram export --all` nuevo, con las mismas 3.814 notas de las mediciones anteriores (3.800 observaciones y 14 prompts). Al terminar se borraron el export y el home. Primero corrió el dry run y después la importación real, una corrida de cada una y sin otros clientes.

| Fase | Dry run | Real | Parte del real | Detalle del real |
|---|---|---|---|---|
| `read` | 0,2 s | 0,2 s | 0,01 % | — |
| `plan` | 0,0 s | 0,0 s | — | 1 llamada |
| `translate` | 72,0 s | 91,9 s | 6,5 % | 54.457 filas |
| `load` | 0,5 s | 0,4 s | 0,03 % | 0 filas leídas, 241 llamadas, 0,2 s en SQLite, 0,0 s en cola, la más lenta 1 ms |
| `decide` | 1,0 s | 0,8 s | 0,06 % | 54.457 filas |
| `embed` | — | 1.289,1 s | 91,3 % | 54.321 filas, 1.698 tandas, 42,1 filas/s, la más lenta 3.745 ms |
| `snapshot` | — | 0,3 s | 0,02 % | — |
| `write` | — | 27,9 s | 2,0 % | 54.457 filas, 235 llamadas, 27,8 s en SQLite, 0,0 s en cola, la más lenta 314 ms |
| `links` | — | 1,0 s | 0,07 % | 1 llamada de 959 ms |
| Total del handler (`done in`) | 73,9 s | 1.411,6 s | | |
| Cliente (`clawg import`) | 74,28 s | 1.412,03 s | | |

**El cierre se cumple.** En la importación real, la suma de las fases (1.411,6 s) queda a 0,43 s del cliente, un resto de 0,03 %. En el dry run el resto es de 0,58 s (0,78 %). Las dos quedan por debajo del 1 %, y el resto es casi el mismo en las dos, así que es un costo fijo. `http.duration{route=/v1/import}` suma 1.485,69 s en las dos peticiones, contra 1.486,30 s del cliente y 1.485,5 s de los dos handlers. Hay unos 0,6 s fuera del daemon (leer el archivo y mandar los 14,6 MB del cuerpo) y unos 0,2 s entre el middleware y el handler (recibir y deserializar el cuerpo).

**El reporte y las filas.** La importación real dio exit 0. Su reporte coincide con el del dry run, salvo la primera línea y el número del import. Hay 54.457 filas en el archivo y 54.321 insertadas, todas vivas. Son un 4,6 % más que las 52.080 de import-decision-pass, por el corte en frontera de palabra y la cabecera de 32 tokens de piece-boundaries.

**Cómo se lee.**

- **`embed` es el cuello de botella: el 91,3 %.** Una tanda de 32 filas tarda 759 ms de media, y la más lenta tarda 4,9 veces eso. 42,1 filas/s confirma los ~40 que import-decision-pass había inferido restando. Cuánto de esa fase es tokenizar, rellenar, inferir o agrupar lo mide embed-throughput (85).
- **`translate` es el 6,5 %, y todo en Rust compilado en debug.** Enmascara, detecta el idioma y parte cada nota con el tokenizer en un solo hilo. Entre el dry run y la real hizo el mismo trabajo con 20 s de diferencia (72,0 s y 91,9 s). Con una corrida de cada lado, esa diferencia no se puede atribuir a la importación.
- **`write` es el 2,0 %, y es SQLite puro.** De sus 27,9 s, 27,8 s fueron dentro de la clausura y nada en cola. Son 118 ms por llamada de ~232 filas: la fila, su vector, el índice de texto y el diario.
- **El resto suma 2,7 s (0,2 %).** `load` y `decide`, que antes de import-decision-pass ocupaban la conexión durante minutos, ahora suman 1,2 s.
- **La cola está en 0,0 s en todas las fases.** En este home no había otro cliente. En prod, las llamadas del puente pueden sumar espera, y la línea de cada fase lo va a mostrar.
- **Contra la medición anterior.** import-decision-pass tardó 1.362 s para 52.080 filas (26,2 ms por fila), y esta tardó 1.411,6 s para 54.457 (25,9 ms por fila). Lo que sumó lo explican las filas de más. Las inferencias de entonces eran ~19 min de embedding, ~23 s de escritura y ~1 min de traducción. Las tres se confirman en orden de magnitud.
- **Es un build debug.** La inferencia de ONNX Runtime es código nativo optimizado igual. Tokenizar, rellenar, agrupar, `translate` y el Rust de `write` no lo son. La corrida en prod, con el release que lleve este cambio, va a dar el primer reparto en release.

## Al archivar (2026-10-24)

- IT1 a IT4 pasan a [`specs/portability`](../../../specs/portability/spec.md) con sus ids. IT1 suma de la propuesta que `prepare` se registra una vez, después de `plan`, y que una importación que falla deja las fases que terminó; IT2, que `snapshot` sólo lleva su duración.
- «Dónde está» de `specs/portability` nombra lo que registra (`timing.rs` y `timed`), y sus citas de `import.rs` y `api.rs`, que este cambio corrió entre 1 y 60 líneas, apuntan otra vez a cada símbolo.
- Las citas `import.rs:N` de la exploración y de la propuesta quedan como estaban: describen el código contra el que se escribieron.
