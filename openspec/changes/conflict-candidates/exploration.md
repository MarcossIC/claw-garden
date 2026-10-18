# conflict-candidates — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (se coordina con recoverable-delete, context-budget y agent-lifecycle-hooks; ver «Interacciones») | cerrada; decisiones en la [propuesta](proposal.md#decisiones) |

**Tesis:** el detector mide **cercanía**, no contradicción. Cuando dos memorias del mismo agente caen en la banda de distancia, la nueva queda `disputed`, el par entra en ACTIVE CONFLICTS y las instrucciones del servidor le piden al agente que le pregunte al usuario. Si las dos dicen cosas distintas pero compatibles, el usuario recibe una pregunta sin sentido, y su respuesta («valen las dos») no se guarda en ningún lado. Antes de construir hay que **medir** cuántas veces pasa; hoy nadie lo midió, y la medición puede achicar este cambio.

## Qué de la premisa es cierto y qué no

| Afirmación | Hoy |
|---|---|
| El detector corre al insertar y marca la fila **nueva** `disputed` | **Cierto.** El worker llama a `detect_contradiction` entre embeber e insertar (`crates/clawg-core/src/memory/worker.rs:550-556`) y el insert escribe `status = 'disputed'` si hay candidatas (`store_insert.rs:338-339`), en la misma transacción que la fila. |
| Todo par disputado sale en ACTIVE CONFLICTS | **Parcial.** Sólo los pares con un lado entre las filas que el prompt ya muestra (`prompt_builder.rs:501-502`), de a lo sumo 5 (`MAX_CONFLICT_PAIRS`, `prompt_builder.rs:43`, `:234`) y sólo del propio agente (`:498`). El resto sólo se cuenta. |
| Las instrucciones mandan preguntar al usuario | **Cierto, y dos textos dicen cosas distintas.** `INSTRUCTIONS`: «ask the user which memory holds» (`crates/clawg-cli/src/mcp/server.rs:111-113`), sin condición. `CONFLICT_NOTE`: «estas memorias se contradicen» y preguntar «si vas a tomar una decisión que dependa de esto» (`prompt_builder.rs:348-351`). El detector no sabe si se contradicen; la nota lo afirma. |
| «Dos hechos distintos del mismo proyecto no tienen salida honesta» | **Falso a medias.** La salida existe: `Outcome::KeepBoth`, «the detector paired two claims that do not conflict» (`dispute.rs:20-27`; `keep_both` en `api.rs:529-531` y `server.rs:276-278`). Lo que **falta** es otra cosa: (1) el veredicto no queda: `drop_dispute_sql` saca el id de `disputed_against` y no escribe nada más (`dispute.rs:175-185`); (2) no hay motivo ni confianza; (3) no hay camino para que el agente cierre un par sin el usuario; (4) «relacionadas», «valen en ámbitos distintos» y «no se contradicen» se aplastan en un booleano. |
| «Los falsos positivos entre hechos distintos del mismo proyecto son probables» | **No medido.** Lo medido apunta a que son raros en el set actual: ningún par compatible del golden set cae bajo L2 0,50 y el primer par compatible escrito a mano está en 0,469 (`specs/disputes`, «Umbral y banda»; `contradiction.rs:15-33`). Pero son 10 pares compatibles: un par son 10 puntos de tasa. Y el barrido mide distancia entre dos textos, **no** el detector entero ni el volumen sobre un flujo de saves (`contradiction.rs:73-83` lo dice). El único fallo masivo medido con el modelo real fue el de los pedazos hermanos (220 de 263 pedazos), que R8 de `specs/disputes` ya cerró. |
| `disputed` cambia lo que se recupera | **Falso.** Puntúa igual que `active` (`score.rs:295-301`) y la búsqueda incluye ambos estados (`search.rs:176`). Sólo `superseded` sale del ranking. Lo único que `disputed` cambia es el bloque de conflictos, el workspace (que lo conserva, `workspace.rs:251-262`) y el filtro `status` de los listados. Además `search_memory` no muestra el estado: cada línea lleva `#id [kind/tipo/tier piece score by]` y el contenido, sin `status` (`crates/clawg-cli/src/mcp/tools.rs:93-102`), así que una fila `disputed` se lee como un hit normal. |
| «El agente se entera del par cuando guarda» | **Falso.** El worker marca la fila nueva (`worker.rs:550-556`, `store_insert.rs:338-339`), pero la respuesta del save no dice nada de filas relacionadas: el texto MCP sólo cubre `saved #N`, `reinforced`, piezas, enmascarados y colas (`crates/clawg-cli/src/mcp/server.rs:439-470`), y el cuerpo HTTP lleva `status`, `reason`, `memories[{kind,id,outcome}]` y `redacted` (`crates/clawg-cli/src/daemon/api.rs:434-466`); `SavedChunk` sólo trae `id` y `outcome` (`worker.rs:38-41`). El agente se encuentra el par más tarde, en el bloque de conflictos de `recall_context`, quizá en otra sesión y sin la conversación que produjo el save. |

## Problema (verificado contra el código)

### Cómo nace y muere hoy una disputa

- **Banda.** `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` = `[0,10, 0,42)` en L2 sobre vectores unitarios con pooling mean (`contradiction.rs:58`, `:66`, filtro en `:164-165`). Para vectores unitarios `L2² = 2 − 2·cos`, así que la banda es `cos ∈ (0,912, 0,995]` (`1 − 0,42²/2 = 0,912`; `1 − 0,10²/2 = 0,995`). Eso es cercanía temática alta, no contradicción: una reformulación, un complemento o una actualización caen en el mismo rango que una negación, que es la razón de fondo de la tesis. La calibración usó 10 pares que se contradicen y 10 compatibles escritos a mano (`contradiction.rs:15-23`). El mismo comentario de documentación todavía conserva la tabla del barrido con pooling CLS que argumenta por 0,35 (`contradiction.rs:35-57`): es texto viejo que contradice la constante vigente (0,42) y se borra cuando este cambio toque el archivo. Sólo `user_preference`, `architectural_decision` y `semantic_fact` entran (`store_insert.rs:45-50`); un `save_memory` sin tipo es `semantic_fact`, así que **todo save explícito de hoy pasa por el detector**.
- **Pool.** 5 candidatas más los hermanos del evento (`CANDIDATE_POOL`, `contradiction.rs:86`, `:152`). Un save puede abrir hasta 5 pares.
- **Alcance.** Sólo filas del mismo agente (`Reach::Own`, `contradiction.rs:131-134`) y, con proyecto, las de ese proyecto, las sin proyecto y las `user_preference` de cualquiera (`search.rs:176-179`). No hay pares entre agentes.
- **Dónde se guarda.** En la fila nueva: `status`, `disputed_against` (JSON de ids) y `last_disputed_at` (`V1__init.sql:24-28`). No hay tabla de pares ni lugar para un veredicto.
- **Cómo se cierra.** `resolve_dispute_core` (`dispute.rs:198`): gana una (`superseded`, `superseded_by`) o valen las dos (`dispute.rs:223-231`). Cerrar saca el id; no queda rastro de que el par existió.
- **Qué lo libera sin el usuario.** Borrar o reemplazar una de las dos (`release_challengers`, `dispute.rs:132`; `admin.rs:334-336`) y la pasada diaria (`release_orphaned_disputes`, `maintenance.rs:211`).

### Qué cuesta un par mal armado

- Cada sesión que recupere contexto sobre cualquiera de los dos lados vuelve a mostrar el par hasta que alguien lo cierra (la consulta no tiene memoria de «ya lo vi», `prompt_builder.rs:485-503`). Hoy son hasta 5 pares por llamada. Con un inyector de `recall_context` al inicio de cada sesión (agent-lifecycle-hooks, planeado) el costo pasa de «cuando el agente decide recordar» a «siempre».
- Cerrar con `keep_both` borra la pista: si más tarde algo vuelve a comparar esas dos filas (la re-detección que recoverable-delete evalúa al restaurar, o una migración), el par renace y se vuelve a preguntar. Hoy no pasa porque el detector sólo sondea al insertar y sólo contra lo anterior; pasaría en cuanto exista un segundo punto de detección.
- Un `resolve_conflict` equivocado de `supersede` saca una memoria del ranking (`score.rs`, `superseded` no puntúa). Es reversible (`specs/disputes` R3), pero el que lo decide es el agente que acaba de oír a un usuario que no sabe de qué le hablan.

### Lo que el detector no puede distinguir

Dos textos cercanos pueden ser: la misma afirmación reescrita; una afirmación y su corrección; dos hechos distintos sobre el mismo tema; o la misma afirmación en dos ámbitos. El vector da una distancia. Sólo la primera y la segunda justifican molestar al usuario, y la segunda sólo si el usuario no la dejó ya resuelta.

## Lo que ya existe y se reutiliza

- `detect_contradiction` (`contradiction.rs:137`) ya conoce la distancia de cada candidata (`h.distance`, `:164`) y la descarta: devuelve `Vec<i64>`. Guardarla es gratis.
- Exclusión de hermanos del mismo evento y de las fuentes de una conclusión (`worker.rs:541`, `contradiction.rs:158`). Es el mismo mecanismo que haría falta para excluir «la misma clave» (topic-revisions).
- `resolve_dispute_core`, `check_pair`, `release_challengers` y `settle_sql` (`dispute.rs:82`, `:132`, `:119`): el efecto de `supersedes` ya está resuelto y probado, incluida la vuelta atrás.
- Pie del bloque ya formado: `rank_conflicts` y `format_conflict_block` citan por id los lados que ya están en el prompt (`prompt_builder.rs:357`, `:376`), lo que context-density dejó fijado.
- Extracción de identificadores (`identifier_tokens`, `search.rs:699`; `is_identifier`, `:712`): token con letra y además dígito, guion bajo o «joroba» (`minúscula` seguida de `MAYÚSCULA`). Es `pub(crate)` y pura.
- Evaluación con el modelo real y el mismo worker del daemon: `EvalIngest` (`crates/clawg-core/src/eval_api.rs:457`), usada ya para medir los pedazos hermanos con 40 documentos (`openspec/changes/archive/2026-10-13-dispute-resolution/verify.md`). El programa de esa prueba no se commiteó.
- Snapshot previo a migrar (`snapshot_into(..., Reason::PreMigration)`, `db.rs:213`) y checksums de migraciones fijados (`migrations.rs:43-55`). La última es V12 (`V12__project_docs.sql`); la que agregue este cambio toma el siguiente número libre.

## Enfoques posibles

### A. Medir y, si el volumen es bajo, no construir

Armar el set etiquetado y la medición de punta a punta (ver «Cómo se mide»). Si la tasa de pares falsos por save es baja, el cambio se reduce a lo barato: corregir el texto de `CONFLICT_NOTE` para que no afirme una contradicción que no conoce, alinearlo con `INSTRUCTIONS`, y guardar el veredicto.

- A favor: sin esquema nuevo; la decisión se toma con datos.
- En contra: no resuelve «el veredicto no queda» ni el cierre sin el usuario.

### B. Tabla de candidatas con veredictos tipados y escalamiento (recomendado, sujeto a la medición)

El detector deja de tocar `status`. Anota **candidatas** en una relación y la fila nueva queda `active`.

Forma candidata (a afinar en el diseño):

```
memory_conflict(
  new_id, old_id,            -- clave primaria; new_id es siempre la fila posterior
  distance,                  -- la que ya calcula el detector
  state,                     -- pending | judged
  verdict, reason, confidence,
  created_at, judged_at
) WITHOUT ROWID
```

- Un par juzgado no vuelve: `INSERT OR IGNORE` sobre la clave, y el detector pregunta `NOT EXISTS` antes de anotar. Cierra la grieta de «el veredicto no queda».
- `resolve_conflict` acepta un veredicto tipado y un motivo. `supersedes` llama a `resolve_dispute_core` con `Outcome::Supersede` y conserva el efecto de hoy (`superseded`, fuera del ranking, reversible). El resto cierra el par sin tocar el ranking.
- **Escalamiento:** sólo los pares de riesgo alto llegan a ACTIVE CONFLICTS y al usuario. Los demás quedan consultables y el agente los cierra solo. Una función pura `needs_user(pair) -> bool` decide, para testearla sin base.
- **Migración:** hay que convertir las `disputed` de hoy (abajo).
- A favor: resuelve el costo para el usuario y deja registro; el ranking no cambia (ver `score.rs:295-301`).
- En contra: tabla nueva, migración sobre datos del usuario, una superficie más en el contexto del agente, y una regla de escalamiento que hay que calibrar.

### C. B más una segunda señal para crear la candidata

Exigir, además de la cercanía, que los dos textos compartan un identificador (`identifier_tokens`) o, cuando exista topic-revisions, la misma `topic_key`.

- A favor: filtra hechos distintos que sólo comparten tema.
- **En contra, verificado:** como **requisito** pierde el caso que justifica al detector. «El proyecto usa PostgreSQL» / «… SQLite» (`contradiction.rs:15-19`) comparte tokens, pero `SQLite` no es un identificador por `is_identifier` (letra sin dígito, guion bajo ni joroba: `S Q L i t e` no tiene minúscula seguida de mayúscula, `search.rs:715`), así que el par no tiene identificadores compartidos. Una lectura a mano de los primeros siete pares de contradicción del golden set muestra al menos tres sin identificador compartido (el botón «Cerrar»/«Cancelar», el componente `<app-person-name-inputs>`, el refactor «persona física»/«persona moral»). Es una lectura, no una medición: se mide sobre el set completo.
- **Uso razonable:** como **entrada del escalamiento** (un identificador compartido sube el riesgo), no como compuerta de creación. Queda a evaluar con la medición; si el identificador compartido separa bien, puede pasar a compuerta sólo para algún tipo.

### D. Guardar el veredicto dentro de `disputed_against`

Cambiar el JSON de `[12, 34]` a objetos con veredicto.

- A favor: sin tabla nueva.
- En contra: `NAMED_DISPUTES` lee `json_each(...).value` como id (`dispute.rs:102`) y `rest_status`, `release_challengers` y la consulta de `fetch_active_conflicts` dependen de esa forma; una versión anterior leería objetos como ids inexistentes y dejaría las filas `active` sin aviso. Cambia un formato guardado sin necesidad. Se descarta (ver abajo).

### E. Candidatas en la respuesta del save (complemento de B, no reemplazo)

La respuesta de `save_memory` lista las candidatas que el detector encontró: una línea sólo cuando hay alguna, por ejemplo `relacionadas: #a, #b — pueden ser una versión anterior; resolve_conflict si lo son`. Así el agente, que **todavía tiene la conversación que produjo el save**, juzga en el momento con `resolve_conflict` y los veredictos tipados, en vez de toparse con el par después, sin contexto.

- **Lo que no cambia:** las candidatas siguen **registradas** en la tabla de B (y por tanto alcanzables desde `recall_context` y los hooks). Se mantiene el punto de B: no pueden vivir **sólo** en la respuesta de un save, porque esa respuesta se pierde con la sesión y un agente que la ignora dejaría el par sin rastro. E hace que el camino barato sea el primero, no el único.
- **Qué hay que construir:** el worker ya tiene las candidatas con su distancia al decidir el insert (`worker.rs:550-556`); `SavedChunk` (`worker.rs:38-41`) gana un campo con los ids relacionados; `SaveBody`/`SavedMemory` (`api.rs:434-466`) lo exponen como campo opcional con `skip_serializing_if` (igual que `redacted`), y `save_outcome_text` (`server.rs:439-470`) lo imprime. Un puente anterior que no conozca el campo tiene que seguir mostrando el save. Con saves largos las candidatas se colapsan por save (ver «Interacciones»).
- **Costo en tokens:** cero en los saves donde nada está cerca (no hay línea). Con candidatas, una línea de unas decenas de tokens, acotada por el pool de 5 (`contradiction.rs:86`); no medido, se mide con el tokenizador real como el resto del texto de herramientas. La línea lleva sólo ids: para juzgar, el agente usa `read_memory`, una llamada más; mostrar un extracto sube el costo por línea y se decide en el diseño.
- **Riesgo propio:** una línea en cada save que cae en la banda entrena al agente a ignorarla si la mayoría son pares compatibles; por eso depende de la tasa de falsos de la medición.
- **Cómo se mide si sirve:** de los pares anotados, qué fracción se cierra con veredicto dentro de la misma sesión frente a cuántos siguen `pending` a las 24 h. La tabla de B no guarda un id de sesión, así que «misma sesión» se aproxima con una ventana fija entre `created_at` y `judged_at` (ambos ya están en la tabla); la ventana se decide en el diseño.
- **Recomendación:** B sigue siendo la base y E se suma a B. Si la medición da que casi todos los pares se cierran en el momento, `needs_user` y la sección de pendientes de `recall_context` pueden achicarse; si casi ninguno, ver la condición de reapertura en «Descartado».

## Descartado

- **Detectar sólo por título o solapamiento léxico.** El detector es denso a propósito: busca reformulaciones (`lexical: None`, `contradiction.rs:104-123`). Un par que cambia una palabra («Cerrar»→«Cancelar») no comparte más léxico que dos hechos distintos del mismo tema, y las contradicciones por negación comparten casi todo. Sin embargo la señal léxica sí sirve de **entrada** del escalamiento (enfoque C).
- **Lista de palabras vacías sólo en inglés.** El corpus es ES+EN (`V8__memory_fts.sql:30-34`); una lista en un idioma sesga el filtro contra el otro y el set de contradicciones es mayoritariamente español.
- **Veredictos que no cambian la recuperación ni el estado.** Un campo de texto libre sin efecto no cierra el par: el par sigue apareciendo y nada impide que el agente repita la pregunta. El veredicto tiene que ser lo que el detector consulta antes de anotar, y lo que el bloque filtra.
- **D (cambiar el JSON de `disputed_against`).** Rompe lecturas de versiones anteriores (ver D) por una ganancia que una tabla da sin riesgo.
- **Resolver pares con un modelo o un reranker local.** Agrega una dependencia y un costo de CPU por save sin evidencia de que el problema de volumen exista (ver «Cómo se mide»). **Condición concreta de reapertura:** si la medición de E muestra que los agentes dejan sin resolver la mayoría de las candidatas, un cross-encoder NLI multilingüe pequeño, en ONNX y local, podría confirmar las candidatas antes de mostrarlas. Costo: un modelo y un contrato de modelo nuevos, y una latencia de CPU por save que **no está medida en este hardware**. Para entrar necesitaría una precisión mínima fijada de antemano (propuesta ≥ 0,8 sobre el set etiquetado) y un presupuesto de latencia p95 por save; sin ambos números, sigue descartado.
- **Subir el umbral o achicar la banda como única medida.** Mueve falsos positivos por falsos negativos sobre 10 pares compatibles; con 0,42 hay 0,05 de margen hasta el primer par compatible conocido (0,469). Se puede ajustar, pero sólo con el set ampliado.

## Interacciones

- **recoverable-delete.** Borrar hoy libera las disputas (`admin.rs:334-336`) y `specs/disputes` R4 dice que un undelete «no restaura las disputas». Con una tabla de pares, una fila borrada deja su par intacto y las lecturas lo ignoran (`deleted_at IS NULL`); al restaurarla el par **vuelve solo**. Eso cambia R4 y hace innecesaria la re-detección al restaurar. Lo decide el usuario (abajo). El par con una fila purgada no puede quedar: `purge` borra `memory_derivation` en ambos sentidos (`secrets.rs:97-98`) y debe borrar los pares igual.
- **Pedazos de un save largo.** La exclusión de hermanos es por evento (`worker.rs:541`), no por el vínculo `memory_piece`. Guardar de nuevo una versión corregida de un texto largo hace que cada pedazo nuevo sondee a los pedazos del save anterior, que se solapan por diseño (el chunker repite 64 tokens, `pieces.rs`): es el mismo caso que dio 220 de 263 con hermanos. **No está medido para saves distintos.** Las candidatas tienen que colapsarse por save (`memory_piece.save_id`, `V11__memory_piece.sql`), no por fila, igual que la búsqueda muestra un hit por save (`specs/pieces` R4).
- **Sharing.** El detector sólo ve filas propias (`contradiction.rs:131`) y el bloque filtra por agente (`prompt_builder.rs:498`), así que no hay pares entre agentes y este cambio no los introduce. Un agente no juzga pares de otro (`specs/sharing` R5, `specs/visibility` R3-R4).
- **context-density.** El bloque cita por id los lados que el prompt ya imprime; la sección de pendientes debe usar el mismo mecanismo de citado para no repetir textos.
- **context-budget** (planeado). Si el presupuesto prioriza lo de mayor riesgo, la función `needs_user` es la entrada natural: un par escalado va primero, uno pendiente va al final o se omite.
- **agent-lifecycle-hooks** (planeado). Convierte «hasta 5 pares por recall» en «hasta 5 pares al abrir cada sesión»; sin escalamiento, el ruido llega siempre al usuario.
- **doctor-checks** (planeado). Debería contar candidatas con una fila purgada o inexistente (no deberían existir) y pendientes muy viejas.
- **topic-revisions (27).** Declarar el reemplazo al guardar (`replaces` con ids, E3 de 27) pertenece a 27, que debe aceptar los ids que la respuesta del save de este cambio devuelve (E). Este cambio define la **semántica del veredicto** `supersedes`; ambos escriben `superseded`/`superseded_by` por el mismo camino, `resolve_dispute_core` (`dispute.rs:198`). Un solo mecanismo, no dos.
- **knn-prefilter (37, cambio nuevo).** Las 5 candidatas salen del mismo KNN que filtra por agente, proyecto y estado **después** del corte de vec0 (`search.rs:169-180`, `knn_one` desde `search.rs:385`). `knn_one` reintenta una vez con una ventana cuatro veces mayor si el resultado quedó corto y hay filas elegibles (`search.rs:332-334`, `:457-470`), pero ese único reintento puede no alcanzar con muchas filas de otros agentes más cercanas: las propias más cercanas pueden no aparecer como candidatas. No está medido; el filtrado previo lo trata 37.
- **topic-revisions.** Una revisión cambia o reemplaza el texto de una fila: sus pares pendientes dejan de describir el contenido actual y hay que cerrarlos o recalcularlos. La tabla da dónde hacerlo. Una `topic_key` igual puede además excluir el par en el detector.
- **memory-provenance (41).** Hoy toda fila guardada por MCP dice `created_by_type = 'user'` (`store_insert.rs:154`), así que ni el detector ni el agente pueden distinguir lo que dijo el usuario de lo que infirió un agente. Con 41, el origen es una entrada natural de `needs_user` y del orden de los pares: entre una afirmación del usuario y una inferencia, la inferencia es la candidata a quedar `superseded`. Nunca como resolución automática, sólo como orden y como texto del par.
- **history-import (44).** Importar miles de notas de un mismo proyecto por el camino del worker las pasaría una por una por el detector. Sin este cambio, cada par cercano queda `disputed` y llega a ACTIVE CONFLICTS; con él, quedan como candidatas. La decisión de saltear el detector para lo importado la toma memory-portability (43). Si no se saltea, la compuerta de este cambio debería incluir un flujo con esa forma: muchas notas largas del mismo tema guardadas seguidas, que es el caso de volumen más alto que se espera.
- **Extracto en la respuesta del save (E).** Con unos 120 caracteres por candidata, el agente puede juzgar sin un `read_memory` por par. Ese es el costo contra el que se mide la decisión «¿sólo ids o con un extracto?».

## Migración de lo guardado

- **Qué hay hoy:** filas con `status = 'disputed'`, `disputed_against` JSON y `last_disputed_at`. Los veredictos pasados **no** se pueden reconstruir: un `keep_both` no dejó nada (`dispute.rs:175-185`). Los `supersede` sí dejan `superseded_by`, que puede convertirse en un par juzgado `supersedes`.
- **Propuesta:** en la migración, cada par vivo se inserta como `pending` con `distance = NULL` (el dato no se guardó) y las filas vuelven a `active`. Las columnas `disputed_against`, `dispute_count` y `last_disputed_at` **no se tocan ni se borran**: una versión anterior lee las filas como `active`, sin avisar de los pares, y para volver basta `UPDATE ... SET status = 'disputed' WHERE ...` desde las columnas intactas.
- **Los pares que ya existían** se escalan todos al principio, porque se perdió la distancia con la que decidir; el usuario los cierra una vez con veredicto y no vuelven.
- La base abre con la copia previa a la migración (`db.rs:213`); una base de una versión posterior se rechaza sin tocarla (`specs/storage`). Hay que sumar el checksum de la migración nueva a los fijados.
- **Estado `disputed`:** la decisión abierta es si deja de existir como estado de fila (los listados y `memory_status` pasan a contar pares) o se conserva como marca derivada de «tiene un par escalado pendiente». Lo primero es más limpio, lo segundo no cambia el filtro `status=disputed` de `GET /v1/memories` (`api.rs:191`) ni los listados que ya lo usan.

## Alcance candidato

1. Migración con la tabla de candidatas y la conversión de las `disputed` de hoy.
2. El detector devuelve `(id, distancia)`, anota candidatas con `INSERT OR IGNORE` en la misma transacción del insert, colapsa por save y no anota un par ya juzgado.
3. Función pura `needs_user` (tipo de los lados, distancia, señal léxica opcional, confianza del veredicto) con su tabla de casos.
4. Veredictos tipados en `resolve_conflict` y `POST /v1/disputes/resolve`, con motivo y confianza, `keep_both` aceptado como alias (compatibilidad con puentes y daemons de versión distinta), y forma por lotes.
5. ACTIVE CONFLICTS sólo con los pares escalados; los pendientes quedan consultables (sección acotada de `recall_context` o conteo en `memory_status`; decisión abierta) para que no vivan sólo en la respuesta de un save.
6. Texto de `CONFLICT_NOTE` e `INSTRUCTIONS` coherentes entre sí y sin afirmar una contradicción no confirmada.
7. `purge` borra los pares que nombran la fila; la pasada de mantenimiento no deja pares huérfanos.
8. Medición de punta a punta (set etiquetado y mecanismo en `clawg-eval`) y el umbral pre-comprometido.
9. Docs: `CLAUDE.md` del repo (herramientas y ruta HTTP) y las specs `disputes`, `mcp` y `retrieval`.
10. (Enfoque E) La respuesta del save lista las candidatas halladas como campo opcional, sin línea cuando no hay ninguna, y la medición de cuántos pares se cierran en el momento.
11. Borrar de `contradiction.rs` la tabla del barrido CLS (`:35-57`), texto viejo que argumenta por 0,35.

### No-alcance

- Un modelo o reranker para juzgar los pares.
- Cambiar el umbral o la banda, salvo que la medición lo pida.
- Pares entre agentes o que un agente juzgue pares ajenos.
- Detección de contradicción entre `episodic_trajectory` y `procedural_skill` (el detector sólo mira `memory`, `contradiction.rs:93-96`).
- Re-detección al restaurar (la decide recoverable-delete; con la tabla puede no hacer falta).
- Cambiar el ranking: `disputed` y `active` puntúan igual hoy.
- La señal `topic_key`: entra con topic-revisions, no antes.

## Decisiones abiertas

Son del usuario.

- [ ] **¿Se mide primero?** Recomendado: sí, con la compuerta de abajo, antes de escribir la tabla. Si la tasa de pares falsos es baja, ¿el cambio se achica a «A» (nota coherente, veredicto guardado) o se hace B igual por el costo de contexto con el inyector de sesión?
- [ ] **Umbrales de la compuerta.** Propuesta: pares falsos por save menor a 5 % sobre el flujo de hechos distintos, y recall de contradicciones reales sin bajar del 0,8 de hoy. ¿Se aceptan o se fijan otros?
- [ ] **Vocabulario de veredictos.** Propuesta mínima: `supersedes` (la otra queda reemplazada), `compatible` (no se contradicen; incluye «relacionadas»), `scoped` (valen en ámbitos distintos, con motivo obligatorio) y `conflicts` (confirmado: queda escalado al usuario). La lista del pedido (`related`, `compatible`, `scoped`, `conflicts_with`, `supersedes`, `not_conflict`) tiene seis; más valores son más texto en el esquema de la herramienta y más lugares donde el agente se equivoca. ¿Cuántos?
- [ ] **Qué puede cerrar el agente solo.** Propuesta: cerrar con `compatible`, `scoped` y `not_conflict` sí; `supersedes` sobre una `user_preference` o `architectural_decision` no, siempre con el usuario. Es lo que protege la regla de no ocultar memorias sin permiso.
- [ ] **Regla de escalamiento.** Propuesta inicial: escala si algún lado es `user_preference` o `architectural_decision` **y** la distancia es menor a un corte tomado de la medición (las contradicciones medidas llegan a 0,350 y las compatibles empiezan en 0,469), o si el veredicto del agente trae confianza baja. ¿Se acepta el criterio y que el corte salga de los datos?
- [ ] **Dónde se ven los pendientes.** (a) Una sección acotada en `recall_context`; (b) sólo un conteo en `memory_status` y una lectura bajo demanda; (c) una herramienta nueva. Cada herramienta suma texto a cada sesión y los tests fijan la cantidad (`server.rs:1361-1384`). Se inclina a (a) o (b).
- [ ] **Estado `disputed`.** ¿Desaparece como estado de fila o queda como marca derivada?
- [ ] **Segunda señal.** ¿Sólo entrada del escalamiento, o compuerta para algún tipo? Se decide con la medición.
- [ ] **Undelete.** ¿El par vuelve solo con la fila restaurada (cambia `specs/disputes` R4) o se cierra al borrar?
- [ ] **Pares ya existentes.** ¿Se escalan todos una vez, o se descartan los de más de N días?
- [ ] **Colapso por save.** ¿Un par por pareja de saves, mostrando el pedazo más cercano?
- [ ] **Candidatas en la respuesta del save (E).** ¿Se suma a B? ¿Sólo ids o con un extracto? ¿Qué ventana define «cerrado en el momento»?

## Cómo se mide

### Compuerta previa (antes de la propuesta)

Un set etiquetado y una medición de punta a punta, porque el barrido actual es de distancia y de 10 pares:

- Clases, con mínimos propuestos: ≥ 30 pares de **hechos distintos del mismo proyecto** (ES y EN, con nombres de código), ≥ 30 **contradicciones** (cambio de valor, negación, decisión reemplazada), ≥ 15 **reformulaciones**, ≥ 15 **evoluciones** (la versión nueva reemplaza a la vieja). Las etiquetas tienen que distinguir cuatro cosas que la banda trata igual: **contradicción**, **actualización** (versión más nueva del mismo hecho: las «evoluciones»), **complemento** (otro hecho sobre el mismo tema) y **reformulación**. Falta una clase explícita: el complemento sólo queda cubierto por «hechos distintos del mismo proyecto» si esos pares se toman del mismo tema; conviene marcarlos aparte, porque son los que la banda más confunde. La comparación pareada y los intervalos los da eval-rigor (33), que debería aterrizar antes de esta medición.
- Un **flujo** de ≥ 200 hechos distintos de un mismo proyecto guardados de a uno (por ejemplo una oración por requisito de las specs del repo), para medir **candidatas por save** y cuántas llegarían a ACTIVE CONFLICTS.
- Mecanismo: ingestar con `EvalIngest` y el embedder real (`eval_api.rs:457`), que pasa por el mismo worker y detector que el daemon, y leer `status` y `disputed_against` al terminar. `clawg-eval` no tiene hoy ningún modo que lea estados de disputa (el barrido está en `metrics.rs:90-150` y sólo recibe distancias). Hace falta un modo nuevo de `rag-eval` o un test de integración con `--real`. `EvalIngest::send` manda `project_id: None`, así que el filtro por proyecto no se ejercita; para un solo proyecto el resultado es equivalente (`search.rs:179`), pero queda dicho.
- Se reporta por clase la tasa de detección con intervalo (Wilson), no sólo el punto: con 30 pares, un par vale 3 puntos.

### Tests RED primero (una vez aceptado B)

1. Guardar una afirmación cercana a otra deja **ambas `active`** y una candidata `pending` con su distancia.
2. Un par juzgado (cualquier veredicto) no vuelve a anotarse aunque se guarde otra fila cercana a una de las dos.
3. `supersedes` deja la otra `superseded` con `superseded_by`, fuera de la búsqueda y de vuelta con el veredicto al revés (el efecto de `specs/disputes` R1 y R3 se conserva).
4. `compatible`, `scoped` y `not_conflict` cierran el par sin cambiar el ranking de ninguna fila.
5. `needs_user`, tabla de casos: tipos × distancia × confianza × señal léxica.
6. `recall_context` imprime en ACTIVE CONFLICTS sólo los escalados, con el tope de 5, y los pendientes donde se decida, citados por id.
7. `resolve_conflict` con `keep_both` sigue funcionando contra un daemon y un puente de versión distinta (campo opcional, valores por defecto).
8. Pedazos: un save largo no se empareja con sus hermanos, y volver a guardar una versión corregida colapsa por save (con el modelo real, en la medición).
9. Borrar una fila oculta su par; `purge` lo elimina; no queda par huérfano tras una pasada de mantenimiento.
10. Otro agente no ve ni cierra pares ajenos (404 uniforme, como `an_agent_cannot_resolve_another_agents_conflict`, `server.rs:2227`).
11. **Base sembrada en la versión anterior** (regla de `config.yaml`): filas `disputed` con `disputed_against` JSON, una con la vieja viva y otra con la vieja borrada sólo por `deleted_at`, más `superseded` ya resueltas. Abre con la migración, todas siguen alcanzables por búsqueda, los pares viven como candidatas, las filas están `active` y un `UPDATE` a partir de las columnas intactas las devuelve a `disputed` (la vuelta atrás). Adapta `disputes_written_by_the_previous_version_resolve_and_release`.
12. (Enfoque E) Un save que cae en la banda devuelve las candidatas en la respuesta (ids, sin duplicar pedazos hermanos); uno sin candidatas no agrega línea; un puente anterior sigue mostrando el save.
13. La migración deja el snapshot `pre-migration` (`db.rs:213`) y un binario anterior abre la base migrada.

### `rag-eval --diff`

El detector no cambia el ranking (`score.rs:295-301`), así que `--diff` del ancla debe dar cero filas distintas. El agent-view de `recall_context` sí cambia (el bloque de conflictos): el hit rate (39/46) no debe bajar y los tokens medios (1086,4) no deben subir; con pocos pares escalados bajan. Se informa el diff del agent-view además del ancla.

## Riesgos

- **Cerrar mal en silencio.** Un agente que concluye `supersedes` sin base saca una memoria del ranking. Es reversible, pero el usuario no se entera. Por eso la decisión sobre qué puede cerrar solo y el motivo obligatorio.
- **Pendientes que nadie cierra.** Si no salen en el contexto, la cola crece y la protección que daba el detector se pierde. Hay que acotar y ver la edad; doctor-checks puede avisar.
- **Perder el aviso que hoy funciona.** El detector acierta con las contradicciones directas. Cambiar de bandera a candidata no debe bajar el recall medido (0,8 a 0,40, 0,9 a 0,45 en el golden set).
- **Número de pares tras reescrituras de saves largos.** Sin medir para saves distintos (ver «Interacciones»).
- **Migración sobre datos del usuario.** Tabla nueva y conversión de estado; se cubre con el test 11 y con el snapshot previo.
- **Un cambio de formato en el contrato HTTP.** Un puente más viejo que el daemon o al revés; `version_warning` (`crates/clawg-cli/src/memories.rs`) detecta la versión distinta pero no impide la llamada. Los campos nuevos tienen que ser opcionales.
- **Texto de la herramienta.** Cada valor de veredicto y cada argumento suma tokens a todas las sesiones; no se mide hoy (context-density trabajó sobre el texto de `recall_context`, no sobre las herramientas).
- **Estado del repo:** el `CLAUDE.md` del proyecto describe `POST /v1/disputes/resolve` y la herramienta; cualquier campo nuevo obliga a actualizarlo.
