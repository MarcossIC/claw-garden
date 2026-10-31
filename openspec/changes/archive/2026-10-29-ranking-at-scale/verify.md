# ranking-at-scale — Verificación

## Paso 1: medir a escala

### Windows

- Con el arreglo del parser del contexto (abajo), `cargo test --workspace --target-dir target-test` pasa. `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` salen con 0, los dos en `target-test`.
- Los tests del paso 1 se escribieron antes que el código y fallaron por la aserción o porque no compilaban (tareas 1 a 10). Este registro no guarda la salida de cada rojo.

### CI

El pull request #99 (`develop` → `main`) pasó en Linux y Windows con el código del paso 1 (tarea 12).

### Los dos cortes (tarea 13)

| Corte | sha256 | Filas vivas | Esquema |
|---|---|---|---|
| `old-cut.db`, `clawg backup --profile dev` | `b7edae9d…` | 51 998 | V24, migrado a V31 en la copia |
| `new-cut.db`, `engram export --all` reimportado en un home aislado | `180b9862…` | 54 731 | V31 |

El `placement` de los dos coincide, salvo las notas que engram sumó o borró entre uno y otro. Los dos viven en `docs/eval/backgrounds/`, fuera de git, y se borran al archivar.

### El costo de la escala en el ancla (tarea 15)

El ancla de 46 casos (`docs/eval/golden.json`), con el modelo real:

| Corrida | recall@5 | MRR |
|---|---|---|
| Sin fondo | 0,652 | 0,552 |
| Sobre el corte viejo | 0,348 | 0,324 |
| Sobre el corte nuevo | 0,326 | 0,308 |

`rag-eval --diff`, sin fondo contra cada corte: 14 y 15 casos empeoran en recall@5 y ninguno mejora (p = 0,0001 en los dos).

### El set minado (tarea 14)

`docs/eval/golden-mined.json`, fuera de git: 89 casos con respuesta (`es>es` 47, `es>en` 42) y 23 sin respuesta. Todas las consultas están en español.

- Se revisaron 156 borradores en dos tandas, uno por uno con la persona. Se descartaron 44: las 36 consultas en inglés, porque la persona le pregunta al agente en español y espera la respuesta en español, y 8 `es>es` que no se entendían o nombraban fases del plan. La pareja `en>en` la sigue midiendo el ancla.
- Por tipo de la save esperada: `architectural_decision` 34, `semantic_fact` 31, `session_summary` 15, `user_preference` 9.
- Por partición: 65 con respuesta en calibración y 24 en prueba, y 18 sin respuesta en calibración y 5 en prueba.
- Ningún caso falla en los dos cortes (`NoLiveRow`, `OutsideProject`): las cuatro corridas de abajo salen con 0.

### Línea base del set minado (tarea 15)

Las cuatro corridas son del 2026-10-29, con 2 minutos entre la del corte viejo y la del nuevo:

- `2026-10-29-real-mined-{old,new}-cut-bg-{b7edae9d,180b9862}.json`: la ruta ordenada.
- `2026-10-29-real-mined-{old,new}-cut-agent-view-bg-{b7edae9d,180b9862}.json`: agent-view.

| | Corte viejo | Corte nuevo |
|---|---|---|
| recall@5 | 0,528 | 0,517 |
| recall@5 `es>es` | 27/47 | 26/47 |
| recall@5 `es>en` | 20/42 | 20/42 |
| recall@10 | 0,663 | 0,629 |
| MRR | 0,473 | 0,439 |
| agent-view `search.found` | 0,663 | 0,629 |
| agent-view `context.found` | 0,640 | 0,629 |
| Sin respuesta en que la búsqueda no mostró nada | 0/23 | 0/23 |
| Casos de pool corto (`scope` / `crowded`) | 0 / 26 | 0 / 13 |

`rag-eval --diff` entre los dos cortes no distingue nada: en recall@5 se mueven 3 casos (p = 1,0) y en `search.found` 7 (p = 0,45). Las puertas se leen sobre el corte nuevo.

La primera corrida de agent-view dio 0 en `context.found` en los dos cortes. `hit_text` (`agent_view/parse.rs`) descartaba todo hit con la marca `by <autor>`, y sobre un fondo casi todo lo guardó otro agente. Se arregló con su test (`a_context_hit_another_agent_saved_is_one_segment`, que falló con `[]` antes del cambio), y las dos corridas de agent-view se repitieron con `--force`. La ruta ordenada no usa ese parser.

## Puerta 1 (tarea 16)

`session_summary` ocupa el **7,1 %** de los lugares del top 3 en el corte nuevo, en calibración, en los casos que no piden un resumen (12 de 168, con los de pool corto incluidos o sin ellos). Es menos que su proporción de filas, que es el 11,7 %. Lo mismo en prueba (5,9 %) y en el corte viejo (6,3 %). Está por debajo del 25 % de U1, así que **el paso 2 se cierra sin código**. Las tareas 17 a 19 no se hacen.

Lo que los números dejan ver además:

- **Los resúmenes inundan solo cuando no hay respuesta.** En los casos sin respuesta, `session_summary` ocupa el 33,3 % del top 3 (18 de 54 en calibración, en los dos cortes). Las 12 búsquedas de validación de la exploración, donde ocuparon el 53 %, tenían 5 sin respuesta. Bajar el peso de los resúmenes solo reordenaría hits que no responden, y ese es el problema de la abstención (paso 3).
- **Los resúmenes que inundan son recientes.** La mediana de edad de los del top 3 es de 10 días, y ninguno pasa de 209 días, así que B3 no tendría sobre qué actuar.
- **Los casos que piden un resumen están mal servidos.** Son 15, y ninguno es el último resumen de su proyecto (`expected_is_last_summary` falso). Solo 4 lo encuentran en el top 5 en cada corte. Un peso menor a 1,0 los empeoraría.

## Puerta 2 (tarea 20)

En el corte nuevo, en calibración, hay 63 casos con respuesta (los que tienen `best_cosine` y `score_gap_1_10`) y 18 sin respuesta. Para U3 haría falta abstener en 15 de 18 (la cota de Wilson 0,584) perdiendo a lo sumo 2 casos con respuesta.

| Criterio | Rango sin respuesta | Mínimo con respuesta | Para abstener en 15/18 | Mejor corte que pierde ≤ 2 |
|---|---|---|---|---|
| C1, `best_cosine` | 0,854–0,891 | 0,851 | pierde 19 de 63 | abstiene en 0/18 |
| C2, `score_gap_1_10` | 0,020–0,126 | 0,021 | pierde 44 de 63 | abstiene en 1/18 |
| C3, C1 y C2 juntos, el mejor par de umbrales | — | — | — | abstiene en 2/18 (y), 1/18 (o) |

En el corte viejo da lo mismo: C1 pierde 15 de 63 para llegar a 15/18, y C3 abstiene a lo sumo en 4/18. Ninguno separa, así que **el paso 3 se cierra**. Las tareas 21 a 23 no se hacen. Los casos y las distribuciones (`retrieval.scale.distributions` y `retrieval.unanswerable` de cada corrida) quedan escritos.

U3 tampoco se podría haber medido con este set: la partición por hash deja 5 casos sin respuesta en prueba, y 5 de 5 da una cota de Wilson de 0,566. Una abstención futura necesita bastantes más casos sin respuesta (unos 65 para tener 20 en prueba) y una señal que no sea la del coseno del mejor hit.
