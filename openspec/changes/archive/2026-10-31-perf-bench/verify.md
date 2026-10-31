# perf-bench — Verificación

**Veredicto: PASS WITH WARNINGS.** La suite y los dos clippy pasan. El banco se verificó a sí mismo (PB8) después de un ajuste del `diff`: la primera lectura de las dos corridas del mismo commit dio un falso `slower`, y el mínimo de muestras para decidir pasó de 20 a 40 (`2890f4f`). Los avisos están al final.

## Suite y clippy

Sobre `136fdc4`, rebasado sobre `develop` (`e01d93c`), en Linux:

- `cargo test --workspace --no-fail-fast`: pasa todo menos `the_shipped_golden_set_names_docs_this_workspace_has` (`crates/clawg-eval/src/bin/rag-eval/docs.rs`). Ese test falló en el `develop` anterior al rebase (`12d043d`), cuyo golden de docs tenía 39 preguntas y el test pide 40; `cabfaa8`, de `develop`, agrega la que faltaba, y después del rebase pasa. Ningún archivo de este cambio lo toca.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: sin avisos.
- Después de `2890f4f` (sólo `perf-bench`): `cargo test -p clawg-eval --bin perf-bench` pasa.

El código de Windows de `usage_of` (`GetProcessTimes`, `GetProcessMemoryInfo`) no se compiló acá: lo compila el clippy de CI en Windows.

## Las corridas (PB8)

Tres corridas de `cargo test -p clawg-cli --test perf_bench -- --ignored --nocapture` sobre `136fdc4`, en un build debug, con el embedder falso, 10 000 filas y la semilla 74, en Linux x86_64 con 4 hilos, una detrás de otra en la misma máquina:

| Corrida | Tag | Qué cambia | Siembra | CPU bajo carga | Pico RSS |
|---|---|---|---|---|---|
| A | `verify-a` | — | 10,9 s | 0,431 | 95,8 MB |
| B | `verify-b` | — | 9,0 s | 0,433 | 97,3 MB |
| S | `verify-sleep` | `tokio::time::sleep(50 ms)` al comienzo de `search` (`crates/clawg-cli/src/daemon/api.rs`), revertido después | 9,6 s | 0,437 | 100,4 MB |

La CPU en reposo dio 0 en las tres: el daemon no gasta un tick de reloj en 5 s sin peticiones. Ningún escenario tuvo fallos. Cada corrida tardó unos 3,5 minutos.

### A contra B, el mismo commit

Primera lectura, con el mínimo en 20: `context_during_backup` salió `slower` (p95 426,2 → 687,1 ms, +61,2 % [+18,5, +87,2]) y `--fail-on-regression` salió con 6. Con 20 muestras por lado, el p95 por rango es la muestra 19 de 20: queda una sola por encima, y el veredicto dependía de una petición que cayó detrás de un backup. Lo mismo, sin llegar al veredicto, en los flujos de 10 y 11 muestras.

Con el mínimo en 40 (`2890f4f`), `perf-bench diff A B --fail-on-regression` sale con 0:

| Escenario | n | p95 ms | cambio p95 [95 %] | Veredicto |
|---|---|---|---|---|
| `search` | 100/100 | 86,5 / 79,4 | −8,2 % [−13,2, +1,3] | no change |
| `context` | 60/60 | 160,3 / 145,7 | −9,1 % [−14,2, −0,2] | no change |
| `session_start` | 60/60 | 87,1 / 87,4 | +0,4 % [−5,3, +7,5] | no change |
| `save_1` | 60/60 | 55,8 / 49,8 | −10,8 % [−53,7, +10,8] | no change |
| `search_during_save_100` | 51/51 | 243,1 / 260,1 | +7,0 % [−19,6, +23,9] | no change |
| `search_during_backup` | 50/50 | 203,8 / 276,1 | +35,5 % [−10,4, +65,6] | no change |
| `search_during_docs_refresh` | 51/46 | 182,6 / 179,9 | −1,5 % [−14,3, +31,6] | no change |

Los demás escenarios tienen menos de 40 muestras de algún lado y salen `too few samples`: `save_10` (20), `save_100` (4) y los `context`, `session_start` y `save_1` de fondo (10 a 21).

### A contra S, 50 ms en la búsqueda

`perf-bench diff A S --fail-on-regression` sale con 6:

| Escenario | p95 ms | cambio p95 [95 %] | p rangos | Veredicto |
|---|---|---|---|---|
| `search` | 86,5 / 139,3 | +61,0 % [+52,0, +77,8] | < 0,0001 | slower |
| `search_during_backup` | 203,8 / 288,3 | +41,5 % [+9,8, +86,1] | < 0,0001 | slower |
| `search_during_docs_refresh` | 182,6 / 233,1 | +27,6 % [+12,3, +44,7] | < 0,0001 | slower |
| `search_during_save_100` | 243,1 / 287,2 | +18,2 % [−13,0, +49,6] | < 0,0001 | no change |
| `context` | 160,3 / 160,0 | −0,2 % [−4,6, +11,6] | 0,027 | no change |
| `session_start` | 87,1 / 93,1 | +6,9 % [−6,5, +15,5] | 0,74 | no change |
| `save_1` | 55,8 / 56,6 | +1,5 % [−48,5, +44,1] | 0,006 | no change |

Ningún escenario que no busca sale `slower`, y `search` sube su p50 de 67,7 a 121,2 ms (+78,9 %): la espera más algo de cola. `search_during_save_100` se mueve (el valor p de rangos lo ve) pero queda bajo el umbral, porque detrás del save largo la espera de la conexión pesa más que los 50 ms.

## Avisos

1. **El mínimo de 40 se eligió leyendo esta verificación.** Es la cota que deja al menos dos muestras sobre el p95 por rango, pero se subió después de ver el falso positivo; la próxima pareja de corridas del mismo commit es la que lo confirma.
2. **Muchos escenarios de fondo no deciden.** Con las tasas de un build debug, `context`, `session_start` y `save_1` mientras corre el fondo dejan 10 a 21 muestras. Las puertas de 57 que los necesiten tienen que alargar la ventana (`MIN_BACKGROUND`) o correr varias veces; `search` de fondo sí decide.
3. **Sin el modelo real y sin release.** Las corridas con `CLAWG_BENCH_MODEL_DIR` y `--release` necesitan autorización; son las de las puertas de 57 y 62.
