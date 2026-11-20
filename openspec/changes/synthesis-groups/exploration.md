# synthesis-groups — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | S para medir (un script fuera del producto, sin código de clawg); M si la puerta abre y se calcula a pedido; L sólo si se elige cachear | — para medir. Se coordina con [stale-conclusions](../stale-conclusions/exploration.md) (140, el mismo export y la misma medida de cercanía), [memory-links](../memory-links/exploration.md) (134), [project-concept-map](../project-concept-map/exploration.md) (125, enfoque C) y los archivados [knowledge-synthesis](../archive/2026-10-13-knowledge-synthesis/proposal.md) y [conclusion-upkeep](../archive/2026-10-19-conclusion-upkeep/proposal.md); usa el mismo corte de vec0 que [knn-prefilter](../knn-prefilter/exploration.md) para descartar una vía | exploración (borrador) |

**Tesis:** `synthesize` le da al agente una lista plana de las memorias que ninguna conclusión cubre, las más nuevas primero y sin ningún criterio de tema (`crates/clawg-core/src/memory/upkeep/synthesis.rs:186`). Una conclusión, según el propio texto de la herramienta, generaliza dos o más memorias (`crates/clawg-cli/src/mcp/tools.rs:556-575`); encontrar cuáles van juntas es hoy trabajo del agente, sobre una ventana de 30 filas elegidas por fecha y no por tema. Con miles de pendientes, esa ventana casi no contiene pares del mismo tema. Este cambio pregunta si clawg, sin ningún modelo de lenguaje, puede **agrupar** las pendientes por cercanía y **nombrar** cada grupo con los términos que lo distinguen, dejándole al agente lo único que sólo él hace: **escribir** la conclusión. Es una pregunta de medición antes que de código: las conclusiones que el agente ya escribió dicen qué memorias juntó, y eso es un golden que no cuesta nada.

**Qué se verificó y qué no.** El código se leyó en `d658c8a9` (0.90.0, `Cargo.toml:6`). **No se midió nada sobre un store real, no se corrió ninguna herramienta, no se compiló y no se leyó ningún perfil de datos.** Las cifras de costo del k-NN son cuentas de operaciones, marcadas como estimación. El dato de escala (2.274 memorias sin revisar, el aviso de `recall_context` del 2026-11-20) sale de un store en uso y no se pudo reverificar desde aquí.

---

## Premisas verificadas

| Premisa | Resultado |
|---|---|
| `synthesize` lista las pendientes: 30 por defecto, 100 como máximo | **Cierta.** `DEFAULT_PENDING_LIMIT = 30` y `MAX_PENDING_LIMIT = 100` (`synthesis.rs:26`, `:28`), con `clamp(1, MAX)` (`:259`). El argumento lo dice: «1 to 100. Defaults to 30» (`crates/clawg-cli/src/mcp/server.rs:293-296`; `crates/clawg-cli/src/mcp/agent_contract.json:321-340`). |
| Una pendiente es una memoria viva que ninguna conclusión viva cubre | **Cierta.** `PENDING` (`synthesis.rs:36-41`): sin `deleted_at`, estado distinto de `superseded`, sin fila propia en `memory_derivation` (no es una conclusión) y sin una conclusión viva que la tenga de fuente. Si esa conclusión se borra o se reemplaza, las fuentes vuelven a ser pendientes (comentario `:32-35`; tabla en `crates/clawg-core/migrations/V10__synthesis.sql:8-14`). |
| El orden es por fecha, sin tema | **Cierta, y no hay otro.** `ORDER BY created_at DESC, id DESC LIMIT ?` (`synthesis.rs:186`); la salida lo declara con «newest first» (`tools.rs:680`; en español `:681`). El tipo `Synthesis` es una lista plana (`synthesis.rs:133-142`), igual que el cuerpo HTTP (`crates/clawg-cli/src/daemon/api/memories.rs:573-579`) y el formateo (`tools.rs:643-686`). |
| El agente ve una línea por memoria | **Cierta.** `#id tipo origin= from= texto`, con el texto cortado a 500 caracteres (`synthesis_line_with`, `tools.rs:608-620`; `CONTENT_PREVIEW_LIMIT = 500`, `:41`; los saltos de línea se aplastan, `:53-67`). Antes van la guía de qué es una conclusión (`:556-575`), las conclusiones ya sacadas (hasta 20, `synthesis.rs:31`) y las que hay que revisar (hasta 10, `:60`). |
| La pendiente es **una fila**, no un save | **Cierta, y difiere de otras vistas.** La búsqueda muestra un hit por save (`crates/clawg-core/src/memory/retrieval/pieces.rs:1-8`) y el contador de idiomas cuenta un save largo una vez (`COALESCE((SELECT MIN(p.save_id) FROM memory_piece …), -m.id)`, `crates/clawg-core/src/memory/retrieval/language.rs:22-30`). `synthesis.rs` **no nombra `memory_piece`** (cero coincidencias): las piezas de un save largo (`V11__memory_piece.sql:11-18`) son pendientes separadas, cada una con su vector, y cada una cuenta contra el límite de 30. |
| Las piezas de un save se embeben detrás de la cabeza del save | **Cierta.** `context` guarda la apertura del save con la que se embebió la pieza (`V11__memory_piece.sql:5-9`); el re-embebido la reconstruye (`crates/clawg-core/src/memory/upkeep/maintenance.rs:603-609`). Consecuencia: **las piezas de un mismo save comparten prefijo y quedan cerca entre sí por construcción**, no por tema. |
| El alcance por agente y por proyecto ya está en la consulta | **Cierta.** `Scope::push` agrega el predicado de alcance del lector (`reach_predicate!`, `synthesis.rs:161-165`; la macro está en `crates/clawg-core/src/memory/retrieval/search.rs:84-92` y sale de `sharing.json`, `crates/clawg-core/src/memory/domain/sharing.rs:5`) y `project_id = ?` (`synthesis.rs:171`). **Matiz:** con proyecto el filtro es una igualdad, así que no entran las filas sin proyecto ni las `user_preference` de otros proyectos, que la búsqueda sí lee (`memory_project_predicate!`, `search.rs:116-123`). Sin proyecto (`all_projects`) es el store entero (`synthesis.rs:169-173`). |
| `synthesize` marca el alcance como revisado aunque muestre 30 de N | **Cierta.** Escribe `synthesis_review` en la misma transacción (`synthesis.rs:275-281`); el recordatorio de `recall_context` cuenta desde esa marca y sólo aparece con 20 o más (`tools.rs:728`, `:731`). Mirar una lista de 30 apaga el aviso de las otras 2.244. No es de este cambio, pero con grupos se nota más (D5). |
| Una conclusión admite como mucho 50 fuentes | **Cierta.** `MAX_SOURCES = 50` (`crates/clawg-core/src/memory/ingest/save.rs:29`; el rechazo, `:247-255`). Un grupo de más de 50 no cabe en un `derived_from`. |
| Las pendientes tienen vector, y se lee por `rowid` | **Cierta.** `memory_vectors` es `vec0(embedding float[384])` (`V2__vectors.sql:6-7`), con `rowid` = `memory.id`; el re-embebido ya lee `v.embedding` con un `JOIN` por id (`maintenance.rs:587-609`) y normaliza a longitud 1 (`to_unit`, `crates/clawg-core/src/memory/domain/vector.rs:41`; `maintenance.rs:574-582`). **No se verificó** el camino de insert. |
| vec0 sirve para el k-NN de las pendientes de un proyecto | **Falsa para este uso.** vec0 corta sus `k` vecinos de **toda** la tabla y recién después se aplican proyecto, agente y estado (`search.rs:269`; ver [knn-prefilter](../knn-prefilter/exploration.md), premisas): pedir 15 vecinos de una pendiente devuelve sólo los que caen dentro del proyecto. Para agrupar **no hace falta**: SQL ya eligió el conjunto (`PENDING` más `Scope::push`) y la distancia exacta entre esas filas se calcula directo; la búsqueda ya usa `vec_distance_l2` por id (`search.rs:1008`). |
| El daemon tiene un mantenimiento diario | **Cierta.** `MAINTENANCE_PERIOD` de 24 h y primera pasada a los 5 min (`maintenance.rs:27`, `:30`, `:339-349`). **No conoce a ningún lector:** el alcance de `sharing.json` se evalúa por agente y por llamada (`reach_bind`, `search.rs:72-78`; `reader_bind`, `:97-99`). |
| Las listas de palabras funcionales de `language.rs` se reutilizan tal cual | **Parcial.** Hay una lista inglesa (`language.rs:86-94`) y una española (`:96-104`), y `is_identifier` (`:147`) y `without_code` (`:158`) separan lo que es código. Pero son `const` y `fn` **privados**, y las listas sirven para **detectar un idioma**: dejan afuera a propósito las palabras que existen en los dos (`a`, `no`, `me`, `he`, `has`, `era`; `:84-85`, `:95`) y no traen otras funcionales comunes (`o`, `u`, `si`, `mi`, `tu`). Sirven de arranque, no de lista de parada. |

### Un hallazgo sobre el propio planteo

«Sin orden temático el agente no puede juntar» es una **hipótesis, no un hecho medido**. No hay medición de cuántas conclusiones salieron de una ventana de 30, ni de cuántas pendientes quedan sin cubrir por no verse. Lo único que el código muestra es que **ningún mecanismo** pone cerca lo parecido. Por eso la puerta mide primero cuánto junta hoy la ventana por fecha: es la línea base a vencer.

---

## Qué dice el descarte de `ideas.md:125`

La fila descarta los grafos episódico-semánticos «por ahora: el paso de abstracción necesita un LLM, y lo determinista (decaimiento, activación) ya está cubierto por el puntaje» (`openspec/ideas.md:125`). Son dos razones y este cambio no las trata igual:

1. **«El paso de abstracción necesita un LLM».** Es cierta y este cambio **la respeta**. Escribir «el usuario prefiere X» a partir de varias memorias sigue siendo del agente, con `save_memory` y `derived_from`, como decidió knowledge-synthesis (opción c: «el agente ya es un LLM», [`archive/2026-10-13-knowledge-synthesis/proposal.md`](../archive/2026-10-13-knowledge-synthesis/proposal.md), «Decisiones»). Lo que se agrega es un paso **anterior** a abstraer: **repartir el trabajo**. No crea texto, no resume, no reemplaza fuentes y no escribe en el store; un grupo es una partición de una lista de ids, igual que hoy lo es «las 30 más nuevas».
2. **«Lo determinista ya está cubierto por el puntaje».** Es cierta **para la búsqueda**: el puntaje ordena hits contra una consulta. `synthesize` **no tiene consulta**: su pregunta es «¿qué va junto?», y la búsqueda por significado no la contesta porque necesita un punto de partida. Ese hueco es el que existe.

Conclusión: **no esquiva** la razón del descarte, se coloca del lado que esa razón deja libre (agrupar es determinista y gratis; abstraer es del agente). Que ese agrupamiento le sirva al agente es lo que decide la puerta, no este argumento.

## Cómo se relaciona con lo que ya hay

| Cambio | Qué hace | Relación |
|---|---|---|
| `memory-links` (134), pregunta 2 | Pregunta si una red de enlaces trae algo que la búsqueda no trae; su caso (b), «clawg enlaza por vecinos de embedding», es «la búsqueda precalculada» | **No lo pisa.** Esa objeción es sobre **enlaces guardados que se ofrecen en una búsqueda**, donde los vecinos ya salen por consulta. Acá los vecinos son un **paso intermedio efímero** para partir una lista de trabajo que no tiene consulta: no se guardan, no se ofrecen y no sufren el costo que 134 le pone al caso (b), una tabla que se rehace con cada cambio de modelo. Si este cambio **cacheara** el grafo (enfoque B), entra en ese terreno y debe esperar la puerta de 134. La puerta de saltos por enlace de 134 es independiente de la de abajo. |
| `project-concept-map` (125), enfoque C | «Conceptos derivados sin LLM: agrupar pasajes por cercanía… en contra: un grupo no trae nombre» | **El mismo mecanismo le serviría, y su «en contra» es lo que el nombrado de acá resuelve.** Qué cambiaría: el corpus son pasajes del índice de docs (`V12__project_docs.sql:22-31`), sin agente y con la partición por proyecto dentro de vec0 (`:33-39`), así que **no hay alcance por agente que cuidar**; y cada pasaje trae un `breadcrumb` (`:25`), una etiqueta mejor que cualquier término calculado. También cambia el uso: allá un grupo es un **nodo** que alguien lee; acá es una **lista de trabajo** que se consume. La función pura (vectores y textos → grupos con etiqueta) se reutiliza en 125 sin tocarla, o al revés. Ninguno bloquea al otro. |
| [`stale-conclusions`](../stale-conclusions/exploration.md) (140) | Pregunta si una conclusión con las fuentes vivas debe marcarse como atrasada cuando después se guardaron memorias cercanas sobre lo mismo; su definición (b) es la distancia L2 en espacio unitario bajo 0.42 entre la memoria nueva y la conclusión o sus fuentes | **Mira el otro extremo de la misma lista de `synthesize`:** allá, las conclusiones que ya existen; acá, las pendientes que todavía no tienen una. No se pisan, pero las dos puertas tienen que hablar la misma escala: **el mismo export y la misma medida de cercanía** (L2 en espacio unitario, con los vectores recalculados con el modelo que declara la cabecera del export). Así el tope que calibra el paso 4 de abajo se compara de forma directa con el 0.42 de 140 y con su sensibilidad de 0.35 y 0.50. Si las dos se abren, la función que calcula vecinas entre filas de `PENDING` y la que busca memorias posteriores cercanas a una conclusión es la misma, y se escribe una sola vez. |
| `knowledge-synthesis` y `conclusion-upkeep` (archivados) | Crean `derived_from`, `synthesize` y la revisión de conclusiones con fuentes perdidas | No se toca el contrato de guardado: una conclusión sigue siendo `save_memory` con `derived_from`. Cambia **cómo se presentan las pendientes**, no cómo se cubren. Los archivados no tratan agrupar ni otro orden que la fecha. |

---

## Enfoques con tradeoffs

Todos comparten una función pura: **vectores y textos de las pendientes → grupos con etiqueta**. Lo que cambia es **cuándo y dónde se calcula**.

### Piezas comunes

1. **Grafo.** Cada pendiente se une a sus `k` vecinas más cercanas (por ejemplo 15), sólo si la distancia queda bajo un tope. **El tope no es un número natural:** el modelo amontona los cosenos hacia arriba; el piso del puntaje es 0,79 porque es el percentil 5 de los cosenos consulta-pasaje del propio repositorio (`crates/clawg-core/src/memory/retrieval/score.rs:156-177`). Con `k = 15` y sin tope, **toda** pendiente tiene 15 vecinas y el grafo es una malla que se funde en una comunidad. Equivalencias en distancia L2 de vectores unitarios: coseno 0,79 es distancia 0,65; el borde de disputa de 0,42 (`crates/clawg-core/src/memory/revision/contradiction.rs:38`) es coseno 0,91. El tope se **calibra con las conclusiones que ya existen** (puerta, paso 4).
2. **Comunidades.** Propagación de etiquetas o Louvain (abajo).
3. **Nombre.** Frecuencia del término en el grupo por rareza en el corpus.
4. **Salida.** Etiqueta, tamaño e ids por grupo, más un tramo de **sueltas** que sigue siendo la lista de hoy.

### A. A pedido dentro de `synthesize`, sin tabla ni migración (recomendado si la puerta abre)

La función corre en cada llamada **sobre las mismas filas que `PENDING` y `Scope::push` ya eligen**.

- **A favor:**
  - **El alcance sale gratis y no se puede filtrar mal.** Grafo, vecinas, rareza de términos y etiqueta se calculan **sólo** con filas que el lector alcanza: un grupo no puede mostrar, ni formarse por, una memoria que el agente no ve. Es el requisito duro del objetivo 4 y la razón principal de esta opción.
  - Sin migración ni dato nuevo: nada del usuario cambia de forma y no hay vuelta atrás que documentar.
  - Se calcula sobre el estado de **ese** instante: si una conclusión nueva cubrió fuentes, la próxima llamada no las agrupa. Sin invalidación.
  - Los grupos son una partición efímera: no persisten, no envejecen.
  - El cálculo incremental (que una memoria nueva herede el grupo de sus vecinas) **no hace falta**: no hay estado que mantener entre llamadas.
- **En contra:**
  - **El costo se paga en la llamada.** Estimación, no medida: todos contra todos entre `n` vectores de 384 floats son `n(n−1)/2` productos de 384 multiplicaciones y sumas. Con `n = 2.000`: unos 2 millones de pares, 0,8 mil millones de operaciones, del orden de **0,2 a 1 s** en un hilo (1 a 4 mil millones por segundo según vectorice el compilador). Con `n = 15.000`: unos 112 millones de pares, 43 mil millones de operaciones, del orden de **10 a 45 s**; memoria `n × 1.536 B` (3 MB y 23 MB). A 2.000 es aceptable; a 15.000 no lo es para una tool que el agente espera.
  - **No puede correr dentro de `store.conn.call`**: la conexión es una sola y serializada, y una cuenta de segundos adentro frena todos los demás pedidos. El patrón ya existe: el relleno de idiomas lee por tandas y clasifica fuera del hilo de la conexión (`maintenance.rs:136-143`). La lectura (ids, textos, vectores) va en la misma transacción de hoy, que garantiza que total, lista y marca describan el mismo estado (`synthesis.rs:251-255`), y la cuenta va después.
  - **Hay que acotar el trabajo.** Con más de `N_MAX` pendientes (se fija con la micro-medición) se agrupan las `N_MAX` más nuevas y el resto queda **a la vista como un conteo** («otras 11.000 sin agrupar»), sin ocultarlas. Es una ventana de trabajo, no un filtro de datos.

### B. Calculado en el mantenimiento diario, con una tabla cacheada

`run_maintenance_once` (`maintenance.rs:339-349`) recalcula y guarda `(memory_id, grupo)` y la etiqueta; `synthesize` sólo lee. Una memoria nueva hereda el grupo mayoritario de sus vecinas.

- **A favor:** `synthesize` es una lectura rápida; el costo de 15.000 se paga una vez al día en el fondo.
- **En contra:**
  - **El alcance no cabe en una tabla compartida.** El mantenimiento no tiene lector (ver premisas). Si el grafo se arma con **todas** las filas, un agente con alcance reducido recibe grupos cuya **forma y etiqueta dependen de memorias que no ve**: una fila oculta que une dos grupos, un término que sólo existe en una fila oculta. Filtrar los ids al leer no alcanza, porque la partición y la etiqueta ya usaron el contenido. La alternativa es una tabla por alcance de lector (agente más lista de ocultos), que se invalida cuando cambia `sharing.json`, un archivo que no está en la base.
  - **Migración.** Una tabla nueva es esquema: `config.yaml` exige un test que siembre la versión anterior y pruebe que sigue alcanzable, snapshot `pre-migration` y vuelta atrás documentada.
  - **Envejece.** Hay que invalidar con cada borrado, reemplazo, restore, re-embebido (`model-reembed`) y mudanza de proyecto; todos pasan por otros módulos. Un olvido deja un grupo con una memoria que ya no es pendiente.
  - Sólo se justifica si A resulta demasiado lento, y eso lo dice la micro-medición, no una premisa.

### C. A pedido con memoria en el proceso del daemon, sin tabla

A con una caché `(alcance, conjunto de pendientes) → grupos` en memoria, que se pierde al reiniciar.

- **A favor:** la segunda llamada es instantánea y sigue sin migración.
- **En contra:** una estructura con estado dentro del daemon, donde `synthesis.rs` hoy es una función sobre la base; la clave de alcance es el JSON de `reach_bind` y `sharing.json` puede cambiar sin tocar la base. **No hay evidencia de que la segunda llamada importe:** el recordatorio pide una síntesis por tarea (`tools.rs:728-731`), no un lazo. Se descarta hasta que A muestre un problema real.

### Detección de comunidades

| | Propagación de etiquetas | Louvain |
|---|---|---|
| Código | unas 30 a 60 líneas, sin dependencia | unas 150 a 250 líneas, sin dependencia (no hay un uso que justifique agregar una) |
| Determinismo | sólo si se fija el orden de visita (por id) y el desempate (la etiqueta menor) | determinista con el mismo orden de nodos |
| Falla conocida | con una malla densa y un tope flojo **colapsa en una comunidad gigante**; un nodo puente funde dos temas | maneja mejor los puentes (maximiza modularidad) pero tiene un **límite de resolución**: junta grupos chicos que eran distintos |
| Calidad | aceptable si el grafo es ralo | mejor en general; más caro de verificar con tests |

**Recomendado:** empezar por propagación de etiquetas y mirar en la puerta las **dos fallas medibles**: el tamaño del grupo mayor y la fracción de memorias sueltas. Pasar a Louvain **sólo si** la puerta muestra el colapso o la fusión. Ambos son una función pura `aristas → grupo por nodo`, así que el cambio no toca lo demás.

### Nombrar un grupo

Para cada término: `df_grupo / tamaño_del_grupo` por `ln(N / df_corpus)`, con `N` y `df_corpus` contados sobre **las mismas filas de entrada** (por la razón de alcance de A).

- Con sólo la frecuencia del grupo salen «memoria», «proyecto», «que», «de». La rareza los baja sola, porque están en casi todas las filas, **antes** de cualquier lista de palabras funcionales. La lista queda como cinturón, no como mecanismo.
- **Piso de presencia:** un término sólo cuenta si está en al menos 2 filas del grupo (o en el 30 %, lo mayor); sin eso, una errata o un identificador de una sola fila gana por rareza.
- **Los identificadores suelen ser buena etiqueta.** `language.rs` ya los reconoce (`is_identifier`: rutas, `_`, `::`, dígitos, punto o mayúscula interna, `:147-154`) y separa el código (`without_code`, `:158`). La puerta lo comprueba con una muestra a mano.
- **Idiomas.** El store mezcla español e inglés (`memory.lang`, `V22__memory_lang.sql`). Con la rareza alcanza para casi todo. Las listas de `language.rs:86-104` se podrían reutilizar volviéndolas `pub(crate)`, pero **no alcanzan solas** (premisas) y compartirlas ataca el detector de idioma: agregar una palabra acá cambiaría esa detección. Si hace falta una lista, es una **constante propia** del nombrado (D4).
- La etiqueta es un **texto que el agente lee, no un dato guardado**: no se persiste ni entra en un export.

### Piezas de un save largo

1. **Agrupar por save, no por pieza (recomendado).** La unidad del grafo es el save (`COALESCE(MIN(save_id), -id)`, el cálculo de `language.rs:22-30`). Su vector es el de la **primera pieza** (la embebida sin cabeza ajena, `V11__memory_piece.sql:5-9`) o el promedio de las suyas. La línea que ve el agente sigue siendo por fila, con la pieza marcada, y un grupo cubre un save entero porque `derived_from` lleva los ids de todas sus piezas, **hasta 50** (`save.rs:29`).
2. **Dejarlo por pieza.** Más simple, pero **contamina el grafo**: las piezas de un save se acercan por construcción (premisas), así que un save de 12 piezas arma solo un grupo «de 12» que es un documento, no un tema. Es el caso en que la pureza miente.

No se sabe qué fracción de las pendientes son piezas; se mide en el paso 1 de la puerta, contando las líneas `piece` del export.

### Alcance, `sharing.json` y qué ve el agente

- **Regla:** grafo, vecinas, rareza y etiqueta se calculan sobre las filas que el lector alcanza y nada más. En A es automático; en B hay que construirlo.
- **Test que falla hoy y es obligatorio:** con una fila de otro agente, oculta por `sharing.json`, que sería el puente entre dos grupos, los grupos del lector son los mismos que sin ella, y ningún término de la etiqueta sale de su texto.
- **Sin proyecto** (`all_projects`): el store entero, como hoy (`synthesis.rs:169-173`). Un grupo puede cruzar proyectos (D3).

### Salida de la tool: tokens y contrato

- **Hoy:** hasta 30 líneas de hasta 500 caracteres: en el peor caso unos 15.000 caracteres, del orden de **3.500 a 5.000 tokens** (estimación a 3 o 4 caracteres por token), más la guía. **No hay medición propia de `synthesize`**: `rag-eval --agent-view` mide `search_memory` y `recall_context` (`config.yaml`, línea base de agent-view).
- **Con grupos:** el agente necesita **leer** las memorias de un grupo para abstraer, así que la salida no baja por sí sola. Cambia **qué** lee: 30 memorias de uno o dos temas en lugar de 30 de 25. Lo nuevo es la etiqueta (unos 10 a 20 tokens por grupo) y el tramo de sueltas. Una variante para controlar tokens es cortar más las líneas de un grupo grande (D2).
- **Contrato:** la descripción de la tool dice «the memories no conclusion covers yet, newest first» (`agent_contract.json:322`; `server.rs:2014`), la salida lo repite (`tools.rs:680`) y hay versión española (`tools_es.rs`, `agent_contract.es.json`). Cambiar el orden o el formato cambia **los dos contratos y los dos textos**: se pasa por `CLAWG_BLESS=1` y por los tests del protocolo en los dos idiomas (como en [memory-links](../memory-links/exploration.md), «Riesgos»). `protocol.md:154-160` describe qué lista `synthesize` y hay que revisarlo; es texto que [contract-diet](../archive/2026-11-20-contract-diet/exploration.md) tocó: el bloque `lean` lo achicó, así que hay que revisarlo en `protocol.md` y en `protocol.lean.md`.
- **Compatibilidad:** `limit` sigue valiendo (1 a 100, `server.rs:293-296`); con grupos pasa a ser un tope de filas mostradas.

---

## Puerta de medición gratis antes de cualquier código

**Pregunta:** ¿los grupos automáticos ponen juntas las memorias que un agente ya juzgó que van juntas, y lo hacen mejor que la ventana de 30 por fecha que ve hoy?

**Golden natural.** Cada conclusión viva con `derived_from` es un juicio: «estas memorias van juntas». Está en `memory_derivation` (`V10__synthesis.sql:8-14`) y sale en el export como líneas `derivation` con `memory` y `source` (`crates/clawg-core/src/memory/portability/format.rs:265`, `:348`).

**Cuántas hay.** **No se sabe desde aquí**: depende del store real, que no se leyó. Lo único que se puede decir sin leerlo: el protocolo ya pide producirlas (`protocol.md:154-160`) y el recordatorio sólo aparece desde 20 pendientes (`tools.rs:728`), pero 2.274 pendientes dicen que **se cubrieron pocas** respecto de lo guardado. El paso 1 las cuenta; si no alcanzan, la puerta dice «no concluyente» y no decide nada.

**Quién lo corre y con qué:** la persona, en local, sobre una **copia** del store o sobre un export, sin tocar el perfil vivo. `clawg export --out FILE` es de sólo lectura y no pide el candado; `clawg backup` escribe un snapshot en `<data>/backups` (`docs/reference/commands.md`, `export` y `backup`). El export **no trae vectores** (su cabecera sólo declara el modelo de origen, «the import embeds again», `format.rs:25-27`), por eso hay un brazo sin modelo y otro con. El mismo export sirve a las puertas de memory-links (134) y de stale-conclusions (140): se exporta una vez y se mide todo sobre ese archivo. **No se corrió nada aquí.**

### Procedimiento

1. **Contar y filtrar** (export). Conclusiones vivas con `derivation`. **Elegibles:** las de un solo proyecto, con al menos 3 fuentes que sigan vivas y sin `source = 'import'` (la historia importada no la escribió este agente). Informar: elegibles, fuentes en total y fracción de fuentes que son piezas de un save largo (líneas `piece`).
2. **Reconstruir el universo.** Las fuentes de esas conclusiones hoy **no son pendientes** (están cubiertas). Para medir como si no lo estuvieran, se agrupan las memorias vivas del proyecto **con las fuentes incluidas**: las pendientes actuales más las fuentes de las elegibles. Es un sustituto: en producción el grafo es sólo de pendientes, más chico. Se anota.
3. **Brazos de agrupamiento**, sobre ese universo, con `k = 15`:
   - **L (léxico, sin modelo):** vecindad por coseno de TF-IDF de los textos, con los identificadores como términos. Corre con sólo el export y es el piso barato: si L alcanza, el modelo no hace falta para esto.
   - **E (embeddings):** distancia L2 sobre `memory_vectors` de la copia, o re-embebiendo el texto con el modelo local, que ya se descarga una vez, sin API de pago.
   - **W (línea base):** lo que ve hoy el agente: ventanas consecutivas de 30 por `created_at DESC, id DESC` (`synthesis.rs:186`).
   - **R (control):** ventanas de 30 al azar, semilla fija.
4. **Calibrar el tope de distancia** (en L y E, antes de mirar las métricas): con las fuentes de las elegibles, dos distribuciones, la de pares de **una misma conclusión** y la de pares **al azar del mismo proyecto**. El tope se fija donde el 80 % de los pares de una conclusión queda por debajo; se informa cuántos pares al azar también (si más del 20 %, el brazo no separa y no sirve). Se compara con los umbrales conocidos: piso de coseno 0,79 (`score.rs:156`) y borde de disputa 0,42 (`contradiction.rs:38`). En E se informa también qué recall de pares darían los topes 0,35, 0,42 y 0,50, la misma sensibilidad que usa stale-conclusions (140), para que las dos mediciones se lean en la misma escala.
5. **Métricas**, sobre pares de fuentes de la **misma** conclusión:
   - **Recall de pares (RP):** fracción de pares positivos que caen en el mismo grupo (o ventana, en W y R). Promedio **por conclusión**, no por par, para que una de 40 fuentes no domine.
   - **Pureza (PU):** en los grupos con al menos 2 miembros de la muestra, qué fracción de esos miembros pertenece a la conclusión mayoritaria del grupo; promedio ponderado. Sólo cuentan miembros **cubiertos**: una memoria sin conclusión puede ir bien en un grupo y la muestra no lo sabe, así que PU es un **piso**, no un valor exacto.
   - **Forma:** tamaño del grupo mayor (% del universo), fracción de memorias sueltas (grupo de 1) y fracción en grupos de 3 a 50, que son los que caben en un `derived_from` (`save.rs:29`).
   - **Nombres:** 30 grupos al azar con al menos 4 miembros; la persona anota, **antes de ver los totales**, si la etiqueta nombra el tema (sí o no), e informa cuántas etiquetas son un identificador.
6. **Comparación pareada** (como la regla de `rag-eval --diff` en `config.yaml`): por conclusión, RP de E (o L) menos RP de W; se cuentan las que mejoran, empatan y empeoran.
7. **Micro-medición de costo** (mismo script, vectores sintéticos de 384 floats): k-NN exacto todos contra todos a 2.000 y 15.000 filas, tiempo y memoria. Fija `N_MAX` y decide A contra B (D1).

### Umbral, fijado antes de medir

- **Mínimo de datos:** al menos **20 conclusiones elegibles** y **150 fuentes**. Con menos, «no concluyente»: ni abre ni cierra, y se repite cuando haya más.
- **Abre** (se escribe la propuesta) si, para E **o** para L:
  - RP medio **≥ 0,60** y **al menos el doble** que el de W;
  - mejora a W en **6 o más conclusiones netas** (mejoran menos empeoran; el mínimo que `config.yaml` toma como conclusión, p < 0,05);
  - PU **≥ 0,50**;
  - el grupo mayor **≤ 25 %** del universo y **al menos 60 %** de las memorias en grupos de 3 a 50;
  - **≥ 70 %** de las etiquetas anotadas nombran el tema.
- **Cierra** si el RP de E y el de L no llegan a **1,3 veces** el de W, o si el grupo mayor pasa del 50 % con ambos detectores (la comunidad gigante, sin arreglo barato). Se archiva en «Ideas evaluadas» del README.
- **Intermedio** (RP bien y forma mal, o al revés): se cambia **una sola vez** el detector (Louvain) o el tope y se mide de nuevo antes de decidir.
- **Sesgo que no se corrige:** las conclusiones se escribieron mirando una lista por fecha, así que sus fuentes están **más cerca en el tiempo** de lo que estarían con otra lista. Eso **favorece a W**, y ganarle es un resultado más fuerte. Deja sin medir la agrupación de temas **lejanos en el tiempo**, donde más debería ayudar. La puerta es una cota inferior de la utilidad, no su medida.

**Costo:** horas de una persona: un script fuera del producto, de unas 150 líneas (TF-IDF, k-NN exacto, propagación de etiquetas), y la muestra a mano de 30 etiquetas. Sin API de pago.

---

## Decisiones abiertas

- **D1. A o B.** A (a pedido, sin tabla) si la micro-medición da menos de unos 2 s a 2.000 y un `N_MAX` razonable; B sólo si A resulta lento a escala y se acepta el costo de alcance por lector.
- **D2. Qué ve el agente de cada grupo.** Todas las líneas cortadas a 500 (como hoy), un encabezado más 3 líneas y el resto por id, o líneas más cortas. Cambia tokens y calidad de la conclusión; se prueba con 5 casos a mano.
- **D3. Orden de los grupos y alcance sin proyecto.** Por tamaño (los más generalizables primero), por la fecha de su miembro más nuevo (continúa lo de hoy) o por cohesión. Con `all_projects`: si un grupo puede cruzar proyectos.
- **D4. Palabras funcionales.** Ninguna (sólo rareza), una constante propia del nombrado, o exponer la de `language.rs`. Recomendado: ninguna al empezar, y la constante propia sólo si el 70 % de etiquetas no se alcanza por culpa de palabras funcionales.
- **D5. Qué marca como revisado.** Hoy mirar 30 apaga el aviso del resto (`synthesis.rs:275-281`). Dejarlo como está (recomendado: no es de este cambio) o marcar sólo lo mostrado.
- **D6. Un grupo de más de 50.** Partirlo con el mismo detector, o mostrar los 50 más cercanos al centro y dejar el resto pendiente. No entra entero en un `derived_from`.
- **D7. Piezas.** Agrupar por save (recomendado) o por fila; lo decide la fracción de piezas del paso 1.

## Cómo se mide

| Qué | Cómo | Quién | Cuándo | Puerta |
|---|---|---|---|---|
| Cuántas conclusiones con `derived_from` hay y si alcanzan | Export; contar elegibles y fuentes | La persona, en local, sin costo | Ya | Menos de 20 conclusiones o 150 fuentes: «no concluyente» |
| Si los grupos juntan lo que el agente juntó | Brazos L y E contra W y R; RP, PU, forma y 30 etiquetas anotadas a mano | La persona, script fuera del producto | Tras el paso anterior | Ver «Umbral» |
| Costo del k-NN todos contra todos | Vectores sintéticos a 2.000 y 15.000 filas | La persona, en local | Con la anterior | Menos de ~2 s a 2.000 y un `N_MAX` claro: A |
| Si el agente escribe mejores conclusiones con grupos | Tarea de agent-eval con brazos «lista plana» y «grupos», tres corridas por celda | La persona autoriza el gasto de API | Sólo si la puerta abre | Fuera de este documento |
| Que nada fuera de alcance entre en un grupo ni en su etiqueta | Test con fila oculta que sería puente | El cambio, al implementarse | Con el código | Obligatorio |

## Riesgos

- **Medir con pocos datos o con un golden sesgado.** Pocas conclusiones, o escritas sobre una lista por fecha. Hay un mínimo, un «no concluyente» y el sesgo está declarado.
- **Comunidad gigante.** Vectores que se amontonan hacen un grupo del 40 % del store. La puerta lo mide con el tamaño del grupo mayor; sin esa comprobación el resultado parece bueno y no sirve.
- **Un grupo que parece un tema y es un documento.** Las piezas de un save largo se parecen por construcción. Agrupar por save lo evita.
- **Filtrar memorias fuera de alcance.** Es el riesgo que decide A contra B: una etiqueta o una partición calculada con filas que el agente no ve es una fuga de contenido. Test obligatorio.
- **Ocultar datos.** Un grupo no puede **sacar** memorias de la vista: las sueltas y las que quedan fuera por `N_MAX` siguen a la vista, con conteo, y `list_memories` las alcanza como hoy. La vista agrega orden, no filtra.
- **Cambio de contrato.** El contrato del agente está fijado en dos idiomas; cualquier cambio de texto pasa por `CLAWG_BLESS=1` y por los tests del protocolo.
- **Bloquear el store.** Una cuenta de segundos dentro de la conexión serializada frena todo lo demás; el cálculo va fuera de ella (`maintenance.rs:136-143`).
- **Tomar un grupo por una conclusión hecha.** La guía ya dice que una conclusión generaliza dos o más memorias y que, si nada generaliza, no se guarda nada (`tools.rs:556-575`). La etiqueta se presenta como tema, no como afirmación.

## Esfuerzo e impacto

| Paso | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| Puerta (script fuera del producto, sin código de clawg) | Hacer primero | Decide si el resto se hace | Bajo | S: horas de una persona |
| Función pura (grafo, comunidades, etiqueta) con tests | Si la puerta abre | Medio: ordena el trabajo del objetivo 5; sin efecto en la búsqueda | Bajo (sin dato nuevo) | M |
| Integrar en `synthesize` (A), salida y contratos en dos idiomas | Con la función pura | Medio | Medio: contrato, tokens, alcance | M |
| Tabla cacheada y mantenimiento (B) | Sólo si A es lento | Bajo a medio | Alto: alcance por lector, migración, invalidación | L |
| Agent-eval de conclusiones con y sin grupos | Opcional, tras la puerta | Cierra la duda de utilidad real | Bajo | M, con gasto autorizado |

No toca la recuperación: búsqueda y `recall_context` no cambian, así que el `rag-eval --diff` que `config.yaml` exige a todo cambio de ranking no aplica. Sí aplican la regla de tests que fallan hoy y, si algún día se cachea, la de esquema.

## Fuentes

- Código de `develop` en `d658c8a9` (0.90.0), con las rutas y líneas citadas.
- [`memory-links`](../memory-links/exploration.md) (134), pregunta 2 y caso (b); [`project-concept-map`](../project-concept-map/exploration.md) (125), enfoque C; [`knn-prefilter`](../knn-prefilter/exploration.md), por el corte de vec0.
- [`archive/2026-10-13-knowledge-synthesis`](../archive/2026-10-13-knowledge-synthesis/proposal.md) (el agente escribe las conclusiones; `derived_from`; `synthesize`) y [`archive/2026-10-19-conclusion-upkeep`](../archive/2026-10-19-conclusion-upkeep/proposal.md) (conclusiones con fuentes perdidas).
- `openspec/ideas.md:125`, el descarte de los grafos episódico-semánticos.
- `docs/reference/commands.md` (`export`, `backup`) y `docs/reference/data-profiles.md` (`sharing.json`).
