# long-save-context — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | eval-golden-expansion, data-preservation (esquema), dispute-resolution (hermanos) | ver [`state.yaml`](state.yaml) |

## Problema

Un texto largo se guarda como pedazos sueltos. Cada pedazo se busca solo, sin saber de qué save viene, y nada lo vuelve a armar. Detalle en la [exploración](exploration.md), revisada contra `main` (9c3ff19) el 2026-10-15 ([qué cambió](exploration.md#revisión-contra-main-2026-10-15)):

1. **Desde el segundo pedazo, el vector pierde el tema.** El worker parte el texto en ventanas de 480 tokens con 64 de solapamiento (`crates/clawg-core/src/memory/chunker/mod.rs:15, 21`; `chunker/chat.rs:67-72`) y embebe sólo el texto de cada una (`crates/clawg-core/src/memory/worker.rs:504`). Si el tema (el crate, la decisión, el bug) sólo aparece al principio, el vector del pedazo 2 no lo lleva, y una búsqueda por el tema no lo encuentra. El solapamiento protege un hecho que cruza el corte, no el tema del save completo.
2. **Los hermanos se pelean el top.** Cada pedazo es una fila de `memory` sin referencia a los demás: ninguna de las diez migraciones (`crates/clawg-core/migrations/V1`–`V10`) los vincula. Todos compiten por los mismos 10 lugares (`K_FINAL`, `crates/clawg-core/src/memory/prompt_builder.rs:34`; el corte en `crates/clawg-core/src/memory/search.rs:1115-1116`), y cuatro pedazos de un save desplazan a tres memorias distintas.
3. **Nada vuelve a armar el save.** `search_memory` muestra 500 caracteres de cada hit (`CONTENT_PREVIEW_LIMIT`, `crates/clawg-cli/src/mcp/tools.rs:26`), un tercio de un pedazo. `GET /v1/memories/{kind}/{id}` devuelve una sola fila y no tiene herramienta MCP (`crates/clawg-cli/src/daemon/api.rs:541-544, 883-893`). La descripción de `save_memory` lo avisa: "a long text is split into separate pieces that are found independently and never reassembled" (`crates/clawg-cli/src/mcp/server.rs:699-701`).

Ya no son problema, y quedan fuera:

- **Los hermanos ya no se disputan.** dispute-resolution los excluye del detector (`worker.rs:528-557`, `contradiction.rs:128-147`; [`specs/disputes`](../../../specs/disputes/spec.md) R8). Con el modelo real, antes quedaban `disputed` 220 de 263 pedazos de 40 saves largos.
- **El agente ya sabe en cuántos pedazos se partió.** El save es síncrono desde ingest-durability, y la respuesta dice `saved as 3 pieces: #57, #58, #59` (`server.rs:416-424`). Era la opción c de la exploración.

## Decisión (cierra las abiertas de la exploración)

**Puntero y contexto (a + b de la exploración).** Tarjeta en el hilo, opción recomendada. Mientras no se elija otra, se sigue con ésta.

- **¿a, b o las dos?** Las dos. b sola no arregla el vector: un pedazo sin el tema sigue sin aparecer por el tema. a sola tiene que guardar la cabecera dentro de cada pedazo, y entonces el agente la lee repetida y los hermanos siguen ocupando el top. Además, con a, los hermanos se parecen más entre sí (comparten el comienzo), así que empujan más fuerte hacia el top: sin el colapso de b, a empeora el problema 2.
- **¿La cabecera se guarda o sólo se embebe?** Sólo se embebe. Se guarda aparte, en el vínculo del pedazo, para que un re-embed la reconstruya igual (`reembed_all`, `crates/clawg-core/src/memory/maintenance.rs:467`, lee el texto de la fila y lo vuelve a embeber). El contenido de la fila, su hash, el FTS y lo que ve el agente no cambian. El desajuste que temía la exploración (`chunker/mod.rs:12-14`) es otro: el truncado silencioso del embedder. Acá el vector lleva el texto completo de la fila más un contexto declarado, y nada se trunca (D2 del diseño).
- **¿Las filas existentes?** Quedan sin vínculo, como hoy: no se infiere nada. Reconstruir saves por solapamiento de texto entre ids vecinos puede unir filas de saves distintos, y un vínculo falso es peor que ninguno. Un save viejo vuelve a quedar vinculado si se guarda de nuevo: sus pedazos se absorben por hash (`worker.rs:479-490`) y el vínculo se escribe igual.
- **¿Cuántos saves pasan hoy el corte?** No se mide antes. Medirlo en prod requiere autorización para leer `~/.clawg`, y la respuesta no cambia la decisión: la migración desde engram trae textos largos de todas formas, y para los saves cortos el cambio no hace nada (R1).

## Alcance

1. **Pedazos con contexto** (`chunker`, `worker.rs`): un texto que entra en un pedazo se guarda igual que hoy. Uno que no entra se parte con un tope más chico, y cada pedazo desde el segundo se embebe precedido del comienzo del save (sus primeros 48 tokens). Se guarda sólo el pedazo.
2. **Vínculo del save** (`V11__memory_piece.sql`, tabla nueva): al terminar un evento de más de un pedazo, una fila por pedazo con el save, la posición y el contexto con el que se embebió. Los pedazos absorbidos por el dedup también se vinculan, en su posición.
3. **Un hit por save** (`search.rs`): la búsqueda se queda con el mejor pedazo de cada save antes de cortar en `k`, y el hit dice qué pedazo es (`piece=2/4`), en `search_memory` y en `recall_context`.
4. **El save entero** (HTTP y MCP): `GET /v1/memories/{kind}/{id}/save` y la herramienta `read_memory`, que devuelve el save de un pedazo, armado y sin el solapamiento, con el scope del agente.
5. **Re-embed** (`maintenance.rs`): re-embebe un pedazo con el mismo contexto que tuvo al guardarse.
6. **Medición** (`golden-mine synth`, `rag-eval --agent-view`): un subconjunto `long` con saves largos cuya respuesta queda después del corte, y un chequeo de que la respuesta sea visible, no sólo un fragmento de la fila.

## Fuera de alcance

- **Editar o borrar un save entero.** `delete_memory` sigue borrando una fila; el save se arma con los pedazos que quedan, y `read_memory` dice cuáles faltan.
- **El workspace.** Lista filas de tier 1 (`crates/clawg-core/src/memory/workspace.rs:255-292`); una decisión larga ocupa una viñeta por pedazo, como hoy.
- **El chunker de código** (`ChunkerKind::Code`): sigue sin productor; es de project-indexing.
- **Truncar menos en `search_memory`.** El tope de 500 caracteres se queda; para leer más está `read_memory`.
- **Vincular filas existentes** (ver arriba).

## Éxito

**Tests que fallan hoy** (detalle en la [spec](spec.md)):

- un texto de un pedazo se parte, se embebe y se guarda exactamente como hoy;
- el pedazo 2 de un save largo se embebe con el comienzo del save, y su fila guarda sólo el pedazo;
- un save de varios pedazos deja su vínculo, también para los pedazos absorbidos, y uno que falla a mitad de camino no deja ninguno;
- la búsqueda devuelve un solo hit por save, el mejor, y sigue llenando `k` con otras memorias;
- `read_memory` devuelve el save armado sin el solapamiento, nombra los pedazos que faltan y trata como "no existe" el de otro agente;
- `reembed_all` re-embebe un pedazo con su contexto;
- una base en V10 con filas y vectores abre en V11 y todo sigue alcanzable, sin vínculos.

**`rag-eval`, con umbrales fijados antes de medir:**

| # | Condición | Corrida |
|---|---|---|
| U1 | Ancla (`golden.json`, n = 46): diff vacío en retrieval y similarity, y el bloque `agent_view` igual campo por campo. El ancla no tiene filas de más de un pedazo, así que nada puede cambiar | `--real --diff`, `--real --agent-view` |
| U2 | `long` (set de la semilla 7 con el subconjunto nuevo): respuesta alcanzable en `search_memory` en al menos 0,75 de los casos y al menos 0,25 por encima de la base | `--real --agent-view` |
| U3 | `long`: ningún resultado de `search_memory` muestra dos pedazos del mismo save | `--real --agent-view` |
| U4 | Los otros cinco subconjuntos del set de la semilla 7: ninguna métrica de agent-view baja más de 0,02 | `--real --agent-view` |

La base de U2 y U4 es el commit del eval, el primero de la rama: ya entiende el subconjunto `long`, pero todavía guarda y busca como `main`. Un binario de `main` no puede leer el set nuevo, porque el parseo rechaza claves desconocidas (`crates/clawg-eval/src/bin/rag-eval/golden.rs:31`). Si una condición no se cumple, se registra tal cual y no se mueven los umbrales (regla de [`specs/eval`](../../../specs/eval/spec.md#criterio-de-salida)).

`rag-eval --real` sin `--agent-view` no sirve para `long`: siembra cada fila entera, sin partirla (`crates/clawg-eval/src/bin/rag-eval/seed.rs:71-82`), y el embedder la trunca a 512 tokens. El subconjunto aparece en ese reporte, pero no tiene umbral.

## Datos guardados

- **Esquema:** una migración aditiva, `V11__memory_piece.sql`, con una tabla nueva: `memory_piece (save_id, piece, memory_id, context)`. No toca ninguna tabla existente. `MemoryStore::open` ya guarda un snapshot `pre-migration` en `<data>/backups` antes de aplicarla ([`specs/storage`](../../../specs/storage/spec.md)).
- **Contenido, hash, FTS, estados e identificadores:** no cambian. Lo único distinto en una fila nueva es el vector de los pedazos desde el segundo.
- **Contrato de embedding:** el modelo, la dimensión y el prefijo `passage:` no cambian (`V5__embedding_contract.sql`). El contexto es parte del texto embebido, como el `trigger` + `skill_name` de `procedural_skill` (`maintenance.rs:604-628`).
- **Datos de la versión anterior:** un test siembra en V10, con SQL directo, filas de `memory` (también un save largo como lo deja `main`: pedazos sueltos) con sus vectores, abre la base con la migración nueva y prueba que se buscan, se listan, se leen con `read_memory` (como saves de un pedazo) y se re-embeben sin contexto.
- **Volcado de la cola (`spill`):** `MemoryEvent` no cambia; un volcado de la versión anterior se lee igual.
- **Rollback:** una versión anterior rechaza la base sin tocarla (`SchemaNewer`). Dos vueltas atrás documentadas:
  - `clawg restore` del snapshot `pre-migration`: pierde lo guardado después de actualizar;
  - sin pérdida: `DROP TABLE memory_piece; DELETE FROM refinery_schema_history WHERE version = 11;` con el daemon parado. Los pedazos quedan como filas sueltas, como hoy, con los vectores con contexto que ya tenían (siguen sirviendo: el contexto sólo agrega tema).

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Con el contexto, un pedazo 2 que no tiene la respuesta sube por el tema y le gana a la fila que sí la tiene | Media | U4 sobre los subconjuntos de entidades parecidas (`multi_ref`, `high_importance`); el colapso deja a lo sumo un pedazo por save en el top |
| El colapso esconde un pedazo que tenía la respuesta detrás de uno que no | Media | El hit dice `piece=k/n` y `read_memory` trae el save entero; U2 mide la respuesta alcanzable con esa segunda llamada |
| Un save largo pasa a tener más pedazos, porque el tope baja de 480 a 424 tokens | Cierta | Un pedazo más cada ~7 en un texto muy largo; los textos de un pedazo no cambian (R1) |
| El vínculo falla después de insertar los pedazos | Baja | Las filas quedan como hoy, sueltas; el save no se informa como fallido (R3). Guardar el mismo texto de nuevo lo vincula |
| Un daemon viejo recibe `read_memory` | Baja | 404 en la ruta nueva; el puente dice que el daemon no la soporta en vez de inventar un save |
