# conflicts-on-demand — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el criterio del piloto «al abrir una sesión, el contexto trae entero el resumen de la sesión anterior» | M, por partes; ninguna toca `memory.db` | —; se coordina con session-lifecycle (99) y memory-links (134), y sigue la regla de la apertura de [contract-diet](../2026-11-20-contract-diet/proposal.md) (133, archivado) | propuesta; cierra D1 a D4 y D7 de la [exploración](exploration.md#decisiones-abiertas) con su opción recomendada. D5 y D6 son de la persona |

## Problema

1. **El inicio de sesión trae ACTIVE CONFLICTS aunque nada dependa de ellos.** `build_session_prompt` pide los pares escalados con un lado entre los saves recientes o las filas de la sección AUTO del workspace (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs:311-331`, `:365`, `:374`), y la apertura del protocolo le pide al agente que pregunte cada vez que el bloque aparece (`crates/clawg-cli/src/mcp/protocol.md:28-30`, `protocol.es.md:27-29`). Dos preferencias cercanas siempre escalan, y la sección AUTO las imprime en cada sesión: el bloque llega a todos los arranques de un store con preferencias revisadas.
2. **El presupuesto corta el traspaso antes que el bloque.** `fit_to_budget` saca LAST SESSION antes que los pares (`prompt_builder.rs:495-504`), y en el arranque, con reserva 0, antes que eso todos los saves recientes, cuyos lados en disputa pasan al bloque con su texto entero (`:657`, `:895-900`). En 12 días del piloto `context.cut` cortó `last_session` en 134 de 250 contextos y `conflicts` en 133; un arranque de este repositorio trajo más de 16 pares y ningún LAST SESSION. El criterio del piloto sobre el resumen de la sesión anterior falla por eso.
3. **Una búsqueda no dice que una fila está en disputa.** La línea de `search_memory` no marca a ninguno de los dos lados (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:232-267`), y la fila vieja de un par ni siquiera queda `disputed` ([exploración](exploration.md#la-búsqueda-no-marca-las-filas-en-disputa)).
4. **No hay forma de pedirlos.** `GET /v1/disputes` lista sólo los pares pendientes (`dispute.rs:545`), `memory_status` sólo cuenta los escalados y `list_memories` no filtra por estado ([exploración](exploration.md#no-hay-forma-de-pedirlos)).

## Alcance

- **El contexto sin consulta no trae ACTIVE CONFLICTS** (A2). Cada fila que imprime y que es un lado de un par escalado lleva la marca `conflict=#N` con el id del otro lado, con la misma consulta y la misma regla de alcance de hoy (`fetch_active_conflicts`, `prompt_builder.rs:1264-1325`): en la línea de un hit, ` conflict=#N` dentro del encabezado (`[2026-11-20 #12 user_preference conflict=#9]`); en una entrada de la sección AUTO, que no imprime su id, `(#12 conflict=#9)` al final de su primera línea. Sólo en el prompt: el archivo `workspace.md` no cambia.
- **El contexto con consulta conserva el bloque y lo corta antes que LAST SESSION** (B1). El orden de `fit_to_budget` pasa a ser: las entradas de AUTO sobre la mitad, los hits sobre la reserva, los pares desde el último, LAST SESSION, los hits que quedan y el resto de AUTO. Sus hits llevan también la marca, así que un par que el presupuesto saca sigue marcado en sus hits.
- **`search_memory` marca los dos lados** (C1). `POST /v1/search` manda en cada hit los ids del otro lado de cada par escalado cuyos dos lados alcanza el lector, y la línea los muestra como ` conflict=#N`. Un daemon anterior no manda el campo y la línea sale como hoy.
- **A pedido, `list_memories` con `conflicts`** (P1). Un booleano opcional: con él, en lugar de filas, la herramienta lista los pares escalados del proyecto abierto (la regla de `memory_status`, BI8 de [`specs/mcp`](../../../specs/mcp/spec.md)), cada lado con su id y su primera línea, y termina con una línea que pide preguntarle a la persona por cada par y registrar sólo su respuesta. Lo sirve `GET /v1/disputes` con un parámetro nuevo que lista los escalados en lugar de los pendientes, con un tope que fija la spec. Contra un daemon que rechaza el parámetro (`deny_unknown_fields`, `crates/clawg-cli/src/daemon/api/disputes.rs:46-47`), pide reiniciarlo con `clawg stop`.
- **Los textos** (E), en las dos lenguas:
  - la frase de ACTIVE CONFLICTS de la apertura pasa a pedir que se pregunte cuando una respuesta depende de una memoria marcada `conflict=`, o cuando la persona pide revisar los conflictos, y que se registre su respuesta con `resolve_conflict` y `user_answered`. En inglés cabe en los 205 caracteres de la vieja (con `en`, la apertura mide 2.047 de 2.048); conserva «ask the user which memory holds» y «pregúntale a la persona qué memoria vale», y la frase de VR3 no cambia;
  - `resolve_conflict` y su `keep` nombran la marca y la lista de `list_memories` en lugar de ACTIVE CONFLICTS; `format_resolution` y `format_open_pairs` dicen dónde se listan los pares que esperan, sin listarlos ni decir cómo juzgarlos (VR4 y VR5 siguen);
  - el comentario de `workspace.rs:390-393` dice por qué la sección AUTO conserva las filas en disputa: la marca.
- **`context.cut` lleva también `state`** (`searched`, `recent`…), el de `context.retrieval_state`, para medir el arranque aparte (`record_context`, `crates/clawg-cli/src/daemon/api/memories.rs:1120-1126`).
- `docs/reference/daemon.md`: el campo del hit, el parámetro de `/v1/disputes` y dónde aparecen los pares escalados.

## Fuera de alcance

- **Sacar el bloque de `recall_context`** (B2). Queda como paso siguiente si `context.cut{block=conflicts,state=searched}` sigue alto con la métrica separada.
- **Una herramienta nueva** (P3) **o una orden nueva de la CLI** (P4). Para mirar sin el agente ya están `clawg memories list --status disputed --here` y `clawg memories show ID`.
- **Listar pares en `memory_status`** (P5): VR4 y VR5 no cambian.
- **Una línea en el arranque con cuántos pares esperan** (D5): el arranque no habla de conflictos; quien quiere verlos los pide con `list_memories`.
- **Resolver desde la CLI.** El veredicto sigue siendo `resolve_conflict`, con `user_answered` para reemplazar una preferencia o una decisión.
- **Lo que corta el resumen fuera del bloque:** la mitad que se lleva la sección AUTO (SC2) y lo que escribió la persona (SC5). Si la medición dice que siguen cortándolo, es otro cambio.
- **La NOTE del bloque, el detector y los veredictos:** no cambian.
- Cursor: recibe el mismo contexto y las mismas instrucciones; no tiene bloque en `CLAUDE.md`.

## Decisiones

Cerradas aquí, con las alternativas y su costo en la [exploración](exploration.md#enfoques):

- **D1 → A2.** Sin el bloque el agente no sabría que una preferencia del workspace está en disputa (A1); el bloque detrás del resumen (A3) sigue llegando a cada arranque con la frase que pide preguntar. La marca cuesta unos 15 bytes por fila.
- **D2 → B1.** Es una búsqueda, y el bloque da los dos textos sin otra llamada. El orden se invierte porque un par se vuelve a encontrar (la marca en cada búsqueda que lo toca, la lista a pedido) y el resumen no tiene otra consulta que lo traiga.
- **D3 → C1.** El `status` marcaría sólo la fila nueva y sin decir contra cuál.
- **D4 → P1.** Un parámetro pesa unos 150 caracteres por idioma, y sólo al cargar `list_memories`; una herramienta nueva paga su nombre en cada pedido y entre 700 y 900 caracteres al cargarla. Filtrar por `status` (P2) listaría filas y no pares: una llamada a `read_memory` por par. El nombre de `list_memories` no dice «conflictos», así que el camino lo dan `resolve_conflict`, que el agente carga cuando se habla de conflictos, y la línea de `memory_status`.
- **D5 → nada.** El arranque deja de interrumpir con conflictos, que es el motivo del cambio; una línea con el número volvería a ponerlos en cada sesión. La lista a pedido cubre que se acumulen.
- **D6 → las dos.** El cambio no elige: los pares que ya esperan salen cuando una búsqueda los toca, y todos juntos cuando se piden.
- **D7 → sí.** Sin `state` en `context.cut` no se sabe si el arranque se arregló.

No hace falta `design.md`: ninguna decisión cambia la arquitectura, y las alternativas ya están medidas en la exploración.

## Cómo se mide el éxito

**Sin pagar, con tests que hoy fallan** (la lista entera en la [exploración](exploration.md#cómo-se-mide)):

- un contexto sin consulta con una fila reciente y una preferencia del workspace en disputa no trae ACTIVE CONFLICTS, y las dos filas llevan la marca;
- un contexto sin consulta con un workspace, un resumen largo, saves recientes y un par que no entran juntos trae LAST SESSION entero: el criterio del piloto como test;
- con consulta, los pares se van antes que LAST SESSION y la marca queda en los hits;
- `POST /v1/search` marca sólo pares escalados cuyos dos lados alcanza el lector, y `search_memory` muestra la marca;
- `list_memories` con `conflicts` lista los escalados del proyecto con los dos lados;
- `context.cut` lleva `state`;
- las tres aperturas siguen en 2.048 caracteres o menos; los contratos se regeneran con `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y `... the_spanish_agent_contract_matches_its_file`, y su diff es sólo la frase y las descripciones de `resolve_conflict` y `list_memories`.

**Con uso real, fijado antes de instalar** (en local, sobre el perfil real y con autorización): con la versión instalada una semana, `context.cut` con `block=conflicts` y `state=recent` en 0, y con `block=last_session` y `state=recent` en el 2 % o menos de los contextos con `context.retrieval_state{state=recent}`. Hoy son 134 de 250, los dos tipos de contexto juntos.

**`rag-eval`:** el ranking no cambia. Una corrida `--real --agent-view` con el golden de `docs/eval/` contra la última línea base confirma que `search.found` y `context.found` no se mueven («no distinguible» con menos de 6 casos en un sentido) y dice cuánto suma la marca en tokens.

## Los pares que ya esperan

No cambian: siguen `escalated`, sus filas siguen `disputed` y se siguen encontrando. Dejan de llegar enteros al arranque, se marcan donde se muestran y se listan cuando la persona lo pide. En este repositorio son más de 16: salen cuando una búsqueda los toca, o todos juntos cuando se piden (D6). Un binario anterior los vuelve a poner en el arranque.

## Riesgos

- **El resumen se puede seguir cortando** por la sección AUTO o por lo que escribió la persona; lo dice la medición con su umbral.
- **La marca puede leerse como orden de preguntar.** La frase y la NOTE ponen la condición; se mira en los transcripts después de instalar.
- **Los pares se acumulan sin verse**, porque el arranque no dice nada (D5): para eso está la lista a pedido.
- **La lista invita a juzgar.** Llega sólo a pedido, su línea final dice que la respuesta es de la persona, y `resolve_conflict` sigue pidiendo `user_answered` para una fila protegida.
- **Un bloque `full` instalado queda `stale`** hasta correr `clawg setup claude-code`; `lean`, el valor por defecto, no cambia.
- **La apertura en inglés no tiene lugar**: la frase nueva no puede pasar de 205 caracteres.

## Datos guardados

Ninguno en `memory.db`: no cambian el esquema, los estados ni ningún identificador, y ningún par se juzga ni se mueve. `metrics.db` gana una etiqueta en `context.cut`: las filas viejas no la tienen, y un reporte por `state` sólo cuenta las nuevas. Volver atrás es instalar la versión anterior.
