# revision-eval — Verificación

## Windows (2026-10-21)

Sobre `4cca8c9`, en `target-test`, con el compilador de `rust-toolchain.toml`. Los commits que siguen hasta este documento sólo cambian docs de openspec. Antes se vació `target-test` (`cargo clean --target-dir target-test`): `f17deac` cambió `Cargo.lock`, y el directorio todavía guardaba los artefactos de las versiones anteriores de `rustls-webpki` y `webpki-root-certs`.

- `cargo test --locked --workspace --target-dir target-test`: pasa, ningún test falla.
- `cargo clippy --locked --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings` sale con 0. El binario que se publica no cambia: fuera de `crates/clawg-eval`, el cambio sólo tocó `crates/clawg-core/src/eval_api.rs` (`c442c5d`, `23fe6c5`), que existe sólo con la feature `eval` (`crates/clawg-core/src/lib.rs:18-19`), y el build de release no la enciende.
- Al terminar, los únicos directorios de build son `target` y `target-test`.

Linux no se corrió a mano: el CI del pull request que lleve el cambio a `main` corre la suite y clippy en los dos sistemas.

## Requisitos y tests

Cada test que nombra la [especificación](spec.md) existe y pasa en la suite de arriba. Las [tareas](tasks.md) nombran los que agregó el diseño.

| Requisito | Dónde |
|---|---|
| E1, el set | `set.rs`; el set entregado pasa los mínimos (`the_shipped_chain_set_meets_every_minimum`) |
| E2, clases | `chains.rs` (`every_transition_falls_in_exactly_one_class`, `a_row_beyond_the_candidate_pool_is_out_of_pool`, `:531`, y los demás) |
| E3, M2 | `chains.rs` (`m2_*`, `an_old_version_above_the_new_one_is_recorded`) |
| E4, M4 | `cost.rs` (`m4_reports_the_fixed_cost_of_the_key_from_the_contract_and_the_protocol`, `:289`, y la reescritura); `the_production_context_still_omits_the_key` en `run.rs:793` |
| E5, `eval_api` | `crates/clawg-core/src/eval_api.rs` (`save_*`, `joined_pieces_equal_the_text_read_memory_returns`, `the_band_constants_are_the_detectors`, `:1528`); `the_router_lists_a_revised_row_superseded_by_the_new_one` en `run.rs:682` |
| E6, `--fake` | `run.rs:715`, `:774`, `:830` |
| E7, `store` | `store.rs` (`an_unknown_export_format_writes_nothing`, `:828`, y los demás); `a_rate_carries_its_wilson_interval` en `stats.rs:96` |
| E8, privacidad | `the_results_file_of_store_holds_no_stored_text` (`main.rs:501`), `the_working_files_are_removed_even_when_the_run_fails` (`run.rs:854`); después de la corrida real no queda ningún `revision-eval-*` en el temporal del sistema, y el resultado de `store` sólo tiene conteos, tasas e intervalos |
| E9, resultados | `a_result_header_names_the_model_contract_and_the_set_hash` (`main.rs:470`) |

Las rutas sin prefijo están bajo `crates/clawg-eval/src/bin/revision-eval/`.

## Compuerta de `rag-eval` (E5, D7)

Corrida el 2026-10-21 sobre `b32089b`, con el modelo del perfil dev (`multilingual-e5-small/mean`) y el golden de `docs/eval/` (46 casos). Los resultados son `2026-10-21-real-revision-eval-agent-view.json` y `2026-10-21-real-revision-eval.json`, en `docs/eval/results/`, fuera de git. `rag-eval --diff` contra `2026-10-20-real-untrusted-recall-fix-agent-view.json` y contra `2026-10-18-real-search-abstention.json` no mueve ningún caso en recuperación, similitud ni vista del agente, tampoco en ningún par de idiomas, y `search.tokens` y `context.tokens` tienen un cambio medio de +0,0000. `save` no movió lo que `send` siembra, y el resto de `eval_api` tampoco.

## `chains --real` (M1, M2, M4)

`docs/eval/results/2026-10-21-real-revision-eval-chains.json`, fuera de git: `model_contract` `multilingual-e5-small/mean`, `clawg_version` 0.43.0, `set_version` 1, `set_sha256` `fcc21149…be5a`, 40 cadenas, 56 transiciones y 684 distractores.

**Brazos** (`arms`): con clave y en el control, una versión viva en las 40 cadenas (`chains_with_one_live_version` 40); sin clave, 96 versiones vivas y ninguna cadena con una sola.

**M1** (`m1`), sobre las 56 transiciones del brazo sin clave:

| Clase | k/56 | Tasa (Wilson) |
|---|---|---|
| `escalated` | 28 | 50 % (37,3–62,7 %) |
| `pending` | 14 | 25 % (15,5–37,7 %) |
| `no_mark` | 14 | 25 % (15,5–37,7 %): 6 `below_floor`, 7 `above_threshold`, 1 `out_of_pool`, 0 `not_claim_bearing` |
| vieja arriba de la nueva (`above`) | 35 | 62,5 % (49,4–74,0 %) |
| `unmarked_or_above` | 40 | 71,4 % (58,5–81,6 %) |

Por corte, `unmarked_or_above` da 14/19 en `value`, 18/20 en `rewrite` y 8/17 en `extend`; 23/28 en `en` y 17/28 en `es`; 20/28 en decisiones, 9/14 en preferencias y 11/14 en hechos. Ninguna transición de `extend` queda sin marca: 14 de 17 se escalan. Los hechos casi nunca se escalan (1 de 14) y quedan `pending` (11), porque sólo se escalan por tipo las preferencias y las decisiones.

**M2** (`m2`): `verdict` `fail`.

- (a) `one_live_version.met` y (b) `no_old_version_shown.met`: se cumplen. Con clave, ninguna versión vieja aparece en `search_memory` ni en `recall_context`.
- (c) `last_where_control.met`: no se cumple. Coinciden con el control 38 cadenas, justo las necesarias (`matching` 38, `needed` 38), pero `en-repo-layout` baja dos puestos (`dropped`): su última versión queda en el puesto 7 de `recall_context` en el control y en el 9 con clave (`positions`). No aparece en `search_memory` en ningún brazo. Sin clave, en las dos salidas aparece primera la versión vieja, y la última no aparece. `en-test-names` baja uno (2 → 3 en las dos salidas), lo que se admite.

**M4** (`m4`), con el tokenizador real y sobre las 40 consultas del brazo con clave:

| Salida | Media por consulta | Total | Bytes | Sobre 8.000 bytes |
|---|---|---|---|---|
| `search_memory`, con las claves que hoy muestra (`search`) | +45,075 | +1.803 (31.737 → 33.540) | +5.249 | 0 |
| `search_memory`, cota con todas las filas con clave (`search_all_keyed`) | +85,05 | +3.402 | +10.046 | 0 |
| `recall_context` reescrito (`context`) | +54,45 | +2.178 (43.130 → 45.308) | +6.416 | 0 |
| `recall_context`, cota con todas las filas con clave (`context_all_keyed`) | +111,0 | +4.440 | +13.202 | 0 |

En las cotas, cada hit sin clave lleva la clave más larga del set (`longest_key_bytes` 32). El costo fijo por sesión (`fixed`) es de 84 tokens de la descripción de `topic_key` en el schema de `save_memory` y 152 del párrafo de `protocol.md`. La línea de aviso (`warning_line_tokens`) cuesta 34 tokens.

## `store --real` (M5)

`docs/eval/results/2026-10-21-real-revision-eval-store.json`, fuera de git: `export_format` `clawg-memories` versión 1, `model_contract` `multilingual-e5-small/mean`, `clawg_version` 0.43.1 y `seed` 1592590341. El export es un `clawg export --profile prod --include-deleted` del `clawg` 0.43.1 instalado. Desde el registro de `chains` (`dcd25cb`), el código de `revision-eval` y de `eval_api` sólo cambió en un test (`8ea89c0`); `f17deac` subió dependencias de TLS y la versión.

- **Sin reproducir** (`export`): 802 filas y 397 saves, todos con agente. 114 saves llevan clave, y 154 saves que afirman tienen fecha posterior a la primera clave. Hay 111 filas `superseded` por `revision` y 12 por `verdict`, y 12 pares juzgados `supersedes`. No hay líneas inválidas.
- **Reproduciendo** (`replay`): 283 saves sin clave, 220 de un tipo que afirma y 63 de otro. Ninguno sólo reforzó una fila. La población son 220 saves. El aviso dispara en 154 (`fires`): en 19 de ellos la candidata ya estaba borrada o reemplazada por veredicto en el store real (`fires_with_gone_candidate`), y 0 tenían su única candidata en otro proyecto. Los 66 restantes forman el resto (`rest`).
- **Etiquetas** (`labelled`): los 154 pares que disparan y una muestra de 50 de los 66 restantes, 204 pares en total.

**Quién etiquetó.** Las etiquetas no siguen lo que fijaron la propuesta («Quién etiqueta M5») y D5:

1. Proponía Haiku; a pedido del dueño del store propuso Opus 5.5, en cuatro sub-agentes ciegos, uno por lote de 51 pares y sin leer `replay.json`. Esa primera pasada leía «el mismo tema» en sentido estricto y dio 32 revisiones en 204 pares.
2. En los pares 1 a 5, el dueño marcó como revisión el 4 y el 5, donde la primera pasada decía `other` con confianza alta. Leyó la regla así: es revisión si el posterior trata la misma cosa (componente, mecanismo, fixture, procedimiento o práctica) y deja desactualizado algo que el anterior afirma de ella (valor, cómo funciona o se hace, una condición nueva, alcance, motivo o estado), aunque su titular sea otro incidente.
3. Opus volvió a proponer con esa lectura, usando esos cinco pares como ejemplo, y dio 80 revisiones: 48 pares pasaron de `other` a revisión y ninguno al revés.
4. El dueño decidió sólo los pares 1 a 5. Los otros 199 llevan como `label` la propuesta recalibrada, que el dueño adoptó sin revisarla par por par.

`proposed` quedó sólo en los pares 1 a 5, con la primera pasada ciega. Así, `agreement` (3/5, Wilson 23,1–88,2 %) mide de verdad al agente contra el dueño, pero en cinco pares y para la pasada que no se usó. El acuerdo de la pasada recalibrada no se puede medir en esos cinco, porque los tuvo como ejemplo, y no se midió en ningún otro.

**Resultados:**

- **Revisiones entre los que disparan** (`revisions_among_firing`): 76/154, 49,4 % (Wilson 41,6–57,2 %).
- **Prevalencia** (`prevalence`): 6 revisiones en la muestra de 50. Estimación del 38,1 % de la población, con cota inferior de 36,2 %.
- **Costo del aviso** (`warning`): 34 tokens donde aparece. Dispara en 154 de 397 saves (38,8 %, Wilson 34,1–43,7 %), es decir 13,19 tokens de media por save del flujo.

## Veredictos contra los umbrales

Los umbrales son los de la [propuesta](proposal.md#umbrales-fijados-antes-de-medir), sin ajustar.

### Aviso «pasá `topic_key`» (M5, M4): pasa, con salvedad

| Condición | Umbral | Medido | Campo |
|---|---|---|---|
| Revisiones entre los sin clave | ≥ 10 % | 38,1 % | `thresholds.revision_share` |
| Su cota inferior de Wilson | ≥ 5 % | 36,2 % | `thresholds.revision_share_lower_bound` |
| Revisiones entre los que disparan | ≥ 40 % | 49,4 % | `thresholds.firing_precision` |
| Tokens donde aparece | ≤ 40 | 34 | `thresholds.max_tokens_where_it_appears` |
| Tokens de media por save del flujo | ≤ 15 | 13,19 | `thresholds.mean_tokens_per_save` |

`thresholds.met` es `true`. La población de «los sin clave» son los saves sin clave de un tipo que afirma, como decidió el dueño antes de medir (`74189c5`). Un save que no afirma no puede disparar el aviso.

**La salvedad.** Con las etiquetas, la medición se aparta de la que fijó la propuesta, que pedía que contara sólo lo que decide el dueño, porque «un agente solo metería su error sin medir en el umbral del 40 %». Eso es lo que pasó en 199 de 204 pares, y el error pesa más que el margen:

- La precisión pasa con 62 revisiones de 154. Se midieron 76, así que el margen es de 14 pares. Entre las dos lecturas de la regla se movieron 48.
- Con la lectura estricta de la primera pasada, la precisión sería a lo sumo 32/154 (20,8 %), bajo el 40 %, y el aviso no pasaría.
- El texto de la regla (propuesta, E7) no cambió, pero su lectura se fijó después de ver los pares 1 a 5, y la propuesta la daba por fijada antes de ver los pares.
- La prevalencia también descansa en etiquetas del agente: las 50 de la muestra.
- La reproducción no repite borrados ni veredictos. En 19 de los 154 disparos la candidata ya no estaba viva en el store real. Si esos 19 no contaran, la precisión quedaría entre 57/135 (42,2 %) y 76/135 (56,3 %), sobre el 40 % en los dos extremos: ese sesgo no cambia el veredicto.

Con eso a la vista, el dueño decidió registrarlo como cumplido (2026-10-21), porque la delegación de las etiquetas en el agente fue suya. El aviso entra al mapa como cambio propio, y esta salvedad pasa a sus riesgos.

### history-import (44) mapea a `topic_key` las claves de tema (M1, M2): no se cumple

Pide que M2 pase y que al menos el 25 % de las transiciones sin clave quede viva sin marca o arriba de la nueva. La segunda mitad se cumple con holgura: `m1.unmarked_or_above` da 40/56, 71,4 %, con cota inferior de 58,5 %. La primera no: `m2.verdict` es `fail` por `en-repo-layout`. Según la propuesta y E3, un M2 que no pasa es un bug que se arregla en un cambio propio, no en este. No se investigó por qué esa cadena baja dos puestos con clave. La diferencia entre el brazo con clave y el control es que el primero guarda las versiones viejas como `superseded`, y esas versiones siguen en `memory_vectors` y en `memory_fts` (propuesta, problema 2). El mapeo de history-import se vuelve a evaluar cuando ese arreglo deje un `chains --real` con M2 en `pass`.

### Pendientes de M3

M3 no corrió (propuesta). Estas tres decisiones quedan sin contestar, con lo que ya dejan medido M1, M2 y M4:

- **Búsqueda por clave en `search_memory`** (M3, M2). Con clave, M2 (b) se cumple: ninguna versión vieja aparece en ninguna salida. Falta saber si el agente busca la clave y la reusa.
- **Mostrar la clave en `recall_context`** (M3, M4). El lado de M4 queda sin decidir, y la lectura preliminar de las tareas, que lo daba por no cumplido, se corrige acá. El umbral pide como máximo 60 tokens de media por prompt «con todas las filas con clave», y lo medido es una cota de arriba: +111,0 (`m4.context_all_keyed`), con la clave más larga del set en cada distractor. Reescrito con las claves que tienen, +54,45 (`m4.context`), porque los distractores no tienen clave. Con claves reales en todas las filas, la media queda entre los dos, a un lado o al otro de 60. Ningún prompt pasa de 8.000 bytes en ninguna medida.
- **Quitar o simplificar reglas de la clave** (M3, M4). La clave cuesta 236 tokens fijos por sesión (`m4.fixed`: 84 del schema y 152 de `protocol.md`). Las dos condiciones del umbral leen corridas de M3.

## Diferencias entre los documentos y el código

1. **Las distancias de E2.** E2 mide «entre todos los pedazos de las dos versiones». El código sigue a D3: las distancias se toman entre las filas que la versión nueva no comparte con la anterior y las filas propias de la anterior (`compared_rows`), y una fila que comparten dos versiones es de la más nueva (`own_rows`). Leído al pie de la letra, E2 compara una fila compartida consigo misma a distancia 0. La corrida `--fake` del set dio así 9 `below_floor` falsos y 7 cadenas con dos versiones vivas en el brazo con clave. Al consolidar en `specs/eval` va lo de D3.
2. **El test del producto que no cambia.** El escenario «el producto no cambia» de E4 nombra `crates/clawg-cli/tests`. `the_production_context_still_omits_the_key` vive en `run.rs:793` y comprueba que ninguna cabecera de los prompts del router en proceso, con clave, lleve ` topic=`.
3. **La población de M5.** La fila del aviso dice «los saves sin clave». El dueño la acotó antes de medir a los de un tipo que afirma (`74189c5`, `ba1a62a`). En el código es `fires + rest` (`store.rs:307`): los que disparan, más los que tienen un save anterior vivo de su agente y proyecto. En este store coincide con los 220 sin clave que afirman, porque ninguno quedó reforzado ni sin save anterior.
4. **Quién etiqueta M5.** Lo cuenta «Quién etiquetó», arriba. El código hace lo que pide E7, que es puntuar sólo con `label` e informar el acuerdo sobre los pares que llevan `proposed` (`store.rs:579-593`). Lo que cambió es quién llenó `label`.

## Al archivar

- **Los requisitos** pasan a [`specs/eval`](../../../specs/eval/spec.md), en «`revision-eval`: revisiones por clave», y los números, a su «Baseline medida», en «Revisiones por clave». Allí se escriben como comportamiento actual, sin los números de requisito y con sus tests. Las distancias de M1 siguen a D3 (diferencia 1), el test del producto que no cambia figura en `run.rs` (diferencia 2), y la población, como `fires + rest` (diferencia 3). Cada cita se verificó contra el código del árbol de trabajo, que no cambió desde `4cca8c9`.
- **El mapa** de [`openspec/README.md`](../../../README.md) suma dos cambios propios, sin carpeta todavía. 68, topic-key-warning, es el aviso, que pasó su umbral con la salvedad de arriba. 69, keyed-revision-rank, es el M2 que no pasa, y va antes de history-import (44), cuyo mapeo de claves espera a que M2 pase.
- **Los enlaces relativos** de `proposal.md`, `spec.md` y `exploration.md` se corrigieron para la carpeta del archivo, igual que los dos de topic-revisions que apuntaban a este cambio.

## Veredicto

Verificado en Windows: la suite pasa, y también el clippy del workspace y el del build de release. La compuerta de `rag-eval` no mueve ningún caso ni token. `chains` y `store` corrieron con el modelo real. El aviso «pasá `topic_key`» pasa sus umbrales, con la salvedad de que 199 de las 204 etiquetas son de un agente bajo la lectura amplia de la regla, y entra al mapa como cambio propio. El mapeo de history-import no se cumple porque M2 falla en una cadena, y ese bug va a un cambio propio. Las otras tres decisiones esperan a M3. Se archivó el 2026-10-21.
