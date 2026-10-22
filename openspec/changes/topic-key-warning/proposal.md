# topic-key-warning — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | revision-eval (48, archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Sale de la medición de 48 y de lo que revisó 70; no tiene exploración propia.

1. **Sin clave, la versión vieja queda viva.** En `revision-eval chains`, sin clave, en 40 de 56 transiciones la versión vieja queda viva sin marca o aparece arriba de la nueva ([verify de 48](../archive/2026-10-21-revision-eval/verify.md#chains---real-m1-m2-m4)).
2. **El agente no sabe que le faltó la clave.** La respuesta de `save_memory` nombra las filas cercanas (`related_text`, `crates/clawg-cli/src/mcp/server.rs:330-346`) y pide un veredicto, pero no dice que una clave haría que la próxima versión reemplace a la anterior sin juzgar nada.
3. **Una clave revisa sólo en su alcance.** `SCOPE_ROWS_SQL` compara agente, `COALESCE(project_id, '')` y clave (`crates/clawg-core/src/memory/topic.rs:81-86`). El detector, en cambio, también alcanza filas sin proyecto y preferencias de otros proyectos (`memory_project_predicate`, `crates/clawg-core/src/memory/search.rs:92-99`), y su candidata (`Candidate`) no decía de qué proyecto era.

## Lo que midió 48

En el export de `prod` el aviso dispara en 154 de 397 saves, el 49,4 % de ellos son revisiones, y cuesta 34 tokens donde aparece y 13,19 de media por save. Pasa sus umbrales con una salvedad: 199 de las 204 etiquetas son de un agente, bajo la lectura amplia de la regla; con la estricta la precisión sería a lo sumo 20,8 %. Por eso el aviso es **provisional** hasta que su criterio se compruebe en casos reales con el protocolo de abajo ([revision-recheck](../revision-recheck/tasks.md#decisión-sobre-el-aviso-68)).

## Decisiones

- **Cuándo dispara**, igual que en la medición (`store.rs`, `analyse`): un save sin `topic_key` cuyo detector lo emparejó con una fila del mismo agente y proyecto. El detector sólo corre para los tipos que afirman y sólo empareja con ellos, y una pieza absorbida por su gemela no deja candidata, así que «de un tipo que afirma» y «que insertó algo» van implícitos.
- **Qué fila nombra:** la candidata más cercana de su alcance, y de dos igual de cercanas la primera que encontró el detector, como `closest` en la medición.
- **Quién decide:** el daemon, en el worker, que tiene la clave, el proyecto y las distancias (`unkeyed_revision`, pura, en `topic.rs`). `Candidate.same_scope` dice si la candidata es del proyecto del save. La respuesta de `POST /v1/memories` la nombra en `revision_candidate`; un daemon anterior no manda el campo y el puente no avisa.
- **El texto:** la línea fijada antes de medir en 48 (`design.md`, D4), palabra por palabra, después de la línea `close to earlier memories`. Un test en `revision-eval` (`cost.rs`) compara la línea del producto con la que se midió, así que cambiarla obliga a medirla otra vez.
- **Qué hace el agente con ella.** Guardar el mismo texto otra vez bajo una clave no inserta nada: el save se absorbe en su fila, que toma la clave (`revise_in`, `topic.rs:145-148`). La fila nombrada no cambia; para ocultarla está el veredicto `supersedes` que ya pide la línea `close to earlier memories`.
- **`protocol.md` no cambia.** La línea se explica sola, y 48 midió su costo sin ningún párrafo nuevo en las instrucciones (`m4.fixed`).

## Criterio fijado antes de medir

Lo pide la tarea 3 de [revision-recheck](../revision-recheck/tasks.md#3-el-criterio-del-aviso-para-68-antes-de-implementarlo) y queda fijado acá, antes de ver ningún caso.

- **Qué es una revisión.** El save posterior revisa al anterior si, guardado el posterior, ocultar al anterior (que es lo que haría la clave) no pierde nada que siga siendo cierto y que el posterior no diga. Sale del mecanismo, no de la lectura de nadie: no hace falta que los dos tengan el mismo titular ni que el posterior contradiga al anterior.
- **Protocolo de etiquetas.** Dos pasadas de LLM ciegas e independientes sobre los mismos pares, cada una con la definición de arriba y sin ejemplos etiquetados, sin saber si el aviso disparó. Se informa su acuerdo como kappa de Cohen con su intervalo. Los desacuerdos los resuelve una tercera pasada, también ciega. No hay pasada contra el dueño del store.
- **Umbral de precisión: al menos el 40 % de los saves en que dispara son revisiones**, el de 48, con otra base. Una falsa alarma cuesta los 34 tokens de la línea y, si el agente la sigue, un save repetido que sólo le pone una clave a su propia fila: no oculta nada, y una clave mal elegida reemplaza algo recién cuando otro save la reusa. Un acierto evita que la versión vieja quede viva sin marca, que sin clave pasa en 40 de 56 transiciones. Con el 40 % hay a lo sumo una falsa alarma y media por acierto. El umbral de 48 se justificaba con que «el agente aprende a ignorarlo», un hábito humano; un LLM no aprende de una sesión a otra.
- **Dónde se mide.** En casos reales: saves hechos con el aviso ya en el producto, recuperables de un export (un save sin clave, de un tipo que afirma, con una línea `conflict` hacia una fila de su agente y proyecto). 70 comprobó que se reconocen sin una marca nueva, así que 68 no agrega marca. La precisión y la tasa de disparo se informan también por tamaño del store en el momento de cada save.

## Alcance

1. `clawg-core`: `Candidate.same_scope`; `unkeyed_revision`; `IngestReport.revision_candidate`, que el worker llena al terminar un save.
2. `clawg-cli`, daemon: `revision_candidate` en la respuesta de un save confirmado, omitido cuando no hay.
3. `clawg-cli`, cliente y puente: `SaveOutcome::Saved.revision_candidate`; `topic_key_warning` en `tools.rs`; `save_memory` agrega la línea.
4. `clawg-eval`: el test que ata la línea del producto a la medida.
5. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/mcp`.

## Fuera de alcance

- Una marca nueva en el save o en el export (70 mostró que no hace falta).
- El modo de `revision-eval store` que lista sólo los pares en que disparó: va con la verificación, cuando haya casos.
- La búsqueda por clave, la clave en `recall_context` y quitar reglas (M3 de 48).
- `protocol.md`, el ranking, el esquema y los datos.

## Cómo se mide el éxito

- Tests con el embedder falso, sin red ni perfil: la regla pura (`topic.rs`), el worker con candidatas de su proyecto, sin proyecto y bajo clave (`worker.rs`), la respuesta del daemon con y sin el campo (`api.rs`, `tests/api.rs`), la línea del puente (`server.rs`) y la línea medida (`cost.rs`).
- La verificación en casos reales con el criterio de arriba. Hasta entonces el cambio no se archiva.

## Datos y vuelta atrás

No toca el esquema, los datos, el detector ni el ranking: sólo agrega un campo a la respuesta del save y una línea al texto de `save_memory`. Un puente anterior ignora el campo; un puente nuevo contra un daemon anterior no avisa. Volver atrás es volver al binario anterior.
