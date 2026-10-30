# perf-bench — Exploración

Objetivo transversal. Las métricas de uso de [usage-metrics](../archive/2026-10-22-usage-metrics/exploration.md) (71) y [pipeline-metrics](../archive/2026-10-30-pipeline-metrics/exploration.md) (73) dicen qué pasa, pero son observacionales: entre dos versiones cambian también el tamaño del store, las consultas, la carga de la máquina y la hora. Un banco reproducible por versión fija todo eso y permite afirmar que una versión es más lenta. Es la propuesta M10 de una propuesta de medición escrita sobre 0.26.0.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa (sobre 0.26.0) | Estado en `develop` (0.45.0) |
|---|---|
| `rag-eval --agent-view` ya da latencias p50/p99 | **Cierta.** `latency_p50_ms` y `latency_p99_ms` (`crates/clawg-eval/src/bin/rag-eval/agent_view/stats.rs:28-29`, `:66`), sobre el set dorado de 46 casos: un corpus chico y fijo, sin carga concurrente ni guardados. |
| No hay ningún banco que arranque el binario real | **Falsa hoy.** `crates/clawg-cli/tests/transport_bar.rs` (`#![cfg(windows)]`, `#[ignore]`) arranca un daemon `testing` con `--port 0` y mide AF_UNIX contra TCP en cinco escenarios, con una barra que falla; se corre con `cargo test -p clawg-cli --release --test transport_bar --target-dir target-test -- --ignored --nocapture`. Es el patrón a seguir: un test ignorado, con el binario de la suite y el embedder falso, que CI nunca corre. |
| `revision-eval` y `conflict-eval` siembran stores sintéticos | **Cierta.** `revision-eval chains` guarda por el worker real en stores bajo el directorio temporal; la siembra reproducible con semilla ya existe en `rag-eval` (`seed.rs`). |
| El modo con modelo real pide un build optimizado | **Cierta, y con una regla.** `.claude/CLAUDE.md` prohíbe builds de release sin autorización explícita; `transport_bar` ya usa `--release` con esa autorización, en `target-test`. El modo real necesita además el modelo del perfil dev, como `revision-eval --real` (sale con 3 sin él). |
| La comparación estadística ya existe | **Cierta.** `rag-eval --diff` (`crates/clawg-eval/src/bin/rag-eval/diff.rs`) da valores p pareados y bootstrap; un banco de latencias compara distribuciones, no casos, así que reutiliza el bootstrap y suma una prueba de rangos. |
| Las puertas de 57 y 62 piden su propia medición | **Cierta.** read-connections (57) pide una prueba en `binary.rs` con un store de decenas de MB y una operación larga; embedder-footprint (62) pide CPU, tiempo y bits de `e5_golden` con el modelo real. Las dos son escenarios de este banco. |

## Problema

Hoy cada verificación mide a mano y una vez (36: latencia del save; 46: memoria por recarga; 56: planes con filas generadas; 59: barra de transporte). No hay una corrida que se repita igual en la próxima versión y diga si algo empeoró.

## Enfoques posibles

### A. Un test ignorado en `clawg-cli/tests`, como `transport_bar` (recomendado)

Siembra con semilla fija un store de 10 000, 50 000 y 100 000 filas (varios agentes y proyectos, una fracción `superseded` y borrada, palabras frecuentes y raras), arranca el binario de la suite con `--port 0`, le aplica una carga a ritmo fijo (búsquedas, contextos, guardados de 1, 10 y 100 piezas, un refresco de docs y un backup en medio) midiendo desde el instante en que cada petición debía salir (sin omisión coordinada), lee `GET /v1/metrics` y la memoria del proceso al final y escribe un JSON con el entorno. Con el embedder falso aísla SQLite, ranking e ingest.

### B. Un binario `perf-bench` en `clawg-eval`

Lo que proponía el documento de origen. Tiene la maquinaria de `--diff` a mano, pero `clawg-eval` no puede lanzar un binario `testing` sin reconstruirlo, y el embedder falso sólo lo lee un build con esa feature. Queda como lugar del `diff` de dos JSON, no del banco.

### C. Bancos de instrucciones en CI (`gungraun`, antes iai-callgrind)

Estables en CI, pero sin Windows ni E/S. Sólo si aparece una regresión algorítmica en una función pura que el banco no vio a tiempo. Descartado por ahora; los runners compartidos tampoco sirven para tiempos de pared.

### Recomendación: A, con el `diff` en `clawg-eval`

## Alcance candidato

1. El escenario y la carga, con semilla, en un test ignorado de `clawg-cli/tests`.
2. Un JSON de resultado con el entorno (versión, SO, CPU, embedder falso o real) en `docs/eval/results/`, con fecha y sin reemplazar uno existente, como `revision-eval --real`; con el embedder falso sólo imprime.
3. `diff` de dos JSON en `clawg-eval`, con bootstrap del cambio de mediana por escenario.
4. Los escenarios de las puertas de 57 (espera de save, `session-start` y búsqueda durante un refresco o un save largo) y de 62 (CPU en reposo y bajo carga, `embedder.batch_tokens`), para que esas puertas se corran con este banco.

### No-alcance

- Correr el banco en CI.
- Publicar resultados con el release.
- Perfilado puntual (`ort` `with_profiling`, `tokio-console`, `perf`/`samply`, WPR): son herramientas de investigación, no de este banco.

## Decisiones abiertas

- [ ] **Tamaños del store.** 100 000 filas con el embedder falso tardan en sembrarse; ¿se siembra por SQL directo (como 56 midió los planes) o por el worker?
- [ ] **Umbral.** ¿Qué empeoramiento de p95 por escenario es una regresión? Candidato: el que el bootstrap separa de cero con 95 % y supera el 20 %.

## Cómo se mide

El banco es la medición; su propia verificación es correrlo dos veces sobre el mismo commit y comprobar que `diff` no ve diferencia, y una sobre un commit con un `sleep` inyectado en la búsqueda y comprobar que sí.

## Interacciones

- **71 y 73** son requisito: el banco lee `GET /v1/metrics` para el desglose.
- **57 y 62** pueden usarlo como puerta; si alguna se adelanta, su escenario se escribe allí y este cambio lo adopta.
