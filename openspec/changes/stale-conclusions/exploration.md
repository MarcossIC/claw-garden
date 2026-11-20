# stale-conclusions — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | S (una medición sin código); lo que sigue, sólo si la puerta se abre: S si la señal se calcula al listar en `synthesize`, M si hace falta guardarla (una tabla y una migración) | — para medir. Se coordina con [memory-anchors](../memory-anchors/exploration.md) (98), que marca `gone=` cuando un archivo citado ya no existe, la otra forma de «atrasado»; [memory-links](../memory-links/exploration.md) (134), cuya medición sobre un `clawg export` tiene la misma forma y puede salir del mismo archivo; [topic-key-warning](../topic-key-warning/proposal.md) (68), que decide qué pasa cuando falta la `topic_key`; [contract-diet](../contract-diet/exploration.md) (133), que recorta lo que el modelo lee y por eso fija el presupuesto de texto de cualquier aviso nuevo; y [synthesis-groups](../synthesis-groups/exploration.md), que agrupa las pendientes de la misma lista. Amplía el [conclusion-upkeep](../archive/2026-10-19-conclusion-upkeep/proposal.md) (45) archivado | exploración (borrador) |

**Tesis:** una conclusión envejece de dos maneras. La primera ya se ve: una fuente se borró, se reemplazó o se purgó (`SourceState` y `reviews`, `crates/clawg-core/src/memory/upkeep/synthesis.rs:72` y `:206`). La segunda no: las fuentes siguen vivas, pero después se guardaron memorias sobre lo mismo que la matizan o la contradicen. Esas memorias salen en «pendientes», pero sueltas, sin decir cuál conclusión dejaron atrasada. Una señal barata y sin modelo de lenguaje sería: nadie la tocó en N días **y** se escribió sobre ella después. La pregunta es si clawg puede tener su equivalente y si vale la pena. **Hay dos razones para no decidirlo hoy.** (1) Falta saber cuántas conclusiones vivas hay en un store real; si son pocas, cualquier señal nueva se lee muy poco. (2) El detector de contradicciones ya ve parte de lo que esta idea buscaría (ver «Lo que el detector ya cubre»), así que lo que suma sólo se sabe midiéndolo. Este documento deja la medición escrita, con el umbral fijado antes, sin correrla. Si la puerta no se abre, el cambio espera.

**Qué se verificó y qué no.** El código y las specs se leyeron en `develop` (0.90.0, `d658c8a9`). **No se midió nada sobre un store real ni se corrió ninguna herramienta de evaluación.** No se verificó cuántas conclusiones hay en ningún store ni cuánto se usa `synthesize`: el export no trae `synthesis_review` ni las llamadas a herramientas (`crates/clawg-core/src/memory/portability/export.rs:162-188` sólo lleva `memory`, `memory_derivation`, `memory_piece` y `memory_conflict`), y los totales de un store real no están en este repositorio.

---

## La señal barata, y lo que no se traslada solo

La forma más simple de la idea, sin modelo de lenguaje, es esta: un documento queda atrasado cuando nadie lo tocó en N días **y** se escribió sobre él después. Tiene cuatro piezas:

| Pieza | Cómo funcionaría |
|---|---|
| Qué cuenta como «escrito sobre» el documento | Una entrada activa, creada **después** de la última modificación del documento, cuyo tema es **la dirección del documento** o cuyo contenido **la nombra** como palabra entera (`sales` no está en `presales`) |
| El umbral | El documento tiene que estar **quieto** N días antes de contar (por ejemplo 90, configurable) |
| La lista | Los documentos quietos más N días **y** con alguna entrada que cumpla lo anterior, con cuántas entradas nuevas tienen |
| El aviso | Se da al guardar, sólo para los documentos que el cambio toca; es un **aviso, nunca un rechazo**. El pedido de limpieza sale como mucho una vez cada 24 horas y sólo si hay algo que revisar; el contexto lleva una frase corta y el informe va aparte, con cada lista cortada a un tope (por ejemplo 20) |

Lo que hace que esa señal funcione sobre documentos curados, cada uno con su dirección, y **no** se traslada solo a clawg:

1. **Un documento tiene una dirección.** El «sobre lo mismo» es una igualdad entre el tema de la entrada y la dirección, o un nombre propio dentro del texto. Una conclusión de clawg no tiene dirección: puede tener `topic_key`, pero el protocolo sólo se la pide a «a decision or a preference that may change later» (`crates/clawg-cli/src/mcp/protocol.md:69`), no a toda conclusión. Lo más parecido a «el contenido la nombra» es `shares_identifier` (`crates/clawg-core/src/memory/revision/contradiction.rs:120`), que compara identificadores de código entre dos textos.
2. **El documento se edita en el lugar y tiene una fecha de última edición.** Una conclusión de clawg casi nunca cambia: se reemplaza por otra fila (`topic_key`) o se refuerza si el texto es el mismo, y la marca de tiempo que vale es `created_at` o `last_modified_at` (a decidir, D1 y D3).
3. **Un umbral fijo de días presupone un conjunto curado.** Un store de clawg recibe memorias de varios proyectos y agentes, así que «hay algo posterior» sin más es casi siempre cierto. Por eso cualquier definición de abajo trae un «sobre lo mismo» y no se queda en la edad.
4. **El pedido hay que cortarlo.** Una vez por día, con tope de lista, o el agente aprende a ignorarlo.

---

## El problema, verificado contra el código

| Qué | Dónde | Efecto |
|---|---|---|
| Una conclusión es una fila de `memory` con filas en `memory_derivation (memory_id, source_id)` | `crates/clawg-core/migrations/V10__synthesis.sql:8-14`; `CONCLUSION`, `synthesis.rs:50` | Sin columna nueva: «conclusión» se calcula |
| Se marca para revisar sólo con una fuente perdida | `LOST_SOURCE` (`synthesis.rs:55`), `needs_review` (`:101`), `reviews` (`:206`) | Una conclusión cuyas fuentes siguen vivas **nunca** entra en `to_review` |
| Una memoria nueva sobre lo mismo es «pendiente» | `PENDING` (`synthesis.rs:36`): viva, no conclusión y **no fuente de una conclusión viva** | Sale en «Memorias sin conclusión», pero no atada a ninguna conclusión. Que atrase a una vieja nadie lo dice |
| El aviso cuenta pendientes, no atraso | `synthesis_nudge` (`crates/clawg-cli/src/mcp/tools.rs:730`), `SYNTHESIS_NUDGE_THRESHOLD = 20` (`:728`); `unreviewed_count` (`synthesis.rs:290`) | Cuenta lo que llegó tras la última revisión y no es import; no distingue lo que atrasa una conclusión |
| El aviso de revisión cuenta pérdidas de fuente | `review_nudge` (`tools.rs:745`), `conclusions_to_review_count` (`synthesis.rs:315`): borrada en `deleted_at`, reemplazada en `last_modified_at`, y una purgada sólo antes de la primera revisión | Es el lugar natural de una razón nueva |
| `synthesize` lista hasta 10 a revisar y 20 conclusiones | `REVIEW_LIMIT` (`synthesis.rs:60`), `CONCLUSIONS_LIMIT` (`:31`); `format_synthesis` (`tools.rs:643`) | Una razón nueva cabe en una línea de `review_line` (`tools.rs:634`) |
| Los dos conteos corren en **cada** `recall_context` y cada `session-start` | `crates/clawg-cli/src/daemon/api/memories.rs:1086-1095`; las dos líneas salen en `crates/clawg-cli/src/hook.rs:259-260` | Un trabajo por conclusión ahí compite con todo lo demás en una sola conexión de SQLite (ver [read-connections](../read-connections/exploration.md)) y con el plazo de 3 s de un hook |

### Lo que el detector ya cubre

Antes de inventar una señal, conviene saber qué ve ya el sistema:

- Cada save de un tipo que afirma algo (`user_preference`, `architectural_decision`, `semantic_fact`: `is_claim_bearing`, `crates/clawg-core/src/memory/domain/record.rs:132-137`) busca sus vecinos más cercanos (un pozo de `CANDIDATE_POOL` = 5 en `contradiction.rs:59`, más los hermanos) y marca como disputa los que caen en la banda `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` = `[0.10, 0.42)` (`contradiction.rs:46`, `:38`, `:214-215`). Una conclusión guardada como `semantic_fact` o `architectural_decision` es candidata como cualquier fila.
- Si el par se juzga `supersedes`, la conclusión queda reemplazada y sale de las vivas; si queda abierto, sale en ACTIVE CONFLICTS (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs:30`); si se juzga `compatible` o `scoped`, queda un par `judged` en `memory_conflict` (`V15__memory_conflict.sql:11-22`) con la conclusión de un lado.
- **Una `topic_key` igual en el mismo proyecto ya reemplaza**: `SCOPE_ROWS_SQL` compara proyecto y clave exacta (`crates/clawg-core/src/memory/revision/topic.rs:88-95`), dentro del alcance del que guarda. Una conclusión con `topic_key` que otro save repite deja de estar viva: no hace falta señal.

O sea: **lo contradictorio y cercano en significado ya tiene dónde ir**. Lo que queda sin marcar es lo que el detector no ve: lo que cae más allá de 0.42, lo que no es de un tipo que afirma (un `code_block`, un `tool_error`), lo que el pozo de vecinos dejó fuera por haber otros más cercanos, lo que se juzgó `compatible` (el agente dijo «no se contradicen», que no es «la conclusión sigue completa») y lo que matiza sin contradecir. Es una porción que esta exploración no sabe medir sin datos, y es la razón de la puerta.

---

## Cómo definir «sobre lo mismo» sin un modelo de lenguaje

| Definición | A favor | En contra | Qué la decide |
|---|---|---|---|
| **(a) Misma `topic_key`, o su prefijo** (`storage/` cubre `storage/engine`) | Una consulta SQL con índice (`idx_memory_topic`, V20); determinista; cero costo de modelo | La clave **igual** ya reemplaza (arriba), así que sólo suma el **prefijo** o una clave entre alcances que no se ven; casi ninguna conclusión la lleva (el protocolo la pide a decisiones y preferencias); un prefijo es una convención del agente, no una regla de clawg | Qué fracción de las conclusiones vivas tiene `topic_key`; si es baja, (a) no alcanza aunque sea exacta |
| **(b) Cercanía de vectores** de la memoria nueva con la conclusión, o con alguna de sus fuentes, bajo un umbral | Es el único que ve «sobre lo mismo» sin nombre en común; las fuentes dan más lugares donde caer cerca; ya hay un umbral calibrado, `CONTRADICTION_THRESHOLD = 0.42` | Esa calibración es para **contradicciones** entre afirmaciones (`contradiction.rs:25-37`), no para «habla del mismo tema»; hay que traer el vector (el export no lo trae: se vuelve a calcular con el modelo local); cuesta una búsqueda de vecinos por conclusión; y envejece si cambia el modelo ([model-reembed](../model-reembed/exploration.md)) | Si a 0.42 se marca poco y mal (se queda corto), o se marca casi todo con un umbral mayor (no distingue) |
| **(c) Mismo proyecto y una ventana de tiempo** | Sólo SQL sobre columnas que ya están; es la señal barata de arriba sin el tema ni el nombre | En un proyecto activo **todo** lo posterior cuenta: no distingue nada. Sirve de **línea base nula** (cuánto marcaría una señal que no mira el tema), no de señal | Su tasa: si marca casi todas, confirma que sola no sirve |
| **(d) Identificador compartido** (`shares_identifier`: una ruta o un nombre de código en común) | Sin modelo, determinista; es el análogo de «el contenido nombra la dirección»; ya existe y está probado | Sólo sirve a conclusiones que nombran código; las de preferencias y hechos del usuario no tienen identificadores | La fracción de conclusiones con algún identificador |
| **(e) El par ya registrado** en `memory_conflict` con la conclusión del lado viejo, creado después, sin veredicto `supersedes` | **Gratis**: lo calculó el detector al guardar y no cuesta nada al leer | Sólo trae lo que el detector ya vio: no suma cobertura, sólo **ata** esos pares a la conclusión en la lista | Cuántas conclusiones tienen un par así que hoy no se ve junto a ellas |
| **Combinaciones** | (a), (d) o (e) como señales baratas y exactas, (b) como refuerzo; (c) siempre como filtro (mismo proyecto, `created_at` posterior, no import) | Cada una suma una condición que explicar al agente | Lo que muestre la medición: se elige la unión mínima que alcance el umbral |

Ninguna se elige hoy. Todas llevan el mismo filtro de base, el de `PENDING`: la memoria posterior es **viva**, **no conclusión**, **no fuente de la propia conclusión** y no la trajo un import (`source <> 'import'`, igual que `unreviewed_condition`, `synthesis.rs:46`).

---

## Cómo se muestra y qué cuesta

### Cómo se muestra

El mismo lugar que ya usa la causa «fuente perdida», **sin herramienta nueva**:

- En la lista de `synthesize` (`to_review`), una **causa nueva** en `ConclusionReview` junto a `live` y `lost` (`synthesis.rs:115-120`): por ejemplo `newer: N` y los ids de las dos o tres memorias posteriores más cercanas, para que el agente las lea. `review_line` (`tools.rs:634`) la escribe en una línea: «N memorias posteriores cercanas».
- En `review_nudge` (`tools.rs:745`), el conteo suma las conclusiones por esa causa, con una frase que nombra las dos causas. No se agrega una tercera línea al arranque.
- `contract-diet` (133) recorta lo que el modelo lee en cada sesión. Esto no suma texto al contrato (`agent_contract*.json`: la herramienta es la misma; cambia lo que **devuelve** y una frase del aviso), pero **cada palabra de `review_nudge` va en cada contexto**. Por eso el presupuesto es: la frase del aviso no crece más de lo que hoy dice sobre fuentes perdidas, y el aviso no sale si el conteo es 0. Si se abre este cambio, se acuerda con 133 antes de tocar el texto.
- **Un detalle que hay que decidir** (D3): hoy una conclusión «a revisar» por fuente perdida **se queda en la lista** aunque el agente la haya revisado y decidido que sigue valiendo; lo que se apaga es el aviso, que cuenta sólo lo posterior a `synthesis_review.reviewed_at`. Con la causa nueva pasa lo mismo y es peor: las memorias posteriores no desaparecen nunca, así que la conclusión estaría en la lista para siempre. Hace falta una forma de «revisada, sigue valiendo». Sin código nuevo, volver a guardar el mismo texto la refuerza (R2 de [`specs/synthesis`](../../specs/synthesis/spec.md)); si el refuerzo mueve `last_modified_at` y ése es el momento desde el cual se cuentan las memorias posteriores, el ciclo cierra sin columna. **No se verificó que el refuerzo mueva esa marca.**

### Cuánto cuesta

| Dónde | Costo | Veredicto |
|---|---|---|
| **A. Al listar en `synthesize`** | Hasta `CONCLUSIONS_LIMIT` (20) búsquedas de vecinos por llamada, o una consulta SQL si es (a), (d) o (e); la llama el agente a propósito, una vez | Aceptable para (a), (d), (e) y (b); el único camino que **no** toca el contexto |
| **B. En el conteo de `recall_context` y `session-start`** | Un trabajo por conclusión en **cada** contexto, sobre la única conexión del daemon | **No** para (b). Aceptable sólo para SQL puro e indexado, (a) y (e), y con tope; (d) pide leer el texto de cada conclusión, y no |
| **C. En el mantenimiento diario** (`crates/clawg-core/src/memory/upkeep/maintenance.rs`, `MAINTENANCE_PERIOD` = 24 h) | Una pasada por conclusión, fuera del camino de las lecturas; pero el resultado hay que **guardarlo** (una tabla o columna: migración y rollback) y envejece hasta 24 h; el pase de hoy sólo demota por tier (`DEMOTE_SQL`) | Sólo si (b) se confirma y A no alcanza; es el paso más caro |
| **D. Al guardar**, reutilizando la búsqueda del detector | El detector ya paga una por cada save que afirma algo; registrar cuando un vecino es una conclusión viva fuera de la banda de disputa sería casi gratis | Ata el costo al guardado, que ya lo paga; pero pide una tabla y toca el camino de ingestión: decisión de diseño, y sólo si la medición muestra que lo que el detector no ve importa |

**Recomendado: A**, con la señal más barata que alcance la puerta; (b) sólo como refuerzo. B queda para SQL indexado; C y D no se abren sin una medición que lo pida.

---

## Enfoques

| Enfoque | A favor | En contra | Qué lo decide |
|---|---|---|---|
| **A. No hacerlo (recomendado hasta que la puerta se abra)** | Cero costo; el detector ya lleva al agente a lo contradictorio y cercano; una señal que se lee poco se aprende a ignorar | Si hay conclusiones que quedan atrasadas sin que nadie lo diga, se quedan así | La puerta de abajo |
| **B. Una causa nueva en `to_review` calculada al listar, con la señal más barata que alcance** | Sin herramienta nueva ni migración; una consulta y una línea más; la usa quien ya revisa conclusiones | Si nadie llama a `synthesize`, nadie la ve; hereda el problema de «revisada» (D3) | Que la puerta se abra con una definición barata |
| **C. Además, el conteo en `review_nudge`** | El agente se entera sin llamar a `synthesize` | Suma texto a cada contexto; toca el presupuesto de 133; el conteo corre en cada contexto (costo B de arriba) | Que la lista (B) se use y sola no se vea |
| **D. Una tabla «conclusión → memorias cercanas» mantenida al guardar o en el mantenimiento** | Lectura barata y vectores precalculados | Migración, rehacer con cada cambio de modelo, y duplica lo que el detector ya paga: es lo que memory-links (134) llama «la búsqueda precalculada» | Una medición que muestre que (b) aporta lo que (a), (d) y (e) no |

**Recomendado:** A mientras se mide; si la puerta se abre, B con la unión mínima de definiciones baratas; C sólo después de saber que B se usa; D queda descartado mientras el número no lo pida.

---

## Puerta de medición (sin código de producto)

**Pregunta que contesta:** de las conclusiones vivas de un store real, ¿cuántas tienen memorias posteriores sobre lo mismo que nadie ató a ellas, y de esas, cuántas están de verdad atrasadas?

**Quién la corre:** la persona, en local, porque necesita el store real (perfil prod). **No se corrió aquí.** No toca el store: `clawg export` es de sólo lectura y no pide el candado (`docs/reference/commands.md`, `export`). El mismo export puede servir a la medición de memory-links (134); el archivo se guarda fuera del repositorio, con permisos de usuario, y se borra al terminar. Nada de lo medido se commitea.

**Procedimiento** (sin llamadas a una API de pago; un script fuera del producto, en un directorio ignorado):

1. Exportar: `clawg export --out atraso.jsonl --profile prod` (sin `--include-deleted`: sólo lo vivo). Cada línea `memory` trae `project_id`, `memory_type`, `created_at`, `last_modified_at`, `topic_key`, `source`, `status`, `deleted_at` y `content`; las `derivation` traen `memory` y `source`; las `conflict`, `new_id`, `old_id`, `state` y `verdict` (`crates/clawg-core/src/memory/portability/format.rs:91-177`). Un save largo se parte en piezas: juntar las líneas `piece` en orden antes de leer el texto. Como el export deja afuera lo borrado, una fuente que no aparece como fila es una fuente perdida.
2. **Conclusiones vivas:** las filas con al menos una línea `derivation` donde son `memory`, vivas (`deleted_at` vacío, `status` distinto de `superseded`). Contar por proyecto y en total. Contar también cuántas ya están a revisar por fuente perdida: la **línea base** de lo que hoy se avisa.
3. **Memorias candidatas de cada conclusión C:** las vivas del mismo proyecto (`project_id` igual), que no son conclusión, no son fuente de C, con `source` distinto de `import` y `created_at` posterior a `max(created_at, last_modified_at)` de C. Éste es el universo «posterior».
4. **Marcar C con cada definición**, por separado y en unión, y contar:
   - (a) `topic_key` igual a la de C o prefijo suyo hasta una `/` (sólo si C tiene clave);
   - (b) distancia L2 en espacio unitario menor que **0.42** entre la memoria y C, o entre la memoria y alguna fuente de C. El export no trae vectores: se vuelven a calcular con el modelo que dice la cabecera (`embedding_model`), en local; 0.35 y 0.50 se informan como **sensibilidad** y no deciden;
   - (c) cualquier memoria posterior del universo (la línea base nula);
   - (d) un identificador de código en común (la idea de `shares_identifier`, reimplementada en el script);
   - (e) un `conflict` con C del lado viejo (`old_id`), creado después, con veredicto distinto de `supersedes` o sin veredicto.
5. **Muestra a mano, etiquetada antes de ver los totales por definición.** Tomar al azar hasta **30** conclusiones marcadas por la unión de (a), (b), (d) y (e), y hasta **10** sin marcar (control). Para cada una, leer la conclusión y sus memorias posteriores y etiquetar: *atrasada* (alguna la matiza, la contradice o la deja corta y habría que reescribirla), *vigente* (las posteriores son de otro tema o ya la repiten) o *dudosa*. Anotar también si ya estaba en la lista por fuente perdida o en un par de `conflict` abierto: lo que la señal nueva **no** suma.
6. **Informar:** por proyecto y en total, conclusiones vivas; marcadas por cada definición y por la unión; cuántas ya estaban a revisar por fuente perdida; de la muestra, el reparto *atrasada* / *vigente* / *dudosa* y cuántas de las *atrasadas* son nuevas (ni fuente perdida ni par abierto); y de los 10 de control, cuántas resultan *atrasadas* (lo que la señal pierde). Informar también la edad de las conclusiones (D1).

**Puerta, fijada antes de medir:**

- **Mínimo de datos:** al menos **10** conclusiones vivas en total y al menos **10** marcadas por la unión. Con menos, el resultado es «no concluyente»: el cambio **espera**, por la regla 4 del [ROADMAP](../../ROADMAP.md) («lo que sólo vale si un problema aparece»), y se repite cuando haya más.
- **Se abre** si, con la unión de las definiciones baratas (todas menos (b)) o con (b) a 0.42, se cumplen las cuatro:
  1. la **precisión** sobre la muestra, *atrasada* ÷ (*atrasada* + *vigente*), es de **50 % o más**;
  2. al menos **la mitad** de las *atrasadas* son **nuevas** (no estaban en la lista por fuente perdida ni en un par abierto): la señal tiene que sumar;
  3. la unión marca **no más del 50 %** de las conclusiones vivas: una señal que marca casi todo no dice nada (la línea base (c) mide el techo);
  4. hay **al menos 5** conclusiones *atrasadas* y nuevas en el store: con menos, la lista casi no se vería.
- **Se cierra** (se archiva en «Ideas evaluadas» del README) si la precisión es **menor que 25 %**, o si menos de un cuarto de las *atrasadas* son nuevas: el detector y la lista de fuentes perdidas ya cubren lo que hay.
- **Entre medio** (precisión de 25 % a 50 %, o la unión marca más del 50 %): una sola vuelta de ajuste, **fijada ahora**: probar (b) a 0.35 en vez de 0.42 y la unión sin (c). Se mide de nuevo sobre las mismas conclusiones, con el mismo etiquetado, y se aplica la misma puerta. Si sigue en el medio, se cierra.
- **Si (b) es la única definición que pasa**, el cambio se abre sólo en el lugar A (al listar en `synthesize`), nunca en el conteo del contexto (costo B). Su costo se mide aparte antes del diseño.

**Costo:** horas de una persona (la muestra de 40 a mano y un script de unas 100 líneas; el modelo local para (b)). Sin créditos de API.

**Límites:**
- **Pocas conclusiones.** El protocolo manda llamar `synthesize` cuando termine la tarea (`crates/clawg-cli/src/mcp/protocol.md:32`) y el aviso sale con 20 o más memorias sin revisar (`tools.rs:728`). Un store con muchas memorias sin revisar sugiere poco uso de la herramienta, pero **ese número es de un store real y no está en este repositorio: no se verificó**. Por eso el mínimo de 10.
- **Lo que el agente escribe con otras palabras.** (a) y (d) lo pierden y (b) lo encuentra; la muestra de control dice cuánto.
- **Precisión leída por una sola persona.** Una conclusión es *atrasada* según quien la lee; se rotula antes de ver los totales y la duda va a *dudosa*, que no entra en la precisión.
- **El tiempo.** Una conclusión de ayer casi no tiene posteriores. Una señal que sólo funciona en conclusiones de más de un mes necesita su propia ventana (D1).

---

## No alcance

- Borrar, reescribir o reemplazar una conclusión sola: el servidor no llama a ningún modelo; el agente escribe las conclusiones y el daemon sólo lista.
- Una herramienta nueva, o un argumento nuevo de `save_memory` o `synthesize`.
- Columnas de confianza o de edad en `memory` (también quedaron fuera de conclusion-upkeep).
- Cambiar el ranking, el detector de contradicciones, sus umbrales (`CONTRADICTION_THRESHOLD` y `NEAR_DUPLICATE_FLOOR` están fijados por un test) o lo que arma `recall_context`.
- Marcar por **edad** sola (el umbral de N días sin un «sobre lo mismo»): arriba se explica por qué en clawg no distingue.
- La otra forma de «atrasado», que un archivo citado ya no exista: es de [memory-anchors](../memory-anchors/exploration.md) (98).
- Relaciones entre memorias que no son de parecido: es de [memory-links](../memory-links/exploration.md) (134).
- Exigir `topic_key` en una conclusión o avisar cuando falta: es de [topic-key-warning](../topic-key-warning/proposal.md) (68).

## Cómo se relaciona con lo que ya hay

| Cambio | Qué hace | En qué se diferencia de éste |
|---|---|---|
| [conclusion-upkeep](../archive/2026-10-19-conclusion-upkeep/proposal.md) (45, archivado) | Marca una conclusión con una fuente perdida; `derived_from` rechaza una fuente reemplazada; el aviso `review_nudge` | Es la causa 1. Esta exploración **no la cambia**: suma una causa 2 a la misma lista |
| [knowledge-synthesis](../archive/2026-10-13-knowledge-synthesis/proposal.md) (archivado) | Una conclusión con sus fuentes (`memory_derivation`, V10), `synthesize`, el aviso de pendientes | La base de la tabla y de la lista; la spec viva es [`specs/synthesis`](../../specs/synthesis/spec.md) |
| [memory-anchors](../memory-anchors/exploration.md) (98) | `gone=` cuando un archivo citado ya no existe | Otra forma de atrasado, por el código. Si 98 se hace, su marca puede entrar como una tercera causa en la misma línea de `review_line`: comparten la forma, no la señal |
| [memory-links](../memory-links/exploration.md) (134) | Pregunta si un enlace trae algo que la búsqueda no trae; su caso (c) es «mismo archivo citado» | La definición (b) de acá es la «búsqueda precalculada» que 134 descarta si no aporta; si 134 cierra por eso, (b) acá se mide sólo como un número y no se guarda. Las dos mediciones pueden salir del mismo export |
| [topic-key-warning](../topic-key-warning/proposal.md) (68) | Avisa al agente cuando falta la clave | Si más conclusiones llevaran `topic_key`, (a) rendiría más; hoy el protocolo no se la pide a una conclusión |
| [contract-diet](../contract-diet/exploration.md) (133) | Recorta lo que el modelo lee | Fija el presupuesto de texto del aviso (arriba); acordar antes de tocar `review_nudge` |
| [synthesis-groups](../synthesis-groups/exploration.md) | Agrupa por cercanía las memorias pendientes de `synthesize` y nombra cada grupo con los términos que lo distinguen, sin modelo de lenguaje | Mira el otro extremo de la misma lista: las pendientes que todavía no tienen conclusión, y éste mira las conclusiones que quedaron atrás de pendientes posteriores. Una memoria nueva cercana a una conclusión puede caer en uno de sus grupos y a la vez marcar esa conclusión acá. Si los dos se hacen, conviene que una misma medida de cercanía sirva a los dos, y que las dos mediciones salgan del mismo export |

---

## Decisiones abiertas

- **D1. La ventana.** Si una conclusión necesita una edad mínima antes de poder marcarse (por ejemplo 90 días) o si alcanza con «posterior», y desde qué marca se cuenta (`created_at` o `last_modified_at`). Se ajusta con la edad de las conclusiones, antes de leer los totales.
- **D2. Qué definición.** La unión mínima que pase la puerta. Recomendado: empezar por las baratas y exactas, (a), (d) y (e); (b) sólo si es la única que alcanza.
- **D3. Qué significa «revisada».** Si volver a guardar el mismo texto basta para mover la marca o hace falta otra cosa. Verificar que el refuerzo mueva `last_modified_at`, y que el aviso cuente sólo lo posterior a `synthesis_review.reviewed_at`, como hace hoy `conclusions_to_review_count`.
- **D4. Dónde se calcula.** Al listar en `synthesize` (recomendado), o también en el conteo del contexto, o en el mantenimiento (costos arriba).
- **D5. El texto del aviso.** Una frase que nombre las dos causas sin crecer; se acuerda con 133 y pasa por `CLAWG_BLESS=1` y por los dos contratos si cambia lo que el modelo ve.
- **D6. El umbral de «cercana».** 0.42 viene de una calibración de contradicciones, no de «mismo tema»: la sensibilidad de 0.35 y 0.50 dice si sirve; no se promueve sin la muestra a mano.

## Cómo se mide

| Qué | Cómo | Quién | Cuándo | Puerta |
|---|---|---|---|---|
| Cuántas conclusiones vivas hay y cuántas ya están a revisar | Export del store real, script de lectura | La persona, en local | Ya; sin costo | Mínimo 10 conclusiones vivas; con menos, «no concluyente» y espera |
| Cuántas tienen memorias posteriores cercanas sin atar, por definición | El mismo export; (b) con el modelo local | La persona, en local | Junto con la anterior | La unión marca no más del 50 % de las conclusiones vivas |
| Si esas memorias dejan de verdad atrasada a la conclusión | Muestra de 30 marcadas y 10 de control, a mano, rotulada antes de ver los totales | La persona | Junto con la anterior | Precisión de 50 % o más y la mitad de las atrasadas nuevas: se abre; precisión menor que 25 %: se cierra |
| El costo de (b) al listar | Tiempo de una llamada a `synthesize` con 20 conclusiones | La persona, sólo si (b) es la única definición que pasa | Antes del diseño | Dentro del plazo de una herramienta del agente |

## Riesgos

- **Una señal que se aprende a ignorar.** Si marca casi todo, el agente deja de mirar la lista; por eso la puerta tiene techo y precisión.
- **Medir con datos pobres.** Pocas conclusiones dan un número que parece una respuesta y no lo es; hay un mínimo y un «no concluyente».
- **Duplicar al detector.** Parte de lo que esta idea busca ya sale en ACTIVE CONFLICTS; si no se resta, la medición cuenta dos veces lo mismo. Por eso se anota qué estaba ya a la vista.
- **Texto en cada contexto.** Cada palabra de `review_nudge` va en cada `recall_context`: el presupuesto es el de 133.
- **Costo en el camino caliente.** Una búsqueda por conclusión en el conteo del contexto frena guardado, búsqueda y hooks (una sola conexión).
- **Privacidad.** El export contiene el texto de todas las memorias: se guarda fuera del repositorio, con permisos de usuario, y se borra al terminar.

## Esfuerzo e impacto

| Paso | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|
| Medición (export, script, muestra) | Decide si hay un cambio | Bajo | Horas, sin código de producto |
| Causa nueva en `to_review`, al listar | Una conclusión atrasada se ve junto a sus memorias | Bajo (sólo lectura) | S |
| Conteo en `review_nudge` | El agente se entera sin llamar a la herramienta | Medio (texto en cada contexto, costo por contexto) | S, con 133 |
| Tabla mantenida al guardar o en el mantenimiento | Lectura barata | Medio (migración y rollback) | M |

## Fuentes

- Código y specs de `develop` en `d658c8a9` (0.90.0), con las rutas y líneas citadas; [`specs/synthesis`](../../specs/synthesis/spec.md).
- [`conclusion-upkeep`](../archive/2026-10-19-conclusion-upkeep/proposal.md) (45) y [`knowledge-synthesis`](../archive/2026-10-13-knowledge-synthesis/proposal.md), archivados.
