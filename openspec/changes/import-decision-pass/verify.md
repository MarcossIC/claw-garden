# import-decision-pass — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace --locked`) y los dos clippy de `.claude/CLAUDE.md` pasan: el del workspace con `--all-targets` y el de `-p clawg-cli` sin `testing`. Se enlazó ONNX Runtime 1.29 en dinámico (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque la descarga de `ort-sys` no estaba disponible. Ningún test carga el modelo real.

- **ID1, la conexión:** con `a_dry_run_leaves_the_connection_to_other_calls` (10.000 notas de una fila, corrida en seco, sonda `SELECT 1` cada 5 ms):

  | | Espera más larga de la sonda |
  |---|---|
  | Antes (`5e6ee21`, la pasada en un `conn.call`) | 1,34 s y 1,44 s, en dos corridas |
  | Después | entre 0,67 ms y 0,94 ms, en tres corridas |

  El test falla con el código de antes y pasa con el de después, con el tope en 300 ms.
- **ID2, equivalencia:** `the_planner_decides_as_the_sql_pass` compara el `Ledger` en los 16 casos de la tabla de la spec y da igual en todos. `a_dry_run_after_a_purge_reports_the_ids_of_the_import` pasa antes y después. El oráculo de import de `import.rs`, `tests/api.rs` y `tests/binary.rs` pasa sin editar ninguna prueba.
- **ID3, la parada:** `a_stopped_planner_decides_nothing`.
- **ID4, los índices:** `the_import_lookups_seek_an_index`, con las cuatro consultas de carga y `TWIN_SQL` con `id` en su orden.

### La pasada de decisión, medida sola

Es `write` con `dry_run` sobre 15.047 notas de una fila en un store vacío, en debug, con un test temporal que no se commiteó:

| | Tiempo | Llamada más larga a la conexión |
|---|---|---|
| Antes | 2,12 s y 2,10 s | la pasada entera, en una llamada |
| Después | 0,32 s y 0,34 s, carga y decisión juntas | unos milisegundos (`LOAD_BATCH`) |

**El umbral de la propuesta no se cumple tal como quedó escrito.** Pedía bajar a menos de la décima parte de «los 16,4 s de hoy con el índice», pero ese número era el de un import entero del banco de 81, no el de la pasada de decisión sola. Contra la pasada medida acá, la reducción es de 6,4 veces. El criterio que el cambio persigue, que ninguna llamada de la pasada ocupe la conexión más que una fracción del plazo del hook, se cumple por tres órdenes de magnitud (ID1).

### El banco de 81

`twice_the_notes_import_in_at_most_three_times_the_time`, en debug y no en release, que pide autorización: 10.000 notas en 3,28 s y 20.000 en 6,32 s, una razón de 1,93, por debajo del tope de 3.

## Pendiente

- La suite y clippy en Windows (`target-test`).
- El CI del pull request en Linux y Windows.
- **[perfil dev, con autorización]** Una importación real con sondas (`/health`, listado y búsqueda): que la fase de decisión no agote ningún plazo.
