# session-start-context — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | S–M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md), verificado otra vez contra `develop` (`592a6e0`).

- **El arranque pide 8.000 bytes.** `session-start` pide el presupuesto por defecto (`SESSION_BUDGET = DEFAULT_BUDGET`, `crates/clawg-cli/src/hook.rs:34`; `DEFAULT_BUDGET`, `crates/clawg-core/src/memory/prompt_builder.rs:64`).
- **Las preferencias de la sección AUTO no tienen tope.** `render_auto_section` recorta skills y decisiones hasta `AUTO_BYTE_CAP` (6.000 bytes), pero nunca una preferencia (`crates/clawg-core/src/memory/workspace.rs:496-522`, el tope en `:19-20`). Además, las preferencias se filtran por agente y no por proyecto (`:360-369`), así que el mismo bloque entra en cada repositorio. En un store real con los hooks sin instalar, el workspace de un agente mide 14.560 bytes, y 14.400 son 18 preferencias.
- **El workspace no se corta nunca.** `fit_to_budget` corta los hits, después LAST SESSION y después los conflictos, y si el texto sigue pasado entrega el workspace entero (`prompt_builder.rs:458-462` y `:498-501`). Con un workspace de 14 KB, el contexto del arranque sale sin el resumen de la última sesión: `specs/retrieval`, en «Un workspace más grande que el presupuesto se entrega y se dice», manda eso. Se midió el 2026-11-03: un `recall_context` de 12.000 bytes devolvió sólo el workspace.
- **El aviso de `synthesize` cuenta lo importado.** `unreviewed_count` (`crates/clawg-core/src/memory/synthesis.rs:282-298`) cuenta con `PENDING` (`:34-38`), que no mira `source`. En un proyecto con 1.839 filas propias da 13.552 pendientes. Desde 20 (`SYNTHESIS_NUDGE_THRESHOLD`, `crates/clawg-cli/src/mcp/tools.rs:717`), `recall_context` y el hook lo dicen en cada contexto (`tools.rs:984`; `hook.rs:135`). Un import trae historia que ya se escribió en otra herramienta; no son memorias nuevas para sacar conclusiones.

## Objetivo que sirve

1, porque la memoria que se recupera al empezar tiene que ser la que sirve para seguir trabajando, y 4, porque las preferencias de un agente entran en todos los proyectos que comparten el store. Antes de instalar los hooks en todos los proyectos, el arranque tiene que traer el traspaso de la sesión anterior. Si no, inyectar el contexto empeora cada sesión ([ROADMAP](../../../ROADMAP.md), «Cómo se ordena», punto 1).

## Decisiones

Las decisiones están cerradas en la exploración ([«Decisiones»](exploration.md#decisiones-cerradas)). Esta propuesta no las reabre; dice cómo se aplican.

1. **Lo que escribió la persona no se corta.** Es el texto antes de `AUTO-START` y la sección USER con lo que la sigue. Lo único que se corta es la sección AUTO, de a una entrada entera: una entrada nunca se acorta. Un encabezado que se queda sin entradas sale del prompt, como hoy sale uno vacío (`without_empty_headings`, `prompt_builder.rs:1134`).
2. **La mitad del presupuesto.** Mientras el texto no entre y el prompt tenga un resumen de LAST SESSION o algún hit en RECENT o LONG-TERM, la sección AUTO cede primero lo que pase de `budget / 2`. Es una prioridad de recorte, como la reserva de los hits (`HIT_ROOM_DIVISOR`, `prompt_builder.rs:70`), y no un tope fijo. Un texto que entra sigue siendo byte a byte el de hoy, y el recorte se detiene en cuanto el texto entra. Sin resumen ni hits, la sección AUTO puede usar todo el presupuesto: se corta al final, sólo hasta que el texto entre.
3. **El orden de corte dentro de AUTO.** Primero sale la entrada de menor importancia; a igual importancia, la más vieja (`created_at`), y a igual fecha, la de menor id. Las entradas compiten entre sí sin importar su sección: preferencias, decisiones y skills. Cada entrada cortada se nombra por id en «Quedaron afuera» del bloque `BUDGET`, antes que los hits, porque el workspace se imprime antes. Un skill lleva su kind (`procedural #id`), como un hit de otra tabla. `read_memory` las devuelve enteras.
4. **El resto del orden es el de hoy.** Una vez que AUTO cedió lo que pasaba de la mitad, el orden sigue como hasta ahora: los hits que pasan de su reserva, LAST SESSION, los conflictos y los hits de la reserva. Después, la sección AUTO (punto 2). Si no queda ninguna entrada de AUTO para cortar y el texto sigue pasado, se entrega como hoy y el bloque `BUDGET` lo dice. La frase ahora nombra la parte que se entrega entera, que ya no es todo el workspace: `The part of the workspace the user wrote is over the budget on its own and is shown whole.`, o `La parte del workspace que escribió la persona ya pasa el presupuesto sola y se muestra entera.` con `lang: es`.
5. **Las dos rutas.** `build_session_prompt` y `build_memory_prompt` arman el texto con `assemble` (`prompt_builder.rs:348`), así que el cambio vale para las dos con un solo código.
6. **El aviso no cuenta lo importado.** `unreviewed_count` deja afuera las filas con `source = 'import'`. `synthesize` (`synthesis_core`) sigue listándolas y contándolas en `pending_total`, y `conclusions_to_review` no cambia: lo que cambia es el aviso, no lo que se puede revisar.

**Cómo se nombra una entrada de AUTO.** `workspace.md` no guarda ids, pero la regeneración que escribe el archivo ya informa qué filas imprimió (`EmittedIds`, `workspace.rs:305-329`), y el daemon la corre antes de cada contexto (`crates/clawg-cli/src/daemon/api.rs:1578-1586`). Ahora informa además, de cada fila, la línea que escribió, su importancia y su `created_at`. Para eso las consultas de `fetch_auto_rows` leen dos columnas más, sin cambiar qué filas eligen. El prompt sigue leyendo el archivo: separa su sección AUTO en entradas y reconoce cada una por su texto exacto. Una entrada es una línea que empieza con `- `, junto con sus líneas de continuación detrás del margen. Una entrada que la regeneración no informó no se corta, porque no se la puede nombrar: es el caso de un archivo que no se regeneró en esa llamada, o de una regeneración que falló. Esa entrada queda como hoy.

**La métrica.** `context.cut` suma el bloque `workspace` (`PromptBlock::Workspace`). Si no lo sumara, un contexto que cortó sólo filas de AUTO no se distinguiría en las métricas de uno que no cortó nada, y `MemoryPrompt.cut` no diría lo que cortó.

## Alcance

1. `crates/clawg-core/src/memory/prompt_builder.rs`:
   - el workspace del prompt pasa a una estructura con la parte de la persona y las entradas de AUTO, que `workspace_for_prompt` arma a partir del archivo y del informe de la regeneración;
   - `fit_to_budget` con los dos pasos nuevos y la frase nueva;
   - `PromptBlock::Workspace`.
2. `crates/clawg-core/src/memory/workspace.rs`: `EmittedIds` informa, de cada fila que imprimió, su línea, su importancia y su `created_at`.
3. `crates/clawg-core/src/memory/synthesis.rs`: el conteo de `unreviewed_count`.
4. Docs: `docs/reference/daemon.md`, que hoy dice «never cutting the workspace» y no dice que el aviso deja afuera lo importado, y `docs/reference/data-profiles.md`, que lista los bloques de `context.cut`. `specs/retrieval`, `specs/synthesis` y `specs/metrics` reciben los requisitos de la [spec](spec.md) al archivar.
5. Lo que el CI del pull request marque en Windows.

## Fuera de alcance

- Qué filas son elegibles para el workspace: el umbral de importancia, el tier, el alcance por proyecto de las preferencias, `AUTO_BYTE_CAP` y el tope de skills. Lo decide [preference-model](../2026-11-06-preference-model/exploration.md).
- `workspace.md` en disco: su sección AUTO sigue siendo la que escribe la regeneración. Sólo cambia lo que el prompt muestra de ella.
- `crates/clawg-cli/src/hook.rs`, que toca client-setup-cursor en paralelo. Este cambio no lo necesita: el hook ya pide el contexto al daemon y repite el aviso que el daemon cuenta.
- El contrato del agente (`agent_contract.json`), el umbral de 20 del aviso, `conclusions_to_review`, `DEFAULT_BUDGET`, la reserva de los hits y el ranking.
- Subir la versión y archivar.

## Cómo se mide el éxito

Primero, tests que fallan hoy. Corren sin modelo, porque el contexto sin consulta no embebe (detalle en la [spec](spec.md)):

- Un workspace de 14 KB y un resumen de sesión, con un presupuesto de 8 KB: el resumen está entero, y las preferencias cortadas se nombran por id. Se prueba por las dos rutas. Hoy el resumen se corta y el workspace se entrega entero.
- La parte de la persona más grande que el presupuesto se entrega entera y se dice, como hoy. Las entradas de AUTO se cortan y se nombran.
- Sin resumen ni hits, las preferencias usan todo el presupuesto, no la mitad.
- Con 25 filas importadas y 3 propias, `unreviewed_count` da 3 y `synthesize` sigue listando las 28. Hoy da 28.

Lo que no puede cambiar:

- Un texto que entra en el presupuesto es byte a byte el de hoy, con el informe de la regeneración o sin él.
- La suite pasa, y `cargo clippy --workspace --all-targets -- -D warnings` sale con 0 en Linux y, por el CI del pull request, en Windows.

`rag-eval --diff` no aplica: el ranking no cambia, y el golden no tiene workspace.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni lo que se guarda, y `workspace.md` queda como lo escribe la regeneración. Sólo cambia qué parte del contexto se corta cuando no entra y qué cuenta el aviso. Para volver atrás alcanza con el binario anterior.
