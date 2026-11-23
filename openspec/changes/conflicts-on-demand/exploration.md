# conflicts-on-demand — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el criterio del piloto «al abrir una sesión, el contexto trae entero el resumen de la sesión anterior» | M, por partes; ninguna toca `memory.db` | —; se coordina con [session-lifecycle](../session-lifecycle/exploration.md) (99) y [memory-links](../archive/2026-11-22-memory-links/exploration.md) (134), que miran el mismo traspaso, y sigue la regla de la apertura de [contract-diet](../archive/2026-11-20-contract-diet/proposal.md) (133, archivado) | exploración |

**Tesis:** ACTIVE CONFLICTS llega a cada inicio de sesión aunque nada de la sesión dependa de esas memorias, y le gana el presupuesto a LAST SESSION, que es el traspaso. Un conflicto sirve cuando una respuesta depende de una de las memorias en disputa, y eso lo sabe una búsqueda, no el arranque. La propuesta: el contexto sin consulta no trae el bloque; cada fila en disputa que clawg muestra (en el arranque, en `recall_context` y en `search_memory`) dice con qué otra fila está en disputa, con una marca de pocos bytes; `recall_context` con consulta conserva el bloque, pero el bloque cede antes que LAST SESSION; y `list_memories` lista, cuando la persona lo pide, todos los pares que la esperan.

**De dónde sale.** El pedido fue resolver los conflictos de memoria cuando se buscan y no cuando arranca la sesión, o tener una forma manual de pedir la revisión. Los datos de 12 días del piloto lo confirman: `context.cut` registró `block=last_session` en 134 de 250 contextos y `block=conflicts` en 133, y un inicio de sesión de este repositorio trajo ACTIVE CONFLICTS con más de 16 pares y ningún LAST SESSION. Por eso falla el criterio del piloto sobre el resumen de la sesión anterior ([ROADMAP](../../ROADMAP.md#piloto-una-semana-de-trabajo-real-no-una-noche)). Lo demás se verificó contra `develop` en 0.92.0 (`d98e9aa9`).

## Qué pasa hoy

### El inicio de sesión trae los conflictos de todo lo que podría mostrar

- `clawg hook session-start` pide `POST /v1/context` sin consulta, con 8.000 bytes menos lo que ocupan las reglas de programas ausentes (`SESSION_BUDGET`, `crates/clawg-cli/src/hook.rs:76`; `context_budget`, `:182-184`). Sin consulta, el handler llama a `build_session_prompt` (`crates/clawg-cli/src/daemon/api/memories.rs:1035` y `:1069`). `recall_context` siempre lleva consulta (`RecallContextArgs.query`, `crates/clawg-cli/src/mcp/server.rs:256`); sólo una en blanco toma este camino.
- `build_session_prompt` arma el workspace, LAST SESSION y, como RECENT, los diez saves más nuevos del agente en siete días, y llama a `assemble` con reserva 0 (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs:311-331`; la reserva, `:329`).
- `assemble` junta en `shown` los ids de los hits y los de las filas que imprimió la sección AUTO del workspace (`:365`), y `fetch_active_conflicts` trae cada par escalado con un lado entre ellos, de cualquier autor, si el lector alcanza los dos lados (`:374`; la consulta, `:1264-1325`). `rank_conflicts` deja 5 (`MAX_CONFLICT_PAIRS`, `:57`; `:865-877`) y cuenta el resto en `(+N conflictos más que no se muestran)` (`format_conflict_block`, `:913-918`).
- Así que un inicio de sesión trae ACTIVE CONFLICTS cada vez que una fila reciente o una fila del workspace está en disputa. Dos preferencias o dos decisiones cercanas escalan siempre (`needs_user`, [daemon.md](../../../docs/reference/daemon.md); escenario «preferencia cercana» de [`specs/disputes`](../../specs/disputes/spec.md)), y la sección AUTO imprime las dos en cada sesión: en un store con preferencias revisadas, el bloque llega a todos los arranques.

### El presupuesto corta LAST SESSION antes que los conflictos

`fit_to_budget` (`prompt_builder.rs:468-540`) quita de a uno, mientras el texto no entra: las entradas de AUTO que pasan de la mitad del presupuesto (`:485-488`), los hits que pasan de la reserva (`:490-494`), **LAST SESSION** (`:495-498`), **después** los pares desde el último (`:499-504`), los hits que quedan y el resto de AUTO. La spec lo justifica así: el resumen «cede ante los conflictos porque esos piden una pregunta al usuario» ([`specs/retrieval`](../../specs/retrieval/spec.md#el-prompt-cumple-el-presupuesto)).

En el inicio de sesión la reserva es 0, así que el orden queda: AUTO sobre la mitad, todos los saves de RECENT, LAST SESSION y recién entonces los conflictos. Y cortar un save de RECENT no siempre achica el texto: los pares se piden antes del presupuesto, con los ids de todos los hits (`:371-377`), y un lado cuyo hit se cortó deja de citarse como `#id` y pasa a imprimirse con su texto entero (`render` recalcula las posiciones, `:657`; `format_conflict_block`, `:895-900`). Cortar el hit mueve su texto al bloque.

La métrica no separa el inicio de sesión de `recall_context`: `record_context` registra `context.cut` sólo con `block` (`memories.rs:1120-1126`), así que no se sabe cuántos de los 134 contextos sin resumen eran inicios de sesión.

### Lo que el agente lee sobre los conflictos

- **La apertura** del protocolo, que son las instrucciones del servidor y llegan a todo cliente en cada pedido: «When `recall_context` lists ACTIVE CONFLICTS, ask the user which memory holds and record the answer with `resolve_conflict`, `user_answered` and their words in `reason`; set it only when the user said so.» (`crates/clawg-cli/src/mcp/protocol.md:28-30`; `protocol.es.md:27-29`). No pone condición: pide preguntar cada vez que el bloque aparece. La NOTE del bloque sí la pone: «Before a decision that depends on them, ask the user» (`prompt_builder.rs:845-859`).
- La frase está en la apertura, así que la repite el bloque `full` que escribe `clawg setup`. `lean` (el valor por defecto desde 0.92.0), `short` y `off` no llevan la apertura (`protocol::lean`, `crates/clawg-cli/src/mcp/protocol.rs:121-132`).
- **`resolve_conflict`** dice que juzga «one under ACTIVE CONFLICTS in `recall_context`», y `keep`, que su id es el que muestran «ACTIVE CONFLICTS or `save_memory`» (`server.rs:458-459` y `:1972-1980`; en español, `crates/clawg-cli/src/mcp/tools_es.rs:101-115`). Un veredicto `conflicts` contesta «the pair waits for the user in ACTIVE CONFLICTS» (`format_resolution`, `crates/clawg-cli/src/mcp/tools.rs:270-282`), y `memory_status`, «conflict candidates: N waiting for the user in ACTIVE CONFLICTS» (`format_open_pairs`, `tools.rs:286-293`).
- **El workspace** conserva las filas `disputed` porque «the conflicts block reaches the prompt for every row this section emits, so the dispute is shown next to it» (`crates/clawg-core/src/memory/retrieval/workspace.rs:390-393`). Sacar el bloque del arranque deja sin esa razón a la sección AUTO si nada la reemplaza.

### La búsqueda no marca las filas en disputa

`search_memory` devuelve las filas `disputed` como cualquier otra (`status IN ('active', 'disputed')`, `crates/clawg-core/src/memory/retrieval/search.rs:287`), y su línea no dice que una fila es un lado de un par: `N. #id [kind/tipo/tN origen pieza score=… topic=… project=… by …] texto` (`format_hits`, `tools.rs:232-267`). El daemon manda el `status` de cada hit (`RankedHit.status`, `search.rs:148`), pero el `Hit` del puente no lo lee (`crates/clawg-cli/src/client.rs:212-246`). Aunque lo leyera no alcanzaría: de un par sólo la fila nueva queda `disputed`, y la vieja sigue `active` sin saber con quién está en disputa, porque `disputed_against` vive en la nueva ([`specs/disputes`](../../specs/disputes/spec.md#definiciones)).

### No hay forma de pedirlos

- `GET /v1/disputes` cuenta los pares pendientes y los escalados, pero lista sólo los pendientes, hasta 5 (`open_pairs`, `crates/clawg-core/src/memory/revision/dispute.rs:524-568`, el filtro en `:545`; `OPEN_PAIRS_SHOWN`, `crates/clawg-cli/src/daemon/api/disputes.rs:59`). Su consulta rechaza un parámetro que no conoce (`deny_unknown_fields`, `disputes.rs:46-47`).
- `memory_status` imprime sólo cuántos escalados hay (`server.rs:1776-1782`) y MUST NOT listarlos ni decir cómo juzgarlos (VR4 y VR5 de [`specs/mcp`](../../specs/mcp/spec.md#los-conflictos-en-las-herramientas)): con una lista, el agente juzgaba los pares sin preguntar. Es lo que corrigieron [verdict-on-replace](../archive/2026-11-12-verdict-on-replace/proposal.md) (128) y [protocol-block](../archive/2026-11-17-protocol-block/proposal.md) (132), que midió que el 97 % de los `resolve_conflict` eran veredictos `compatible`, los que pedía el bloque viejo.
- `list_memories` no filtra por estado ni por proyecto: lista todo lo que el agente alcanza (`ListMemoriesArgs`, `server.rs:313-333`; `list_memories_impl`, `:1393-1425`), aunque el daemon ya acepta `status` en `GET /v1/memories` (`ListQuery.status`, `memories.rs:286`).
- La CLI ya da una vista cruda, de sólo lectura: `clawg memories list --status disputed --here` lista la fila nueva de cada par del proyecto (`Filters.status`, `crates/clawg-cli/src/cli.rs:1069-1071`), y `clawg memories show ID` muestra en `against` los ids de la otra (`crates/clawg-cli/src/memories.rs:253-254`). Un lado por fila; el otro, por id.

## Restricciones

1. **La apertura en inglés tiene un carácter libre.** Con la preferencia `en` mide 2.047 de los 2.048 que lee Claude Code anterior a la 2.1.296 (`the_instructions_fit_older_clients`, `server.rs:2972-2977`; [`specs/mcp`](../../specs/mcp/spec.md#las-instrucciones-caben-en-2048-caracteres)). Medidas con la regla de `opening` (`protocol.rs:60-78`): sin preferencia 2.030, con `en` 2.047, con `es` 1.933. La frase de ACTIVE CONFLICTS mide 204 en inglés y 205 en español, así que su reemplazo cabe en 205 caracteres en inglés y en unos 320 en español. En inglés no entra una oración más.
2. **Los tests fijan frases de la apertura:** «ask the user which memory holds» y «pregúntale a la persona qué memoria vale» (`the_instructions_carry_the_saving_habits` y su versión en español, `server.rs:3019-3064`), y que el español y el inglés nombren lo mismo entre comillas invertidas (`protocol.rs:270-280` y `:444-454`). Una marca como `conflict=` no cuenta como nombre para `the_protocol_names_only_what_exists` (`is_name`, `server.rs:2935-2938`).
3. **Cambiar la apertura cambia el contrato y el bloque `full`.** Los dos contratos (`agent_contract.json`, `agent_contract.es.json`) se regeneran con `CLAWG_BLESS=1`. Un bloque `full` instalado queda `stale` y su sesión recibe el aviso del bloque en cada inicio hasta que se corra `clawg setup claude-code` (PB4 de [`specs/hooks`](../../specs/hooks/spec.md)). `lean`, `short` y `off` no llevan la apertura y siguen `current`.
4. **Una lista invita a juzgar sin preguntar** (ver arriba). Lo que liste pares tiene que llegar sólo cuando la persona lo pide, y decir que la respuesta es suya.
5. **El peso del contrato.** contract-diet acaba de bajar 1.856 caracteres de las herramientas y midió que la apertura pesa distinto que el resto: lo que se le suma paga en cada pedido de todo cliente. Claude Code carga las herramientas como diferidas, así que una herramienta nueva paga su nombre en cada pedido y su esquema sólo al cargarla, y un parámetro nuevo sólo cuando se carga su herramienta ([exploración de contract-diet](../archive/2026-11-20-contract-diet/exploration.md#qué-se-manda-y-cuándo)).

## Enfoques

### A. El contexto sin consulta

| # | Qué | A favor | En contra |
|---|---|---|---|
| A1 | Sin bloque y sin nada en su lugar | Lo más chico: `build_session_prompt` no pide pares | El agente arranca sin saber que una preferencia del workspace está en disputa y puede seguirla; deja sin su razón a `workspace.rs:390-393` |
| A2 | Sin bloque. Cada fila que el arranque imprime y que es un lado de un par lleva la marca `conflict=#N`, con el id del otro lado | La misma consulta de hoy, otra forma de mostrarla: unos 15 bytes por fila marcada en lugar de los textos de los pares. El agente sabe de la disputa y nada le pide preguntar ya | Una entrada del workspace no imprime su id (`format_conflict_block`, `:879-883`), así que su marca nombra los dos: `(#12 conflict=#9)`. Cambia cómo el prompt imprime la sección AUTO (el archivo no) |
| A3 | El bloque sigue, detrás de LAST SESSION en el presupuesto | Arregla el criterio del piloto con un cambio de orden | No es lo que se pidió: el bloque sigue llegando a cada arranque, y con él la frase que pide preguntar |

**Recomendado: A2.**

### B. El contexto con consulta (`recall_context`)

| # | Qué | A favor | En contra |
|---|---|---|---|
| B1 | El bloque sigue y cede antes que LAST SESSION. Los hits llevan también la marca | Es una búsqueda: la consulta hizo relevantes esas filas, y el bloque da el texto de los dos lados sin otra llamada. Si el presupuesto saca un par, la marca queda en sus hits | El prompt dice dos veces lo mismo para un par con los dos lados impresos (unos 15 bytes por hit) |
| B2 | Sin bloque, sólo marcas, como el arranque | Una sola regla para el agente; `context.cut{block=conflicts}` desaparece por construcción | Para preguntar, el agente lee el otro lado con `read_memory`. Borra cuatro requisitos de `specs/retrieval` y la NOTE, y mueve la línea base de `rag-eval --agent-view`, donde 45 de 46 casos traían el bloque ([«Medición del contexto»](../../specs/retrieval/spec.md#medición-del-contexto)) |
| B3 | Como hoy | — | LAST SESSION se sigue cortando antes que los pares |

**Recomendado: B1.** El orden nuevo, después de los hits que pasan de la reserva: los pares desde el último, LAST SESSION, los hits que quedan y el resto de AUTO. La razón se invierte: un par se vuelve a encontrar (la marca en cada búsqueda que lo toca, la lista a pedido), y el resumen no tiene otra consulta que lo traiga. B2 queda como paso siguiente si, con la métrica separada por estado, `context.cut{block=conflicts,state=searched}` sigue alto.

### C. La búsqueda (`search_memory`)

| # | Qué | A favor | En contra |
|---|---|---|---|
| C1 | `POST /v1/search` manda, por hit, los ids del otro lado de cada par escalado cuyos dos lados alcanza el lector (la regla de `fetch_active_conflicts`), y la línea lo muestra: `[Memory/user_preference/t1 score=0.83 conflict=#9]` | Marca los dos lados, con el id que recibe `resolve_conflict`. Un `SELECT` por búsqueda sobre a lo sumo 50 ids. Un daemon anterior no manda el campo y la línea sale como hoy | Un campo más en el hit y en el `Hit` del puente |
| C2 | Mostrar el `status` en la línea | Ya viaja | Marca sólo la fila nueva, y sin decir contra cuál |
| C3 | Nada | — | Es justo donde se pidió resolverlos |

**Recomendado: C1.** Una fila en disputa con varias puede nombrar a todas o a unas pocas y contar el resto; lo fija la spec.

### D. A pedido de la persona

| # | Qué | Peso en el contrato | A favor | En contra |
|---|---|---|---|---|
| P1 | `list_memories` con `conflicts: true`: en lugar de filas, los pares escalados del proyecto abierto (la regla de `memory_status`, BI8), cada lado con su id y su primera línea, y una línea final que pide preguntarle a la persona por cada uno y registrar sólo su respuesta. Lo sirve `GET /v1/disputes` con un parámetro que lista los escalados en lugar de los pendientes | Un booleano: unos 150 caracteres por idioma, sólo al cargar `list_memories` | Extiende una herramienta y una ruta que existen. Un pedido trae todos los pares con los dos textos | El nombre no dice «conflictos»: el agente llega por la descripción de `resolve_conflict` y por la línea de `memory_status`, que lo nombran. `list_memories` lista de todos los proyectos y este modo, del abierto |
| P2 | `list_memories` con `status` | Unos 100 caracteres | El daemon ya filtra por estado | Lista filas y no pares: sólo la nueva, y la vieja hay que leerla con `read_memory`, una llamada por par (más de 16 en este repositorio) |
| P3 | Una herramienta nueva, `list_conflicts` | Su nombre en cada pedido y unos 700 a 900 caracteres al cargarla; cambian la cuenta de herramientas ([`specs/mcp`](../../specs/mcp/spec.md), R3), el contrato y «Las otras herramientas» del protocolo | Se encuentra por su nombre | Lo que contract-diet acaba de achicar, para una sola lista |
| P4 | Sólo la CLI de lectura: una orden nueva (`clawg memories conflicts`) o la vista que ya existe (`list --status disputed` y `show`) | Nada | La persona mira sin el agente | Resolver igual pide al agente, con cada id pasado a mano; no es el flujo en la conversación que se pidió |
| P5 | Listarlos en `memory_status` | Nada nuevo | Ya llama a `GET /v1/disputes` | Va contra VR4 y VR5: `memory_status` es el diagnóstico, y una lista ahí volvía a invitar a juzgar |

**Recomendado: P1.** La vista de la CLI que ya existe alcanza para mirar sin el agente; una orden nueva no suma al flujo pedido.

### E. Los textos

- **La apertura**, en las dos lenguas: la frase de ACTIVE CONFLICTS pasa a decir que, cuando una respuesta depende de una memoria marcada `conflict=`, o cuando la persona pide revisar los conflictos, se le pregunte cuál vale y se registre su respuesta con `resolve_conflict` y `user_answered`. En inglés, en 205 caracteres o menos; por ejemplo, con 195: «When an answer rests on a memory marked `conflict=`, or the user asks to review them, ask the user which memory holds and record their words with `resolve_conflict`, `user_answered` and `reason`.» Ninguna de las dos nombra `list_memories`: en inglés no entra, y el camino lo dan `resolve_conflict` y `memory_status`. La frase de VR3, sobre los saves con cercanas, no cambia.
- **`resolve_conflict`:** «one under ACTIVE CONFLICTS in `recall_context`» pasa a nombrar la marca y la lista de `list_memories`; la descripción de `keep`, lo mismo.
- **`memory_status`** y **`format_resolution`:** «waiting for the user in ACTIVE CONFLICTS» pasa a decir dónde se listan. VR4 sigue: un número, sin listar ni decir cómo juzgar.
- **La NOTE del bloque** (`prompt_builder.rs:845-859`) no cambia: ya pide preguntar antes de una decisión que dependa de esas memorias.
- **El comentario de `workspace.rs:390-393`** dice la razón nueva: la marca.

## Qué pasa con los pares que ya esperan

Nada cambia en `memory.db`: los pares siguen `escalated`, las filas nuevas siguen `disputed` y las dos se siguen encontrando (`search.rs:287`). Dejan de llegar enteros a cada arranque, donde las filas que se imprimen llevan la marca, y aparecen cuando una búsqueda los toca o cuando la persona los pide. `memory_status` los sigue contando. En este repositorio son más de 16: se pueden revisar de una vez con el modo nuevo de `list_memories`, o dejar que salgan cuando una búsqueda los toque. Un binario anterior los vuelve a poner en el arranque, sin nada que migrar.

## Cómo se mide

**Sin pagar, con tests que hoy fallan:**

1. Un contexto sin consulta, con una fila de RECENT y una preferencia del workspace en disputa, no trae ACTIVE CONFLICTS, y las dos filas llevan la marca con el otro lado (`prompt_builder.rs`).
2. Un contexto sin consulta con un workspace, un resumen largo, saves recientes y un par que no entran juntos trae LAST SESSION entero: el criterio del piloto como test.
3. Con consulta, cuando no entran LAST SESSION y los pares, los pares se van primero y el resumen queda entero; la marca queda en los hits cuyo par se fue. Cambian `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts` y el escenario «la reserva» de [`specs/retrieval`](../../specs/retrieval/spec.md#el-prompt-cumple-el-presupuesto).
4. `POST /v1/search` manda los ids del otro lado sólo de pares escalados cuyos dos lados alcanza el lector: ni un par pendiente, ni un lado oculto, ni uno borrado (`crates/clawg-cli/tests/api.rs`).
5. `search_memory` muestra ` conflict=#N`; contra un daemon sin el campo, la línea de hoy (`tools.rs`, `server.rs`).
6. `list_memories` con `conflicts` lista los escalados del proyecto con los dos lados, sin los pendientes; contra un daemon que rechaza el parámetro, pide reiniciarlo con `clawg stop`.
7. `context.cut` lleva también `state` (`a_context_records_its_state_its_size_and_the_blocks_it_cut`, `crates/clawg-cli/tests/api.rs`).
8. Las tres aperturas siguen en 2.048 caracteres o menos y ninguna dice ACTIVE CONFLICTS; los dos contratos se regeneran con `CLAWG_BLESS=1` y su diff es sólo la frase y las descripciones de `resolve_conflict` y `list_memories`.

**Con uso real, después de instalar** (en local: lee el perfil real y pide autorización): con la versión instalada una semana, `context.cut` con `block=conflicts` y `state=recent` en 0, por construcción, y con `block=last_session` y `state=recent` en el 2 % o menos de los contextos `context.retrieval_state{state=recent}`. Por encima, lo que corta el resumen es la mitad que se lleva la sección AUTO (SC2) o lo que escribió la persona (SC5), y eso es otro cambio.

**`rag-eval`:** el ranking no cambia, así que `--diff` sobre `recall@5` no decide nada. Cambia el prompt de `recall_context` (la marca en los hits): una corrida `--real --agent-view`, con el modelo real y el golden de `docs/eval/`, contra la última línea base confirma que `search.found` y `context.found` no se mueven (con menos de 6 casos en un sentido el resultado es «no distinguible», [`docs/reference/testing.md`](../../../docs/reference/testing.md#evaluation-tools)) y dice cuánto suma la marca en tokens. Con el presupuesto por defecto ningún caso del golden se corta, así que el orden nuevo no se ve ahí.

**Sin campaña paga:** la frase de VR3, que es la que mueve J en agent-eval, no cambia, y ninguna métrica del arnés mide la frase de los conflictos.

## Qué tocaría

| Archivo | Para qué | Parte |
|---|---|---|
| `crates/clawg-core/src/memory/retrieval/prompt_builder.rs` | sin bloque en `build_session_prompt`; la marca en `format_hit_line` y en las entradas de AUTO del prompt; el orden de `fit_to_budget` | A2, B1 |
| `crates/clawg-core/src/memory/retrieval/workspace.rs` (`:390-393`) | el comentario | A2 |
| `crates/clawg-core/src/memory/retrieval/search.rs` | los ids del otro lado por hit | C1 |
| `crates/clawg-core/src/memory/revision/dispute.rs` (`open_pairs`) | listar los escalados, con la primera línea de cada lado | P1 |
| `crates/clawg-cli/src/daemon/api/memories.rs` | el campo del hit; `state` en `context.cut` | C1, medición |
| `crates/clawg-cli/src/daemon/api/disputes.rs` | el parámetro de `GET /v1/disputes` | P1 |
| `crates/clawg-cli/src/client.rs` | el campo en `Hit`; el parámetro en `open_disputes` | C1, P1 |
| `crates/clawg-cli/src/mcp/server.rs`, `tools.rs`, `tools_es.rs` | la marca en `format_hits`; `ListMemoriesArgs.conflicts` y su texto; `resolve_conflict`, `format_resolution`, `format_open_pairs` | C1, P1, E |
| `crates/clawg-cli/src/mcp/protocol.md`, `protocol.es.md` | la frase de la apertura | E |
| `crates/clawg-cli/src/mcp/agent_contract.json`, `agent_contract.es.json` | se regeneran | E |
| `openspec/specs/retrieval`, `disputes`, `mcp`, `hooks`, `metrics` | al archivar | — |
| `docs/reference/daemon.md` | el campo del hit, el parámetro de `/v1/disputes` y dónde aparecen los pares escalados | — |

## Decisiones abiertas

| # | Pregunta | Opciones | Recomendada |
|---|---|---|---|
| D1 | ¿Qué recibe el contexto sin consulta? | A1 / A2 / A3 | A2: sin bloque, con la marca |
| D2 | ¿Y `recall_context` con consulta? | B1 / B2 / B3 | B1: el bloque, cediendo antes que LAST SESSION |
| D3 | ¿Cómo dice `search_memory` que una fila está en disputa? | C1 / C2 / C3 | C1: `conflict=#N` con el id del otro lado |
| D4 | ¿Por dónde los pide la persona? | P1 a P5 | P1: `list_memories` con `conflicts` |
| D5 | ¿El arranque dice cuántos pares esperan? | Nada / una línea con el número, sin pedir que se pregunte | Nada: el arranque no habla de conflictos, y quien quiere verlos los pide (P1). Cerrada en la propuesta |
| D6 | ¿Qué se hace con los pares que ya esperan? | Revisarlos de una vez / dejar que salgan al buscar | Las dos, sin elegir: salen cuando una búsqueda los toca, y todos juntos cuando se piden. Cerrada en la propuesta |
| D7 | ¿Se separa `context.cut` por estado? | Sí / medir de otra forma | Sí: sin eso no se sabe si el arranque se arregló |

## Riesgos

1. **El resumen se puede seguir cortando.** Sin el bloque, en el arranque LAST SESSION compite sólo con el workspace: la sección AUTO cede lo que pasa de la mitad del presupuesto (SC2) y lo que escribió la persona no se corta nunca (SC5). Un resumen de más de unos 3.500 bytes con un workspace lleno se sigue acortando. Lo dice la medición con `state`, con su umbral fijado arriba.
2. **La marca puede leerse como orden de preguntar.** La frase de la apertura y la NOTE ponen la condición (cuando la respuesta depende de esa memoria); un agente que pregunte por cada marca devolvería la interrupción del arranque. Ningún test lo mide; se mira en los transcripts del piloto.
3. **Los pares se acumulan sin que nadie los vea.** Hoy el arranque los muestra; después, sólo una búsqueda o un pedido. D5 lo acepta: para verlos todos está la lista a pedido.
4. **La lista invita a juzgar.** El modo de `list_memories` llega sólo cuando se lo pide, y su línea final repite que la respuesta es de la persona; `resolve_conflict` sigue pidiendo `user_answered` para reemplazar una preferencia o una decisión (R3 de `specs/disputes`).
5. **Un bloque `full` instalado queda `stale`** hasta que se corra `clawg setup claude-code`, con el aviso del bloque en cada inicio. El valor por defecto (`lean`) no cambia.
6. **El inglés no tiene lugar.** La frase nueva tiene que caber en los 205 caracteres de la vieja; una regla más en la apertura en inglés obliga a acortar otra, que es B4 de contract-diet y pide medir.
7. **Cursor recibe el mismo contexto** por `sessionStart` y las mismas instrucciones, sin bloque en `CLAUDE.md`. No se probó con Cursor real.
