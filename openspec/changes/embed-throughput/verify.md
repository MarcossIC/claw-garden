# embed-throughput — Verificación

## Windows (2026-10-27)

`cargo test --workspace --target-dir target-test` pasa entero, sin fallas, con `target-test` limpio de los artefactos del workspace antes del primer build. También pasan los dos clippy de `.claude/CLAUDE.md`: `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`.

Los tests de [ET1](spec.md#et1-una-importación-embebe-primero-las-filas-de-más-tokens) primero no compilaron: faltaban `count_all` y `longest_first`, y `import` no tomaba el chunker. Con `count_all` y `longest_first` ya escritos, `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order` y `counting_together_counts_each_text_as_alone` pasaron. Con `write` todavía en el orden del archivo, `an_import_of_another_tool_embeds_its_rows_longest_first` y `a_clawg_import_embeds_its_rows_longest_first` fallaron por la aserción: los tokens llegaban como `[4, 9, 10, 4, 9, 14, 8, 13, …]` y `[4, 6, 6, 4, 11, 11, 5, 2, …]`. Con `embedding_order` en `write`, pasan. `each_imported_row_keeps_the_vector_of_its_own_text` ([ET2](spec.md#et2-el-orden-no-cambia-lo-que-se-escribe)) pasó antes de cambiar `write` y sigue pasando después. Las pruebas de import que ya existían pasan sin cambiar ninguna aserción.

## La importación real en un perfil dev aislado (Windows, 2026-10-27)

Con autorización. Los dos binarios son builds debug con la feature `testing`, compilados en `target-test`:

- **Base:** `29d67c6`, el padre de `aab6993`, compilado desde un worktree que se borró al terminar.
- **Cambio:** `aab6993`. La cabeza `e469539` sólo suma docs.

Cada corrida usó su propio `CLAWG_TEST_HOME`, con un store vacío y una copia del modelo de `~/.clawg-dev/models`. De `~/.clawg-dev` sólo se leyó el modelo para copiarlo. El daemon se arrancó con `--port 0` y sirvió sólo por TCP, porque la ruta del socket mide 179 bytes. No hubo `--project-map`, así que todas las filas quedaron sin proyecto.

El export fue un `engram export --all` nuevo, de engram 3.5.2, con 3.814 observaciones y 14 prompts. Son 14 observaciones más que en la medición de la [propuesta](proposal.md#problema), y por eso hay 54.386 filas para embeber y no 54.321. Las dos corridas usaron el mismo archivo y corrieron en serie, la base primero, una de cada una y sin otros clientes. Al terminar se borraron el export, los dos homes y los artefactos del workspace en `target-test`.

Un primer intento de la base no importó nada. El script esperaba `"embedder": "ready"` con un `rg` que no estaba en el `PATH` de su shell, así que nunca lo vio, y a los 10 minutos paró el daemon de forma ordenada sin haber escrito ninguna fila. La base se repitió con un home nuevo.

| Fase | Base | Cambio | Diferencia |
|---|---|---|---|
| `read` | 0,2 s | 0,2 s | — |
| `translate` | 98,8 s | 86,5 s | no la toca el cambio |
| `load` | 0,5 s | 0,5 s | — |
| `decide` | 1,1 s | 1,0 s | — |
| `embed` | 1.187,2 s: 54.386 filas, 1.700 tandas, 45,8 filas/s, la más lenta 3.488 ms | 825,8 s: 54.386 filas, 1.700 tandas, 65,9 filas/s, la más lenta 3.099 ms | **−30,4 %** |
| `snapshot` | 0,1 s | 0,0 s | — |
| `write` | 32,5 s | 32,4 s | — |
| `links` | 1,4 s | 1,2 s | — |
| Total del handler (`done in`) | 1.322,0 s | 947,9 s | −28,3 % |
| Cliente (`clawg import`) | 1.322,36 s | 948,31 s | −28,3 % |

**Los tres criterios se cumplen.** Se fijaron en la propuesta, antes de correr la del cambio:

- **`embed` tarda al menos un 15 % menos.** Tardó un 30,4 % menos: de 1.187,2 a 825,8 s, y de 45,8 a 65,9 filas/s.
- **El reporte es el mismo.** Las dos importaciones salieron con exit 0. Sus reportes son iguales línea por línea, salvo la ruta del snapshot `pre-import` y el número del import: 54.386 de 54.522 filas insertadas, las mismas por tipo, estado y proyecto, `by_lang` con en 18.824, es 26.907 y und 8.655, y el mismo bloque `foreign`.
- **La memoria no sube más de un 5 %.**
  - La memoria privada con que termina `embed` pasa de 982,4 a 981,2 MiB (−0,1 %).
  - En `metrics.db`, el máximo por minuto de `process.private_peak_bytes` durante `embed` pasa de 1.561,3 a 1.561,7 MiB (+0,03 %). En las dos corridas ya tiene ese valor en el primer minuto de `embed`: el pico de la vida del proceso lo pone una fase anterior, no la de embeddings.
  - El máximo por minuto de `process.private_bytes` dentro de `embed` baja de 1.268,0 a 999,9 MiB (−21,1 %).
  - `process.rss_peak_bytes` pasa de 1.581,5 a 1.581,7 MiB.

**Cómo se lee.**

- **El total gana menos que la muestra, como anticipaba la propuesta.** Las primeras 5.000 filas bajaron un 37,0 % (M3) porque rellenaban 1,458 veces sus tokens. El total rellena 1,293 veces los tokens y 2,042 veces la atención (M1), y baja un 30,4 %.
- **Contar los tokens queda dentro de `embed`, y no se midió aparte acá.** La propuesta lo midió en 5,4 s en debug para todas las filas, el 0,5 % de la fase.
- **`translate` dio 12,3 s menos en el cambio, y el cambio no la toca.** Con una corrida por lado, esa diferencia no se le puede atribuir: import-timing vio 20 s entre dos corridas con el mismo trabajo. La diferencia de `embed`, 361,4 s, está muy por encima de eso.
- **Es un build debug.** La sesión de ONNX Runtime es la misma en debug y en release, y es casi toda la fase (M2), así que la ganancia en release debería parecerse. Esa medida la va a dar el próximo import en prod.
