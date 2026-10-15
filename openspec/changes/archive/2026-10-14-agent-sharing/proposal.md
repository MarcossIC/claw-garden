# agent-sharing — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 (y 2) | M (parte C; la D queda fuera) | visibility-policy (A + B), secret-hygiene | ver [`state.yaml`](state.yaml) |

## Problema

El objetivo 4 pide que varios agentes lean y escriban el mismo cerebro, **con control del usuario sobre qué ve cada uno**. Detalle en la [exploración](exploration.md); verificado de nuevo contra `main` (9c3ff19):

1. **La lectura está aislada del todo.** Las tres consultas KNN filtran por un solo agente (`crates/clawg-core/src/memory/search.rs:121, 138, 153`), igual que el conteo de elegibles que decide el reintento (`search.rs:179, 190, 201`), los candidatos léxicos (`search.rs:558`), la bonificación por identificador (`search.rs:688`) y el puntaje de los candidatos léxicos (`search.rs:842`). Lo que Claude Code aprendió del usuario no le sirve a Cursor ni a Codex, aunque el usuario quiera.
2. **No hay dónde decir qué ve cada agente.** No existe archivo, comando ni endpoint de política. visibility-policy dejó esa pieza para acá (su D1).
3. **Un hit no dice quién lo guardó.** La línea de `recall_context` es `[fecha #id tipo] contenido` (`crates/clawg-core/src/memory/prompt_builder.rs:294-303`) y la de `search_memory`, `N. #id [kind/tipo/tier score] contenido` (`crates/clawg-cli/src/mcp/tools.rs:69-88`). `RankedHit` no lleva el autor (`search.rs:54-64`). Mientras todo es propio no importa; en cuanto se comparta, el agente no puede distinguir lo suyo de lo ajeno ni tratar lo ajeno como dato y no como orden.
4. **`enum Scope` sigue sin uso** (`crates/clawg-core/src/memory/record.rs:16-23`, reexportado en `crates/clawg-core/src/memory/mod.rs:46`). El insert escribe el literal `"agent"` (`crates/clawg-core/src/memory/store_insert.rs:178`). Está en el backlog del mapa y su destino se decide acá.

Lo que la exploración daba por pendiente y ya no lo está: secret-hygiene se archivó el 2026-10-14 ([`specs/secrets`](../../../specs/secrets/spec.md)), así que la condición del mapa ("tiene que cerrar antes de 16") se cumple.

## Decisiones (cierran las preguntas de la exploración)

- **Por defecto se comparte todo; el usuario excluye** (pregunta 2, tarjeta en el hilo: Marcos eligió "Todo compartido"). Sin política, cada agente lee lo que guardaron todos los demás: es lo que pide el objetivo 4, un solo cerebro, y no obliga a configurar nada para que sirva. El usuario excluye de a pares, con dirección: "cursor no lee a claude-code" no implica lo contrario. `*` vale por cualquier agente en los dos lados, para decir "nadie lee a codex", "aider no lee a nadie" o "nadie lee a nadie" con una sola línea. El costo: al actualizar, cada agente empieza a ver lo de los otros, y una memoria envenenada o un secreto que el enmascarado no reconoce viaja a otro cliente sin que nadie lo haya pedido (ver Riesgos).
- **La política vive en un archivo del directorio de datos y se edita con `clawg sharing`** (pregunta 3). `<data>/sharing.json`, que el daemon lee en cada búsqueda: un cambio vale desde la próxima llamada, sin reiniciar. Un archivo ilegible no comparte nada y lo dice: si compartiera todo, un error de tipeo borraría las exclusiones que el usuario escribió.
- **Regla de lectura, no columna** (pregunta 8). Ninguna fila cambia: siguen en `scope = 'agent'` con su `agent_id`, que es el autor. La visibilidad se decide en la consulta. La columna obligaría a poner `agent_id` en NULL (el CHECK de `crates/clawg-core/migrations/V1__init.sql:30-34`), perdería al autor y sería una migración de datos; ocultar a un agente sería otra. `enum Scope` se borra: con esta regla nada lo va a usar, y la columna y su CHECK quedan como están.
- **Lo ajeno es de sólo lectura** (pregunta 10). Borrar, cambiar y resolver siguen siendo sólo de lo propio (`specs/visibility` R3 y R4): una fila ajena sigue siendo "no existe". Las descripciones de esas herramientas lo dicen, y el aviso de lo compartido también.
- **El autor va en cada hit ajeno, y lo ajeno se presenta como dato** (pregunta 7). Lo propio no cambia de formato.
- **Lo que sigue siendo de cada agente:** el workspace (las filas de tier 1 que se cargan siempre), el bloque ACTIVE CONFLICTS, el dedup y el detector de contradicciones al guardar, `synthesize` y `list_memories`. Lo ajeno llega sólo por búsqueda, cuando es relevante a la pregunta.
- **Proyectos (pregunta 4): misma regla que lo propio.** Una fila ajena pasa el mismo filtro de proyecto: la del proyecto actual, las sin proyecto y `user_preference` de cualquier proyecto.

## Alcance

1. **Core.** Módulo `sharing.rs`: la política (parseo y validación puros, ocultar, mostrar, a quién lee un agente) y su lectura y escritura atómica en `<data>/sharing.json`. `SearchScope` lleva el alcance del agente y las consultas de búsqueda filtran por "el agente, o cualquier otro que no tenga oculto". `RankedHit` lleva el autor. `build_memory_prompt` marca los hits ajenos y agrega el aviso.
2. **Daemon.** `POST /v1/search` y `POST /v1/context` aplican la política del directorio de datos al agente que llama. El cuerpo de los pedidos no cambia; la respuesta de `/v1/search` suma `agent_id` en cada hit.
3. **Puente MCP.** `search_memory` marca los hits ajenos y agrega el aviso; `memory_status` dice a quién lee el agente; `delete_memory`, `update_memory` y `resolve_conflict` dicen que lo compartido es de sólo lectura.
4. **CLI.** `clawg sharing` muestra la política; `clawg sharing hide <LECTOR> <AUTOR>` y `clawg sharing unhide <LECTOR> <AUTOR>` la cambian. Funciona con el daemon corriendo.
5. **Limpieza.** Se borra `enum Scope`.

## Fuera de alcance

- **Parte D, el prefiltro en vec0.** No hay medición de crowding que lo justifique, y es una migración de las tablas vectoriales. El conteo de elegibles se ajusta a la regla nueva, así que el reintento a 4× sigue valiendo cuando lo compartido amontona la ventana.
- **Pesos por scope en el ranking** (pregunta 9). Lo propio y lo ajeno puntúan igual. Sin casos multi-agente en el golden set no hay cómo justificar un peso.
- **Compartir el workspace, los conflictos o la síntesis**, y detectar contradicciones entre filas de agentes distintos.
- **Escribir sobre lo ajeno.**
- **Grupos** en la política ("todos los agentes de este proyecto"). `*` alcanza para los casos que hoy existen.
- **Normalizar mayúsculas del `agent_id`.** La política compara exacto, como la base. El cruce de carpetas en NTFS es del workspace y no cambia (`specs/visibility` R5).
- **Incluir `sharing.json` en `clawg backup`.** El snapshot es de `memory.db`; la política se rehace con unos pocos comandos.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base (1.674 pasan, 10 fallas de entorno en Linux), con los tests nuevos justificados en `verify.md`. Tests que fallan hoy:

- sin `sharing.json`, B encuentra las filas de A en `search_memory` y en `recall_context`, marcadas con el autor;
- con "B no lee a A", la próxima búsqueda de B ya no las trae, sin reiniciar el daemon, y A sigue viendo las de B;
- `*` oculta de todos o a todos;
- un `sharing.json` ilegible no comparte nada y `memory_status` dice por qué;
- B no puede borrar, cambiar ni resolver una fila de A que ve;
- el workspace y ACTIVE CONFLICTS de B no traen nada de A;
- filas sembradas como las deja la versión anterior quedan iguales y alcanzables (R8 de la spec).

`rag-eval --real --diff` contra el baseline: diff vacío esperado en el ancla y en agent-view. El golden set es de un solo agente: sin filas ajenas, compartir todo trae las mismas filas que antes.

## Datos guardados

- **Esquema:** no cambia. Ninguna migración.
- **Filas:** ninguna se escribe por este cambio. `scope`, `agent_id` y `project_id` quedan como están; ocultar o mostrar sólo cambia `sharing.json`.
- **Datos de la versión anterior:** un test siembra con SQL directo filas de dos agentes como las deja la versión actual y prueba que sin política B las ve con su autor, que al ocultar dejan de verse, que al mostrar vuelven y que las filas no se tocaron. Lo que sí cambia al actualizar es lo que ve cada agente: desde el primer arranque, todo lo guardado por los demás.
- **Volver atrás sin bajar de versión:** `clawg sharing hide '*' '*'` deja a cada agente con lo suyo, como antes.
- **Rollback:** volver al binario anterior no necesita nada. Ignora `sharing.json` y cada agente vuelve a leer sólo lo suyo.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Al actualizar, cada agente empieza a ver lo de los otros sin que el usuario lo pida | Alta (es la decisión) | El README y `memory_status` lo dicen; `clawg sharing hide '*' '*'` vuelve a lo de antes en un comando |
| Una memoria envenenada de A le llega a B como una orden | Media | Cada hit ajeno lleva el autor y el aviso dice que es información, no instrucción (spec R4); lo ajeno es de sólo lectura; `hide` corta a un agente que no es de fiar |
| Un secreto que el enmascarado no reconoce viaja al proveedor de B | Baja | El enmascarado corre antes de guardar (`specs/secrets`); `clawg secrets` lista lo que reconoce y `clawg purge` lo borra; el README lo advierte |
| El usuario excluye un nombre mal escrito y cree que ocultó | Media | `memory_status` del lector dice a quién lee; `clawg sharing` muestra la política tal como queda |
| Un `sharing.json` roto deja de compartir y nadie se entera | Baja | `memory_status` lo dice con el motivo, igual que `clawg sharing` y el log del daemon |
| Lo ajeno desplaza a lo propio en la ventana de vec0 | Media | El conteo de elegibles incluye lo ajeno, así que el reintento a 4× actúa; la parte D queda lista si aparece crowding medido |
| Las lecturas de B suben el tier de una fila de A por accesos | Baja | Es una lectura real de una memoria útil; el decaimiento la vuelve a bajar. Queda en el diseño (D5) |
| Leer `sharing.json` en cada búsqueda cuesta | Baja | Un archivo de pocos bytes; se lee una vez por llamada, no por tabla |
