# session-start-context — Especificación

Cambia [`specs/retrieval`](../../../specs/retrieval/spec.md): «El prompt cumple el presupuesto», «Lo cortado se nombra por id», «Un workspace más grande que el presupuesto se entrega y se dice» y la definición de «Fila del workspace». Cambia también R8 de [`specs/synthesis`](../../../specs/synthesis/spec.md) y el recuento `context.cut` de [`specs/metrics`](../../../specs/metrics/spec.md). Al archivar, estos requisitos pasan a esas specs.

## Definiciones

- **Parte de la persona:** lo que el prompt muestra del texto de `workspace.md` antes de `AUTO-START` y de la sección USER con lo que la sigue, sin marcadores. Un archivo sin `AUTO-START` es todo parte de la persona.
- **Entrada de AUTO:** una línea de la sección AUTO que empieza con `- `, con las líneas que la siguen detrás del margen (`  |`). Así escribe la regeneración una preferencia, una decisión o un skill.
- **Entrada conocida:** una entrada de AUTO cuyo texto es exactamente la línea que la regeneración del mismo pedido informó de una fila, junto con el kind, el id, la importancia y el `created_at` de esa fila. Si dos filas tienen la misma línea, las entradas se asignan en orden.
- **Algo esperando:** el prompt todavía tiene un resumen de LAST SESSION, o algún hit en RECENT o LONG-TERM.
- **Bytes de AUTO:** los bytes que la sección AUTO ocupa en el texto del prompt, con sus encabezados y sus entradas.

## SC1. La regeneración informa cada línea que escribió

`regenerate_auto_section` MUST informar, de cada fila que imprimió en la sección AUTO, su kind, su id, su importancia, su `created_at` y la línea tal como quedó en el archivo, sin el salto final. MUST NOT cambiar qué filas imprime ni el texto del archivo.

#### Escenario: una preferencia de dos líneas y un skill
- GIVEN una preferencia de importancia 9 cuyo texto tiene dos líneas, y un skill
- WHEN se regenera el workspace
- THEN el informe trae las dos filas con su importancia y su fecha, y cada línea informada está en la sección AUTO del archivo

**Test:** `the_regeneration_reports_each_line_it_wrote` (`crates/clawg-core/src/memory/workspace.rs`). Hoy no compila: el informe no tiene líneas.

## SC2. La sección AUTO cede lo que pasa de la mitad

Mientras el texto no entre en el presupuesto, haya algo esperando y los bytes de AUTO pasen de `budget / 2`, `fit_to_budget` MUST quitar una entrada conocida antes que cualquier otra cosa. Después MUST seguir el orden de hoy: los hits que pasan de su reserva, el resumen de LAST SESSION, los pares de conflictos y los hits de la reserva. Si el texto entra en cualquier punto, el recorte MUST detenerse. Un texto que entra entero MUST quedar byte a byte igual, se pase o no el informe de la regeneración. Vale para `build_session_prompt` y para `build_memory_prompt`.

#### Escenario: un workspace de 14 KB al iniciar la sesión
- GIVEN 18 preferencias de 800 bytes en el workspace de un agente y un resumen de la sesión anterior en el proyecto
- WHEN se pide el contexto con el presupuesto de 8.000 bytes, sin consulta, y después con una consulta y sin modelo
- THEN cada prompt ocupa como mucho 8.000 bytes, trae el resumen entero bajo LAST SESSION y nombra por id en `BUDGET` las preferencias cortadas

#### Escenario: el mismo workspace con lugar
- GIVEN el mismo store
- WHEN se pide el contexto con 24.000 bytes
- THEN el prompt es el mismo que se arma sin el informe de la regeneración

#### Escenario: AUTO por encima de la mitad y hits
- GIVEN entradas de AUTO por 5.000 bytes y dos hits de 1.000, con un presupuesto de 6.000
- WHEN el texto no entra
- THEN se cortan entradas de AUTO, no los hits

**Tests:** `a_stocked_workspace_leaves_room_for_the_last_session` (con el store, las dos rutas y el texto con lugar) y `the_auto_rows_over_half_go_before_the_hits` (`prompt_builder.rs`). Hoy el primero falla porque el resumen se corta y el workspace se entrega entero, y el segundo, porque se cortan los hits.

## SC3. Sin nada esperando, AUTO usa todo el presupuesto

Cuando ya no queda nada esperando, ni resumen ni hits, y no quedan pares de conflictos, `fit_to_budget` MUST quitar entradas conocidas sólo mientras el texto no entre.

#### Escenario: sólo preferencias
- GIVEN 18 entradas de AUTO de 800 bytes, sin resumen ni hits
- WHEN el presupuesto es de 8.000 bytes
- THEN el prompt ocupa como mucho 8.000 bytes, y sus bytes de AUTO pasan de 4.000

**Test:** `with_nothing_waiting_the_auto_rows_take_the_whole_budget` (`prompt_builder.rs`). Hoy falla: no se corta ninguna entrada y el prompt pasa el presupuesto.

## SC4. El orden de AUTO y cómo se nombra lo cortado

Entre las entradas conocidas, MUST salir primero la de menor importancia; a igual importancia, la de `created_at` más viejo, y a igual fecha, la de menor id. Las preferencias, las decisiones y los skills compiten entre sí. Una entrada MUST quitarse entera, sin acortarse. Una entrada que no es conocida MUST NOT quitarse. Un encabezado que se queda sin entradas MUST salir del prompt, como hoy sale uno vacío.

Cada entrada quitada MUST nombrarse en «Left out» / «Quedaron afuera» del bloque `BUDGET`, en el orden en que se imprimiría, antes que los hits. Un skill MUST llevar su kind (`procedural #id`). `MemoryPrompt.cut` MUST incluir `PromptBlock::Workspace`, cuyo nombre es `workspace`, y el daemon MUST registrarlo en `context.cut` como los otros bloques.

#### Escenario: importancia y edad
- GIVEN cuatro entradas de AUTO: #1 de importancia 9 y #2 de importancia 8, de hace 10 días, y #3 de importancia 8 y #4 de importancia 10, de hace 1 día
- WHEN el presupuesto deja lugar para dos
- THEN quedan #4 y #1, y el bloque `BUDGET` dice `Left out: #2, #3.`

**Tests:** `the_auto_rows_go_by_importance_then_newest` y `the_fit_names_each_block_it_cut`, que suma el bloque `workspace` (`prompt_builder.rs`). Hoy fallan: no se corta ninguna entrada, y `PromptBlock::Workspace` no existe.

## SC5. Lo que escribió la persona no se corta

La parte de la persona MUST NOT cortarse. Si al no quedar ninguna entrada conocida, ningún hit, resumen ni par el texto sigue por encima del presupuesto, el prompt MUST llevar la parte de la persona entera. El bloque `BUDGET` MUST nombrar las entradas cortadas, cuántos bytes ocupa el contexto entero y la frase `The part of the workspace the user wrote is over the budget on its own and is shown whole.`, o `La parte del workspace que escribió la persona ya pasa el presupuesto sola y se muestra entera.` con `lang: es`. Es el único caso en que el prompt pasa el presupuesto.

#### Escenario: una sección USER de 9.000 bytes
- GIVEN una sección USER de 9.000 bytes, tres entradas de AUTO y un hit, con el presupuesto de 8.000
- THEN el prompt trae la sección USER entera, ninguna entrada ni hit, y el bloque `BUDGET` nombra las tres entradas y el hit y dice la frase

**Tests:** `what_the_user_wrote_is_never_cut` (nuevo), `a_workspace_over_the_budget_is_kept_and_said` (un archivo sin marcadores, con la frase nueva) y `the_notes_of_the_prompt_follow_its_language` (`prompt_builder.rs`). Hoy el primero falla porque las entradas de AUTO no se cortan.

## SC6. El aviso de `synthesize` no cuenta lo importado

`unreviewed_count` MUST contar sólo las memorias pendientes con `source` distinto de `'import'`, con las mismas reglas de alcance, de lector y de `reviewed_at` que hoy. `synthesis_core` MUST seguir listando y contando en `pending_total` todas las pendientes, las importadas incluidas. `conclusions_to_review_count` no cambia.

#### Escenario: un proyecto con un import
- GIVEN 25 memorias importadas y 3 propias, todas pendientes en el proyecto P
- WHEN se cuenta el aviso de P
- THEN da 3
- WHEN se sintetiza P
- THEN `pending_total` es 28 y la lista trae las 28

**Test:** `the_reminder_leaves_out_what_an_import_wrote` (`crates/clawg-core/src/memory/synthesis.rs`). Hoy falla: el aviso da 28.
