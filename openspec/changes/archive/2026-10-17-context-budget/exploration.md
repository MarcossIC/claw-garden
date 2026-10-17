# context-budget — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (24 y 28 enriquecen la prioridad cuando existan; 25 lo necesita) | exploración (borrador) |

**Tesis:** `recall_context` no tiene un tope de tamaño que se cumpla siempre, y cuando el embedder no está listo devuelve éxito con sólo el workspace, sin decir que faltan los recuerdos. Un presupuesto duro, con prioridad entre bloques y un aviso explícito de estado, vuelve el costo predecible y quita una conclusión falsa ("no hay memorias"). Importa más cuanto más automática sea la inyección de contexto: un costo variable se paga en cada sesión.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `recall_context` no tiene presupuesto duro de tamaño | **Cierta.** No hay parámetro de tamaño en la petición (`ContextRequest`, `crates/clawg-cli/src/daemon/api.rs:340-345`) ni en la herramienta (`RecallContextArgs`, `crates/clawg-cli/src/mcp/server.rs:167-175`). |
| El tope del workspace no es duro porque las preferencias nunca se recortan | **Cierta, y hay una fuga más.** `AUTO_BYTE_CAP = 1500 * 4` bytes (`crates/clawg-core/src/memory/workspace.rs:18-19`) recorta primero los skills y luego las decisiones; las preferencias no (`workspace.rs:392-402`). Además la sección USER, escrita a mano, se entrega entera y sin tope (`prompt_builder.rs:436-445`), y el tope no cuenta ni los encabezados. |
| Mientras el embedder carga, `recall_context` devuelve sólo el workspace y únicamente registra un `warn` | **Cierta.** Sin embedder `hits` queda vacío y sólo hay un `log::warn!` (`prompt_builder.rs:173-176`). Peor: el mismo silencio ocurre si el embedder falló (`prompt_builder.rs:149-172`) o si la búsqueda dio error (`prompt_builder.rs:162-165`). El handler pasa `embedder.get().cloned()`, que es `None` en `loading` y en `failed` (`api.rs:849`). |
| `search_memory` sí lo dice | **Cierta.** `ready_embedder` responde 503 `embedder_not_ready` o `embedder_failed` (`api.rs:756-763`) y el puente lo traduce a un texto accionable (`server.rs:415-431`). |
| Con workspace vacío y embedder cargando se ve algo engañoso | **Cierta y peor de lo planteado.** Un prompt vacío se renderiza como `no context yet` (`crates/clawg-cli/src/mcp/tools.rs:285-291`), que un agente lee como "no hay memorias". `ContextBody` sólo lleva `prompt` y `unreviewed` (`api.rs:477-483`): el puente no tiene de dónde sacar el estado. |
| El repo cuenta tokens en agent-view | **Cierta, con otro tokenizador.** `rag-eval --agent-view` cuenta con el `tokenizer.json` del modelo de embedding (`EvalTokenCounter::real`, `crates/clawg-core/src/eval_api.rs:595-625`; uso en `agent_view/runner.rs:177,186`). No es el tokenizador de Claude. El resultado guarda `mean_tokens` y `max_tokens` por herramienta. |
| Línea base de ~1.086 tokens medios | **Cierta, pero con un corpus chico.** `recall_context`: media 1.086,4, máximo 1.408 (`docs/eval/results/2026-10-15-real-project-indexing-relcut-agent-view.json:73-74`). Son 46 casos sobre filas cortas. No dice nada sobre una base de meses. |

## Problema (verificado contra el código)

### De qué se compone `recall_context`

`build_memory_prompt` arma, en este orden, cinco bloques (`prompt_builder.rs:236-282`):

| Bloque | Qué entra | Tope hoy |
|---|---|---|
| `=== WORKSPACE ===` | Sección AUTO (preferencias con importancia >= 8, decisiones >= 8, hasta 5 skills) más la sección USER | AUTO: 6.000 bytes salvo preferencias (`workspace.rs:392-402`). USER: ninguno |
| `RECENT CONTEXT` y `LONG-TERM KNOWLEDGE` | Hasta `K_FINAL = 10` hits entre ambos, ya sin las filas del workspace (`prompt_builder.rs:42,178-179`) | 10 filas, **contenido completo** de cada una (`prompt_builder.rs:344`) |
| `ACTIVE CONFLICTS` | Hasta 5 pares y un conteo de los omitidos (`MAX_CONFLICT_PAIRS`, `prompt_builder.rs:44,234`) | 5 pares; el texto se repite si el lado no está en los hits |
| `SHARED` | Una nota fija si algún hit es de otro agente (`prompt_builder.rs:276-282`) | constante |

Después el puente agrega la línea de síntesis si hay memorias sin revisar (`tools.rs:272-278`).

### Tamaño: lo medido y lo que se estima

- Medido: media 1.086 y máximo 1.408 tokens en el corpus de evaluación (arriba).
- **Cota superior teórica (estimación, no medida).** Un hit es una fila, y una fila tiene hasta `CHUNK_MAX_TOKENS = 480` tokens (`crates/clawg-core/src/memory/chunker/mod.rs:15`). Diez hits suman hasta unos 4.800 tokens. El AUTO suma hasta 1.500 más las preferencias. Los conflictos pueden agregar hasta cinco pares con texto. La sección USER no tiene tope. El orden de magnitud supera los 10.000 tokens, y no lo acota nada.
- El límite del cliente: Claude Code advierte arriba de 10.000 tokens de salida de una herramienta MCP y limita a 25.000 por defecto (https://code.claude.com/docs/en/mcp). Un resultado que lo supere se guarda en un archivo y el agente ve una ruta, no el contexto.

### Por qué ahora

- Una inyección automática al iniciar la sesión (cambio 25) paga el tamaño completo en **cada** sesión. Un contexto que varía entre 300 y 10.000 tokens no se puede planificar.
- El hook de `SessionStart` tiene su propio tope: su stdout se limita a 10.000 caracteres, y si lo supera se guarda en un archivo y el agente ve una vista previa de 2.000 caracteres (https://code.claude.com/docs/en/hooks). Un presupuesto de bytes menor a ese tope evita que la parte más valiosa quede fuera.

### Orden de corte y conflictos con el diseño actual

- Los conflictos se piden **sobre las filas mostradas** (`shown`, `prompt_builder.rs:211-234`) para que el bloque crezca con la pregunta y no con la base. Si un presupuesto recorta hits **después** de buscar los conflictos, un conflicto puede nombrar una fila que ya no está. Si recorta **antes**, se pierde el conflicto de la fila que se cortó. Hay que decidir el orden (ver decisiones).
- `rank_conflicts` ordena por la posición del hit más temprano (`prompt_builder.rs:357-369`): un par que sólo toca el workspace va último. Con la prioridad "conflictos primero" hay que decidir si ese orden cambia.

## Lo que ya existe y se reutiliza

- Recorte por prioridad dentro del workspace (`render_auto_section`, `workspace.rs:388-414`) y la regla "se informa lo que se entregó" (`EmittedIds`, `workspace.rs:192-224`): lo que se corta no cuenta como entregado.
- La conversión de 4 bytes por token ya usada en el repo (`workspace.rs:17-19`; `chunker/token_splitter.rs:23`).
- El patrón de tope con conteo de omitidos: `rank_conflicts` y `format_conflict_block` (`prompt_builder.rs:357-408`).
- `ready_embedder` y el estado del embedder (`api.rs:756-763`; `EmbedderStatus` en `crates/clawg-core/src/memory/bootstrap.rs`).
- El texto accionable de los estados (`server.rs:415-431`).
- `bucket_by_recency` (siete días) para separar reciente de largo plazo (`prompt_builder.rs:411-423`).
- `rag-eval --real --agent-view` y sus campos `found` y `tokens` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:69-73,170`).
- `ContextRequest` no usa `deny_unknown_fields` (`api.rs:339-345`): un campo opcional nuevo es aditivo y un daemon viejo lo ignora.

## Enfoques posibles

### Unidad del presupuesto

| | Bytes | Tokens |
|---|---|---|
| Puro y testeable | Sí: una función de longitudes | Necesita un tokenizador; el del modelo de embedding no es el de Claude |
| Coherente con el repo | Sí: el tope de workspace ya es bytes (`workspace.rs:18-19`) | Sólo en eval |
| Coherente con el cliente | Sí: los topes de Claude Code son en caracteres (hooks 10.000, resultado MCP 50.000) | La advertencia del cliente es en tokens |
| Costo en ejecución | Nulo | Tokenizar en el daemon en cada llamada |

**Recomendación:** bytes, documentados como "aprox. 4 por token". El reporte de eval sigue en tokens.

### E1. Presupuesto de bytes con prioridad entre bloques (recomendado)

Un parámetro opcional `budget` (bytes) en `POST /v1/context` y en `recall_context`, con un default y un máximo. La asignación es una función pura que recibe los bloques ya armados y devuelve el texto final:

1. Reservar primero el pie (notas fijas: `SHARED`, la línea de síntesis, el aviso de corte).
2. Llenar por prioridad: conflictos de alto riesgo, workspace, última sesión (cuando exista 28), reciente, largo plazo.
3. Dentro de cada bloque se mantiene el orden actual por tier e importancia.
4. Cortar siempre en un límite de línea o de carácter UTF-8, y marcar el corte con `[truncated]` y cómo pedir el resto (por ejemplo `search_memory` o `read_memory` con el id).
5. Nombrar por id lo que el recorte dejó afuera, en una sola línea del pie y con un tope de ids, por ejemplo `omitted (budget): #12 #40 #41 — read_memory <id>`. El agente puede pedir exactamente eso sin repetir la búsqueda, y sabe que hubo un recorte: un contexto más corto no significa que no haya más memorias.
6. Si el primer hit de un bloque no entra entero, se recorta en lugar de saltarlo. Un bloque vacío por un único hit largo dice menos que uno con el hit recortado y su id.
7. Si lo no recortable (preferencias y sección USER) solo ya supera el presupuesto, la respuesta lo dice con cuánto haría falta, en lugar de recortarlo en silencio o de ignorar el tope sin avisar.

- A favor: costo predecible, pura y testeable, aditiva en el protocolo HTTP.
- En contra: toca el armado del prompt, que `context-density` ya afinó. Con el default por encima del máximo medido (1.408 tokens) no debería cambiar los resultados del eval.

### E2. Sólo el aviso de estado, sin presupuesto

`ContextBody` gana un campo con el estado del embedder y el puente antepone una línea cuando no es `ready`.

- A favor: chico, resuelve la conclusión falsa.
- En contra: no resuelve el tamaño. Se puede hacer **antes** de E1 como tarea independiente dentro del mismo cambio.

### E3. Resumen compacto del historial viejo (opcional)

Conteo por período más unos pocos títulos, en un presupuesto fijo pequeño.

- A favor: da orientación sin traer filas completas.
- En contra: hoy no hay títulos (el contenido es texto libre; ver el cambio 22 sobre títulos en la primera línea), así que habría que inventar el título, y el conteo por período es una consulta nueva. **Recomendación: descartar en este cambio** y reabrir cuando 22 haya cambiado cómo se guarda y exista 28.

**Recomendación:** E2 y E1 en el mismo cambio, en ese orden; E3 fuera.

## Aviso explícito de estado

- El daemon devuelve el estado del embedder en la respuesta (`ready`, `loading`, `failed` con causa y si reintenta), reusando la misma fuente que `ready_embedder` (`api.rs:756-763`).
- El puente antepone una frase sin importar si hay workspace: *"memories are not searchable yet: the embedding model is still loading; only the workspace is shown"* (y la variante `failed`, con la causa).
- **No** conviene cambiar el status HTTP a 503 en `/v1/context`: hoy el workspace llega, y es información útil (preferencias). El aviso va en el cuerpo.
- Un daemon viejo no manda el campo: el puente trata su ausencia como "desconocido" y no inventa un aviso (patrón de `unreviewed`, `client.rs:235-238`).
- Los otros dos silencios (fallo de `embed` y fallo de la búsqueda, `prompt_builder.rs:162-170`) quedan sin aviso a menos que se decida lo contrario.

## Descartado

| Opción | Razón técnica |
|---|---|
| Subir o bajar `K_FINAL` para controlar el tamaño | Cambia cuántos hits ve el agente en todos los casos y mueve las métricas del eval. Un tope por bytes recorta sólo cuando hace falta. |
| Recortar las preferencias del workspace para cumplir el presupuesto | `workspace.rs:392-394` lo prohíbe por diseño: una preferencia perdida cambia en silencio cómo se comporta el modelo durante toda la sesión. El presupuesto debe ser más grande que lo no recortable, o dejar el exceso a la vista. |
| Contar tokens en el daemon con el tokenizador del modelo de embedding | No es el tokenizador de Claude, así que no da exactitud. Cuesta una tokenización por llamada y vuelve el recorte dependiente de un archivo que puede faltar. |
| Truncar el texto final con un corte ciego en el puente | Parte una línea o un carácter multibyte, y puede cortar la nota de conflicto. El corte tiene que respetar bloques y estar marcado. |
| Devolver 503 si el embedder no está listo | Se pierde el workspace. Ver más arriba. |
| Presupuesto por bloque con porcentajes fijos | Un bloque vacío desperdicia su parte. La prioridad con relleno sobrante es más simple de razonar. |

## Alcance candidato

1. Función pura `allocate(blocks, budget)` que reserva el pie, llena por prioridad, corta en un límite válido, marca el corte, nombra por id lo omitido y recorta el primer hit de un bloque en lugar de saltarlo.
2. Campo opcional `budget` en `ContextRequest` y en `RecallContextArgs`, con default y máximo; validación del rango.
3. Campo de estado del embedder en `ContextBody`, y la frase en `format_recall`.
4. Orden de conflictos y recorte definido (ver decisiones).
5. Actualizar la descripción de `recall_context` (hoy: "costs several times the tokens of `search_memory`", `server.rs:826-830`).
6. Reportar `rag-eval --real --agent-view` antes y después.
7. `CLAUDE.md` del repo: parámetro nuevo en la API HTTP y en la herramienta.

### No-alcance

- Compresión o resumen del historial (E3).
- Cambiar `K_FINAL`, el ranking, la ventana de siete días o el tope AUTO.
- Presupuestar `search_memory` o `search_project`.
- Prioridades que dependen de 24 (conflictos de alto riesgo) y 28 (última sesión): el diseño las deja como bloques opcionales vacíos hasta que existan.
- Tope de la sección USER del workspace (es texto del usuario; decisión abierta).

## Decisiones abiertas

Son del usuario.

- [ ] **Unidad.** ¿Bytes (recomendado) o tokens?
- [ ] **Default y máximo.** El máximo medido es 1.408 tokens (~5.600 bytes). ¿Default de unos 8.000 bytes (por debajo del tope de 10.000 caracteres del hook) y máximo de 24.000? Los números salen de las fuentes citadas; la elección es tuya.
- [ ] **Qué pasa si el contenido no recortable (preferencias y sección USER) supera el presupuesto.** ¿Se entrega entero y se marca que se superó, o se recorta la sección USER? Recortar USER descarta texto escrito a mano.
- [ ] **Orden entre búsqueda de conflictos y recorte.** ¿Conflictos sobre los hits antes del recorte (puede citar una fila cortada) o después (puede perder un conflicto)?
- [ ] **Prioridad actual de conflictos.** Antes de 24 no hay "alto riesgo": ¿se trata como prioridad máxima todo conflicto, o se difiere esa prioridad a 24?
- [ ] **Avisos de estado.** ¿Sólo `loading` y `failed`, o también cuando `embed` o la búsqueda fallan?
- [ ] **Texto del aviso y del corte.** Idioma (inglés, igual que las herramientas) y redacción.
- [ ] **Resumen del historial (E3).** ¿Descartado aquí, o evaluado en un cambio propio?

## Cómo se mide

Tests RED primero:

1. `allocate` es pura: con bloques conocidos y un presupuesto, el texto nunca lo supera y respeta el orden de prioridad.
2. El pie se reserva primero: con un presupuesto mínimo la nota de corte sigue ahí.
3. El corte cae en un límite UTF-8 con contenido multibyte, y no parte una línea de hit.
4. El corte se marca con `[truncated]` y dice cómo pedir el resto.
5. Una base con muchas filas largas y preferencias de más de 6.000 bytes: hoy el contexto supera cualquier tope; con el presupuesto, no (salvo la regla elegida para lo no recortable).
6. Embedder `loading` y `failed`: `recall_context` empieza con la frase de estado, aunque el workspace tenga contenido y aunque no lo tenga (hoy devuelve `no context yet` o sólo el workspace sin aviso).
7. Un daemon viejo sin el campo de estado no produce un aviso inventado, y un `ContextRequest` sin `budget` se comporta como hoy.
8. El máximo del presupuesto se aplica: un valor mayor se recorta y uno de cero o negativo se rechaza.
9. Conflictos: con un hit recortado, el bloque no cita un id que no se imprimió.
10. Lo que el presupuesto dejó afuera aparece por id en el pie, y `read_memory` sobre esos ids lo devuelve. Sin recorte, no hay línea.
11. Un primer hit más largo que lo que queda del presupuesto se recorta y no desaparece.
12. Con preferencias que superan el presupuesto, la respuesta dice cuánto haría falta.

`rag-eval`: **obligatorio**, porque se toca el contexto. Correr `rag-eval --real --agent-view` (el modo existe en `rag-eval/cli.rs:69-73`) antes y después, y reportar `found` y `tokens` de `recall_context`. Resultado esperado con el default alto: `found` sin cambios (39/46) y tokens sin aumento. Con un presupuesto bajo forzado en una corrida aparte, medir cuántos casos se pierden: eso calibra el default. El corpus de eval es chico, así que la prueba del recorte real es el test 5, no el eval.

No cambia esquema, ids, estados ni contrato de embedding: no hace falta test de base sembrada. El campo nuevo de la API es aditivo en los dos sentidos (cliente nuevo con daemon viejo y al revés), y eso se cubre con el test 7.

## Riesgos

- **Perder la preferencia importante.** Si el presupuesto recorta algo que `workspace.rs` jamás recortaba, se rompe esa garantía. Por eso lo no recortable se decide explícitamente.
- **Presupuesto mal calibrado.** Muy bajo, el agente pierde recuerdos que antes veía (`found` baja). El eval lo mide sobre un corpus chico; la base real de un usuario es otra cosa.
- **Conflictos huérfanos o perdidos** según el orden elegido (ver decisiones).
- **Aviso que se desfasa.** El aviso depende de que el campo del daemon refleje el mismo estado que `ready_embedder`. Una segunda fuente de verdad se desviaría. Tiene que leer la misma.
- **Interacción con 25.** Si el hook inyecta el contexto con un presupuesto, el default tiene que quedar por debajo de 10.000 caracteres o el cliente lo manda a un archivo (https://code.claude.com/docs/en/hooks). Si el default de 23 y el tope del hook se desalinean, el agente ve una vista previa de 2.000 caracteres.
- **Estimación de tamaño sin medir.** La cota teórica de más arriba es una estimación. Antes del diseño conviene medir el contexto de una base real con autorización.
