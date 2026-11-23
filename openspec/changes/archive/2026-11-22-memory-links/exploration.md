# memory-links — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 3 | S (dos mediciones sin código); lo que sigue, sólo si una puerta se abre | — para medir. **Alimenta al bloque `lean` de [contract-diet](../2026-11-20-contract-diet/proposal.md) (133, archivado el 2026-11-20):** 133 dejó `lean` como valor por defecto con las secciones del `session_summary` tal como están. La respuesta de la pregunta 1 dice si esas secciones pueden salir de `lean`; eso lo decide la medición de abajo, y no debe quitarse nada antes de que se corra. Se coordina con [memory-anchors](../2026-11-22-memory-anchors/exploration.md) (98), [project-concept-map](../2026-11-22-project-concept-map/exploration.md) (125), el [knowledge-synthesis](../2026-10-13-knowledge-synthesis/proposal.md) archivado y [session-lifecycle](../../session-lifecycle/exploration.md) (99), que toca el mismo resumen; y con [agent-eval](../2026-11-17-agent-eval-sessions/verify.md), que mediría la tarea de varias sesiones | exploración (borrador) |

**Tesis:** son dos ideas de producto que se parecen en una cosa: las dos preguntan si clawg guarda estructura que la búsqueda por significado ya da gratis. (1) Un `session_summary` junta en un párrafo hechos que podrían ser memorias sueltas; lo único que sólo él tiene es el estado de lo que quedó a medias. Si eso vive en `tasks.md` y en git, el resumen es un duplicado caro del contexto de arranque. (2) Una red de notas enlazadas, al estilo Zettelkasten, sólo suma si un enlace trae algo que la búsqueda no trae: una relación que no es de parecido. **Ninguna de las dos se decide hoy.** Cada una tiene una medición gratis y una puerta con el umbral fijado antes de medir; este documento deja los procedimientos escritos, sin correrlos, porque necesitan datos que sólo hay en la máquina de quien usa clawg (el store real, el set dorado y el modelo real).

**Qué se verificó y qué no.** El código y las specs se leyeron en `develop` (0.88.0, `8f20b542`). El estado del arte de la pregunta 2 se buscó en la web (una sola consulta, sobre el resumen de arXiv). **No se midió nada sobre un store real ni se corrió ninguna herramienta de evaluación.**

---

## Pregunta 1: ¿para qué sirve el `session_summary`?

### Lo que hace clawg con él, verificado contra el código

| Qué | Dónde | Efecto |
|---|---|---|
| Es un tipo de memoria más | `MemoryType::SessionSummary` (`crates/clawg-core/src/memory/domain/record.rs:99`, `:122`) | Se guarda con `save_memory`; no hay herramienta ni esquema propio |
| Decae el doble que un hecho | `session_summary: 0.0077` contra `semantic_fact: 0.0039` (`crates/clawg-core/src/memory/retrieval/score.rs:123-125`) | A igual parecido, sale cada vez más abajo en la búsqueda que un hecho de la misma edad |
| Nunca entra en un conflicto | `is_claim_bearing` es falso para él (`record.rs:132-137`); el detector sale antes de buscar (`crates/clawg-core/src/memory/revision/contradiction.rs:191`) | Un resumen nuevo nunca reemplaza ni marca a uno viejo: se acumulan, aunque se contradigan |
| Entra al tier 2 y sube al 1 con uso | importancia 6 al insertar (`crates/clawg-core/src/memory/ingest/store_insert.rs:93`); regla de `architectural_decision` y `session_summary` en `crates/clawg-core/src/memory/store/tier.rs:50-57` | Si se lo lee tres veces queda en tier 1; con importancia 9 o más nunca baja |
| Abre el contexto como LAST SESSION | `LAST_SESSION_SQL` (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs:926-935`), `with_last_session` (`:940`) | Es el más nuevo **del proyecto**, de cualquier checkout, entero aunque sea largo, sin umbral de edad, y se saca de RECENT y LONG-TERM para no repetirlo |
| El hook avisa al compactar | `crates/clawg-cli/src/hook.rs:221-234`: si la fuente fue `compact`, agrega que el protocolo pide un `session_summary` del trabajo compactado | Es la única vez que el sistema, y no el protocolo, le recuerda guardarlo |
| El protocolo lo pide | `crates/clawg-cli/src/mcp/protocol.md:109-110` (tipo), `:129-142` (siete secciones: Goal, Instructions, Discoveries, Accomplished, Next steps, Relevant files, Validation), `:146-150` (tras compactar) | Lo pide «at the end of a long session, or before the context is compacted», y aclara que las decisiones y preferencias van además en sus propios saves |
| Se cuenta | `SUMMARY` en `crates/clawg-core/src/metrics/session_report.rs:12`; la rúbrica `summary_sections` de agent-eval (`crates/clawg-eval/src/bin/agent-eval/rubric.rs:20-22`, `:115`) | Se sabe cuántas sesiones lo guardan |

El protocolo ya separa lo duradero de lo transitorio: prohíbe guardar «the state of the task at hand» como memoria y deja el resumen como la excepción. Es decir, **el resumen es hoy el único lugar donde el protocolo permite escribir estado de tarea**, y a la vez le pide a ese mismo texto Discoveries y Accomplished, que son hechos.

### La evidencia que hay, y por qué no alcanza

- En la campaña de agent-eval ninguna sesión guardó un `session_summary` (S = 0 de 9 en los tres brazos, [protocol-block](../2026-11-17-protocol-block/verify.md)), y el relevo de `a3` aprobó la tarea partida 3 de 3 ([agent-eval-sessions](../2026-11-17-agent-eval-sessions/verify.md), H1): los hechos se guardaron sueltos y llegaron en el arranque.
- **Límites.** Son tareas cortas con `claude -p`, de unos 66.000 tokens de contexto al bifurcar, que no compactan; en ellas el agente casi no guarda resúmenes, así que la campaña mide la ausencia del resumen sólo donde nunca iba a hacer falta. Y la información necesaria para continuar era un hecho (una decisión, un puerto), no un estado a medias. Es evidencia a favor de «los hechos sueltos bastan para un relevo de hechos», no de «el resumen sobra».
- En sesiones reales el agente sí lo guarda. Cuánto de lo que escribe ahí no está en otra memoria **no se sabe**: es lo que mide el paso 1.

### Enfoques

| Enfoque | A favor | En contra | Qué lo decide |
|---|---|---|---|
| **A. Dejarlo como está** | Cero código; ya funciona y lo cubren los tests y la rúbrica; es lo que la sesión siguiente lee primero | Cuesta tokens en cada arranque (se imprime entero, sin tope de edad); duplica hechos que ya son memorias, y como nunca entra en un conflicto un hecho corregido sigue vivo en el resumen viejo; el agente escribe estado que `tasks.md` o git ya tienen | Una fracción alta de contenido original (ver paso 1) |
| **B. Reemplazarlo por algo mecánico que arme clawg** | Sin depender de que el agente se acuerde ni de que escriba bien; podría armarse con lo que ya hay: los saves de la sesión (`source_session_id`, V1), la rama y el commit si 98 los registra, las memorias nuevas desde el último arranque | Un resumen mecánico lista qué se guardó, no qué quedó a medias: justo el contenido que el paso 1 mide si es el único original. Pide una vista nueva en el arranque y un cambio de contrato. Sólo sirve si el estado a medias vive en otro lado | Que el paso 1 muestre que el contenido original es chico y de clase «próximos pasos» recuperable de `tasks.md` o de git |
| **C. Sacarlo del protocolo y apoyarse en memorias sueltas y el repo** | Menos texto en el bloque del protocolo (lo que 133 busca); el agente guarda hechos, que es lo que el resto del sistema sabe ordenar, conflictar y reemplazar; LAST SESSION podría quedar para los resúmenes que alguien guarde igual, o irse | Pierde el estado de lo que quedó a medias en repos **sin** `tasks.md` (uso fuera de openspec), sin commits recientes o con trabajo sin commitear; un repo sin git queda sin nada. Los hechos sueltos dependen de que el agente los guarde, que hoy es el punto flojo del protocolo | Dos datos: la fracción de contenido original de los resúmenes vivos, y el resultado de la tarea de agent-eval con estado a medias (abajo) |

**Recomendado:** A mientras se mide; después, C **con una excepción mínima** si el paso 1 muestra que lo original es casi todo «próximos pasos» y esos viven en el repo. B se descarta hasta que haya datos: es la opción más cara y ataca el contenido que menos se conoce. Para empezar no hay que tocar código: la decisión de 133 es sólo de texto del protocolo (`protocol.md`, `protocol.es.md` y los contratos fijados en `agent_contract*.json`, con `CLAWG_BLESS=1`).

### Paso 1 — medición gratis sobre el store real (S, sin código de producto)

**Pregunta que contesta:** de los `session_summary` vivos, qué fracción de su contenido no aparece en ninguna otra memoria del mismo proyecto guardada en la misma sesión o después, y de qué clase es ese contenido.

**Quién la corre:** la persona, en local, porque necesita el store real (perfil prod). **No se corrió aquí.** No toca el store: `clawg export` es de sólo lectura y no pide el candado (`docs/reference/commands.md`, `export`).

**Procedimiento** (sin llamadas a una API de pago; un script fuera del producto, guardado en un directorio ignorado):

1. Exportar: `clawg export --out resumen.jsonl --profile prod` (sin `--include-deleted`: sólo lo vivo; con él se ve además qué se perdió). Cada línea `memory` trae `project_id`, `memory_type`, `created_at`, `source_session_id`, `content`, `status` y `deleted_at` (`crates/clawg-core/src/memory/portability/format.rs`).
2. Tomar los `session_summary` vivos. Un resumen largo se parte en piezas: el export trae las líneas `piece`; hay que juntar las piezas en orden antes de cortar.
3. Cortar cada resumen en **unidades**: una por viñeta u oración, ignorando los títulos de sección (`Goal:`, `Next steps:`…). Anotar la sección de cada unidad.
4. Para cada unidad, el **corpus de comparación** son las memorias vivas, no `session_summary`, del **mismo proyecto**, con `created_at` mayor o igual al inicio de la sesión del resumen (`source_session_id` igual o, si es `NULL`, las guardadas desde 6 horas antes del resumen).
5. Una unidad está **cubierta** si alguna memoria del corpus la dice. Se calcula en dos pasos, para que sea reproducible y gratis: (a) candidatas por palabras: la unidad comparte con la memoria al menos 60 % de sus identificadores y palabras de contenido (sin las de parada); (b) las candidatas se revisan a mano en una muestra (D4). **No usar un modelo de lenguaje de pago.** Con el modelo de embeddings local se puede sumar un umbral de coseno, pero es un refinamiento: el número que decide es el de palabras más la muestra a mano.
6. Etiquetar la clase de cada unidad **no cubierta**: *estado a medias* (lo empezado y no terminado), *próximos pasos* (lo que sigue), *hecho* (algo que se hizo o se encontró), *decisión* (una elección y su motivo), *archivos* (rutas relevantes), *validación* (el comando). Se etiqueta a mano una muestra de 40 unidades no cubiertas, tomada al azar, **antes de ver** el recuento por clase del script.
7. Para las clases «estado a medias» y «próximos pasos», comprobar si se recuperan de `tasks.md` y de `git log` y `git status` del proyecto (si hay `tasks.md`; si no, anotar «sin tasks.md»).

**Qué se informa:** por proyecto y en total: resúmenes vivos; unidades; fracción no cubierta; reparto de lo no cubierto por clase; de «a medias» y «próximos pasos», qué fracción se recupera del repo.

**Puerta, fijada antes de medir:**

- Un mínimo de datos: **al menos 15 resúmenes vivos y 100 unidades**. Con menos, el resultado es «no concluyente» y no decide nada; se repite cuando haya más.
- Si **menos del 30 %** de las unidades no están cubiertas, el resumen es sobre todo un duplicado: se recomienda C (sacarlo del protocolo) y 133 puede quitar sus secciones.
- Si **más del 60 %** no están cubiertas **y** la mayoría de lo no cubierto es «estado a medias» no recuperable del repo, el resumen se queda (A) y 133 no lo toca.
- Entre 30 % y 60 %, o si lo no cubierto es de clase «hecho» o «decisión» (que deberían ser memorias sueltas): el problema no es el resumen sino que el agente no guarda esos hechos aparte; se recomienda A, más una línea del protocolo que insista en guardarlos, y la tarea de agent-eval.

**Costo:** horas de una persona (la muestra de 40 a mano y un script de unas 100 líneas). Sin créditos de API.

**Límites:** un store con pocos resúmenes (el protocolo es reciente y las campañas no los guardan); la cobertura por palabras subestima lo cubierto cuando la memoria suelta usa otras palabras (por eso la muestra a mano); y un resumen guardado después de los saves de su sesión, que es el orden normal, es justo el que tiene todo para estar cubierto, así que el número es un techo de «duplicado», no un piso.

### Paso 2 — cómo lo mediría agent-eval (M, con crédito de API; no se corre hasta que el paso 1 lo pida)

Una tarea de **varias sesiones** en la que la información que la última sesión necesita es un **estado a medias, no un hecho**. La forma que ya tiene agent-eval (`split` y la bifurcada, de [agent-eval-sessions](../2026-11-17-agent-eval-sessions/verify.md)) alcanza; lo nuevo es la tarea:

- La sesión 1 empieza una migración de varios archivos y se corta con un archivo a medias (la tarea fija cuál y hasta dónde); el prompt le pide parar ahí. Ningún `tasks.md` ni commit refleja el punto exacto, que es lo que la sesión 2 tiene que retomar.
- La sesión 2 tiene que terminar sin rehacer ni romper lo hecho. El chequeo es de archivos (como los de las tareas actuales), no de texto.
- **Brazos:** `a3` (protocolo completo, que pide el resumen), `a3` con el resumen sacado del protocolo (un brazo nuevo, como `a3-short` pero sin las secciones) y `l0`. Tres corridas por celda, como las campañas anteriores.
- **Métricas:** aprobación 3 de 3; tokens de relevo (el sobrecosto que ya calcula H1); cuántas veces la sesión 2 rehace lo hecho.
- **Condición de validez:** que el brazo sin resumen no pueda sacar el estado del repo; si lo saca, la tarea no mide lo que debe y se reescribe.
- Costo orientativo: las 18 corridas de protocol-block costaron 5,46 USD; ésta es del mismo orden. La autoriza la persona, y no se corre si el paso 1 ya cerró la pregunta con un margen grande.

### Qué decide

Ver «Puerta» del paso 1. **Para 133:** hasta que el paso 1 se corra, 133 debe tratar las secciones del resumen como **intocables**; si necesita acotar el bloque antes, que lo haga sin quitar `Next steps` ni `Accomplished`, que son las que el paso 1 mide.

---

## Pregunta 2: ¿vale una red de notas enlazadas?

### Qué enlaces entre memorias ya existen, verificado contra el código

| Enlace | Dónde | Qué dice | Quién lo crea |
|---|---|---|---|
| `superseded_by` y `supersede_cause` (`revision` o `verdict`) | `V1__init.sql:24`; `V17__revival.sql:9` | Esta fila fue reemplazada por aquélla | clawg: un save con el mismo `topic_key`, o un veredicto `supersedes` |
| `revived_from` | `V17__revival.sql:14` | Esta fila volvió a estar viva y viene de aquélla | clawg |
| `topic_key` | `V16__topic_key.sql:7`; índice único por proyecto en `V20__project_scoped_indexes.sql:15` | Dos saves del mismo tema: el nuevo reemplaza al anterior | **El agente**, que decide la clave |
| `memory_derivation (memory_id, source_id)` | `V10__synthesis.sql:8-14`; se lee como `derived_from` en `crates/clawg-core/src/memory/retrieval/listing.rs:214` | Esta conclusión descansa en estas memorias | **El agente**, con `derived_from` en `save_memory` |
| `memory_conflict (new_id, old_id, verdict)` | `V15__memory_conflict.sql:11-22`; veredictos `supersedes`, `compatible`, `scoped` | Dos memorias que el detector vio cercanas y cómo se juzgaron | clawg (el detector, por vecindad de embedding); el veredicto lo da el agente o la persona |
| `disputed_against` | columna de `memory` (`V1__init.sql`) | Lista de ids con los que la fila está en disputa | clawg |
| `memory_piece (save_id, piece, memory_id)` | `V11__memory_piece.sql:11-18` | Las piezas de un save largo | clawg |
| `source_session_id` | `V1__init.sql:16` | Los saves de una misma sesión | el hook, vía el daemon |

Dos cosas que importan para la pregunta:

1. **Ningún enlace existente es «esto causó aquello».** Todos son de reemplazo (`superseded_by`, `revived_from`, `topic_key`), de fuente (`derived_from`), de cercanía que la búsqueda ya vio (`memory_conflict`, que sólo existe para pares cercanos en embedding) o de estructura (`memory_piece`, `source_session_id`). Un bug que causó una decisión no tiene dónde vivir.
2. **Los enlaces que crea el agente son los que hoy fallan.** `topic_key` y `derived_from` dependen de que el agente se acuerde; en protocol-block, tres de seis saves de la variante corta salieron sin `topic_key` ([protocol-block](../2026-11-17-protocol-block/verify.md), nota de J). Un enlace nuevo creado por el agente heredaría esa tasa de olvido, y cada uno suma tokens a cada save.

### Estado del arte (búsqueda web, una consulta)

A-MEM (Xu et al., arXiv [2502.12110](https://arxiv.org/abs/2502.12110), de febrero a octubre de 2025 en sus revisiones): cada memoria nueva se guarda como una nota con una descripción de contexto, y el sistema la enlaza con notas anteriores «según similitud semántica y atributos compartidos»; además puede actualizar las notas viejas (la «evolución de la memoria»). Es decir: el **enlace de A-MEM es, en lo esencial, vecindad de embedding más atributos**, y lo crea un modelo de lenguaje en cada inserción. Eso es la búsqueda por significado precalculada, el caso (b) de abajo. **No se verificó** si su ganancia sobre una línea base de búsqueda sale de los enlaces o de las descripciones de contexto que agrega a cada nota: se leyó sólo el resumen de la página, no el artículo, así que no se afirma cuánto aportan los enlaces. Se cita como motivación, no como evidencia de que valga aquí.

### Pregunta que decide

**¿Un enlace trae algo que la búsqueda por significado no trae?** Las notas parecidas ya salen juntas. Un enlace sólo suma cuando la relación **no** es de parecido. Y quién lo crea decide el costo:

| Quién crea el enlace | Qué trae | Costo |
|---|---|---|
| (a) El agente | Relaciones que no son de parecido: «este bug causó aquella decisión» | Más tokens por save; depende de que se acuerde, lo que hoy falla (`topic_key`, `derived_from`). Un enlace al id equivocado es peor que ninguno |
| (b) clawg solo, por vecinos de embedding | Nada que la búsqueda no dé: es la búsqueda precalculada | Una tabla, un trabajo de fondo y datos que envejecen cuando cambia el modelo (`model-reembed`); el detector de contradicciones ya paga ese costo para los pares cercanos |
| (c) clawg por reglas | Lo que el sistema ya sabe sin preguntar: misma sesión, mismo `topic_key`, mismo archivo citado (98) | Barato y determinista, pero casi todo eso ya es una columna y un filtro; no suma una relación nueva, sólo la hace consultable |

### Medición gratis (puerta antes de cualquier código)

**Pregunta que contesta:** sobre las preguntas del set dorado que la búsqueda **falla hoy**, ¿cuántas tienen su respuesta a **un salto** de un hit, por un enlace que existe? Si son casi cero, la idea cierra barata.

**Lo que existe hoy en `rag-eval`:**

- El set dorado es `docs/eval/golden.json` (48 filas, 46 casos) o `golden-expanded.json` (1.280 filas, 154 casos, con subconjuntos `lexical`, `lexical_related`, `multi_ref`, `long`…). Ambos están **fuera de git** (`docs/eval/**` en `.gitignore`). El esquema (`crates/clawg-eval/src/bin/rag-eval/golden.rs:30-153`) **no tiene enlaces entre filas del corpus**: sólo `stale_refs` en los casos `update` (`:103`; `golden-expanded.json` no tiene ninguno) y `contradiction_pairs` (28 pares `a` y `b` en ambos sets; `:158`), y esos pares son de filas que ya son parecidas.
- Una corrida de `rag-eval` calcula por caso `ranked_refs` (`main.rs:698`), que es lo que haría falta para ver qué hits dio la búsqueda; la corrida `--real` usa el modelo real. **No hay hoy ningún modo que cuente saltos por enlace**, ni un esquema de enlaces en el golden. El `docs/eval/results/` local sólo tiene corridas de `--docs` y las viejas de engram, ninguna de memoria con `ranked_refs`, así que tampoco se puede reutilizar una existente.

**Qué haría falta** (un modo nuevo, `rag-eval --link-reach`, que **no existe y no se escribió**):

1. Un campo opcional `links` en cada fila del corpus del golden (`{ "to": "ref2", "kind": "causes|supersedes|derived|same_topic" }`), con `deny_unknown_fields` actualizado y un validador (la ref existe, sin ciclos).
2. Etiquetar a mano esos enlaces sólo sobre los **casos que fallan**: primero correr `rag-eval --real`, tomar los casos con `recall@10 = 0` y, para cada uno, decir si su fila esperada está a un salto de alguna de las 10 primeras, por qué tipo de enlace y si ese enlace es de los que existen hoy (tabla de arriba) o de uno que habría que inventar.
3. El modo lee el resultado de esa corrida (`ranked_refs` por caso) y el golden con `links`, y cuenta: casos fallados; casos fallados con la respuesta a un salto **por un enlace que existe hoy**; y, aparte y sólo como dato, por uno nuevo. **Sin modelo ni store**: lee dos archivos, como `--lang-detect` (`docs/reference/testing.md`). Lo único que pide el modelo real es la corrida previa de `--real`.

**Quién la corre y qué cuesta:** la persona, en local: una corrida `--real` (modelo local, sin API de pago, minutos), el etiquetado a mano de los casos fallados (pocos con 46 casos) y un modo de unas 150 líneas con tests, que es código de las herramientas de evaluación, no del producto. **No se corrió aquí**: necesita el modelo real y el golden.

**Puerta, fijada antes de medir:**

- Sea *F* el número de casos que la búsqueda falla hoy (`recall@10 = 0`) y *L* los que tienen su respuesta a un salto de un hit por un enlace **que existe hoy**.
- *F* debe ser al menos 10; si el set falla menos, se mide sobre `golden-expanded.json`, y si ni así llega, «no concluyente».
- Si *L* es **menor que 3** o menor que el **10 % de F**, la idea cierra: el enlace no trae nada que la búsqueda no traiga. Se archiva en «Ideas evaluadas» del README.
- Si *L* es de 3 o más **y** al menos el 10 % de F, sigue un paso de diseño, no de implementación: qué enlace, quién lo crea (tabla de arriba) y cómo se lee (D2).
- **Sesgo conocido:** el golden se escribió para medir la búsqueda por significado y casi no tiene relaciones de causa; un resultado casi cero puede decir más de cómo se armó el set que de la idea. Por eso hay una segunda puerta sobre el store real: tomar 30 memorias de un proyecto real y, por cada una, anotar a mano «qué otra memoria necesito leer para entender ésta», y contar cuántas de esas respuestas **no** están entre sus 10 vecinos más cercanos de `search_memory`. Si son menos de 6 de 30, la idea cierra aunque el golden no lo hubiera dicho.

### Enfoques

| Enfoque | A favor | En contra |
|---|---|---|
| **A. No hacerlo (recomendado hasta que la puerta se abra)** | Cero costo; la búsqueda por significado ya junta lo parecido; los enlaces que sí aportan (`topic_key`, `derived_from`, veredictos) ya existen | Si hay relaciones de causa útiles, se pierden |
| **B. Mostrar mejor los enlaces que ya existen** | Barato: `read_memory` ya trae `derived_from`; sumar «reemplazada por» y «veredicto con» a la línea de un hit sale de columnas que ya están | El agente tiene que usarlos; cada marca suma tokens al hit (se mide con `rag-eval --agent-view`) |
| **C. El agente enlaza con un argumento nuevo de `save_memory`** | Es la única forma de traer relaciones de causa | Más tokens por save; hereda el olvido de `topic_key`; suma un argumento al contrato que el modelo ve (`agent_contract.json`); un enlace falso engaña |
| **D. clawg enlaza por vecinos de embedding (estilo A-MEM)** | No depende del agente | Es la búsqueda precalculada; una tabla que se rehace con cada cambio de modelo; el detector de contradicciones ya paga ese costo para los pares cercanos |

**Recomendado:** A, y B si la puerta se abre sólo por enlaces que ya existen. C y D quedan descartados mientras el número no los pida.

### Cómo se relaciona con lo que ya hay (sin pisarlo)

| Cambio | Qué enlaza | En qué se diferencia de éste |
|---|---|---|
| `memory-anchors` (98) | Una memoria con **código del proyecto** (rutas, nombres), para avisar cuando lo citado ya no existe | Enlaza memoria con código, no memoria con memoria; el aviso es de obsolescencia. Si 98 llega a registrar «mismo archivo citado», eso es el caso (c) de arriba, gratis |
| `project-concept-map` (125) | Un grafo **del código y los conceptos** del proyecto, sobre el índice de docs | Es un mapa de lo que el proyecto *es*; éste sería de lo que se *recordó*. Si 125 prospera, un enlace memoria a concepto sería la unión de los dos, y recién entonces |
| `knowledge-synthesis` (archivado) | Una conclusión con sus fuentes (`memory_derivation`, V10) | Ya es un enlace de memoria a memoria, creado por el agente, y el único de tipo «esto se concluyó de aquello»; este cambio no lo duplica: lo cuenta como existente en la medición |

---

## Decisiones abiertas

- **D1. ¿133 puede sacar las secciones del resumen antes de que se corra el paso 1?** Recomendado: no. 133 puede acotar el bloque sin quitar las secciones, o esperar. Lo decide quien prioriza 133.
- **D2. Si la puerta de enlaces se abre, qué enlaces cuentan.** Sólo los que existen hoy (recomendado para empezar) o también uno nuevo de causa; y dónde se leen: en la línea del hit, en `read_memory` o en una herramienta aparte.
- **D3. Si el golden se extiende con `links`, en qué archivo.** En `golden.json` (46 casos, poco para una conclusión) o en `golden-expanded.json` (154 casos, más datos, más etiquetado).
- **D4. El umbral de «cubierta» del paso 1** (60 % de palabras) es una conjetura; se ajusta con la muestra a mano antes de leer los totales, no después.
- **D5. El modelo del agente de la tarea de estado a medias.** Cambiarlo mezcla dos efectos; se deja el mismo que en las campañas anteriores.

## Cómo se mide

Resumido, con las puertas ya fijadas arriba:

| Qué | Cómo | Quién | Cuándo | Puerta |
|---|---|---|---|---|
| Fracción de contenido del `session_summary` que no está en otra memoria | Export del store real, corte por oración, cobertura por palabras y muestra a mano de 40 | La persona, en local | Ya; sin costo | Menos de 30 % no cubierto: sacarlo (C); más de 60 % y de clase estado a medias: se queda; mínimo 15 resúmenes y 100 unidades |
| Si el resumen cambia lo que hace una sesión siguiente | Tarea de estado a medias en agent-eval, tres brazos | La persona autoriza el gasto | Sólo si el paso 1 deja la duda | Aprobación 3 de 3 sin el resumen: se saca; si no, se queda |
| Casos que la búsqueda falla y están a un salto por un enlace que existe | Un modo nuevo de `rag-eval` sobre el resultado de `--real` y un golden con `links` | La persona, en local; código de herramientas de evaluación | Cuando se decida escribir el modo | *L* menor que 3 o que el 10 % de *F*: la idea cierra |
| Relaciones de causa fuera de los 10 vecinos | 30 memorias reales, a mano | La persona | Junto con la anterior | Menos de 6 de 30: cierra |

## Riesgos

- **Medir con datos pobres.** Un store con pocos resúmenes o un golden sin relaciones de causa da un resultado que parece una respuesta y no lo es. Cada puerta tiene un mínimo de datos y un «no concluyente».
- **Confundir «duplicado» con «sobra».** Que un hecho del resumen esté también suelto no prueba que el resumen no sirva: es el único lugar donde el agente lo ordena para quien llega. Por eso el paso 1 mide y el paso 2 contrasta con una tarea.
- **Quitar lo que el protocolo ya fijó.** El contrato del agente está fijado en `agent_contract.json` y `agent_contract.es.json`; cualquier cambio de texto pasa por `CLAWG_BLESS=1` y por las pruebas del protocolo, en los dos idiomas.
- **Enlaces falsos.** Un enlace puesto por el agente al id equivocado engaña más que la ausencia; su costo no se mide con recall.
- **Privacidad.** La exportación del paso 1 contiene el texto de todas las memorias: se guarda fuera del repositorio, con permisos de usuario, y se borra al terminar. Nada de lo medido se commitea.

## Esfuerzo e impacto

| Paso | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|
| Medición 1 (resumen) | Decide el texto del protocolo (alimenta a 133) | Bajo | Horas, sin código de producto |
| Tarea de agent-eval | Cierra la duda si el paso 1 no alcanza | Bajo | Medio, con gasto de API autorizado |
| Modo de `rag-eval` para saltos | Decide si hay una idea de enlaces | Bajo (sólo evaluación) | Bajo a medio |
| Cualquier código de producto | Ninguno hasta que una puerta se abra | — | — |

## Fuentes

- Código y specs de `develop` en `8f20b542` (0.88.0), con las rutas y líneas citadas.
- [`2026-11-17-protocol-block/verify.md`](../2026-11-17-protocol-block/verify.md) y [`2026-11-17-agent-eval-sessions/verify.md`](../2026-11-17-agent-eval-sessions/verify.md) (S = 0 de 9 y relevo de `a3`); [`2026-10-19-session-handoff/proposal.md`](../2026-10-19-session-handoff/proposal.md) (LAST SESSION).
- [`session-lifecycle`](../../session-lifecycle/exploration.md) (99): el traspaso por checkout y el comando de validación.
- Xu et al., *A-MEM: Agentic Memory for LLM Agents*, arXiv:2502.12110 (https://arxiv.org/abs/2502.12110); sólo el resumen de la página, no el artículo completo.
