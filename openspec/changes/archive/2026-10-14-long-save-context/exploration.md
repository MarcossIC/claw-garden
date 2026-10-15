# long-save-context — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | eval-golden-expansion; data-preservation si toca el esquema | cerrada: sigue en la [propuesta](proposal.md) |

**Tesis:** un save largo se guarda como pedazos sueltos, y cada uno se busca solo, sin saber de qué save viene. El pedazo que se queda con la segunda mitad del texto pierde el tema, y su vector también.

## Revisión contra `main` (2026-10-15)

La exploración es del 2026-10-12. Desde entonces entraron ingest-durability, dispute-resolution, knowledge-synthesis, secret-hygiene, memory-update-tool y context-density. Se revisó cada referencia contra `main` en `9c3ff19`:

- **Los hermanos ya no se disputan.** dispute-resolution confirmó la hipótesis con el modelo real (220 de 263 pedazos quedaban `disputed`) y los excluye del detector (`worker.rs:528-557`, `contradiction.rs:128-147`). Esa parte sale del alcance.
- **El save ya no responde "queued" antes de trocear.** Es síncrono desde ingest-durability, y `save_memory` dice `saved as 3 pieces: #57, #58, #59` (`crates/clawg-cli/src/mcp/server.rs:416-424`). La opción c quedó hecha.
- **Los secretos se enmascaran antes de trocear** (`worker.rs:429-434`). La cabecera sale del texto enmascarado.
- **Las líneas se movieron.** El embebido está en `worker.rs:504`, la descripción de `save_memory` en `server.rs:696-704` y su test en `server.rs:1526-1534`. `K_FINAL` está en `prompt_builder.rs:34`. Los hits de `recall_context` llevan id desde context-density (`prompt_builder.rs:296-303`).
- **El save entra por `api.rs:711-726`**, con `Role::User` y `MemorySource::Explicit`; `runtime.rs:262` ya no es ese camino.
- **eval-golden-expansion no agregó memorias largas.** Su generador suma distractores, `lexical`, `multi_ref` y los de importancia ([`specs/eval`](../../../specs/eval/spec.md)); ninguna fila llega al corte y ningún caso tiene la respuesta después de él. La medición tiene que venir con este cambio (R9 de la spec).
- **Siguen igual:** el chunker (`chunker/mod.rs:15, 21`, `chat.rs:67-72`), una fila por pedazo sin vínculo (ninguna migración hasta V10 lo agrega), los 500 caracteres de `search_memory` (`tools.rs:26`), `seed.rs:71-82` sin trocear y el criterio de `found` del agent-view (`agent_view/mod.rs:116-146`).

Las líneas de abajo son las del 2026-10-12.

## Estado actual (verificado el 2026-10-12)

### Cómo se parte un save

- Un save explícito entra con `Role::User` (`crates/clawg-core/src/runtime.rs:262`) y lo trocea el chunker de chat.
- Más de 480 tokens se parten con 64 de solapamiento (`crates/clawg-core/src/memory/chunker/mod.rs:15, 21`; `chunker/chat.rs:67-72`). No se antepone ningún contexto: "No markdown-fence or AST-aware enrichment" (`chat.rs:9`).
- Se embebe sólo el texto de cada pedazo (`crates/clawg-core/src/memory/worker.rs:321`).
- Cada pedazo tiene su propio hash de dedup (`worker.rs:98`). Pasa solo por el detector de contradicciones y se inserta de a uno (`worker.rs:344-372`).
- Queda como una fila de `memory` sin referencia a sus hermanos: ninguna migración agrega una columna que los vincule.

### Qué ve el agente

- Cada pedazo es un hit distinto, y todos compiten por los 10 lugares (`crates/clawg-core/src/memory/prompt_builder.rs:32`). `search_memory` muestra 500 caracteres de cada uno (`crates/clawg-cli/src/mcp/tools.rs:23`).
- Ninguna tool los rearma. La propia descripción de `save_memory` lo avisa y pide "one fact per call" (`crates/clawg-cli/src/mcp/server.rs:395-398`), y un test fija esa guía (`server.rs:706-714`).

### Qué pasa con el vector

El segundo pedazo empieza a mitad del texto. Si el tema (el nombre de un crate, la decisión, el bug) sólo aparecía al principio, el vector de ese pedazo no lo lleva, y una búsqueda por el tema no lo encuentra.

Los 64 tokens de solapamiento protegen un hecho que cruza el corte, no el tema del save completo.

### Con qué frecuencia pasa

- **No se sabe.** El corpus del golden set (`docs/eval/golden.json`) tiene 48 filas, y la más larga tiene 662 caracteres: ninguna llega al corte.
- **Los datos de prod no están medidos.** Leer `~/.clawg` requiere autorización.
- **Dónde va a aparecer:** en los `session_summary` y en la migración planificada desde engram. Una importación no sigue el "one fact per call" que la tool le pide al agente.

## Por qué hoy no se puede medir

- **`rag-eval --real` no trocea.** Siembra cada fila del corpus entera (`crates/clawg-eval/src/bin/rag-eval/seed.rs:71-82`): una fila, un vector. Una fila larga quedaría truncada por el embedder a 512 tokens, en lugar de partida como en producción.
- **`--agent-view` sí trocea**, porque usa el worker real (`agent_view/runner.rs:82-84`). Pero da una memoria por encontrada si cualquier fragmento visible le pertenece (`agent_view/mod.rs:127-132, 149-153`). Un pedazo sin la respuesta cuenta como acierto.

Medirlo necesita tres cosas de eval-golden-expansion:

- memorias largas en el corpus;
- casos cuya respuesta caiga después del corte;
- un chequeo de que la respuesta sea visible, no sólo un fragmento de la memoria.

## Enfoques

| Opción | Qué hace | A favor | En contra |
|---|---|---|---|
| a. Cabecera por pedazo | Antepone a cada pedazo, desde el segundo, el comienzo del save (su primera línea o sus primeros tokens) antes de embeber | Sin cambio de esquema. Ataca justo el vector que pierde el tema | La cabecera come parte de los 480 tokens. Falta decidir dónde vive (ver abajo) |
| b. Puntero al save y rearmado | Cada pedazo guarda de qué save viene y en qué posición. El ranking junta a los hermanos en un solo hit, y el agente puede pedir el save completo | También resuelve que los hermanos ocupen el top 10 y que nada los rearme | Toca el esquema: migración, rollback y test de supervivencia de datos. Las filas existentes no tienen puntero |
| c. Avisar al guardar | La respuesta de `save_memory` dice en cuántos pedazos se partió | Barato. Refuerza la guía que la tool ya tiene | Sólo sirve si el agente reacciona, y no ayuda a una importación. El save responde "queued" antes de trocear, así que hay que contar tokens antes de encolar |

**Dónde vive la cabecera de a:**

- **Guardada en `content`:** cambia el hash, el FTS y lo que ve el agente.
- **Sólo embebida:** el texto guardado deja de coincidir con su vector. Es el desajuste que `chunker/mod.rs:12-14` busca evitar.

## Relación con otros cambios

- **dispute-resolution:** tiene sin verificar la hipótesis de que los pedazos hermanos se disputan entre sí. La causa es la misma. Con b, el detector puede saltear a los hermanos.
- **ingest-durability:** su opción A (save síncrono) devolvería varios ids para un save con varios pedazos. Con b tendrían un id común.
- **eval-golden-expansion:** pone los casos que hacen falta para medir (ver arriba).
- **data-preservation:** su red tiene que estar antes, si se elige b.

## Decisiones abiertas

- [ ] **¿Cuántos saves pasan hoy el corte?** Medirlo en prod (con autorización) y sobre el export de engram antes de migrar. Si casi ninguno pasa, alcanza con c.
- [ ] **¿a, b o una combinación?** a es chico y no toca datos guardados. b es el arreglo completo.
- [ ] **Si a: ¿la cabecera se guarda o sólo se embebe?**
- [ ] **Si b: ¿qué pasa con las filas existentes?** Quedan sin puntero, como hoy, o se intenta inferirlo.

## Cómo se mide

- **Casos con memorias largas** y la respuesta después del corte, medidos con `--agent-view` sobre la respuesta visible.
- **Las memorias cortas no se mueven:** `rag-eval --real` igual al baseline.
- **Si b:** cuántos lugares del top 10 ocupan pedazos del mismo save, antes y después.
