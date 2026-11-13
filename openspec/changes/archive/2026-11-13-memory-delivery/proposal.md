# memory-delivery — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| entrega y medición (1, 2) | M: esta propuesta cubre el paso 1, que es sólo del arnés; los pasos 2 y 3 se proponen con el resultado de la campaña | [agent-eval](../../agent-eval/proposal.md) (96), del que toma el arnés | archivado el 2026-11-13, sale en 0.86.0; el paso 1 está hecho y la campaña no dio ganador ([verify](verify.md)) |

## Problema

La [exploración](exploration.md) muestra que en el piloto pago la memoria nativa resolvió las tareas K 6 de 6 y clawg 0 de 6, porque la memoria de clawg nunca llegó al agente, mientras clawg pagaba unos 4.400 tokens fijos por sesión. Propone medir cuatro palancas con brazos de agent-eval antes de tocar el producto: sacar el bloque del protocolo (A), entregar las reglas al arrancar (C) o por archivo (D), y decidir qué memoria es una regla (E). El arnés de hoy no puede medir ninguna:

- La siembra no puede decir que una memoria la dijo la persona: cada fila sale con `created_by_type = 'agent'` (`crates/clawg-eval/src/bin/agent-eval/seed.rs:47`), y el arranque sólo toma las preferencias con `created_by_type = 'user'` (`crates/clawg-core/src/memory/workspace.rs:344`).
- Una tarea no puede marcar una memoria como regla ni darle globs (`crates/clawg-eval/src/bin/agent-eval/task.rs:46-62`).
- No hay brazo sin el bloque del protocolo: los brazos con `setup` escriben siempre su `CLAUDE.md` (`arm.rs:188-194`).
- Ningún brazo entrega reglas, ni al arrancar ni por archivo.
- En `a4` un trazador sale `never_reached` aunque el agente haya seguido `MEMORY.md`, que entra en el contexto de entrada sin pasar por el stream, y el puntaje sólo mira los resultados de las herramientas y los hooks (`score.rs:61-75`).
- El resultado no guarda los tokens del primer pedido ni qué archivos de instrucciones cargó el cliente; la tabla de tokens de la exploración salió de un script fuera del repositorio.
- Cada tarea siembra una o dos memorias de su propio tema, así que empujarlo todo no cuesta ruido, y un store real no es así.

## Alcance

El paso 1, en `crates/clawg-eval` (el binario `agent-eval` y sus datos):

- `SeedMemory` suma `user_stated`, que siembra la fila como dicha por la persona, y `rule`, que la marca como regla, sin globs (vale en toda sesión) o con ellos (vale cuando el agente trabaja en un archivo que coincide).
- `data/agent-tasks/noise.json`: reglas de otros temas que ningún check lee, sembradas en todo brazo con memoria, también en el `MEMORY.md` de `a4`.
- Los brazos `l0` (`a3` sin el bloque del protocolo), `l1` (`l0` más las reglas al arrancar) y `l2` (`l1` con las reglas que tienen globs en archivos de reglas por ruta en vez de al arrancar).
- Un hook `InstructionsLoaded` en todo brazo que registra cada archivo de instrucciones que carga el cliente.
- Lo que entra en el contexto sin pasar por el stream cuenta como traído: el `MEMORY.md` de `a4` y un archivo de reglas que el registro da por cargado.
- El resultado y el reporte llevan los tokens del primer pedido y las reglas cargadas por ruta.
- Las seis tareas del piloto marcan sus memorias como reglas, y la de K1 como dicha por la persona.

## Fuera de alcance

- El producto. La selección de E, la entrega de C o D y la dieta de A son el paso 2, y se proponen con el resultado de la campaña.
- B (`anthropic/alwaysLoad`), que se mide en una segunda tanda sobre el ganador porque cambia la lista de herramientas del servidor.
- El canal de reglas de Cursor.
- Correr la campaña. Es paga, y su guía se escribe aparte, fuera de git, como la del piloto.

## Decisiones

1. **`l2` mueve, no duplica.** Una regla con globs va al archivo de reglas y sale del texto del arranque; una sin globs queda al arrancar. Si `l2` repitiera lo de `l1`, siempre costaría más que `l1` y la regla de decisión, que elige el de menos tokens entre los que ganan, nunca podría elegirlo. Así `l2` mide si una regla que llega con un `Read`, un `Write` o un `Edit` rinde como una que llega al arrancar, que es la pregunta abierta de K1 y K3.
2. **El arranque simulado entra por un hook `SessionStart`.** Es el canal que usaría el producto (`additionalContext`, el mismo del hook de clawg). Lo corre el propio `agent-eval` (`agent-eval hook-context FILE`), que el plan ya conoce por ser el cliente de las corridas en seco, así que no depende de un shell ni de un programa del sistema. El hook de `session-start` de clawg sigue corriendo al lado, como en `a3`.
3. **Una regla es una línea con su antigüedad.** El título y el cuerpo de la memoria unidos con ` — `, precedidos por la antigüedad (`[30 days ago]`), bajo una cabecera que dice que una regla vieja puede estar desactualizada. La línea no lleva el id de la fila: la importación da sus propios ids (`crates/clawg-core/src/memory/portability/import.rs:1146`), y un id que no es el del store le haría leer al agente otra memoria. Con el texto entero en la línea el id no hace falta para medir; el producto lo pone en el paso 2, cuando escribe la línea con la fila de verdad.
4. **Las reglas van de la más nueva a la más vieja.** Las del ruido y las de la tarea se mezclan por edad, en el arranque, en los archivos de reglas, en el `MEMORY.md` de `a4` y en el orden del import, para que la regla de la tarea no quede siempre en el mismo lugar.
5. **El ruido es un archivo de los datos, no una opción.** Todo brazo con memoria lo siembra, así ningún brazo de una campaña puede quedarse sin él por un argumento olvidado. Sus edades pasan de 7 días, así el arranque de clawg, que muestra lo de los últimos 7 días (`RECENT_WINDOW_MS`, `crates/clawg-core/src/memory/prompt_builder.rs:55`), queda vacío como en el piloto. Ningún trazador de una tarea puede estar en el ruido: `agent-eval tasks` y cada corrida lo rechazan.
6. **`InstructionsLoaded` se registra en todo brazo.** Es medición, no memoria: el hook no agrega nada al contexto. El hook (`agent-eval hook-log FILE`) agrega su entrada a `instructions.jsonl`, en el directorio de la corrida, que el puntaje lee junto al stream.
7. **Lo entregado fuera del stream cuenta como traído.** En `a4`, el `MEMORY.md`, que el cliente pone en el contexto de entrada; en todo brazo, un archivo del plan que el registro da por cargado. Las reglas del arranque de `l1` ya cuentan como traídas por el stream, que lleva la salida de cada hook.

## Cómo se mide el éxito

- El paso 1 está hecho cuando pasan los tests de la [spec](spec.md), que fallan hoy, y `agent-eval tasks` da válidas las tareas y el ruido.
- La campaña aplica la regla de decisión que la exploración fijó antes de correr: un brazo gana si resuelve en K al menos lo que resuelve `a4`, resuelve S más veces que `a4`, no empeora C ni N frente a `a0` y su primer pedido no pasa del de `a3`; entre los que ganan, el de menos tokens, y los dos mejores repiten con cuatro corridas.

## Riesgos

- Que el cliente no lea `.claude/rules/` del directorio de trabajo en `-p`, o que no avise `InstructionsLoaded`. Lo muestra la primera sesión de `l2`: el registro sale vacío y las reglas por ruta en 0.
- Que el ruido cambie lo que hacen los brazos en las tareas del piloto. Es el punto: un store real tiene reglas de otros temas. Por eso el ruido evita lo que leen los checks (nombres de variables, imports, marcadores de listas, `.gitignore`, versiones en `requirements.txt` y la firma de las funciones de los fixtures).

## Datos guardados

Ninguno. El arnés escribe sólo en el directorio de cada corrida y en el home de la corrida; no hay migración ni vuelta atrás que dar.
