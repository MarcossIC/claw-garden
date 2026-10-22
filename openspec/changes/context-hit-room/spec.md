# context-hit-room — Especificación

Cambia [`specs/retrieval`](../../specs/retrieval/spec.md): «El prompt cumple el presupuesto» y «El primer hit de un bloque se recorta en lugar de saltarse».

## R1. Un contexto con consulta reserva lugar para sus hits

`build_memory_prompt` MUST pasarle a `fit_to_budget` una reserva para los hits de un cuarto del presupuesto (`budget / 4`). Mientras el texto no entre, `fit_to_budget` MUST quitar, midiendo otra vez el texto entero después de cada paso:

1. mientras los bloques RECENT y LONG-TERM ocupen más bytes que la reserva (su separador, sus líneas con el margen y la línea en blanco que los abre), un hit de ellos, con la regla de 23 (el de menor score, un exacto sólo cuando no queda otro, el empate desde el final de LONG-TERM);
2. después, el resumen de LAST SESSION;
3. después, los pares de conflictos desde el último;
4. después, los hits que quedan, con la misma regla.

El workspace MUST NOT cortarse, como hoy. Un texto que entra entero MUST quedar byte a byte igual.

#### Escenario: un store maduro
- GIVEN un workspace de 5.000 bytes, un resumen de sesión de 3.000, un par en disputa con los dos lados de 1.000 y diez hits largos, el de la consulta entre ellos
- WHEN se pide el contexto de esa consulta con el presupuesto default
- THEN el prompt ocupa como mucho 8.000 bytes y trae el hit de la consulta bajo RECENT o LONG-TERM

#### Escenario: lo que pasa de la reserva se va primero
- GIVEN cuatro hits de 1.000 bytes, un resumen de LAST SESSION y un par, y un presupuesto cuya reserva cabe dos hits
- WHEN el texto no entra y quitar dos hits alcanza
- THEN se van los dos de menor score, y LAST SESSION y el par quedan enteros

#### Escenario: lo que está dentro de la reserva se va al final
- GIVEN los mismos bloques y un presupuesto que no alcanza para todo
- THEN LAST SESSION y el par se van antes que los hits que caben en la reserva

**Tests:** `a_mature_store_still_shows_the_hit_of_the_query` (con el store), `the_hits_over_their_room_go_before_the_last_session`, `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts` y `a_room_never_takes_the_prompt_over_the_budget` (`prompt_builder.rs`). Los tests de 23 corren con reserva 0 y siguen pasando sin cambios.

## R2. Un hit solo que pasa de la reserva se acorta hasta ella

En el paso 1, un hit que queda solo en su bloque MUST acortarse sólo en lo que los hits pasan de la reserva (o en lo que el texto pasa del presupuesto, si es menos), con la marca ` [truncated]` de 23, y MUST NOT quitarse en ese paso. Lo que siga sobrando lo cortan los pasos 2 a 4.

#### Escenario: un hit largo y relevante
- GIVEN un hit de 5.000 bytes solo en RECENT, un resumen de LAST SESSION de 1.000 y una reserva de 1.000
- WHEN el presupuesto es de 2.000
- THEN el hit queda acortado hasta la reserva y después se acorta el resumen
- AND con un presupuesto de 4.000 el resumen queda entero y el hit se acorta sólo en lo que sobra

**Test:** `a_lone_hit_over_its_room_is_shortened_to_it` (`prompt_builder.rs`).

## R3. El contexto sin consulta no reserva nada

`build_session_prompt` MUST pasar una reserva de 0, así que su orden de recorte MUST seguir siendo el de 23: todos los hits de RECENT, después LAST SESSION, después los conflictos.

#### Escenario: inicio de sesión en un store maduro
- GIVEN un workspace, un resumen de sesión y saves recientes que no entran juntos en el presupuesto
- WHEN se pide el contexto sin consulta
- THEN los saves recientes se cortan antes que el resumen

**Test:** `a_context_without_a_query_keeps_the_last_session_over_its_saves` (`prompt_builder.rs`).
