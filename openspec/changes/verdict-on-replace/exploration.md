# verdict-on-replace — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar barato: lo que clawg le cuesta a cada sesión que guarda) | S (textos del puente y del protocolo; sin esquema ni rutas nuevas) | — ; se coordina con [topic-key-warning](../topic-key-warning/proposal.md) (68), que agrega otra línea a la misma respuesta, y con [memory-delivery](../memory-delivery/proposal.md) (127), que mide brazos sin el bloque del protocolo | exploración |

**Tesis.** Después de cada save con memorias cercanas, el agente gasta un pedido entero al modelo en juzgar cada par, y casi siempre contesta `compatible`, un veredicto que no cambia nada de lo que se recupera. La idea de partida era pedir el juicio sólo cuando el par parece un reemplazo. Los datos dicen que el detector no puede saberlo: ni la distancia, ni el tipo, ni la posición en la lista separan los reemplazos del resto. Quien sí puede es el agente, que ya lee la lista. Lo que ahorra es dejar de pedirle el veredicto `compatible` y pedirle la llamada sólo cuando el save reemplaza una memoria.

## El problema verificado contra el código

1. **La respuesta del save pide un veredicto para cada cercana.** `related_text` arma la línea `close to earlier memories: #id "<línea>"; … If this save replaces one, call resolve_conflict with verdict supersedes (keep = the new id); if both hold, compatible or scoped` (`crates/clawg-cli/src/mcp/server.rs:391-411`). La respuesta de `restore_memory` pide lo mismo para las memorias guardadas mientras la restaurada estuvo borrada (`crates/clawg-cli/src/mcp/tools.rs:497-505`).
2. **El protocolo lo repite como una orden.** «When `save_memory` names earlier memories close to the new one, judge each with `resolve_conflict` while you still have the conversation» (`crates/clawg-cli/src/mcp/protocol.md:24-27`, y `protocol.es.md:24-27`). Ese texto es la apertura que el servidor MCP declara y lo que `clawg setup` escribe en el archivo de instrucciones del usuario (`crates/clawg-cli/src/mcp/protocol.rs:5-8`).
3. **El detector nombra hasta cinco candidatas por pieza.** Toda fila viva a menos de `CONTRADICTION_THRESHOLD` = 0,42 es candidata (`crates/clawg-core/src/memory/contradiction.rs:37`), con `CANDIDATE_POOL` = 5 por sondeo (`contradiction.rs:58`). El agente ve de cada una los primeros `EXCERPT_CHARS` = 80 caracteres de su primera línea (`contradiction.rs:73`).
4. **`compatible` y `scoped` no cambian lo que se recupera.** `judge_pair` deja las dos filas en su estado de reposo y anota el par como `judged` (`crates/clawg-core/src/memory/dispute.rs:452-468`); la spec lo dice con sus palabras: «el ranking de las dos no cambia» ([`specs/disputes`](../../specs/disputes/spec.md), R2). Sobre un par pendiente no hay nada que cerrar, porque las dos filas ya estaban `active`. Sobre un par escalado sí: ahí `compatible` es la respuesta de la persona y saca el par de ACTIVE CONFLICTS (`crates/clawg-core/src/memory/prompt_builder.rs:846-859`), y eso no está en discusión.
5. **Un par sin juzgar tampoco.** Un par `pending` deja las dos filas `active` ([`specs/disputes`](../../specs/disputes/spec.md), «El detector anota candidatas»). El único lector de ese estado es la cuenta de `memory_status` (`open_pairs`, `dispute.rs:529-562`), que lista los `OPEN_PAIRS_SHOWN` = 5 más viejos (`crates/clawg-cli/src/daemon/api.rs:1141`) y pide juzgarlos (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:292-335`).
6. **Un reemplazo necesita que el par exista.** `check_pair` rechaza con `NotInDispute` dos filas que ningún par ni `disputed_against` enlaza (`dispute.rs:152-161`). El par hay que seguir escribiéndolo aunque nadie lo juzgue.
7. **Los pares que van a la persona siguen otro camino.** `needs_user` decide cuáles escalan (`contradiction.rs:107-116`); esos salen en ACTIVE CONFLICTS y no dependen del veredicto del agente.

## Qué se midió

Dos fuentes, las dos de sólo lectura. Los textos quedan fuera del repositorio; acá van los números.

### Los transcripts de Claude Code de este repositorio

298 transcripts, cadena principal. 891 saves confirmados; 532 nombraron cercanas, 2.071 pares en total.

- **El costo es por pedido, no por par.** 382 pedidos al modelo sólo llaman a `resolve_conflict` (1.572 llamadas, 4,1 por pedido) y leen 93 millones de tokens de contexto, el 2,7 % de todo lo que leyeron esas sesiones. Otros 67 pedidos juzgan junto con otro trabajo. Una regla ahorra sólo si deja un save sin ningún par para juzgar: bajar de cinco pares a uno no ahorra el pedido.
- **Casi todo es `compatible`.** De 1.623 veredictos aceptados: 1.539 `compatible`, 39 `scoped`, 19 `supersedes`, 3 `conflicts` y 23 de una forma anterior de la herramienta. 191 llamadas más fueron rechazadas.
- **El veredicto se da sin leer.** En el 88 % de los `compatible` el agente no había leído la memoria vieja con `read_memory`: juzgó con los 80 caracteres del extracto. Al leer a mano los pares `compatible` más parecidos a un reemplazo apareció al menos uno que lo era (una nota «pendiente» seguida de otra que la da por hecha). Los reemplazos contados son un piso.
- **Con un store grande casi todo save trae cercanas.** Hasta el 2026-10-22, el 37 % de los saves nombraba cercanas (1,2 por save). Desde el import del historial, el 78–80 % (3,1 a 3,4 por save).

### La tabla `memory_conflict` del store

3.571 pares, de todos los proyectos: 2.496 `compatible`, 83 `scoped`, 33 `supersedes`, 920 `pending` que nadie juzgó y 39 `escalated`. 2.833 tienen la distancia registrada.

| Veredicto | n | p10 | p25 | p50 | p75 | p90 |
|---|---|---|---|---|---|---|
| `supersedes` | 24 | 0,275 | 0,318 | 0,370 | 0,390 | 0,400 |
| `scoped` | 67 | 0,319 | 0,344 | 0,378 | 0,401 | 0,413 |
| `compatible` | 1.854 | 0,345 | 0,367 | 0,389 | 0,405 | 0,414 |
| `pending` | 863 | 0,363 | 0,385 | 0,403 | 0,413 | 0,417 |

Los reemplazos van de 0,160 a 0,420, hasta el borde mismo del umbral.

Qué quedaría si el detector filtrara, sobre los 752 saves que dejaron pares para el agente (2.808 pares, 24 reemplazos con distancia):

| Pedir juicio sólo si… | Saves que siguen juzgando | Reemplazos conservados |
|---|---|---|
| hoy | 100 % | 24 |
| las dos memorias son del mismo tipo | 90 % | 23 |
| distancia < 0,40 | 80 % | 21 |
| mismo tipo, y distancia < 0,38 o la vieja tiene menos de 24 h | 75 % | 20 |
| mismo tipo y la más cercana del save | 62 % | 16 |
| distancia < 0,38 | 57 % | 16 |
| mismo tipo y distancia < 0,38 | 45 % | 16 |
| distancia < 0,34 | 20 % | 9 |

También se probaron el solapamiento de palabras entre los dos textos, los identificadores en común y las horas entre los dos saves, solos y combinados. Ninguna regla ahorra la mitad de los saves sin perder un tercio de los reemplazos.

### Qué son los reemplazos

- La mayoría es un estado que se cerró: la vieja dice «abierto», «pendiente» o «sin verificar» y la nueva dice «arreglado» o «verificado». El resto es el mismo hecho ampliado, o una decisión que la persona cambió.
- 23 de 24 son entre memorias del mismo tipo.
- La mitad se guardó a menos de 2 h de la vieja; entre los `compatible`, el 24 %.
- En el 46 % la vieja tenía una `topic_key` y el save nuevo no se guardó bajo ella: llegó sin clave o con otra parecida.

Esto coincide con lo que `conflict-eval` midió sobre pares etiquetados: las clases `update` (15 de 15), `contradiction` (30 de 30) y `rephrase` (14 de 15) caen todas en la banda del detector (`docs/eval/results/2026-10-18-real-conflict-eval-final.json`). La distancia dice que dos textos hablan de lo mismo, no cuál de las clases son.

## Enfoques posibles

### A. Filtrar en el detector (Descartado)

Bajar el umbral, nombrar sólo la más cercana o sólo las del mismo tipo. La tabla de arriba lo descarta: lo que ahorra pierde reemplazos, y lo que no pierde no ahorra.

### B. Pedir la llamada sólo cuando el save reemplaza una (Recomendado; S)

La respuesta del save sigue nombrando las cercanas, pero pide `resolve_conflict` sólo en dos casos: `supersedes` si este save reemplaza una, y `conflicts` si sólo la persona puede decir cuál vale. Si valen las dos, no pide nada. El protocolo dice lo mismo. La herramienta no cambia: `compatible` y `scoped` se siguen aceptando, para un cliente con el protocolo anterior y para cerrar un par a propósito.

- A favor: no se pierde ningún reemplazo por un filtro, porque el agente ve la misma lista. Desaparece el pedido que sólo juzga en casi todos los saves. Sin esquema, sin rutas nuevas, y un puente anterior sigue funcionando.
- En contra: los pares sin juzgar se acumulan como `pending` (ya hay 920). Y no se sabe si, sin el ritual de juzgar cada una, el agente detecta menos reemplazos; hoy ya juzga con 80 caracteres.

### C. Reemplazar sólo por `topic_key` y no nombrar cercanas (Descartado)

En el 54 % de los reemplazos la vieja no tenía clave, y la mayoría son estados que se cerraron en notas que nadie pensó como revisables. Sin la lista, esos quedarían vivos los dos.

## Decisiones abiertas

1. **Qué dice `memory_status` de los pares sin juzgar.** Hoy los cuenta como «para que juzgues», lista los cinco más viejos y pide leer los dos lados y juzgar. Con B, esa cuenta crece con cada save y una llamada a `memory_status` volvería a pedir el trabajo que se quiso sacar.
   - Recomendado: `memory_status` deja de listar los pendientes y de pedir que se juzguen; sigue diciendo cuántos pares esperan a la persona. El par se sigue escribiendo, con su distancia (punto 6 del problema, y sirve para volver a medir).
   - Alternativa: dejar de escribir los pares pendientes. Descartada: `resolve_conflict` los necesita y se pierde la distancia.
2. **Si la respuesta del save sigue ofreciendo `scoped`.** Guarda un motivo, pero no cambia lo que se recupera. Recomendado: la línea nombra sólo `supersedes` y `conflicts`.
3. **El aviso de la clave parecida.** Cuando la cercana tiene una `topic_key` y el save nuevo llega con otra, avisar que se guarde bajo la que ya existe. Recomendado: fuera de este cambio; es de la familia de [topic-key-warning](../topic-key-warning/proposal.md), que todavía espera casos reales para verificarse.
4. **Cómo se mide que el agente no detecta menos reemplazos.** Recomendado: repetir la pasada sobre los transcripts con algunas semanas de uso de la versión nueva y comparar los pedidos que sólo juzgan por save con cercanas (hoy 0,79) y los `supersedes` por cada cien saves con cercanas (hoy 3,3). Con tan pocos reemplazos el segundo número se informa, no decide. Una sesión paga de `agent-eval` no hace falta.

## Esfuerzo e impacto

| Enfoque | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Descartado | Bajo o con pérdida | Medio (pierde reemplazos) | Bajo |
| B | Recomendado | Medio: saca un pedido al modelo de cuatro de cada cinco saves con cercanas, cerca del 2,7 % del contexto leído | Bajo (texto; reversible con otra versión) | Bajo |
| C | Descartado | Alto en costo | Alto (deja vivas notas viejas) | Bajo |

Va en la nube: son textos del puente y sus tests. La medición con uso real queda para local.

## Fuentes

- Código de `develop` en `62874c49`, con las rutas citadas.
- Los transcripts de Claude Code de este repositorio, 298 archivos, leídos el 2026-11-11.
- La tabla `memory_conflict` de un store en uso, abierta en sólo lectura y sin tomar el lock, el 2026-11-11.
- `docs/eval/results/2026-10-18-real-conflict-eval-final.json`.
