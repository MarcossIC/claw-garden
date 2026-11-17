# contract-diet — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el costo de cada sesión | S–M la clase (a), el esquema; M la clase (b), el bloque, más una campaña paga chica | [protocol-block](../archive/2026-11-17-protocol-block/proposal.md) (132, archivado), de donde salen el sello, las variantes y la regla de decisión D6; [agent-eval-sessions](../archive/2026-11-17-agent-eval-sessions/proposal.md) (129), que da las tareas y las métricas. Sólo la parte que saca las secciones del resumen espera a memory-links (134, en exploración), que mide qué aporta un `session_summary` | exploración (borrador); la clase (a) hecha |

**Tesis:** lo que clawg le hace leer al modelo en cada sesión se puede achicar, pero **no todo se puede achicar con el mismo riesgo**. Hay dos clases. La **clase (a)** es ruido del generador de esquemas (`$schema`, `"default": null`, `"null"` en cada tipo): unos 1.490 caracteres de 19.600 en las herramientas, no cambia ninguna palabra que el agente lea y se cierra con tests y el contrato fijado. La **clase (b)** es el bloque del protocolo, que es donde está el peso (10.700 a 11.300 caracteres en cada pedido) y donde cualquier recorte cambia lo que el agente lee. La campaña de protocol-block ya mostró que `short` ahorra el 30 % y pierde dos conductas, así que (b) no se decide por lectura: se decide con un brazo nuevo contra `full` y las reglas de D6. Esta exploración mide qué hay, qué se repite de verdad (casi nada fuera de la apertura) y propone qué probar y en qué orden.

**De dónde sale.** De la medición del 2026-11-11 sobre sesiones reales (el bloque en cada pedido, las instrucciones del servidor, el hook de `session-start`) y de la sección «La campaña» del [verify de protocol-block](../archive/2026-11-17-protocol-block/verify.md#la-campaña), que deja como hipótesis nueva «un bloque corto que conserve la parte de `topic_key`». La palanca «la dieta del protocolo» ya estaba en [memory-delivery](../archive/2026-11-13-memory-delivery/exploration.md) (127, «Las palancas», A); protocol-block le dio el sello y las variantes, y esto es lo que queda: el texto mismo. Los números se contaron sobre `develop` en 0.88.0 (`8f20b542`) con un script que lee `protocol.md`, `protocol.es.md`, `agent_contract.json` y `agent_contract.es.json`; los tokens son caracteres ÷ 4, salvo donde se dice otra cosa.

## Qué se manda, y cuándo

Tres momentos distintos, porque Claude Code carga las herramientas MCP como diferidas:

- **En cada pedido, desde el primero:** las instrucciones del servidor (`info.instructions = opening(...)`, `crates/clawg-cli/src/mcp/server.rs:2069`) y el bloque del `CLAUDE.md` (`assemble`, `crates/clawg-cli/src/setup.rs:689-697`, con el texto de `protocol::text`, `crates/clawg-cli/src/mcp/protocol.rs:82`). Medido en el piloto de memory-delivery: el primer pedido de `a3` pesa 4.413 tokens más que el de `a0` (20.469 contra 16.056), que es el bloque, las instrucciones y los nombres de las herramientas.
- **Una vez por sesión, y después se queda en la conversación:** la salida de `session-start` (`session_start_text`, `crates/clawg-cli/src/hook.rs:198`).
- **Al cargar una herramienta:** su esquema y su descripción, cuando el agente la pide con `ToolSearch`. Hasta entonces el modelo sólo ve el nombre.

**Qué implica lo diferido para achicar los esquemas.** Sólo se paga lo que se carga, y de las 13 herramientas el agente típico carga unas pocas (`search_memory`, `save_memory`, a veces `recall_context` o `search_project`). Pero una herramienta cargada **se queda**: desde ese punto viaja en cada pedido como el bloque, sólo que empezando más tarde. Un carácter de esquema vale lo mismo que uno del bloque menos los pedidos que faltaban cuando se cargó; lo que cambia es cuántos caracteres hay en juego. Con las cuatro herramientas habituales, el ruido es de 684 caracteres, no de 1.490.

Lo que el repositorio ya resuelve: el brazo `a3` del arnés deja las herramientas diferidas y `a3-eager` pone `ENABLE_TOOL_SEARCH=false` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:436-437`), así que la campaña de `a3` ya paga sólo lo que carga, y la diferencia entre los dos brazos es el costo de cargarlo todo. Lo que no se verificó: si Cursor manda siempre todas las herramientas, que es lo que haría valer más la clase (a).

## Inventario medido

Caracteres de cada texto, contados sobre los archivos del repositorio. «Tokens» es ÷ 4; para el castellano la medición real del bloque da entre 3.550 y 4.000 tokens para unos 11.300 caracteres (2,8 a 3,2 caracteres por token), así que el ÷ 4 subestima el texto en español entre un 25 y un 40 %. Las herramientas se cuentan como JSON compacto de `name`, `description` e `inputSchema`: el total en español sale en 19.616, no en los 20.173 de la medición anterior, que se contó con otro criterio y no cambia ninguna conclusión.

| Texto | Cuándo entra | es (caracteres) | en (caracteres) | Tokens ÷ 4 (es) |
|---|---|---|---|---|
| Bloque `full` (`protocol.es.md` / `protocol.md`; más una línea de sello de unos 35) | cada pedido, sólo Claude Code con bloque | 11.271 | 10.691 | 2.818 (medido: 3.550–4.000) |
| Bloque `short` (título, una línea y las dos secciones del resumen) | cada pedido, con `--protocol short` | ~1.140 | ~1.030 | ~285 |
| Instrucciones del servidor (`opening`) | cada pedido, todo cliente que las lee | 1.933 | 2.030 (tope 2.048, `server.rs:2951`) | 483 |
| Las 13 herramientas, juntas | al cargar cada una | 19.616 | 18.940 | 4.904 |
| `save_memory` | al cargarla | 3.640 | 3.559 | 910 |
| `update_memory` | al cargarla | 2.416 | 2.349 | 604 |
| `search_memory` | al cargarla | 2.312 | 2.200 | 578 |
| `resolve_conflict` | al cargarla | 1.742 | 1.698 | 436 |
| `search_project` | al cargarla | 1.619 | 1.560 | 405 |
| `synthesize` | al cargarla | 1.535 | 1.453 | 384 |
| `recall_context` | al cargarla | 1.468 | 1.404 | 367 |
| `list_memories` | al cargarla | 1.324 | 1.263 | 331 |
| `answer_suggestion` | al cargarla | 1.306 | 1.279 | 327 |
| `read_memory` + `restore_memory` + `delete_memory` | al cargarlas | 654 + 646 + 643 | 632 + 595 + 646 | ~486 las tres |
| `memory_status` | al cargarla | 311 | 302 | 78 |
| Ruido de esquema, de las 13 | al cargar | 1.489 | 1.489 | 372 |
| Salida de `session-start` | una vez por sesión | hasta 10.000 bytes | hasta 10.000 bytes | hasta 2.500 |

**La salida de `session-start`.** El contexto de memorias tiene un tope de 8.000 bytes (`DEFAULT_BUDGET`, `crates/clawg-core/src/memory/retrieval/prompt_builder.rs:61`; la hook lo usa en `hook.rs:76`) y lo que el hook agrega en su propia voz (la línea de idioma, el aviso del bloque, las reglas de programas ausentes, los avisos de síntesis) queda por debajo de 2.000 bytes: `what_the_hook_adds_stays_small` (`hook.rs:2120`) fija el máximo en 10.000. El tamaño **típico** no se pudo medir sin leer el perfil real: el registro de `session-start` guarda los `bytes` de cada corrida (`crates/clawg-cli/src/hook_log.rs:69`), y mirarlo pide autorización.

**Cómo se reparte el esquema (es):** 19.616 caracteres son 13.533 de texto (descripciones de herramienta y de parámetro), 1.489 de ruido y 4.594 de estructura que hace falta (nombres, `type`, `required`, `enum`, `properties`). `save_memory` lleva 1.900 de sus 3.640 en las descripciones de sus 10 parámetros y 680 en la de la herramienta. Los parámetros que se repiten en varias herramientas suman mucho: la familia `project`, `project_path`, `all_projects`, `to_project*`, `clear_project`, `new_project` ocupa 3.058 caracteres repartidos en 6 herramientas; `user_answered` está en 3 (353), `memory_type` en 3 (373), `id` en 4 (367), `limit` en 4 (208).

**Cómo se reparte el ruido (igual en es y en):** `"$schema": "https://json-schema.org/draft/2020-12/schema"` en cada herramienta, 741; `"default": null` en cada parámetro opcional, 510; `"null"` como segundo tipo (`["string", "null"]`), 238. Hay además unos 353 de `format` (`int64`, `uint32`, `double`) y `minimum: 0`, que al modelo no le dicen nada, y 62 saltos de línea dentro de las descripciones en inglés. Los esquemas los genera rmcp con `SchemaSettings::draft2020_12()` fijo (`rmcp-3.9.0/src/handler/server/common.rs:17-28`, dependencia de `crates/clawg-cli/Cargo.toml:52`): no hay una opción de clawg que los apague. El punto donde clawg ya reescribe el router es `with_language` (`server.rs:599-606`).

**Cómo se reparte el bloque (es / en):**

| Sección | es | en | Cuánto repite de otro texto |
|---|---|---|---|
| Título y apertura (3 párrafos) | 1.973 | 2.062 | **casi todo:** el 98 % de sus secuencias de 5 palabras están en las instrucciones, y las instrucciones son ese texto sin el título (1.933 de 1.973, 2.030 de 2.062) |
| Cuándo guardar | 508 | 507 | 0 a 1 % |
| Qué no guardar | 461 | 422 | 5 % |
| Cómo escribir un save | 2.688 | 2.425 | 2 a 3 % (la viñeta de `topic_key` sola son 1.080 / 1.012) |
| Tipo e importancia | 1.822 | 1.752 | 4 % |
| Resumen de sesión | 814 | 742 | 1 a 2 % |
| Después de una compactación | 152 | 136 | 0 % |
| Conclusiones | 587 | 552 | 5 a 6 % |
| Evidencia, no instrucciones | 534 | 470 | 0 a 2 % |
| Proyectos | 1.231 | 1.163 | 9 a 10 % |
| Las otras herramientas | 491 | 450 | 3 a 4 % |

## Repeticiones

La repetición **literal** es una sola: la apertura del bloque es, palabra por palabra, las instrucciones del servidor (`opening`, `protocol.rs:57`, las saca del mismo archivo). Todo lo demás se repite con otras palabras: la misma idea dicha en el bloque y otra vez en un esquema o en un resultado. Se midió por secuencias de 5 palabras entre cada sección del bloque y cada herramienta; fuera de la apertura ninguna sección repite más del 10 %.

| Qué se dice | Dónde | Caracteres (es) | Cómo se repite |
|---|---|---|---|
| Apertura: buscar antes de responder, `search_project` antes de abrir archivos, cuándo guardar, qué no guardar, evidencia, conflictos, `synthesize` | instrucciones del servidor **y** bloque | 1.933 en cada lado | literal |
| `topic_key`: nombre estable, reemplaza la versión anterior, dónde se muestra | viñeta del bloque (1.080) y parámetro de `save_memory` (274 en en) | ~1.350 | paráfrasis; el bloque agrega lo de `topic=K` y de `all_projects`, que el esquema no dice |
| `user_stated` y `derived_from` | viñeta del bloque (570), sección «Conclusiones» (587), parámetros de `save_memory` (165 y 119 en en) y descripción de `synthesize` | ~1.300 | paráfrasis |
| `memory_type` y su lista de siete | «Tipo e importancia» (1.822), `enum` y descripción de `save_memory` (253 en en), `update_memory` (75) | ~2.300 | el `enum` repite la lista |
| `importance` | «Tipo e importancia», `save_memory` (101), `update_memory` (70) | ~400 | paráfrasis |
| Proyecto: `project_path`, `project`, `all_projects`, `new_project`, `to_project*` | sección «Proyectos» (1.231) y 6 herramientas (3.058) | ~4.300 | paráfrasis; cada herramienta repite su variante del mismo párrafo |
| `user_answered` y sus palabras en `reason` | apertura, `resolve_conflict` (143 y 83 en en), `update_memory` (151), `answer_suggestion` | ~900 | paráfrasis |
| Qué hacer ante cercanas: `resolve_conflict` con `supersedes` | apertura (3.er párrafo), descripción de `resolve_conflict` (497 en en) y **el resultado de `save_memory`**, que lo dice al momento: «close to earlier memories… If this save replaces one, call resolve_conflict» (`server.rs:403-407`) | ~1.100 | paráfrasis; es la regla que se dice tres veces, y la tercera llega cuando se necesita |
| Las otras herramientas y qué hace cada una | sección del bloque (491) y la descripción de cada una (213 a 368) | ~1.500 | paráfrasis |
| «Después de una compactación»: guardar un `session_summary` | sección del bloque (152) y la línea `compacted` de `session-start` (`hook.rs:221-238`, «el protocolo de memoria pide un `session_summary`…») | ~500 | paráfrasis; la línea del hook llega justo cuando corresponde |
| El resumen «abre el contexto de la sesión siguiente» | sección del bloque y el encabezado `LAST SESSION` del contexto | — | el bloque lo promete, el hook lo cumple |

La tabla dice qué es **candidato**, no qué se puede sacar: que dos textos digan lo mismo no prueba que el agente obre igual con uno solo (ver «Por qué la apertura no es gratis»).

## Qué recibe cada cliente

| Cliente | Instrucciones del servidor | Bloque en `CLAUDE.md` | Esquemas | `session-start` | Qué se puede sacar de qué lado |
|---|---|---|---|---|---|
| Claude Code con bloque `full` (el valor por defecto) | sí, 1.933 / 2.030 | sí, 11.271 / 10.691, con la apertura repetida | diferidos | sí, más el aviso del bloque viejo si el sello no coincide (`block_notice`, `hook.rs:147`) | La apertura está dos veces. Sacarla de las instrucciones deja sin reglas a los dos clientes de abajo; sacarla del bloque es lo que hace `short`. Las secciones 2 a 10 existen sólo aquí |
| Claude Code sin bloque (`--protocol off`: marcadores y sello sin texto, `setup.rs:689-697`; o sin `setup`) | sí | no | diferidos | sí | Sólo la apertura. Es el brazo `l0` de la campaña, que perdió E y G |
| Cursor (`clawg setup cursor`: servidor y hooks, «no rules file», `crates/clawg-cli/src/setup/cursor.rs:10`) | sí | **no existe**: no se escribe ningún archivo de reglas | no verificado | sí (`sessionStart`), con la línea de idioma | Sólo lee la apertura. Lo que se quite de las instrucciones le llega sin reemplazo, y lo que sólo está en el bloque nunca le llegó |

De ahí sale una regla de diseño: **la apertura es lo único que reciben los tres clientes**, así que no se puede achicar sin medir también a `l0`; **todo lo demás lo lee sólo Claude Code con bloque**, así que ahí se puede recortar sin dejar a otro cliente sin la regla. Nada de lo que está sólo en el bloque lo lee otro cliente.

## Por qué la apertura no es gratis

La repetición literal es tentadora: son unos 2.000 caracteres, el 18 % del bloque, y el agente ya los lee en las instrucciones. Es lo que hace `short`, y los datos de la campaña dicen que cuesta:

- `short` ahorró el 30 % de los tokens por sesión (T: 156.340 contra 111.482 sin la K; 187.750 contra 131.177 con ella) y perdió **B** (buscar antes de editar: 3 de 12 contra 7 de 12) y **J** (`resolve_conflict` por save con cercanas: 0,83 contra 0).
- `l0` tiene las instrucciones, es decir la misma apertura, y aun así pierde E (7 de 12), B (2 de 12) y G (8 de 18). Eso dice que las **mismas palabras en las instrucciones del servidor y en el `CLAUDE.md` no pesaron igual** en la campaña. Por qué, no está medido: una explicación es que el agente obedece el archivo de instrucciones del usuario más que las instrucciones de un servidor, pero es una hipótesis, no un dato.
- Lo que `short` conserva (las dos secciones del resumen) y lo que quita (la apertura y las otras ocho secciones) viajan juntos, así que los datos no dicen **cuál** de las partes quitadas sostiene B ni J. Lo único que el verify muestra es que, con `topic_key`, la versión revisada reemplaza sola a la primera y nadie llama a `resolve_conflict`; sin `topic_key`, tres de los seis saves salen con cercanas.

Por eso la regla de (b) es **no tocar la apertura en el primer brazo** y recortar sólo lo que viaja en las otras secciones.

**Cuánto vale cada carácter.** `short` quitó unos 9.660 caracteres de bloque (10.691 menos ~1.030) y bajó T en 44.858 tokens (156.340 menos 111.482): **unos 4,6 tokens de T por carácter de bloque**, en sesiones largas de unos 15 pedidos. Como regla de dedo, 1.000 caracteres de bloque son unos 4.600 tokens de T por sesión, el 3 % de `a3`. T suma entrada, caché leída y caché escrita sin ponderar, así que en dinero pesa menos (la caché leída cuesta una fracción); la regla sirve para ordenar enfoques, no para facturar.

## Enfoques

### Clase (a): no cambia el significado

| # | Qué | Caracteres | T por sesión (regla de 4,6, para las 4 herramientas habituales) | Riesgo |
|---|---|---|---|---|
| A1 | Quitar `$schema` de cada herramienta | 741 en las 13; 232 en las 4 | hasta ~1.000 | Bajo. La especificación de MCP toma 2020-12 cuando falta (a verificar contra la versión que implementa rmcp 3.9.0 antes de proponer) |
| A2 | Quitar `"default": null` | 510; ~150 en las 4 | hasta ~700 | Bajo: un parámetro ausente da lo mismo |
| A3 | Pasar `["string", "null"]` a `"string"` y sacar `null` de los `enum` | 238 | hasta ~600 | **Medio:** un cliente que valide el esquema y mande `null` explícito para un parámetro que no quiere pasar lo vería rechazado. El servidor sigue aceptándolo (`Option<T>`); se prueba con un test que mande `null` y con los clientes reales |
| A4 | Quitar `format` y `minimum: 0` | ~353 | hasta ~400 | Bajo; el `minimum` de `limit` lo valida el servidor de todas formas (a verificar) |

A1 a A4 son un solo cambio: **~1.490 caracteres sobre 19.600 (7,6 %)**, es decir ~370 tokens ÷ 4 si se cargan las 13, y de 2.500 a 3.000 de T en una sesión típica (1,6 a 1,9 % de `a3`; es una cota alta, porque las herramientas se cargan después del primer pedido). Se hace una vez, sobre el router (junto a `with_language`, `server.rs:599`, o en un paso que corra sobre `tool_router.list_all()`), vale para es y en, y no toca ni una descripción: `tools_es.rs` queda como está.

**Cómo se cierra:** `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs:3421` y `:3444`) cambian y se regeneran con `CLAWG_BLESS=1`. El diff tiene que ser sólo las claves quitadas: un test nuevo compara el contrato nuevo con el viejo después de borrar a mano `$schema`, `default: null`, `null` y `format`, y exige igualdad. Otro prueba que un parámetro opcional se deserializa ausente, con `null` y con valor (A3). No hay campaña: el agente lee las mismas palabras.

### Clase (b): cambia lo que el agente lee

| # | Qué | Caracteres (es) | T esperado | Riesgo |
|---|---|---|---|---|
| B1 | Bloque `lean`: la apertura **intacta**; las secciones 2 a 10 comprimidas conservando cada regla y sin repetir lo que dicen los esquemas (la lista de tipos, «Las otras herramientas», «Conclusiones» a una línea, «Proyectos» a dos frases). Conserva entera la parte de `topic_key` | de 11.271 a ~6.000 (−5.000) | ~−23.000 (−15 % de `a3`) | Medio: cada regla comprimida puede perder una conducta; se mide con E, B, G, J |
| B2 | B1 sin las secciones «Resumen de sesión» y «Después de una compactación» (−966) | ~5.000 | ~−27.000 (−17 %) | **Depende de memory-links (134):** hay que saber qué aporta el `session_summary` antes de sacar la forma que el bloque le da. S fue 0 de 9 en los tres brazos, pero son tareas con `-p`: no prueba que sobre. Y el relevo de `a3` aprobó 3 de 3 porque los hechos se guardaron sueltos, no porque el resumen sobrara |
| B3 | Mover la plantilla del resumen de donde se paga en cada pedido a donde se paga al usarla: una frase en el parámetro `memory_type` de `save_memory` (entra sólo al cargarla) y/o en la línea `compacted` de `session-start`, que ya llega justo después de compactar (`hook.rs:221`) | −742 del bloque, +~400 en un esquema o en el hook | ~−3.000 | Bajo a medio: no pierde la regla, la pasa a un canal que llega cuando hace falta. La línea `compacted` dice hoy «el protocolo pide un `session_summary`»: si la sección sale del bloque, la línea tiene que decir el qué por sí sola |
| B4 | Achicar la apertura (por ejemplo, el párrafo de conflictos, que el resultado de `save_memory` ya repite al momento, `server.rs:403`) | hasta −600, y lo mismo en las instrucciones | hasta −2.800 | **Cambia a los tres clientes y el contrato fijado.** Sólo después de B1, como segundo brazo; hay que medirlo también contra `l0`. Los tests que fijan las frases de la apertura (`server.rs:3005-3037`) cambian |
| B5 | A5: un solo texto largo para la familia de proyecto (3.058 caracteres en 6 herramientas) y descripciones cortas en las otras cinco | hasta ~1.000 en el esquema | hasta ~3.000 | No es clase (a): cambia palabras. Va con B1 o después |
| B6 | Sacar la apertura del bloque, o dejar `off` | −2.000 / −11.000 | −9.000 / −51.000 | **Descartados por la campaña** (B y J con `short`; E y G con `off`). Se anotan para que no vuelvan como ideas nuevas |

`full` queda como control. `lean` es una variante nueva (la 4.ª, junto a `full`, `short` y `off`), no un reemplazo del texto de `full`, hasta que la campaña decida.

**Orden recomendado:**

1. **A1 a A4**, en un solo commit, con los contratos regenerados. Es lo único que se puede cerrar hoy sin campaña.
2. **El paso 0 de medición** (ver «Cómo se mide»): cuántas herramientas carga una sesión real y el tamaño real de `session-start`. No bloquea al punto 1.
3. **B1**, como variante `lean`, medida contra `full`.
4. **B2 y B3**, cuando memory-links haya dicho qué mide el `session_summary`. Pueden correr en la misma campaña que B1 como segundo brazo (`lean` sin las secciones del resumen) para no pagarla dos veces: si B1 pasa y B2 no, queda B1.
5. **B4 y B5**, sólo si `lean` pasó y todavía se quiere ir más abajo.

## Cómo se mide

### Clase (a)

- **Qué baja:** caracteres del contrato (el script de esta exploración, que va a ser un test) y tokens con el contador que ya usa clawg-eval, antes y después, en es y en. El objetivo es ≥ 1.400 caracteres menos en las 13 herramientas. Si A3 se descarta por riesgo, el umbral baja a ~1.250 (A1, A2, A4).
- **Umbral de cierre:** el diff de los dos archivos del contrato son sólo las claves del ruido (lo comprueba un test) y los tests de deserialización pasan.
- **Paso 0, sin campaña:** contar en los transcripts de la campaña cuántas herramientas de clawg carga cada sesión de `a3` y en qué pedido, y comparar `a3` con `a3-eager`. Dice cuánto valen realmente los ~1.490 caracteres.

### Clase (b)

Un brazo nuevo de `agent-eval`, `a3-lean`, que se agrega donde está `A3Short` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:30`, `:84`, `:114` y `:316`, con `protocol: Some(Variant::Lean)`), contra `a3` (`full`), con las mismas tareas de la campaña (R, D y la K larga), 3 corridas por celda, las mismas reglas de ruido y el mismo build. Valen las reglas de [D6](../archive/2026-11-17-protocol-block/design.md#d6-la-regla-de-decisión-del-valor-por-defecto) tal como están: las seis condiciones sobre E, B, G, S, J y T, con sus márgenes (E, B, G, S: no más de 1 por debajo de `a3`; J: no más de 0,5 por encima; T: menor).

Dos agregados que se fijan **antes** de correr, como D6:

- **Un umbral de ahorro.** D6 pide sólo T(V) < T(`a3`). Para que valga el riesgo de tocar el texto, `lean` tiene que bajar T al menos un 10 %: B1 quita la mitad del bloque y se espera un 15 %, así que menos del 10 % es ruido de la campaña. `lean` pasa sólo si cumple las seis y además T(`lean`) ≤ 0,9 × T(`a3`).
- **J se mira también por `topic_key`.** El verify dijo por qué `short` perdió J: sin `topic_key` en el save hay cercanas y el agente resuelve a mano. En `lean` se cuenta cuántos de los saves de D llevan `topic_key`, aparte de J: si J falla y los `topic_key` también, la causa es la viñeta comprimida.

**Métricas:** las de D6 (E, B, G, S, J y T) y, aparte, el gasto en USD de cada brazo, porque T no pondera la caché leída.

**Costo.** La campaña de protocol-block gastó 5,46 USD en 18 corridas (~0,30 USD cada una). Un brazo nuevo de 12 corridas (R continua 3, R nueva 3, D 3, K 3) son ~3,7 USD; con una segunda variante (B2), ~7,3; y si hay que volver a correr `a3` por un cambio de build, otros ~3,7. El tope de 15 USD de D6 alcanza. Es una campaña paga: la autoriza la persona y corre después de publicar, con la versión instalada, como la de protocol-block.

**Qué no puede probar.** Con 12 corridas por brazo la campaña sólo descarta una pérdida de más de una corrida, y que S valga 0 en todos los brazos ya dijo que las tareas con `-p` no miden el resumen. Tampoco mide el relevo de un `session_summary` real ni sesiones de varios días.

## Qué tocaría

| Archivo | Para qué | Clase |
|---|---|---|
| `crates/clawg-cli/src/mcp/server.rs` (cerca de `with_language`, `:599`; tests `:3421` y `:3444`) | quitar el ruido del esquema, en es y en; tests nuevos de equivalencia y de `null` | (a) |
| `crates/clawg-cli/src/mcp/agent_contract.json` y `agent_contract.es.json` | se regeneran con `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` y `... the_spanish_agent_contract_matches_its_file`; se revisa el diff y se vuelve a correr sin la variable | (a); (b) sólo si cambia la apertura |
| `crates/clawg-cli/src/mcp/protocol.md`, `protocol.es.md` | `full` no cambia; el texto de `lean` va en archivos nuevos o en secciones marcadas, como las de `SHORT_SECTIONS` (`protocol.rs:110`) | (b) |
| `crates/clawg-cli/src/mcp/protocol.rs` | `Variant::Lean` (`Variant::ALL`, `parse`, `as_str`) y su texto; los tests de `server.rs:3046-3110` (`the_protocol_describes_the_session_summary`, `the_protocol_names_only_what_exists`) tienen que valer para `lean` | (b) |
| `crates/clawg-cli/src/setup.rs` (`assemble`, `:689`; `freshness`, `:735`) | el sello `protocol=lean`, la elección con `--protocol` y que `freshness` compare contra el texto de esa variante | (b) |
| `crates/clawg-cli/src/doctor/checks.rs` (`:435-500`) | el chequeo `protocol_block` usa el mismo `freshness`; sólo cambia si cambia la lista de variantes | (b) |
| `crates/clawg-cli/src/hook.rs` (`:221-238`, la línea `compacted`) | sólo con B2 o B3: que diga el qué sin remitir al protocolo | (b) |
| `crates/clawg-eval/src/bin/agent-eval/arm.rs` | el brazo `a3-lean` | medición |
| `openspec/specs/setup`, `specs/doctor`, `specs/hooks`, `specs/mcp`, `specs/eval` | la variante, el sello, el aviso y el brazo, al archivar | — |
| `docs/reference/commands.md` (`--protocol full\|short\|off`) y `docs/reference/daemon.md` | la variante nueva, si se agrega | (b) |

**Una consecuencia a decidir antes de apurarse.** El sello compara por **texto**: un bloque instalado que difiere del que escribe el binario es `Stale`, y el hook le dice al agente en cada inicio de sesión que avise a la persona (`block_notice`, `hook.rs:147`; unos 330 caracteres cada vez). Cambiar `protocol.md` o `protocol.es.md` deja todos los bloques instalados `Stale` hasta que cada usuario corra `clawg setup claude-code`. Por eso `lean` es una variante nueva y no un cambio de `full`: quien tiene `full` sigue `Current`. Si la campaña eligiera `lean` como valor por defecto, el aviso se paga una vez por máquina, que es lo que el sello de protocol-block quiso que fuera. Cambiar la apertura (B4) cambia además las instrucciones de todo cliente y el contrato fijado, no sólo el bloque. Un sello `protocol=lean` que un binario anterior no reconoce se lee como bloque `Stale` (a verificar en `Stamp::parse`).

## Decisiones abiertas

| # | Pregunta | Opciones | Recomendada |
|---|---|---|---|
| D1 | ¿La clase (a) incluye A3 (`null` fuera del tipo)? | Sí / no, sólo A1, A2 y A4 | Sí, con el test de `null` explícito y probando Claude Code y Cursor; si algún cliente falla, A3 queda fuera y el cambio sigue valiendo ~1.250 caracteres |
| D2 | ¿`lean` es una variante nueva o reemplaza el texto de `full`? | Variante nueva / reemplazo | Variante nueva, por el sello y para que quien tiene `full` no reciba el aviso de bloque viejo |
| D3 | ¿Se toca la apertura? | No / sí | No en el primer brazo (ver «Por qué la apertura no es gratis»). B4 sólo si `lean` pasa |
| D4 | ¿Se sacan las secciones del resumen? | Sacarlas (B2) / moverlas a la herramienta o al hook (B3) / dejarlas | Mover la plantilla (B3) y esperar a memory-links (134) para saber si el resumen hace falta siquiera; no se saca nada antes |
| D5 | ¿Cuál es el umbral de ahorro? | 5 % / 10 % / 15 % de T | 10 %: debajo, el ahorro no justifica ni el riesgo ni la campaña |
| D6 | ¿`a3-lean` es un brazo del arnés o se corre a mano con `--protocol lean`? | Brazo / a mano | Brazo, para que las reglas de D6 se apliquen igual |
| D7 | ¿Cuánto del esquema repetido (B5) se hace? | Nada / la familia de proyecto / todo | La familia de proyecto, **después** de B1, porque cambia palabras y se mide en la misma campaña |
| D8 | ¿Qué se mide antes de decidir? | — | El paso 0 y el tamaño típico de `session-start` (los `bytes` del registro, `hook_log.rs:69`); el segundo lee el perfil real y pide autorización |

## Hecho: la clase (a)

A1 a A4 están en `develop` (D1 quedó en sí: A3 entra). `strip_noise` (`crates/clawg-cli/src/mcp/schema.rs`) limpia cada `inputSchema` una vez, en `served_router` (`server.rs`), antes de que `with_language` ponga las descripciones en español, y `memory_type_enum` ya no agrega `null` al `enum`. Las 13 herramientas bajaron de 18.940 a 17.084 caracteres en inglés y de 19.616 a 17.760 en español: **1.856 menos (9,5 %)**, más que los ~1.490 estimados porque también salieron el `null` del `enum` de `memory_type` y los `minimum: 0`. Se comprobó con un script que el contrato viejo, después de borrarle a mano esas claves, es igual al nuevo, y que las instrucciones no cambiaron. Lo fijan `the_served_schemas_carry_no_generator_noise` y `an_optional_argument_reads_the_same_absent_or_null`, y el requisito «Los esquemas sin el ruido del generador» de [`specs/mcp`](../../specs/mcp/spec.md).

Lo que no se verificó: un cliente que valide el esquema y mande `null` explícito (Cursor no se probó). El servidor lo sigue aceptando; lo que cambia es que el esquema ya no lo ofrece.

## Riesgos

1. **Un recorte del bloque pierde una conducta que el arnés no ve.** B y J son las que la campaña vio caer; otras (`new_project`, `user_answered`, `topic=K`) no tienen métrica. Mitigación: conservar entera cada regla con consecuencia sobre los datos y recortar sólo explicaciones.
2. **El ahorro es menor al esperado.** La caché leída cuesta una fracción de la entrada nueva y T no la pondera: un ahorro del 15 % en T puede ser bastante menos en dinero. La campaña da los dos números.
3. **El ruido de esquema vale menos de lo que parece** si el agente carga pocas herramientas, o más si Cursor las manda todas. Está sin medir (paso 0).
4. **A3 rompe a un cliente que valida `null`.** Se prueba antes de quitarlo; con duda, queda para después.
5. **Todo bloque instalado queda `Stale` si cambia `full`.** Cada sesión recibe ~330 caracteres de aviso hasta que se corra `setup`. Se evita con la variante nueva.
6. **La apertura está atada a `the_instructions_fit_older_clients`** (`server.rs:2951`): las instrucciones tienen que seguir en ≤ 2.048 caracteres (hoy 2.030 en inglés). Llevarla a los 4.096 que corta el cliente actual deja afuera a los clientes anteriores a la 2.1.296 (anotado en D6 de protocol-block).
7. **Depende de otra exploración.** Si memory-links (134) concluye que el `session_summary` es lo que hace útil el relevo en sesiones largas, B2 queda descartado aunque la campaña diga que ahorra; si concluye que no aporta, B2 es el recorte más barato (966 caracteres sin riesgo para una regla de datos).
8. **Se mide en inglés, se usa en español.** Los 3.550 a 4.000 tokens medidos son del bloque en español; el arnés de la campaña usa el protocolo en inglés. Un recorte en inglés ahorra en español un número parecido de caracteres y más tokens. Sin una campaña en español, `lean` en español hereda el resultado por analogía.
