# import-timing — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar memorias de forma eficiente) | S | — (antes del import en prod); se coordina con import-undo, import-job, pipeline-metrics y embed-throughput | exploración (borrador) |

**Tesis:** sabemos cuánto tarda una importación entera, pero no en qué se va ese tiempo. Las fases de las tres mediciones reales (51 min, 1.218 s y 1.362 s) se infirieron desde fuera, con una sonda cada 20-50 s, el total del dry run y la hora del snapshot `pre-import`. Por eso la diferencia de 144 s entre import-linear e import-decision-pass sólo pudo atribuirse a «embedding y escritura». El cambio hace que la importación registre en el log, al terminar cada fase, cuánto duró, cuántas filas y llamadas procesó y, en las fases que usan la conexión, cuánto de ese tiempo fue SQLite y cuánto espera en la cola. Va antes del import en prod porque esa corrida es la primera con un binario release (un build debug rechaza `prod`) y la única a escala completa. Sin esto, sus tiempos por fase se pierden.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La importación no mide nada por dentro | **Cierta.** `crates/clawg-core/src/memory/portability/` no tiene ninguna llamada a `log`. Los únicos `Instant::now` de `import.rs` están en sus tests. `import_memories` (`crates/clawg-cli/src/daemon/api.rs:1894-1951`) no registra ni la duración ni las fases. |
| `metrics.db` ya guarda algo del import | **Sólo el total.** El middleware registra `http.duration` por ruta (`crates/clawg-cli/src/daemon/metrics.rs:79`, `:94`), así que una importación deja una sola muestra de `route=/v1/import`. No hay series del import, del embedder ni de SQLite (`crates/clawg-core/src/metrics/`). |
| Las fases se pueden cortar sin tocar la lógica | **Cierta.** `write` ya las separa en bloques consecutivos (`import.rs:1422-1498`): `load` (`:1430`), `decide` en `spawn_blocking` (`:1434-1437`), el bucle de embeddings (`:1444-1457`), el snapshot (`:1459`), los lotes de escritura (`:1463-1484`) y los vínculos (`:1485-1495`). Antes de `write`, `prepare_export` planea las raíces (`:1377`) y traduce en `spawn_blocking` (`:1390-1395`). Un export de engram se lee en el handler, antes de llegar a `import_foreign` (`api.rs:1924`). |
| Medir alrededor de `conn.call` da el tiempo de SQL | **Falsa.** El `.await` de un `conn.call` incluye la espera hasta que el hilo de la conexión toma la clausura, que comparte con las búsquedas y los listados de ese momento. Sólo el reloj dentro de la clausura mide SQLite (y el Rust que la rodea). `load` hace una llamada por cada `LOAD_BATCH` = 256 búsquedas (`:621`, `known` en `:792-814`, los vínculos en `:859-876` y las raíces en `:879-892`), y la escritura una por lote de `WRITE_BATCH` = 256 filas (`:59`, `:1468-1484`). |
| El embedding se puede medir como una fase | **Cierta, de punta a punta.** El bucle manda tandas de `EMBED_BATCH` = 32 (`import.rs:61`, `:1450`) y `E5Embedder::embed_batch` las parte en tandas de 16 (`embedder.rs:173`, `:420`). Tokenizar, rellenar, inferir y agrupar ocurren juntos en el hilo del embedder (`Inference::embed`, `embedder.rs:483-505`). Separarlos es el M2 de embed-throughput. Una recarga del modelo por inactividad cae dentro de la fase y se ve como una tanda lenta. |
| Un log de `info` llega al log del daemon en prod sin configurar nada | **Cierta.** `init_logging` filtra en `info` salvo que `RUST_LOG` diga otra cosa (`crates/clawg-cli/src/main.rs:110-120`), y un daemon que arranca el puente escribe en `<data>/logs/serve.log` (`crates/clawg-cli/src/serve_log.rs`). Ya hay un precedente de duración en `info`: la carga del modelo (`crates/clawg-core/src/memory/bootstrap.rs:472-476`). |
| Los tiempos pueden ir en el reporte | **No sin romper un requisito.** El dry run y la importación real MUST dar el mismo reporte salvo `dry_run` y el snapshot (`openspec/specs/portability/spec.md:333-336`, `:444-445`). Los tiempos cambian en cada corrida. |

## Problema

1. **El reparto del tiempo es una inferencia.** De los 1.362 s de la última medición, ~19 min se atribuyeron al embedding restando el dry run del total, y ~23 s a la escritura por la hora del snapshot. `load` y `decide` comparten los ~70 s del dry run con la traducción sin que se sepa cuánto es de cada una. Un cambio que mueva 2 minutos no se puede explicar.
2. **No se separa la cola de SQLite.** Con una sola conexión, lo lento puede ser la consulta o la espera detrás de otra. Hoy esa pregunta sólo la contesta la sonda externa.
3. **La corrida que más vale se haría a ciegas.** Los cambios que instrumentan (85, 87, 73, 74) van después del import en prod. Esa importación es la primera en release: contestaría si los ~40 filas/s del embedding son un artefacto de debug, que es la primera pregunta de embed-throughput.

## Lo que ya existe y se reutiliza

- Los límites de fase de `write` y `prepare_export`, sin cambios de lógica.
- `log::info!` con la duración en milisegundos, como `bootstrap.rs:472-476`.
- `http.duration{route=/v1/import}` como total externo: la suma de las fases contra ese total da lo no contabilizado (recibir el cuerpo, deserializar `text`).
- Las pruebas de import con `FakeEmbedder` de `import.rs` y la paridad entre dry run y real, que tienen que seguir pasando sin tocarlas.

## Enfoques posibles

### A. Un registro de fases en el log, al terminar cada una (recomendado)

Una estructura pura, `ImportTimings`, que acumula por fase la duración, las filas y, para las fases con conexión, el número de llamadas, el tiempo dentro de la clausura y la llamada más lenta. Cada fase se registra al terminar con una línea de `info`, y al final sale un resumen. Ejemplo de forma, con números inventados y sin texto de ninguna fila:

```
import: load 3.1 s (812 calls, 2.6 s in sqlite, 0.5 s queued, slowest 41 ms)
import: embed 1142.0 s (51068 rows, 1596 batches, 44.7 rows/s, slowest 2.3 s)
import: done in 1362.4 s: read 1.8, translate 61.0, load 3.1, decide 4.9, embed 1142.0, snapshot 1.6, write 23.0, links 0.4
```

Las fases: `read` (en el handler), `translate` (sólo para otra herramienta), `plan`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`. Un dry run registra hasta `decide`. Las clausuras de `load` y de la escritura devuelven, junto a su resultado, el tiempo que corrieron. Es el único cambio de firma, y es interno.

**A favor:** se ve en vivo durante una importación de 20 minutos, sin configurar nada en prod. No cambia lo que se escribe ni el reporte. Las pruebas leen la estructura, sin capturar logs. **En contra:** cada import deja unas diez líneas en `serve.log`, y es un desglose de una corrida, no una serie para comparar versiones (eso es de 73).

### B. Los tiempos en el reporte de la importación

Visible en `clawg import` sin leer el log. **En contra:** rompe la paridad entre el dry run y el real que fija la spec, y vuelve contrato algo que es diagnóstico. Descartado.

### C. Series en `metrics.db`

`db.op_duration{op=import}` y un evento `import` es lo que propone pipeline-metrics (`openspec/changes/pipeline-metrics/exploration.md:31`, `:36`). **En contra:** llega después del import en prod, y un total por minuto no da el desglose de una corrida. Cuando llegue, 73 puede alimentarse de `ImportTimings`.

### D. Perfilar en lugar de instrumentar

`samply` o WPR sobre el daemon dan el reparto por función, incluido SQLite. **En contra:** es una corrida de investigación (perf-bench lo excluye a propósito, `openspec/changes/perf-bench/exploration.md:47`), no algo que acompaña al import en prod. Sigue disponible sin ningún cambio.

### Recomendación: A

## Descartado

| Opción | Razón |
|---|---|
| Una línea por tanda o por lote | ~1.600 tandas de embedding y ~200 lotes de escritura por importación. El agregado con la más lenta dice lo mismo sin llenar `serve.log`, que rota a los 10 MiB. |
| `debug` en lugar de `info` | El daemon de prod lo arranca el puente sin `RUST_LOG`; en `debug` la corrida que importa no dejaría nada. |
| Perfil de SQLite por sentencia | Necesita la feature `trace` de rusqlite, que hoy no está (`Cargo.toml:32`). El tiempo dentro de la clausura ya separa SQLite de la cola. Por sentencia sólo vale si ese tiempo resulta grande, y en las mediciones previas toda la parte de SQL suma como mucho un minuto y medio sobre más de 20. |
| `call_timed` en todas las llamadas del daemon | Es el no-alcance de pipeline-metrics (`exploration.md:41`, «migrar sólo el camino caliente si hace falta el desglose»). Este cambio es ese camino caliente, y nada más. |

## Interacciones

- **piece-boundaries (83).** Ninguna en el código. Está en apply, así que este va después.
- **import-undo (84).** Escribe su diario dentro de `write`, en los mismos lotes. Con este cambio antes, la verificación de 84 dice cuánto le suma su diario a `write` con la línea de esa fase. Hay que coordinar el orden de los commits: tocan la misma función.
- **import-job (87).** Sus fases (`reading`, `translating`, `deciding`, `embedding`, `snapshot`, `writing`) salen de los mismos puntos: `load` + `decide` son su `deciding` y `links` es parte de su `writing`. 87 puede tomar los contadores de `ImportTimings` en vez de poner otros.
- **pipeline-metrics (73).** `db.op_duration{op=import}` y el evento `import` se llenan desde el resumen.
- **embed-throughput (85).** La línea de `embed` del import en prod es su punto de partida en release, a escala completa. Las etapas internas (M2) y el relleno (M1, sólo con el tokenizer) siguen siendo suyas.

## Alcance candidato

1. `ImportTimings`: estructura pura con fases, duración, filas, llamadas, tiempo dentro de la conexión y la llamada más lenta, su `Display` por fase y su resumen.
2. Los puntos de registro en `write`, `prepare_export`, `prepare_file` y el handler (`read`), con una línea de `info` al terminar cada fase.
3. Las clausuras de `load` y de la escritura devuelven su tiempo de ejecución.

### No-alcance

- Cambiar qué se escribe, el reporte o la paridad entre el dry run y el real.
- Series en `metrics.db` (73), un trabajo con progreso (87) y las etapas internas del embedder o el relleno (85).
- Perfilar SQLite por sentencia u optimizar cualquier fase.

## Decisiones

Tomadas el 2026-10-24.

- [x] **Granularidad.** Una línea por fase con agregados y la llamada más lenta, sin líneas de progreso por lote. El progreso es de import-job (87), y una línea por lote llenaría `serve.log`.
- [x] **Unidad.** Segundos con un decimal para las fases y milisegundos para la llamada más lenta. Se lee de un vistazo, y la llamada más lenta no se pierde en el redondeo.
- [x] **`read`.** Se mide en el handler con el mismo formato, para que la suma de las fases cierre contra `http.duration{route=/v1/import}`.

## Cómo se mide

**RED (cada uno falla hoy):**

1. `an_import_times_each_step_once_in_order`: con `FakeEmbedder`, una importación real de otra herramienta registra `plan`, `translate`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`, una vez cada una y en ese orden. Las filas de `embed` son las insertadas no borradas del reporte, y los lotes de `write` son `note_batches(..).len()`.
2. `a_dry_run_times_only_what_it_runs`: hasta `decide`, sin `embed`, `snapshot` ni `write`.
3. `time_in_sqlite_never_exceeds_the_call`: en `load` y `write`, el tiempo dentro de la conexión es menor o igual que el total, y las llamadas de `load` son las que dan `LOAD_BATCH` y los conjuntos de búsqueda.
4. `the_summary_names_every_step`: el `Display` del resumen, sobre una estructura armada a mano, da el texto esperado. Sólo números y nombres de fase, por construcción.

Las pruebas actuales de paridad entre dry run y real siguen pasando sin cambios.

**Medición:** una importación real en el perfil dev, en un `CLAWG_TEST_HOME` aislado con la copia del snapshot `pre-import` y del modelo, como en import-linear e import-decision-pass. La suma de las fases tiene que cerrar contra el tiempo del cliente con un resto menor al 1 %, y el reparto se compara con las inferencias previas (traducción ~1 min, embedding ~19 min, escritura ~23 s). La corrida en prod, con el release que lleve este cambio, deja el primer reparto en release. Requiere el import en prod autorizado, como ya está previsto.

## Riesgos

- **Que el registro cambie el resultado.** Mitigación: las pruebas de paridad y las de import no se tocan.
- **Costo.** Dos lecturas de reloj por llamada y por tanda, unos pocos miles en una importación de 20 minutos. Despreciable.
- **Que no llegue a tiempo.** Sólo sirve si sale en el release con el que se corre el import en prod. Ocurre lo mismo con 83 y 84.
