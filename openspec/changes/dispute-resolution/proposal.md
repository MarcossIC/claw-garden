# dispute-resolution — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 5 | M | eval-golden-expansion (archivado) | verify |

## Problema

Las contradicciones se detectan y nunca se resuelven. Detalle verificado en la [exploración](exploration.md); lo que decide esta propuesta:

1. **No hay salida de `disputed`.** El detector marca la fila nueva `disputed` con `disputed_against` (`crates/clawg-core/src/memory/store_insert.rs:313-316`). Ningún código de producción escribe `superseded` ni `superseded_by`: el único que tocaba disputas es `apply_resolution`, sin llamador, y deja a los perdedores en `disputed` (`crates/clawg-core/src/memory/resolver.rs:158-160, 225`). El bloque de conflictos le pide al agente que pregunte al usuario (`crates/clawg-core/src/memory/prompt_builder.rs:306-307`), pero después de la respuesta no hay nada que el agente pueda hacer salvo `delete_memory`.
2. **Disputas huérfanas.** Borrar la fila vieja deja a la nueva `disputed` para siempre: `delete_memory_core` sólo marca `deleted_at` (`crates/clawg-core/src/memory/admin.rs:205, 225-259`), y el bloque de conflictos exige `m_old.deleted_at IS NULL` (`prompt_builder.rs:390`), así que la disputa ni siquiera se muestra.
3. **Pedazos hermanos.** Un save explícito sin tipo es `semantic_fact` (`store_insert.rs:139-143`), que carga afirmaciones. Un texto largo se parte en pedazos que se insertan de a uno, y cada pedazo se compara contra los ya insertados del mismo save (`crates/clawg-core/src/memory/worker.rs:508-531`). Dos pedazos de un mismo save pueden quedar en la banda del detector y disputarse entre sí. El comentario de `worker.rs:508-519` dice que eso no puede pasar porque ningún pedazo carga afirmaciones, y no es cierto para los saves explícitos (ya figura en el backlog del mapa).
4. **El workspace no mira el estado.** Sus consultas a `memory` sólo filtran `deleted_at` (`crates/clawg-core/src/memory/workspace.rs:201-207, 227-234, 245-251`). Hoy no importa porque nadie escribe `superseded`; en cuanto este cambio lo escriba, una fila reemplazada seguiría en `workspace.md`.

## Decisiones (cierran las abiertas de la exploración)

**a. El agente resuelve, con una herramienta MCP `resolve_conflict`.** Ni b ni c.

- El agente ya es un LLM, ya ve el bloque de conflictos y ya tiene la instrucción de consultar al usuario. Sólo le falta con qué registrar la respuesta. No hace falta proveedor, clave ni disparador: es local-first.
- b (revivir el resolver LLM en background) necesita todo eso, que hoy no existe. Queda para knowledge-synthesis, que ya tiene abierta esa decisión.
- c (heurística: gana la más nueva) puede tapar el hecho correcto sin que nadie lo vea. Choca con la regla de que la base del usuario no se pierde ni se esconde.
- Costo aceptado: sin agente que la use, una disputa sigue esperando. Es el comportamiento de hoy, no una regresión.

**`resolve_conflict` tiene dos salidas:**

- **Una gana:** la otra pasa a `superseded` con `superseded_by` apuntando a la ganadora.
- **Valen las dos:** el detector se equivocó (por ejemplo, dos hechos complementarios). Ninguna se reemplaza y la disputa entre ellas se cierra.

La operación también deshace una resolución anterior: si la que se mantiene estaba reemplazada por la otra, vuelve. Esa es la vuelta atrás documentada que pide `config.yaml` para un cambio de estados.

**La precisión del detector no entra en este cambio.** Exigir otro tipo de memoria o una señal léxica de negación cambia qué se disputa, y eso se mide con los pares del golden set. Sí entra excluir a los pedazos hermanos (punto 3): dos partes de un mismo save son una sola afirmación del mismo autor, y no tiene sentido que se disputen, pase o no en la práctica.

**`superseded` sale del ranking, del contexto y del workspace, y sigue en `list_memories`.** La búsqueda ya lo excluye (`crates/clawg-core/src/memory/search.rs:119, 177, 717`) y el listado ya lo muestra (`admin.rs:406-410`, test `list_memories_excludes_deleted_keeps_superseded`). Lo nuevo es el workspace.

**`disputed` sigue en el workspace.** El borrador de la exploración proponía sacarlo. No lo hacemos:

- Una fila `disputed` puntúa igual que una activa (`46c518c`) y sigue en la búsqueda: sacarla sólo del workspace sería esconderla en un lugar y no en otro.
- El bloque de conflictos ya cubre las filas del workspace (`prompt_builder.rs:189-194`, test `a_conflict_on_a_workspace_row_reaches_the_prompt`), así que el agente ve la disputa junto a la fila.
- Si la nueva es la de mayor importancia, sacarla dejaría en el workspace sólo la vieja, que puede ser justo la desactualizada.

## Alcance

1. **Resolver una disputa** (`clawg-core`): una operación que, en una transacción, aplica la salida elegida y deja `status`, `superseded_by` y `disputed_against` coherentes en las dos filas y en las que disputaban a la reemplazada.
2. **Liberar disputas:** cuando una fila se borra o se reemplaza, las filas que la disputaban la sacan de `disputed_against` y vuelven a `active` si no les queda otra disputa pendiente. Lo usan la resolución, `delete_memory_core` y un barrido de mantenimiento que arregla las huérfanas que ya existen.
3. **`POST /v1/disputes/resolve`** en el daemon y **`resolve_conflict`** en el puente MCP. El bloque de conflictos y las instrucciones del servidor nombran la herramienta.
4. **Workspace:** las consultas a `memory` excluyen `superseded`.
5. **Pedazos hermanos:** el detector no compara un pedazo contra los pedazos ya insertados del mismo save. Se corrige el comentario de `worker.rs`.

## Fuera de alcance

- Mejorar la precisión del detector (tipo, negación, umbral). Va en otro cambio y se mide con los pares del golden set.
- Revivir o borrar `resolver.rs`: es de knowledge-synthesis.
- Resolución automática por heurística (opción c).
- Restringir quién puede resolver qué: hoy `list`, `delete` y `PATCH` no tienen scope, y `resolve_conflict` sigue la misma regla hasta visibility-policy.
- El formato y la densidad del bloque de conflictos: es de context-density. Acá sólo cambia la línea que dice qué hacer.
- Restaurar disputas cerradas si una fila borrada vuelve con un undelete futuro (ver Riesgos).

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base. Tests nuevos que fallan hoy:

- resolver con una ganadora deja a la perdedora `superseded` con `superseded_by`, y a la ganadora `active`;
- resolver con "valen las dos" deja a las dos `active` y sin la otra en `disputed_against`;
- una fila con dos disputas sigue `disputed` hasta que se cierra la segunda;
- resolver al revés deshace una resolución anterior;
- borrar la fila vieja deja a la nueva `active`, y el barrido de mantenimiento arregla una huérfana sembrada como las que ya existen;
- una fila `superseded` no aparece en el workspace;
- dos pedazos de un mismo save que caen en la banda del detector no se disputan entre sí;
- `POST /v1/disputes/resolve` y `resolve_conflict` responden 404 / 409 / 400 según el caso.

`rag-eval --real --diff` contra el baseline: el golden set no tiene filas `disputed` ni `superseded`, y la búsqueda no cambia, así que se espera diff vacío.

| Métrica | Hoy | Umbral |
|---|---|---|
| R@1 / R@5 / R@10 | 0,4348 / 0,7826 / 0,8913 | sin bajar ninguna |
| MRR / nDCG@10 | 0,5979 / 0,6693 | sin bajar ninguna |

La precisión y el recall del detector sobre los pares de contradicción no se miden antes y después, como decía la exploración: el detector no cambia para un save de un solo pedazo, que es lo que son esos pares.

**Hipótesis de los pedazos hermanos:** la exclusión se prueba con vectores sintéticos. Cuánto pasaba con el modelo real (un save largo de `semantic_fact` que se disputa a sí mismo) se mide en la verificación.

## Datos guardados

- **Esquema:** no cambia. `superseded_by`, `status`, `disputed_against` y `last_disputed_at` existen desde V1 (`crates/clawg-core/migrations/V1__init.sql:24-28`). No hay migración nueva.
- **Estados:** aparece la primera escritura de `superseded`, siempre por una decisión explícita. Una fila reemplazada no se borra: conserva contenido, vector y FTS, sigue en `list_memories` y en `GET /v1/memories/memory/{id}`, y se recupera resolviendo al revés.
- **`disputed_against`** pasa a significar "disputas pendientes": al cerrar una disputa se saca el id de la otra fila. `last_disputed_at` no cambia, y `superseded_by` guarda quién reemplazó a quién.
- **Huérfanas existentes:** el barrido de mantenimiento las pasa a `active`. Es un cambio de estado, no una pérdida: el contenido no se toca y la fila sigue en la búsqueda igual que antes (`disputed` y `active` puntúan igual).
- **Datos de la versión anterior:** un test siembra filas `disputed` como las escribe hoy el detector (con la fila vieja viva y con la vieja borrada) y prueba que siguen alcanzables y que la resolución y el barrido funcionan sobre ellas.
- **Rollback:** revertir los commits. Una versión anterior lee `superseded` como hoy (fuera del ranking, visible en el listado) y trata las `active` liberadas como cualquier fila activa. Para recuperar una fila reemplazada con la versión anterior basta `UPDATE memory SET status='active', superseded_by=NULL WHERE id=?`.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El agente reemplaza el hecho correcto | Media | La herramienta pide consultar al usuario antes, la fila no se borra y la resolución se deshace llamando al revés |
| El agente usa la herramienta para reemplazar filas que no están en disputa | Baja | Sólo acepta un par con una disputa pendiente o con un `superseded_by` entre ellas; si no, responde 409 |
| Un undelete futuro (data-preservation, parte E) devuelve una fila cuyas disputas ya se liberaron | Media | La disputa no se restaura sola. Si el texto vuelve a guardarse, el detector la abre de nuevo. Se anota para quien diseñe el undelete |
| Un daemon viejo recibe `resolve_conflict` | Baja | Responde 404 por la ruta inexistente y el puente lo informa como error; se corrige al reiniciar el daemon |
