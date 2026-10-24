# import-timing — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (antes del import en prod) | ver [`state.yaml`](state.yaml) |

## Problema

Verificado en la [exploración](exploration.md#premisas-que-se-verificaron-y-cuáles-no-eran-del-todo-ciertas). Nada en la importación mide su tiempo por dentro: `crates/clawg-core/src/memory/portability/` no tiene ninguna llamada a `log`, y `import_memories` (`crates/clawg-cli/src/daemon/api.rs:1895-1952`) tampoco registra nada. De una importación sólo queda la muestra de `http.duration{route=/v1/import}` que deja el middleware. Las fases de las tres importaciones reales medidas (51 min, 1.218 s y 1.362 s) se infirieron desde fuera, con una sonda, el total del dry run y la hora del snapshot `pre-import`. Por eso los 144 s que separan a import-linear de import-decision-pass sólo pudieron atribuirse a «embedding y escritura».

El import en prod es la primera corrida con un binario release y la única a escala completa. Si sale sin esto, su reparto también se infiere desde fuera.

## Objetivo

1: guardar de forma eficiente, empezando por saber en qué se va el tiempo. Cada importación deja en el log del daemon, al terminar cada fase, cuánto duró y qué procesó. En las fases que usan la conexión, separa el tiempo dentro de SQLite de la espera en la cola. Al final, una línea con el total y el reparto. Ni lo que se escribe ni el reporte cambian.

## Decisiones

Las de la [exploración](exploration.md#decisiones) (una línea por fase, segundos con un decimal y milisegundos para lo más lento, `read` medido en el handler) y el enfoque A. Además:

1. **Las fases.** `read` (el handler lee el archivo), `plan` (los ids de las raíces), `prepare` para un export de clawg o `translate` para el de otra herramienta (enmascarar, partir y armar las filas), `load`, `decide`, `embed`, `snapshot`, `write` y `links`. Un export de clawg se analiza antes de `plan` y se arma después; las dos partes suman en `prepare`, que se registra una vez, después de `plan`. Un dry run termina en `decide`.
2. **Dónde viven.** `ImportTimings` y `Timing` son datos puros en `crates/clawg-core/src/memory/portability/timing.rs`, con su `Display`. `import` e `import_foreign` reciben un `&mut ImportTimings`. Así el handler registra `read` antes y el resumen después, y las pruebas leen la estructura sin capturar logs.
3. **La conexión.** Cada `conn.call` de `plan`, `load`, `write` y `links` pasa por un ayudante que lee el reloj al pedir la llamada y otra vez dentro de la clausura. El tiempo dentro de la clausura es lo que la llamada tuvo la conexión (`in sqlite`). Lo que queda hasta que vuelve el `.await` es la espera (`queued`). La más lenta es la que más tiempo tuvo la conexión. `snapshot` abre una conexión propia en un hilo bloqueante (`crates/clawg-core/src/memory/db.rs:299-314`), así que sólo lleva su duración.
4. **El formato.** Con números inventados:

   ```
   import: load 3.1 s (812 rows, 5 calls, 2.6 s in sqlite, 0.5 s queued, slowest 41 ms)
   import: embed 1142.0 s (51068 rows, 1596 batches, 44.7 rows/s, slowest 2300 ms)
   import: done in 1362.4 s: read 1.8, plan 0.0, translate 61.0, load 3.1, decide 4.9, embed 1142.0, snapshot 1.6, write 23.0, links 0.4
   ```

   Un dry run cierra con `import: dry run done in …`. Las líneas sólo tienen nombres de fase y números, nunca un texto, un id ni una ruta.
5. **Una importación que falla** deja las líneas de las fases que terminó. No hay resumen, porque no hay total que comparar.

## Alcance

1. `timing.rs`: `Step`, `Timing` e `ImportTimings`, con su `Display` y el resumen.
2. Los puntos de registro en `prepare_file`, `prepare_export`, `load` y `write`, el ayudante de las llamadas y el parámetro en `import` e `import_foreign`.
3. El handler mide `read` y registra el resumen.
4. `.claude/CLAUDE.md` (lo que deja una importación en el log) y el delta de `specs/portability`.

### No-alcance

Lo que deja fuera la [exploración](exploration.md#no-alcance): cambiar lo que se escribe, el reporte o la paridad entre el dry run y el real; series en `metrics.db` (73); un trabajo con progreso (87); las etapas internas del embedder (85); perfilar SQLite por sentencia u optimizar cualquier fase.

## Cómo se mide

Tests (la lista está en la [spec](spec.md)). Todos fallan hoy, porque `ImportTimings` no existe:

1. Una importación real de otra herramienta registra `plan`, `translate`, `load`, `decide`, `embed`, `snapshot`, `write` y `links`, una vez cada una y en ese orden. Las filas de `embed` son las insertadas no borradas del reporte, y las llamadas de `write`, los lotes de `note_batches`. Una de clawg registra `plan` y `prepare` en lugar de `plan` y `translate`.
2. Un dry run registra hasta `decide`.
3. En `load` y `write`, el tiempo en SQLite más la espera no supera la duración de la fase, y las llamadas de `load` son las que dan `LOAD_BATCH` y los conjuntos de búsqueda.
4. El `Display` de una fase y el del resumen, sobre una estructura armada a mano, dan el texto esperado.

Las pruebas de import que ya existen, incluida la paridad entre el dry run y el real, pasan sin editarlas, salvo el argumento nuevo.

Medición: una importación real en el perfil dev, como en import-linear e import-decision-pass. La suma de las fases tiene que cerrar contra el tiempo del cliente con un resto menor al 1 %. Requiere el modelo real, la copia del snapshot `pre-import` y autorización explícita.

## Migración y vuelta atrás

No hay esquema nuevo ni datos que cambien. Volver es revertir los commits.
