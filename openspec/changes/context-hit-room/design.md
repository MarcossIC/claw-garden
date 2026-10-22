# context-hit-room — Diseño

## D1. Una reserva para los hits dentro del recorte de 23

`fit_to_budget` sigue siendo un bucle que renderiza el prompt entero y quita una pieza por vuelta (D1 de [context-budget](../archive/2026-10-17-context-budget/design.md)). Lo único nuevo es qué pieza toca:

1. mientras los bloques RECENT y LONG-TERM ocupen más bytes que la reserva, un hit de ellos, con `next_to_cut`;
2. si no, el resumen de LAST SESSION;
3. si no, el último par de conflictos;
4. si no, un hit de los que quedan, con `next_to_cut`;
5. si no queda nada, el workspace va entero y se dice.

Lo que ocupan los hits se cuenta como en el render: el separador de cada bloque, cada línea con su margen y la línea en blanco que lo abre (`hit_bytes`). El hit que queda solo en su bloque se acorta en lugar de saltarse, como en 23; en el paso 1 se acorta sólo en lo que pasa de la reserva, para que un hit largo y relevante quede en ella en vez de irse entero. Con una reserva de 0 el paso 1 corta todos los hits antes que nada, que es el orden de 23 byte a byte.

| | A favor | En contra |
|---|---|---|
| **Reserva en bytes, cortada al final (elegida)** | Pura; los mejores hits sobreviven a cualquier tamaño de los demás bloques; LAST SESSION sigue por encima del resto de los hits | Si el workspace solo llena el presupuesto, la reserva no cabe y los hits se van igual |
| Un número fijo de hits | Simple | Un hit va de una línea a 480 tokens: tres hits largos pasan el presupuesto y tres cortos desperdician lugar |
| Cortar LAST SESSION antes que los hits | Una línea | Pierde el traspaso de sesión en cada contexto de un store maduro, y no alcanza: los conflictos se cortan después de LAST SESSION |
| Subir el default | Nada cambia | El caso real ya ocupaba 19.134 bytes y un store sigue creciendo; el hook comparte el default y su salida se corta a 10.000 caracteres |

## D2. Un cuarto del presupuesto

`build_memory_prompt` pasa `budget / 4`. Con el default son 2.000 bytes: unos cinco hits del golden (la media de `recall_context` es de 1.037 tokens en diez hits) o un hit de 480 tokens. Quedan 6.000 bytes para el workspace, LAST SESSION y los conflictos, lo mismo que el tope del AUTO (`AUTO_BYTE_CAP`, `crates/clawg-core/src/memory/workspace.rs:19-20`): un workspace en su tope ya no deja afuera a los hits. Con `MAX_BUDGET` (24.000) la reserva es de 6.000 bytes, lo que pide quien quiere más memorias.

La proporción, y no una cifra fija, sigue al presupuesto que pide el llamador: con 400 bytes la reserva es de 100, y el hit más relevante todavía se acorta antes que irse.

## D3. Sin consulta no hay reserva

`build_session_prompt` pasa 0. El contexto sin consulta es el que abre una sesión: LAST SESSION es el traspaso que [session-handoff](../archive/2026-10-19-session-handoff/proposal.md) puso para eso, y RECENT son los saves más nuevos, todos con score 0, no la respuesta a una pregunta. Reservarles lugar cortaría el resumen para mostrar un save por ser nuevo.

## D4. El workspace sigue entero

Cortar el workspace dejaría sin canal a las filas que imprimió, porque la búsqueda las saltea (`EmittedIds`, D2 de 23). Ponerle un tope pide decidir el corte antes de buscar y devolverles el canal a las filas cortadas, y tocar las preferencias, que son «always active». Si una medición real muestra que el workspace solo llena el presupuesto, es un cambio propio.
