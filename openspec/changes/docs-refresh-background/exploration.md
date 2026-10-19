# docs-refresh-background — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | S | — (aterriza antes que 20 docs-search-quality) | exploración (borrador) |

**Tesis:** `search_project` no se cuelga esperando que se reindexe lo que cambió, y un doc que no se puede indexar no frena a los demás. Hoy, cuando el último refresco del proyecto empezó hace 60 s o más, la búsqueda que lo encuentra vencido refresca **en línea** y recién después busca: tras un `git checkout` de otra rama, un pull grande, un reformateo masivo o un cambio de modelo, esa llamada dura lo que tarde en embeberse todo lo cambiado, y el cliente no tiene plazo. Además, un solo doc con un vector inservible aborta el refresco entero, y lo hace en cada refresco mientras ese doc no cambie. Nada de esto toca `memory.db` más allá de las tablas del índice de docs, que se derivan del disco y se rehacen sin perder nada del usuario.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La búsqueda refresca en línea cuando el refresco está vencido | **Cierta.** `search_docs` hace `await` de `index.try_refresh(..)` antes de buscar y un error pasa a `warning` (`crates/clawg-cli/src/daemon/api.rs:1632-1636`); sólo en el otro caso (hay uno en curso y el índice está vacío) espera hasta `FIRST_INDEX_WAIT` = 20 s (`api.rs:1637-1642`, `:1559`). |
| Sólo bloquea la búsqueda que dispara el refresco | **Cierta.** Con un refresco en curso y el índice con docs, la siguiente búsqueda no espera y responde `indexing: true` (`api.rs:1637`, `:1647`). `is_due` exige `!running` (`crates/clawg-core/src/memory/project_docs/mod.rs:182-185`). |
| El refresco tarda lo que tarde embeber lo cambiado | **Cierta.** `refresh` recorre y hashea todo en `spawn_blocking` (`mod.rs:359`) y después, doc por doc, trocea, embebe y escribe (`:372-393`). Un cambio de `model_id` vuelve a embeber todo (`plan_refresh`, `mod.rs:114`). Un reindexado completo de este repositorio midió 645 s en un binario debug (`openspec/changes/archive/2026-10-15-project-indexing/verify.md:188`); no hay cifra de release ni de una edición chica. |
| El cliente no tiene plazo | **Cierta, y a propósito.** `DaemonClient::search_docs` no llama `.timeout(..)` (`crates/clawg-cli/src/client.rs:808-821`) y `only_health_sets_a_timeout` lo exige para todo lo que no sea `/health` (`client.rs:1456-1463`), porque el puente repite una llamada fallida con `NotRunning` y un timeout rompería esa garantía. Un plazo en el cliente no es el remedio. |
| El embedding bloquea a los demás | **Falsa en lo que toca a SQLite.** El embedding se hace antes de pedir la conexión (`mod.rs:388`; el spec lo dice en `openspec/specs/project-docs/spec.md:146`); la conexión sólo se ocupa en cada `replace_doc` (`mod.rs:391`, `:478-511`, una transacción por doc). Lo compartido es el embedder, que usa también el worker de ingest, y la cola FIFO de la única conexión, que cada `replace_doc` ocupa un momento (asunto de read-connections (57)). |
| El primer índice casi nunca corre en línea | **Cierta con matices.** El registro de una raíz aceptada lanza `spawn_doc_refresh` (`api.rs:1524-1526`, `:1534-1554`), que espera al embedder y refresca. Pero `last_started` es `None` hasta que `begin` corre, así que una búsqueda que llegue antes de ese `begin` refresca ella misma en línea. |
| Un fallo del refresco de fondo llega al modelo | **Falsa.** `spawn_doc_refresh` sólo lo registra con `log::warn!` (`api.rs:1551`) y `Progress` guarda sólo `running` y `last_started` (`mod.rs:129-133`): no hay dónde dejarlo para que la próxima búsqueda lo ponga en `warning`. |
| Cancelar la petición deja el índice consistente | **Cierta, con un efecto lateral.** `RefreshGuard::drop` suelta la marca aunque el futuro se caiga (`mod.rs:152-161`) y cada doc se reemplaza en una transacción (`:478-511`), así que no hay docs a medias. Pero `begin` ya fijó `last_started` (`:172`): tras una cancelación el proyecto no vuelve a estar vencido hasta pasados 60 s, y en ese lapso las búsquedas contestan con el índice parcial y `indexing: false`. `remove_docs` no corrió (`:394`). |
| El spec es coherente con el código | **Falsa.** `specs/project-docs` dice en `:197` que antes de buscar el daemon MUST refrescar, y en `:311` que «ni el registro ni la búsqueda lo esperan, salvo la espera del primer índice». El código cumple lo primero; el invariante dice lo contrario. |
| Un doc que falla aborta todo, por diseño | **Cierta.** `embed_passages(..).await?` y `replace_doc(..).await?` (`mod.rs:388`, `:391`) cortan el bucle: los docs que siguen en el plan no se indexan y `remove_docs` (`:394`) no corre, en cada refresco mientras el doc siga igual. El spec lo exige para el embedder (`specs/project-docs/spec.md:148`); ese texto habla de «el embedder falla», no de un vector inservible de un doc. |
| `DocIndexError::Vector` es un fallo de un doc | **Cierta a medias.** Sale de `embed_passages` por una cuenta de vectores distinta de la de pasajes o por `to_unit` (dimensión, valor no finito o norma cero; `mod.rs:412-422`, `crates/clawg-core/src/memory/vector.rs:41-56`). Lo segundo es de un doc; una cuenta distinta apunta más a una falla general del embedder, y hay que decidir si cuenta. |
| `rag-eval --docs` pasa por `search_docs` | **Falsa.** `EvalDocIndex::index` llama a `project_docs::refresh` directamente (`crates/clawg-core/src/eval_api.rs:765`); el cambio de la búsqueda no lo mueve. Sí lo alcanzaría el salto por doc, porque `refresh` es compartido. |

## Problema (verificado contra el código)

1. **Una llamada de herramienta puede colgarse.** El refresco en línea (`api.rs:1632-1636`) no tiene tope y el cliente tampoco (`client.rs:808-821`). El modelo ve una herramienta que no vuelve; si el anfitrión corta la llamada, el futuro del handler se cae y el refresco se cancela a mitad, con `last_started` ya sellado.
2. **La frescura de un doc editado se pagó con latencia sin medirla.** El diseño archivado eligió «antes de buscar, si pasaron 60 s» para que un doc editado aparezca en la búsqueda siguiente (`openspec/changes/archive/2026-10-15-project-indexing/design.md:125`). Con huellas, un repo sin cambios cuesta recorrer y hashear, no embeber; el costo malo es el de muchos docs cambiados a la vez.
3. **El error del refresco de fondo se pierde** (`api.rs:1551`), y con él la única forma de que el modelo sepa que el índice está viejo.
4. **El spec se contradice** (`:197` contra `:311`), de modo que ningún test ni lector puede decir cuál es el contrato.
5. **Un doc malo detiene a los que le siguen** (`mod.rs:388-394`): no hay tope de reintentos y los docs posteriores del plan no se indexan nunca mientras el doc malo siga igual.

## Lo que ya existe y se reutiliza

- `spawn_doc_refresh` (`api.rs:1534-1554`): espera al embedder y llama a `try_refresh`; es la pieza del enfoque A, hoy sólo para el registro.
- `DocIndex::wait_idle` (`mod.rs:189-204`) y su `Notify`: ya espera con tope a que termine un refresco; sirve para una espera acotada.
- `RefreshGuard` (`mod.rs:147-161`): suelta la marca también si la tarea se cancela.
- `DocsSearchBody.indexing` y `warning` (`api.rs:1584-1595`), y su lectura en el puente (`crates/clawg-cli/src/mcp/tools.rs:418-423`: «still being indexed; the results may be incomplete» y «The index could not be refreshed (…); these results are from before»). El contrato hacia el modelo no cambia de forma.
- `RefreshReport.skipped` (`mod.rs:77`) y `plan_refresh`, pura (`mod.rs:108-121`).
- En las pruebas: `Harness` con `FakeEmbedder` (`crates/clawg-cli/tests/api.rs:624`), `project_with_docs` (`:2439`), el patrón de `a_search_during_the_first_index_waits_for_it` (`:2645-2668`, que toma la marca con `begin` y la suelta) y dobles locales como `FixedEmbedder` (`:52`).

## Enfoques posibles

**A. Stale-while-revalidate.** Si está vencido, la búsqueda lanza el refresco en un `tokio::spawn` (el de `spawn_doc_refresh`, extraído) y contesta enseguida desde lo indexado con `indexing: true`; sólo espera (`FIRST_INDEX_WAIT`, como hoy) si el índice está vacío. El último error del refresco se guarda en `Progress` y la búsqueda lo pone en `warning`.
- **A favor:** la búsqueda tiene una latencia acotada por construcción; el refresco ya no depende de que el cliente siga ahí (la tarea no se cae con el futuro del handler); un solo camino de refresco.
- **En contra:** un doc editado hace un momento puede no aparecer en la búsqueda que lo dispara, sino en la siguiente: es justo lo que el diseño archivado quiso evitar. `Progress` deja de ser `Copy` (guarda un `String`).

**B. Tarea propia con espera acotada.** El refresco corre en una tarea propia y la búsqueda espera `wait_idle(.., GRACE)`; pasado el tope contesta con lo que hay y `indexing: true`, y el refresco sigue.
- **A favor:** una edición chica entra en la misma búsqueda; reutiliza `wait_idle`.
- **En contra:** hay que elegir `GRACE`. Con 645 s para 226 docs en un debug (unos 2,9 s por doc), un tope de unos segundos cubre uno o dos docs en esa compilación; en release no hay medición. Con `GRACE = 0` es A.

**C. Híbrido por tamaño del plan.** Recorrer y hashear en línea (barato frente a embeber; `plan_refresh` es pura) y refrescar en línea si el plan es chico, en segundo plano si es grande.
- **A favor:** frescura exacta para una edición, sin colgarse en un checkout.
- **En contra:** hay que partir `refresh` en «escanear y planear» y «aplicar» y pasar la marca de una tarea a otra; un umbral más que elegir; el recorrido y el hash corren en el camino de la búsqueda aunque luego se difiera lo demás. Más piezas que B para un resultado parecido.

**D. Salto por doc de `Vector`.** `refresh` cuenta el doc como no indexado, guarda la causa y sigue con el resto; `remove_docs` corre. Un fallo del embedder (`DocIndexError::Embed`) y uno de la tienda (`Store`) siguen abortando. El doc salteado conserva su huella vieja, así que se reintenta en el refresco siguiente.
- **A favor:** un doc malo ya no deja sin indexar a los que le siguen ni sin quitar a los borrados.
- **En contra:** hay que decidir dónde se cuenta (un campo nuevo en `RefreshReport` o `skipped`, que hoy significa «grande, pasado el tope o no UTF-8», `mod.rs:76`) y qué ve el modelo. Un doc determinísticamente malo se vuelve a embeber en cada refresco.

### Recomendación

**B con tarea propia y `GRACE` corto, más D.** Lo esencial es que el refresco deje de colgarse del futuro del handler (de A) y que la búsqueda no espere más de `GRACE` (de B): con un `GRACE` corto una edición chica entra en su propia búsqueda y un checkout no cuelga nada. Si el diseño no justifica el tope, se baja a A puro, que es el mismo código con `GRACE = 0`. C se descarta por el costo de partir `refresh` sin ganar frescura sobre B.

Qué cambia en `specs/project-docs`:
- `:197`: ya no «MUST refrescar antes de buscar»; la búsqueda MUST iniciar un refresco vencido sin esperarlo más de `GRACE` (o sin esperarlo, si el diseño queda en A) y MUST contestar con `indexing: true` mientras corra. Se conserva «un refresco fallido MUST NOT impedir la búsqueda», y `warning` MUST traer el error del último refresco, también el de uno que corrió en segundo plano, hasta que uno termine bien.
- `:311`: el invariante pasa a decir lo que de verdad se cumple, con la espera del primer índice y la de `GRACE` como únicas esperas; deja de contradecir a `:197`.
- «Límites conocidos» (`:317`): un doc editado aparece a más tardar en la búsqueda siguiente al fin del refresco, no «a los 60 s»; con `GRACE` puede aparecer en la misma.
- `:148`: se agrega que un vector inservible de un doc no aborta; el resto del bullet (si el embedder falla, aborta) queda.
- `:154`: se agrega que el refresco que inicia una búsqueda corre en una tarea que no depende de la petición.

Lo que ve el modelo: los mismos textos de siempre (`tools.rs:418-423`), pero «still being indexed» aparece tras toda edición grande, no sólo en el primer índice. Los docs salteados por vector inservible se nombran en el log y, si se decide, en el `warning`.

## Descartado

| Opción | Razón |
|---|---|
| Un plazo en `DaemonClient::search_docs` | Rompe la garantía que `only_health_sets_a_timeout` protege (`client.rs:1456-1463`) y deja el refresco cancelándose a mitad. |
| Un watcher del filesystem | Ya descartado en el diseño archivado (`openspec/changes/archive/2026-10-15-project-indexing/design.md:127`): una dependencia y un hilo por raíz. |
| Cancelar el refresco cuando el cliente se va | Es el caso actual; el índice queda parcial y `last_started` sellado. |
| Saltar cualquier `DocIndexError` por doc | Un fallo de `Embed` no es de un doc: es del embedder, y seguir con el resto repetiría el fallo N veces. |

## Interacciones

- **docs-search-quality (20).** Mide la búsqueda de docs; si el refresco en línea sigue, una medición con docs cambiados mezcla latencia de embedding con calidad. Este cambio aterriza antes. `rag-eval --docs` no pasa por `search_docs` (`eval_api.rs:765`), así que sus números no se mueven por A/B.
- **read-connections (57).** El refresco es un ocupante largo de la cola de la conexión, doc por doc. Hacerlo en segundo plano no lo acorta (sigue ocupando la cola en cada `replace_doc`), sólo lo desvincula de una petición; 57 es el que mide esa espera.
- **embedder-footprint (62).** Si los refrescos de fondo se disparan con más frecuencia (una tarea por búsqueda vencida), el CPU de los lotes de docs compite con el ingest: 62 mide eso. Este cambio no crea paralelismo nuevo (un refresco por proyecto, `mod.rs:165-174`).
- **model-download (60).** `ready_embedder` (`api.rs:1144`) sigue siendo la puerta: sin embedder listo la búsqueda responde 503 antes de refrescar, igual que hoy. El refresco de fondo hereda la espera por el estado del embedder de `spawn_doc_refresh`. 60, archivado el 2026-10-19, no cambió esos estados: una recarga con la caché dañada responde «no listo» y el embedder sigue `ready`.
- **shutdown-budget (55).** Un refresco de fondo vivo durante `clawg stop` es una tarea más; `spawn_doc_refresh` ya existe en esa condición, pero ahora puede haber una tras cada búsqueda vencida. 55 decide si la cuenta; la escritura por doc es transaccional, así que un corte deja cada doc entero o intacto.
- **untrusted-recall (52).** Define qué raíces indexa el puente; este cambio no amplía ese conjunto, sólo mueve cuándo se lee. Sigue valiendo `index_docs` (archivado en `openspec/changes/archive/2026-10-15-project-root-guard/`).

## Alcance candidato

1. Extraer el refresco de fondo de `spawn_doc_refresh` para que lo use también `search_docs` (B, `GRACE` a decidir).
2. Guardar el último error del refresco en `Progress` y devolverlo como `warning`.
3. `refresh` no aborta por `DocIndexError::Vector` de un doc: lo cuenta con su causa y sigue; `remove_docs` corre.
4. Corregir `specs/project-docs` (`:148`, `:154`, `:197`, `:311`, «Límites conocidos») y sus escenarios.

### No-alcance

- Un watcher, una herramienta `index_project` o un plazo en el cliente.
- Cambiar `REFRESH_INTERVAL` o el algoritmo de `plan_refresh`.
- Paralelizar el embedding de docs ni acotar su CPU (62).
- Priorizar los docs de un refresco (los editados primero).
- Cualquier cambio a las tablas de memorias o al contrato de embeddings.

## Decisiones abiertas

Son del usuario.

- [ ] **Frescura contra latencia.** ¿A (no se espera), B (espera acotada) o C? Se recomienda B.
- [ ] **El tope.** Si B, ¿cuántos segundos de `GRACE`? Se mide con el refresco de una edición chica en release; no hay cifra hoy.
- [ ] **Persistencia del error.** ¿`warning` hasta el próximo refresco exitoso o sólo hasta la próxima búsqueda?
- [ ] **Qué cuenta como doc malo.** ¿Sólo `Vector` por `to_unit`, o también la cuenta distinta de vectores?
- [ ] **Dónde se informa.** ¿Un campo nuevo en `RefreshReport` (`failed`) o `skipped` más el log? ¿El modelo lo ve en `warning`?
- [ ] **Cancelación.** ¿Se corrige que `last_started` quede sellado tras una cancelación, o con la tarea propia ya no importa?

## Cómo se mide

Tests RED primero (TDD estricto), sobre el `Harness` de `crates/clawg-cli/tests/api.rs`, sin modelo real ni red. El `FakeEmbedder` no tiene retardo (`crates/clawg-core/src/memory/embedder.rs:724`) y `CLAWG_TEST_EMBEDDER=stalled` (`crates/clawg-core/src/memory/bootstrap.rs:302`) sólo deja el modelo sin cargar: no sirve. Hace falta un doble local, como `FixedEmbedder`, que espere una puerta (`tokio::sync::Notify`) en `embed_batch`.

1. **Búsqueda con docs cambiados.** Indexar, modificar N archivos, vencer el refresco y llamar a `POST /v1/projects/docs/search` con el doble bloqueado: hoy no vuelve; con el cambio vuelve dentro de `GRACE` con `indexing: true`, y al abrir la puerta el refresco termina y la búsqueda siguiente ve los docs nuevos.
2. **El error de fondo llega al modelo.** Un embedder que falla en el refresco de fondo: hoy la búsqueda siguiente no trae `warning`; con el cambio sí, con la causa.
3. **Un doc malo no frena a los demás.** Un doble que devuelve un vector no finito para un doc: hoy los docs posteriores del plan no se indexan y `remove_docs` no corre; con el cambio sí, y el reporte cuenta el fallo con su causa. Otro caso con un fallo de `Embed` sigue abortando.
4. **Cancelación.** Soltar el futuro de la petición no deja el refresco a medias: la tarea termina y el índice no tiene docs partidos.
5. **Escenarios de `specs/project-docs`** con su prueba nombrada para cada MUST cambiado.

`rag-eval --docs` no se corre para esto: no pasa por la ruta que cambia. La suite completa sí, porque el salto por doc está en `refresh`, que comparten el daemon y la evaluación.

## Riesgos

- **Resultados viejos tras una edición.** Con A o un `GRACE` corto, la búsqueda que dispara el refresco puede no ver el doc editado. Es el precio explícito de no colgarse; `indexing: true` lo avisa.
- **Índice con vectores de dos modelos.** Tras un cambio de `model_id`, el refresco reemplaza los docs de a uno (`mod.rs:114`) y las búsquedas intermedias comparan contra una mezcla. Ya ocurre con un refresco en curso; con este cambio ocurre en más búsquedas, no en una espera.
- **Reintento perpetuo de un doc malo.** Un doc salteado conserva su huella vieja y se re-embebe en cada refresco; si es determinístico cuesta lo mismo cada 60 s. Se acota contando fallos o recordando la huella fallida, a decidir en el diseño.
- **Ruido del `warning`.** Un error persistente se repite en cada búsqueda hasta que un refresco termina bien.
- **Una tarea por búsqueda vencida.** El techo sigue siendo uno por proyecto (`mod.rs:165-174`), pero el costo pasa del llamador al daemon, y con 55 y 62 hay que decidir si eso se cuenta.
