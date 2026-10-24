# topic-key-warning — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | revision-eval (48, archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Sale de la medición de 48 y de lo que revisó 70; no tiene exploración propia.

1. **Sin clave, la versión vieja queda viva.** En `revision-eval chains`, sin clave, en 40 de 56 transiciones la versión vieja queda viva sin marca o aparece arriba de la nueva ([verify de 48](../archive/2026-10-21-revision-eval/verify.md#chains---real-m1-m2-m4)).
2. **El agente no sabe que le faltó la clave.** La respuesta de `save_memory` nombra las filas cercanas (`related_text`, `crates/clawg-cli/src/mcp/server.rs:342-365`) y pide un veredicto, pero no dice que una clave haría que la próxima versión reemplace a la anterior sin juzgar nada.
3. **Una clave revisa sólo en su alcance.** `SCOPE_ROWS_SQL` compara agente, `COALESCE(project_id, '')` y clave (`crates/clawg-core/src/memory/topic.rs:81-86`). El detector, en cambio, también alcanza filas sin proyecto y preferencias de otros proyectos (`memory_project_predicate`, `crates/clawg-core/src/memory/search.rs:92-99`), y su candidata (`Candidate`) no decía de qué proyecto era.

## Lo que midió 48

En el export de `prod` el aviso dispara en 154 de 397 saves, el 49,4 % de ellos son revisiones, y cuesta 34 tokens donde aparece y 13,19 de media por save. Pasa sus umbrales con una salvedad: 199 de las 204 etiquetas son de un agente, bajo la lectura amplia de la regla; con la estricta la precisión sería a lo sumo 20,8 %. Por eso el aviso es **provisional** hasta que su criterio se compruebe en casos reales con el protocolo de abajo ([revision-recheck](../archive/2026-10-22-revision-recheck/tasks.md#decisión-sobre-el-aviso-68)).

## Decisiones

- **Cuándo dispara**, igual que en la medición (`store.rs`, `analyse`): un save sin `topic_key` cuyo detector lo emparejó con una fila del mismo agente y proyecto. El detector sólo corre para los tipos que afirman y sólo empareja con ellos, y una pieza absorbida por su gemela no deja candidata, así que «de un tipo que afirma» y «que insertó algo» van implícitos.
- **Qué fila nombra:** la candidata más cercana de su alcance, y de dos igual de cercanas la primera que encontró el detector, como `closest` en la medición.
- **Quién decide:** el daemon, en el worker, que tiene la clave, el proyecto y las distancias (`unkeyed_revision`, pura, en `topic.rs`). `Candidate.same_scope` dice si la candidata es del proyecto del save. La respuesta de `POST /v1/memories` la nombra en `revision_candidate`; un daemon anterior no manda el campo y el puente no avisa.
- **El texto:** la línea fijada antes de medir en 48 (`design.md`, D4), palabra por palabra, después de la línea `close to earlier memories`. Un test en `revision-eval` (`cost.rs`) compara la línea del producto con la que se midió, así que cambiarla obliga a medirla otra vez.
- **Qué hace el agente con ella.** Guardar el mismo texto otra vez bajo una clave no inserta nada: el save se absorbe en su fila, que toma la clave (`revise_in`, `topic.rs:145-148`). La fila nombrada no cambia; para ocultarla está el veredicto `supersedes` que ya pide la línea `close to earlier memories`.
- **El par registra que el aviso disparó.** Seguir el aviso borra lo que identifica al caso. El save repetido se absorbe en su fila, y `reinforce_in` (`store_insert.rs`) sólo toca `last_accessed_at`, `access_count` y `tier`. La fila toma la clave sin cambiar `last_modified_at`, y la fila del outbox se borra al terminar (`outbox_finish`). Un export lleva sólo la clave actual: ese save queda igual que uno que nació con clave, y una reproducción lo guarda con ella, donde `unkeyed_revision` no nombra nada. Sin una marca, la verificación vería sólo los avisos que el agente ignoró. Por eso el worker calcula la candidata antes de terminar el save (hoy la calcula después, `worker.rs:719-728`), y la transacción que lo termina (`outbox_finish` o `finish_save`) marca el par que une el save con la fila nombrada: `memory_conflict.topic_key_warned`, en V19. El export y la importación la llevan en la línea `conflict`; un lector anterior ignora el campo, y una línea sin él vale sin marca.
- **`protocol.md` no cambia.** La línea se explica sola, y 48 midió su costo sin ningún párrafo nuevo en las instrucciones (`m4.fixed`).

## Criterio fijado antes de medir

Lo pide la tarea 3 de [revision-recheck](../archive/2026-10-22-revision-recheck/tasks.md#3-el-criterio-del-aviso-para-68-antes-de-implementarlo) y queda fijado acá, antes de ver ningún caso real. Es el único lugar donde está escrito; [Cómo cambió el criterio](#cómo-cambió-el-criterio) dice qué reemplazó y por qué.

- **Qué es una revisión.** El save posterior revisa al anterior si, guardado el posterior, ocultar al anterior (que es lo que haría la clave) no pierde nada que siga siendo cierto y que el posterior no diga. Sale del mecanismo, no de la lectura de nadie: no hace falta que los dos tengan el mismo titular ni que el posterior contradiga al anterior. Es también la condición en que `supersedes` sobre la fila nombrada es correcto.
- **Protocolo de etiquetas.** Dos pasadas de LLM ciegas e independientes sobre los mismos pares, cada una con la definición de arriba y sin ejemplos etiquetados, sin saber si el aviso disparó. Se informa su acuerdo como kappa de Cohen con su intervalo. Los desacuerdos los resuelve una tercera pasada, también ciega. No hay pasada contra el dueño del store.
- **Población.** Los pares marcados de un export (R4), se haya seguido el aviso o no. Se suma una muestra al azar de los saves sin clave, de un tipo que afirma, en que no disparó y que tienen un save anterior vivo de su agente y proyecto, con su par como en 48 (`rest`). Las dos partes se etiquetan juntas. La muestra da la prevalencia y la sensibilidad, que mirando sólo los disparos no se ven.
  - **Son 100 saves, o todos si hay menos**, y entonces son un censo como los marcados. No son 50 como en 48 porque la regla de abajo toma π puntual, y todo su error de muestreo pasa a `p_min` sin que la regla lo vea.
  - Con las proporciones de 48 (66 de resto por 154 disparos), el resto es unos 0,43 de los marcados. 100 lo cubren entero hasta unos 233 marcados, más allá de `N_MAX`. 50 lo cubrían sólo hasta unos 116.
  - Con 50 y 200 marcados, el error de `p_min` sería de unos 0,9 puntos, contra 3,5 de la precisión. Llegaría a 3 o 4 puntos si el resto fuera 1,5 veces los marcados.
  - Los 50 pares de más cuestan sólo pasadas de LLM.
- **Umbral de precisión, relativo a la prevalencia.** El aviso vale si dispara al menos el doble en las revisiones que en las demás: razón de verosimilitud positiva (sensibilidad sobre tasa de falsas alarmas) de al menos 2. Con la prevalencia π de la población, medida con la misma definición, eso es una precisión de al menos `p_min = 2π / (1 + π)`: cerca de 55 % con π = 38 %, cerca de 26 % con π = 15 %. Un aviso que dispara siempre tiene precisión π, así que el umbral pide que el detector discrimine con cualquier lectura de la regla.
- **Regla de decisión, con tres salidas.** Se usa la cota de Wilson del 95 % de la precisión entre los pares marcados, y `p_min` con la estimación puntual de π.
  - El aviso se queda si la cota inferior supera `p_min`.
  - Se ajusta o se quita si la cota superior queda por debajo.
  - Si no, se juntan casos hasta `N_MAX` = 200 pares marcados, unos 500 saves al ritmo de disparo de 48 (38,8 %). Si al llegar sigue sin definirse, el aviso se queda: su costo ya pasa los topes de tokens de 48, y una falsa alarma sólo le pone una clave a su propia fila. Como referencia, con una precisión real 10 puntos sobre `p_min`, la cota se despega con unos 90 a 100 avisos.
- **Antes de quitar o ajustar, la precisión por distancia.** El par guarda la distancia del detector. Se mira sólo cuando la cota superior de la precisión quedó por debajo de `p_min`.
  - **Los tramos son tercios.** Los pares marcados se ordenan por distancia y el primer tercio, redondeado hacia arriba, es el más cercano. Un par a la misma distancia que el último de un tramo va con él.
  - **Los cortes salen sólo de las distancias**, antes de leer una etiqueta, así que fijarlos no es mirar resultados.
  - **Un tramo acierta si su cota inferior de Wilson supera `p_min`.**
  - Si el tercio más cercano acierta, se ajusta la regla a ese tramo en vez de quitar el aviso, en un cambio propio que se vuelve a medir. Si no acierta, se quita.
  - Con `N_MAX`, cada tercio tiene unos 67 pares. Partir en mitades daría más pares por tramo y menos resolución. Unos tramos fijos en la banda del detector (de 0,10 a 0,42) servirían para comparar stores, pero alguno podría quedar casi vacío.
- **Lo que hizo el agente, informado sin decidir.** Entre los pares marcados, en aciertos y en falsas alarmas por separado:
  - cuántas filas avisadas tienen clave (siguió el aviso);
  - cuántos pares tienen `supersedes` (ocultó la fila nombrada);
  - cuántas de esas claves reusó un save posterior.

  La marca dice que el detector disparó, no que el agente vio la línea: un save que respondió `202`, o uno hecho con un puente anterior, no la muestra. Por eso estas tasas son una cota baja.
- **Siguen de 48**, porque no dependen de la lectura de la regla: los topes de tokens (como mucho 40 donde aparece y 15 de media por save) y la prevalencia (al menos 10 % de revisiones, con cota inferior de Wilson ≥ 5 %).
- **Dónde se mide.** En casos reales de un clawg que ya escribe la marca. Un release con el aviso y sin la marca no da casos que cuenten. La precisión y la tasa de disparo se informan también por tamaño del store en el momento de cada save.

## Cómo cambió el criterio

El 2026-10-22, antes de etiquetar un solo caso real, se reemplazaron tres piezas. Las razones quedan acá para que no se vuelva a ellas. El criterio nuevo se fija conociendo los resultados de 48 con las dos lecturas de la regla, y se dice así.

1. **La precisión de equilibrio `p* = W / (s · T · A)`**, de la tarea 3 de 70, aprobada ese mismo día. Su beneficio no corresponde al mecanismo: supone que un acierto evita que la fila nombrada aparezca `s · T · A` veces, pero seguir el aviso no la oculta (ver «Qué hace el agente con ella»). El aviso rinde recién en la próxima versión del tema, si el agente lo sigue y un save posterior reusa la clave. Además tenía otros tres problemas:
   - `s` salía del set de cadenas sin fondo; con el fondo real, la última versión no aparece en `recall_context` en ninguna cadena (75).
   - `A` contaba accesos pasados de todas las filas, cuando importan los futuros de las que se revisan.
   - El error del umbral era mucho mayor que el de la precisión con que se comparaba.
2. **El 40 % fijo** de esta propuesta. No es relativo a la prevalencia. En el export de 48, con la lectura amplia, un aviso que dispara siempre habría tenido 38,1 % de precisión, así que pasar el 40 % casi no prueba que el detector discrimine: sensibilidad cerca de 90 % (76 de unas 84 revisiones), falsas alarmas sobre las no revisiones cerca de 57 %, razón cerca de 1,6. Con la lectura estricta la prevalencia baja, y el mismo 40 % pide mucho más.
3. **Verificar sin marca**, que 70 había dado por comprobado. Un export reconoce sólo los avisos que el agente ignoró (ver «El par registra que el aviso disparó»), y el sesgo va contra el aviso. Con una precisión real de 50 %, si el agente sigue el 80 % de los aciertos y el 10 % de las falsas alarmas, quedan visibles 10 aciertos y 45 falsas alarmas: 18 %. Cuanto mejor filtra el agente, peor mide el aviso, y la tasa con que lo sigue no se puede medir.

La definición de revisión y el protocolo de etiquetas no cambian.

El mismo día, también antes de ver ningún caso real, se fijaron dos cosas que el criterio dejaba abiertas: el tamaño de la muestra del resto (en «Población») y los tramos de distancia con lo que cuenta como acertar (en «Antes de quitar o ajustar»).

## Alcance

1. `clawg-core`: `Candidate.same_scope`; `unkeyed_revision`; `IngestReport.revision_candidate`, que el worker calcula antes de terminar el save. La marca del par (`memory_conflict.topic_key_warned`, V19) se escribe en la transacción que lo termina, y `ConflictLine.topic_key_warned` va en el export y en la importación.
2. `clawg-cli`, daemon: `revision_candidate` en la respuesta de un save confirmado, omitido cuando no hay.
3. `clawg-cli`, cliente y puente: `SaveOutcome::Saved.revision_candidate`; `topic_key_warning` en `tools.rs`; `save_memory` agrega la línea.
4. `clawg-eval`: el test que ata la línea del producto a la medida.
5. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/mcp`.

## Fuera de alcance

- El modo de `revision-eval` que lista y puntúa los pares marcados no es parte del producto: va con la verificación ([R5](spec.md#r5-revision-eval-warned-mide-el-aviso-sobre-los-pares-marcados)), y se arma antes del primer caso real para que el criterio quede escrito en código antes de ver ninguno.
- Cambiar la línea para que pida también `supersedes` sobre la fila nombrada. La línea es la que midió 48 (R3), y ocultar esa fila ya lo pide `close to earlier memories`. Con la marca, la verificación mide las dos acciones. Si el agente pone la clave pero no oculta la fila nombrada, cambiar la línea va en un cambio propio que vuelve a medir su costo.
- Suprimir el aviso en una parte de los saves, por la paridad de un hash del id, para medir su efecto causal: con el volumen de un solo store la potencia no alcanza. Se reconsidera con más casos.
- La búsqueda por clave, la clave en `recall_context` y quitar reglas (M3 de 48).
- `protocol.md`, el ranking, el esquema y los datos.

## Cómo se mide el éxito

- Tests con el embedder falso, sin red ni perfil:
  - la regla pura (`topic.rs`);
  - el worker con candidatas de su proyecto, sin proyecto y bajo clave (`worker.rs`);
  - la marca del par, que sobrevive a seguir el aviso (`worker.rs`), la migración, y el export y la importación (`portability`);
  - la respuesta del daemon con y sin el campo (`api.rs`, `tests/api.rs`);
  - la línea del puente (`server.rs`) y la línea medida (`cost.rs`).
- La verificación en casos reales con el criterio de arriba. Hasta entonces el cambio no se archiva.

## Datos y vuelta atrás

No toca el detector ni el ranking. Agrega un campo a la respuesta del save, una línea al texto de `save_memory` y una columna a `memory_conflict` (V19, sin marca por defecto en los pares que ya existen). Como toda migración, abrir un store con V19 pendiente escribe antes el snapshot `pre-migration`. Un puente anterior ignora el campo, y un puente nuevo contra un daemon anterior no avisa. Volver atrás ya no es sólo volver al binario anterior: uno anterior rechaza el store migrado. Hay que restaurar el snapshot `pre-migration`, como hace `clawg update` al deshacer, y se pierde lo guardado después.
