# context-density — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | — | exploración |

**Tesis:** `recall_context` gasta dos tercios de sus tokens en el bloque de conflictos. Ese bloque repite completo el contenido que el mismo prompt ya mostró, y es el único sin techo. Además le manda al modelo marcadores y encabezados vacíos del workspace, aunque el comentario del código dice que los saca. No se trata de resumir: ningún original se reemplaza ni se recorta. Se trata de dejar de repetir y de mandar ruido.

## De dónde sale

Dos ideas de diseño de contexto para agentes aplican acá:

- **Referencias en vez de copias.** El contexto lleva identificadores cortos que apuntan a lo ya registrado, por ejemplo `[DEC-042] … (ref: …)`, en lugar del texto repetido (PROV-AGENT).
- **Markdown denso:** una línea por registro con sus atributos, sin repetir etiquetas en cada ítem (Improving Agents, TianPan).

Lo que no aplica, y por qué, está en el README, en "Ideas evaluadas".

## Estado actual (verificado)

### Bloque ACTIVE CONFLICTS

- Un par entra si **cualquiera** de sus lados está entre las filas mostradas, que son los hits más el workspace (`prompt_builder.rs:198-203`, `:400-401`).
- Cada par repite el contenido **completo** de las dos filas y una NOTE de dos líneas (`prompt_builder.rs:309-320`). Si el lado mostrado ya está en RECENT o LONG-TERM, su texto aparece dos veces en el mismo prompt.
- Una fila nueva en disputa contra varias viejas genera un par por cada vieja, y repite el texto nuevo en cada uno (join en `:394-395`).
- La consulta no tiene `LIMIT` ni `ORDER BY` (`:390-401`). Es el único bloque sin techo: los hits se cortan en `K_FINAL = 10` (`:32`, `:157`) y el workspace tiene un tope de unos 1.500 tokens.
- Las líneas de hits no llevan id (`:300-306`), pero los conflictos sí (`memory:12`). Para relacionar un conflicto con lo que leyó arriba, el agente tiene que comparar textos.

### Encabezado de cada hit

- Tiene la forma `[fecha] SOURCE: {kind:?} | TYPE: {tipo}` seguida de `CONTENT: {texto}` (`prompt_builder.rs:300-306`).
- En producción `SOURCE` siempre vale `Memory`, porque las tablas episodic y procedural no tienen escritor (ver knowledge-synthesis).

### Workspace

- El comentario dice "Strip structural markers — emit only the payload content to the LLM" (`prompt_builder.rs:109`). Pero `strip_empty_workspace_markers` devuelve el archivo entero apenas hay un bullet o texto en USER (`:345-370`).
- Por eso llegan al modelo `<!-- AUTO-START -->`, `<!-- AUTO-END -->`, `<!-- USER-START -->` y `<!-- USER-END -->`, y también los tres encabezados aunque estén vacíos (`workspace.rs:388-407`). En producción `## Procedural Skills` está siempre vacío.
- Los marcadores sí hacen falta **en el archivo**, porque la regeneración los usa para encontrar la sección USER y preservarla (`split_existing` en `workspace.rs:416`, `assemble_file` en `:475`). El arreglo va en lo que se le entrega al modelo, no en el archivo.

### Lo que mide el eval

Datos de `rag-eval --real --agent-view`, archivo `docs/eval/results/2026-10-12-real-lines-conflicts-shown-agent-view.json` (46 casos, corpus de 48 filas):

| Sección | Presente | Tokens medios cuando aparece | Parte del total |
|---|---|---|---|
| RECENT CONTEXT | 46/46 | 1.130 | 35 % |
| ACTIVE CONFLICTS | 45/46 | 2.112 | **65 %** |
| Total `recall_context` | — | 3.196 (máx. 4.625) | — |

En `found` (la fila esperada queda visible) hay 41 de 46 casos.

Límites de la medición:

- Los tokens salen del tokenizer de e5 (`eval_api.rs:567-572`), no del de Claude. Sirven para comparar corridas, no como costo absoluto.
- El corpus es chico y denso. Que haya conflictos en 45 de 46 casos probablemente exagera la proporción de una base real. La duplicación, en cambio, es estructural y no depende del corpus.
- LONG-TERM y WORKSPACE no aparecen en el eval: el corpus es reciente y no tiene filas tier 1. Su costo no está medido.
- `found` sólo cuenta los bloques `CONTENT:` y los bullets del workspace. Para el parser, las líneas `OLD:` y `NEW:` son markup (`parse.rs:78-126`). Compactar los conflictos no puede bajar `found`.

## Opciones

### Conflictos

| Opción | A favor | En contra |
|---|---|---|
| a. **Referencia al lado ya mostrado** con `#id`, y contenido sólo del lado que el prompt no muestra. Requiere ids en las líneas de hits | Elimina la duplicación sin perder información | Cambia el formato de los hits y obliga a tocar el parser del eval |
| b. **Sólo ids** en los dos lados | Lo más barato | El agente no tiene una tool para leer por id, así que pierde el texto del lado no mostrado |
| c. **Recortar** cada contenido a N caracteres, como hace `search_memory` con 500 (`tools.rs:23`) | Simple, no toca los hits | Sigue duplicando, y el recorte puede cortar justo la parte que contradice |

Cualquiera de las tres se acompaña de esto:

- La NOTE aparece una vez por bloque, no una por par.
- Los pares que comparten la fila nueva se agrupan.
- La consulta lleva un `ORDER BY` determinista y un tope de pares.

### Encabezado de los hits

- Pasa a una sola línea: `[2026-10-10 #57 semantic_fact] texto`. `SOURCE` se muestra sólo si el kind no es `memory`.
- Obliga a actualizar `context_segments` (`parse.rs:78-137`) en el mismo commit, porque hoy segmenta por `CONTENT: ` y por la cabecera de fecha.
- El ahorro es chico, unos 10 tokens por hit. Lo que más vale es que aporta el id que necesita la opción a.

### Workspace

- Al armar el prompt se sacan los marcadores, la sección USER vacía y los encabezados sin bullets. El archivo no cambia.

### Presupuesto total de tokens

- Con los hits en 10, el workspace con tope y los conflictos con tope, el prompt queda acotado sin un presupuesto global.
- Un presupuesto global obligaría a contar tokens en el camino caliente con un tokenizer que no es el del cliente. No se propone ahora.

## Fuera de alcance

- Resumir o reemplazar originales: lo prohíbe la regla de `config.yaml` y ya está descartado en el README.
- Cambiar el formato a JSON, YAML o Markdown: lo que recibe el agente ya es texto plano (`server.rs:205-207`).
- `search_memory`: ya recorta a 500 caracteres y corta por relevancia (`tools.rs:23`, `:57`).
- Bajar la cantidad de falsos `disputed`: eso corresponde a dispute-resolution. Los dos cambios se complementan, porque aquél reduce la cantidad de pares y éste el costo de cada uno.

## Decisiones (cerradas el 2026-10-12)

- [x] **Conflictos: opción a.** El lado ya mostrado se cita por `#id` y el lado no mostrado lleva su texto completo. Si una fila nueva choca con varias viejas, su texto aparece una sola vez.
- [x] **Ids en los hits:** los hits pasan a una sola línea, `[fecha #id tipo] texto`. El agente ve el id y puede usarlo con `delete_memory`.
- [x] **Tope:** 5 pares por `recall_context`. El orden es determinista: primero los pares cuyo lado mostrado aparece más arriba en el prompt. Si quedan pares afuera, una línea final lo dice.
- [x] **Filas del workspace en un conflicto:** cuentan como no mostradas y se imprime su texto. Los bullets del workspace no tienen id (`workspace.rs:394`), así que una referencia `#id` no le serviría al agente. Es un caso raro (tier 1 con importancia de 8 o más) y evita tocar el archivo, que el usuario edita a mano.
- [x] **Orden de los pares que sólo tocan el workspace:** van al final. Si fueran primero, un conflicto sobre una preferencia ocuparía el tope en cada llamada, sea cual sea la pregunta.
- [x] **`delete_memory` y el kind omitido:** se actualiza la doc del argumento `id` (`server.rs:128-129`) para que diga que también sirven los ids de `recall_context` y que un hit sin kind explícito es `memory`. La tool exige `kind` (`server.rs:131-134`).
- [x] **`found` mantiene su definición:** cuenta los hits y los bullets del workspace. El texto de un conflicto no cuenta como visible, igual que hoy, para que la corrida siga siendo comparable con la de base.
- [x] **Umbrales comprometidos de antemano** (`--real`, contra `2026-10-12-real-lines-conflicts-shown`):
  - `found` = 41/46, sin bajar;
  - `search_memory` sin cambios (852 tokens de media);
  - ACTIVE CONFLICTS ≤ 800 tokens de media cuando aparece (hoy 2.112);
  - total de `recall_context` ≤ 2.100 de media (hoy 3.196) y ≤ 3.000 de máximo (hoy 4.625);
  - RECENT CONTEXT no crece (hoy 1.130 de media).

## Cómo se mide

- `rag-eval --real --agent-view --diff` contra `2026-10-12-real-lines-conflicts-shown`, con los umbrales de la sección Decisiones.
- Tests que hoy fallan:
  - el bloque no repite el contenido de una fila ya mostrada;
  - el prompt no contiene `<!-- `;
  - un encabezado sin bullets no se emite;
  - el bloque respeta el tope de pares y su orden es estable.
