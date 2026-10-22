# context-hit-room — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | 23 (context-budget) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md), verificado contra `develop` (`4ff5da4`).

En un store maduro, `recall_context` no devuelve ninguna memoria de la consulta. `fit_to_budget` corta primero todos los hits de RECENT y LONG-TERM, después LAST SESSION y después los conflictos, y nunca el workspace (`crates/clawg-core/src/memory/prompt_builder.rs:404-454`). Con un AUTO cerca de su tope de 6.000 bytes (`crates/clawg-core/src/memory/workspace.rs:19-20`), un resumen de sesión largo y un par en disputa, el presupuesto por defecto de 8.000 bytes se va antes de llegar a un solo hit. Se vio en un `recall_context` real (19.134 bytes de contexto entero, los 10 hits afuera) y en `revision-eval chains --real --background` (la última versión no aparece en ninguna de las 40 cadenas). 23 se midió con el golden, sin workspace, y ahí nunca cortaba.

## Objetivo que sirve

1: recuperar memorias con un costo predecible sin que la recuperación pierda lo que se le pidió. Un contexto que se inyecta sin la respuesta a su consulta es peor que no inyectar nada.

## Decisiones

El porqué y las alternativas están en el [diseño](design.md).

- **Reserva para los hits:** un contexto con consulta reserva a sus hits un cuarto del presupuesto (`budget / 4`, 2.000 bytes con el default).
- **Orden de recorte:** primero los hits que pasan de la reserva, con la regla de 23 (el de menor score, un exacto sólo al final); después LAST SESSION; después los conflictos; después los hits de la reserva, con la misma regla. El workspace sigue sin cortarse.
- **Sin consulta, sin reserva:** el contexto del hook `session-start` y el de `recall_context` en blanco conservan el orden de 23: ahí LAST SESSION es el traspaso de la sesión y RECENT son los saves más nuevos, no la respuesta a una pregunta.
- **Ni el default ni el workspace cambian:** subir el presupuesto no arregla un store que crece, y cortar el workspace dejaría sin canal a sus filas (`EmittedIds`).

## Alcance

1. `prompt_builder.rs`: `PromptParts::hit_room`, el orden de recorte con reserva en `fit_to_budget` (pura) y la reserva que pasa `build_memory_prompt`.
2. Docs: `specs/retrieval` (al archivar), `.claude/CLAUDE.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- Un tope del workspace o de sus preferencias, o recortar la sección USER.
- Cambiar `DEFAULT_BUDGET`, `MAX_BUDGET`, `K_FINAL`, `MAX_CONFLICT_PAIRS` o el ranking.
- Una reserva en el contexto sin consulta.
- `search_memory` y `search_project`, que no tienen workspace ni este presupuesto.

## Cómo se mide el éxito

- Un test que falla hoy (ver [spec](spec.md)): con un store de la forma madura (un workspace de 5.000 bytes, un resumen de sesión de 3.000, un par en disputa con los dos lados largos y hits largos), el `recall_context` con el presupuesto default trae el hit de la consulta. Hoy no trae ninguno.
- Los tests puros del recorte: lo que pasa de la reserva se corta antes que LAST SESSION y los conflictos, y lo que está dentro, después; con reserva 0 el orden es el de 23 y sus tests siguen pasando sin cambios.
- `rag-eval --real --agent-view` con el default: ningún prompt del golden se corta (23), así que la regla nueva no lo mueve. Se corre sólo si cambia algo más que el orden de corte.
- Con el modelo real y un export de un store (autorización del dueño del store): el mismo `recall_context` real y `revision-eval chains --real --background`, donde la última versión de una cadena tiene que aparecer en `recall_context`.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding, ni lo que se guarda. Sólo cambia qué parte del contexto se corta cuando no entra. Volver atrás es volver al binario anterior.
