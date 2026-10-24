# ram-footprint — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S (A y el paso 0); B, C y D con puerta | — | ver [`state.yaml`](state.yaml) |

## Problema

Verificado en la [exploración](exploration.md#premisas-que-se-verificaron-y-cuáles-no-eran-del-todo-ciertas). En prod, con una sola sesión de Claude Code, el modelo estuvo cargado el 73 al 81 % de los minutos y la memoria privada fue de 283,9 a 1.278,4 MiB. Hay dos causas comprobadas en el código y tres huecos de medición:

1. **Cada prompt lanza un refresco completo del índice de docs.** Los hooks registran la raíz en cada evento sin `index_docs` (`crates/clawg-cli/src/hook.rs:242-264`). Antes de este cambio, `register` lanzaba `spawn_doc_refresh` siempre que la raíz quedaba aceptada (`RegisterOutcome::index_docs`), y ese refresco toma la marca con `DocIndex::begin` (`crates/clawg-core/src/memory/project_docs/mod.rs:209`), que no respeta `REFRESH_INTERVAL`. Cada prompt recorría y hasheaba cada archivo, embebía los que cambiaron y, con eso, mantenía el modelo cargado.
2. **Cada prompt despierta el modelo** (`embedder.wake()` en `register`, `crates/clawg-cli/src/daemon/api.rs:2063-2067`). No se sabe cuántas de esas cargas terminan en una llamada que embebe.
3. **`metrics report` no agrupa por minuto**, aunque `metrics.db` guarda los minutos durante 14 días (`crates/clawg-core/src/metrics/store.rs:24`), así que un pico no se puede poner al lado de lo que pasaba en ese minuto.
4. **En Windows no hay pico privado.** `process.rss_peak_bytes` es el working set, que el sistema recorta, y `process.private_bytes` se muestrea una vez por minuto.
5. **El import no dice cuánta memoria tiene en cada paso**, así que no se separa lo suyo de lo del modelo cargado debajo.

## Objetivo

Transversal: que el daemon no retenga lo que no usa y que cada pico tenga dueño. Este cambio saca el trabajo por prompt (A) y deja las mediciones (paso 0) con las que se deciden B, C y D. No cambia qué se guarda, cómo se busca ni los vectores.

## Decisiones

Las abiertas en la exploración, con estos valores por defecto:

1. **A va dentro del cambio y primero.** Es un commit propio (`fix(daemon): …`), así que puede salir solo si hace falta. Un registro lanza el refresco sólo cuando trae `index_docs: true` y la raíz queda aceptada. La frescura queda a cargo de `search_project`, que ya refresca un índice de más de un minuto (docs-refresh-background, 64). El puente sigue pidiendo `index_docs` para su raíz al arrancar, así que el primer indexado de una sesión no cambia. `clawg projects add` también lo pide.
2. **M1 es `--by minute` en `metrics report`**, no un subcomando de exportación. La clave es `AAAA-MM-DD HH:MM` local, con el desplazamiento de su ejecución, igual que `--by day`. El reporte empieza no antes de la primera hora entera de los 14 días que se guardan por minuto (`since_for`). Una fila más vieja es una hora plegada (`prune`, `store.rs:214`) y se leería como su primer minuto. Redondear a la hora deja afuera la hora que el último `prune` pudo dejar a medias en el borde.
3. **M2 es `process.private_peak_bytes`**: `PeakPagefileUsage`, de la misma llamada a `GetProcessMemoryInfo` que ya da `PrivateUsage`. Linux no tiene un pico de `RssAnon` y, por la regla de 71, se deja afuera en lugar de escribir 0.
4. **M3 va en el log, no en `metrics.db`.** La carga que hace `IdleEmbedder` dice quién la pidió (`for a wake`, `for a call that embeds`, `for a call that found its worker gone`) y la memoria privada antes y después. Si una carga la pidió un `wake`, el log dice cuánto tardó la primera llamada en usar el modelo. Si nadie lo usó, la descarga por ocio termina con `; no call used it after the wake that loaded it`. La memoria se lee con un `MemoryProbe` (`fn() -> Option<u64>`) que pasa el daemon, porque las llamadas al sistema viven en `clawg-cli` (`process.rs`) y `clawg-core` no tiene dependencias de sistema. Sin sonda, en las pruebas y en `clawg-eval`, la línea no lleva memoria. Las series permanentes son de pipeline-metrics (73).
5. **M4 agrega `private N MiB` al final de cada línea de paso del import.** Es la memoria privada al terminar el paso, tomada con la misma sonda. Sólo la registra el handler del daemon (`ImportTimings::with_memory`); las pruebas de `import.rs` siguen sin sonda.
6. **B, C, D y E no entran en este cambio.** Cada uno espera su medición sobre un día de uso real con el modelo real (M3 para B, M2 para C, M4 para D), y esa medición necesita un perfil con uso real y un release que traiga el paso 0. Al principio iban a decidirse en este mismo cambio. Al archivar, el cambio se partió: el día de uso y B, C, D y E pasaron a [ram-footprint-gates](../../ram-footprint-gates/exploration.md) (92). El día de uso no verifica ningún requisito de esta spec: es lo que decide esos brazos. Además, esperarlo dejaba a `specs/project-docs` contradiciendo a A en el release que lo publica.

## Alcance

1. A: `register` lanza el refresco sólo con `index_docs: true`.
2. M1: `By::Minute`, `since_for` y `--by minute`.
3. M2: `process.private_peak_bytes` en Windows.
4. M3: la causa y la memoria de cada carga, el primer uso después de un `wake` y la descarga sin uso.
5. M4: la memoria privada en cada línea de paso del import.
6. `.claude/CLAUDE.md` y la fila 90 de `openspec/README.md`.

### No-alcance

- B, C, D y E, y el día de uso que los decide: [ram-footprint-gates](../../ram-footprint-gates/exploration.md) (92).
- El modelo, el contrato de embedding y el tamaño de las tandas.
- El tokenizer (91) y la CPU del embedder (62).
- Series nuevas de `metrics.db` para el embedder (73).

## Cómo se mide

Tests que fallan sin el cambio (la lista está en la [spec](spec.md)):

1. **A.** Registrar sin `index_docs` una raíz ya aceptada a la que se le agregó un doc no lo indexa. Sin el arreglo, el índice queda con 2 docs en lugar de 1.
2. **M1.** El agrupado por minuto usa la clave local de cada ejecución, y `since_for` no empieza antes de la primera hora entera de los minutos guardados. El reporte por minuto lee esas filas de un `metrics.db` de prueba.
3. **M2.** El escritor guarda `process.private_peak_bytes` cuando la muestra lo trae. En Windows, la muestra del proceso lo trae y es mayor o igual que `private_bytes`; en Linux no lo trae.
4. **M3.** La línea de carga nombra la causa y la memoria. Un modelo que cargó un `wake` y que nadie usó se descarga marcado como tal; uno que usó una llamada, o que cargó una llamada, no.
5. **M4.** Con sonda, cada paso lleva la memoria y su línea la muestra; sin sonda, no.

Lo que decide B, C y D es el día de uso real, que hace ram-footprint-gates (92): `metrics report --by minute` sobre `process.private_bytes`, `process.private_peak_bytes` y `embedder.resident`, contra las líneas de carga, descarga y refresco de `serve.log` ([exploración, «Cómo se mide»](exploration.md#cómo-se-mide)).

## Datos

No toca `memory.db`, el esquema, los ids ni el contrato. `metrics.db` gana una serie más (`process.private_peak_bytes`), sin cambio de esquema: una serie es una fila con su nombre. Un `metrics report` viejo la lee como cualquier otra.
