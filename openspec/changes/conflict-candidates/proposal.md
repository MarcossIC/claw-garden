# conflict-candidates — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — | ver [`state.yaml`](state.yaml) |

## Problema

El detector de contradicciones mide **cercanía**, no contradicción, y aun así todo lo que encuentra se trata como una contradicción que el usuario tiene que resolver. Detalle y verificación en la [exploración](exploration.md#problema-verificado-contra-el-código). Lo que sigue se volvió a comprobar contra `main` (`fdd71eb`):

1. **Toda candidata marca la fila nueva `disputed`.** El worker llama a `detect_contradiction` entre embeber e insertar (`crates/clawg-core/src/memory/worker.rs:752-757`) y el insert escribe `status = 'disputed'` en cuanto hay una (`crates/clawg-core/src/memory/store_insert.rs:397-400`). La banda `[0,10, 0,42)` en L2 (`crates/clawg-core/src/memory/contradiction.rs:58`, `:66`, `:164-165`) es `cos ∈ (0,912, 0,995]`: una reformulación, un complemento o una actualización caen igual que una negación.
2. **Cada par llega al usuario.** ACTIVE CONFLICTS afirma que las memorias «contradict each other» y pide preguntar (`crates/clawg-core/src/memory/prompt_builder.rs:529-532`), y la apertura del protocolo manda preguntar sin condición (`crates/clawg-cli/src/mcp/protocol.md:24-26`).
3. **El veredicto no queda.** Cerrar un par saca el id de `disputed_against` y no escribe nada más (`crates/clawg-core/src/memory/dispute.rs:183-191`, `:225-237`). «Valen las dos» es un booleano (`crates/clawg-cli/src/daemon/api.rs:618-627`), sin motivo ni matiz.
4. **El agente se entera tarde.** La respuesta del save sólo dice qué filas guardó (`crates/clawg-cli/src/mcp/server.rs:521-545`; `SavedChunk`, `worker.rs:38-41`). El par aparece después, en otro `recall_context`, quizá en otra sesión y sin la conversación que produjo el save.
5. **La documentación del umbral se contradice.** El comentario de `CONTRADICTION_THRESHOLD` conserva la tabla del barrido con pooling CLS que argumenta por 0,35 (`contradiction.rs:35-57`), y la constante vale 0,42.

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas) de la exploración:

- **Se construye B + E ahora y se mide después.** El modelo real y el golden set no siempre están disponibles. La herramienta de medición y el set etiquetado se entregan con el cambio y se corren donde estén el modelo real y `docs/eval/`. El corte de escalamiento arranca en L2 0,40, entre la última contradicción medida (0,350) y el primer par compatible (0,469), y se recalibra con esa medición.
- **Cuatro veredictos:** `supersedes`, `compatible`, `scoped` (con motivo obligatorio) y `conflicts` (queda escalado al usuario).
- **Los pendientes se ven en `memory_status`:** el conteo y los pares más viejos, citados por id. ACTIVE CONFLICTS muestra sólo los pares escalados.
- **`disputed` queda como marca derivada:** una fila está `disputed` mientras tenga un par escalado sin juzgar. El filtro `status=disputed`, los listados y una versión anterior siguen leyendo igual.
- **Lo que el agente cierra solo:** `compatible` y `scoped` siempre. `supersedes` sobre un par pendiente con una `user_preference` o una `architectural_decision` se rechaza: primero `conflicts` y la pregunta al usuario.
- **Undelete:** el par vuelve con la fila restaurada (cambia `specs/disputes` R4).
- **Pares que ya existían:** todos quedan escalados, porque no se guardó su distancia.
- **Colapso por save:** un par por pareja de saves.
- **Respuesta del save:** ids con la primera línea de cada candidata (hasta 80 caracteres), y una línea sólo cuando hay alguna.

## Alcance

1. Migración V15: tabla `memory_conflict` y conversión de las disputas guardadas a pares `escalated`.
2. El detector devuelve candidatas con su distancia, tipo y texto. Una función pura `needs_user` decide si el par escala. El insert anota los pares en la misma transacción que la fila, y el worker colapsa por save.
3. Veredictos tipados en el núcleo (`judge_pair`), en `POST /v1/disputes/judge` y en `resolve_conflict`. `POST /v1/disputes/resolve` sigue aceptando `keep_both` y guarda el veredicto equivalente.
4. `GET /v1/disputes` (conteo y pares pendientes del agente) y su resumen en `memory_status`.
5. La respuesta del save lista las candidatas (campo opcional `related`).
6. `CONFLICT_NOTE` y el protocolo, coherentes y sin afirmar una contradicción que no se confirmó.
7. Purge borra los pares de la fila. Undelete devuelve el par. El mantenimiento borra los pares que nombran una fila que ya no existe.
8. Herramienta `conflict-eval` en `clawg-eval` y set etiquetado de pares por clase.
9. Se borra la tabla CLS de `contradiction.rs`. Se actualizan `.claude/CLAUDE.md` y las specs `disputes`, `mcp`, `retrieval` y `storage`.

## Fuera de alcance

- Un modelo o un reranker que juzgue los pares.
- Cambiar el umbral o la banda del detector.
- Pares entre agentes, o que un agente juzgue pares de otro.
- Detección sobre `episodic_trajectory` o `procedural_skill`.
- Cambiar el ranking: `disputed` y `active` siguen puntuando igual (`crates/clawg-core/src/memory/score.rs`).
- La señal `topic_key` (topic-revisions) y el origen de la fila (memory-provenance) como entradas de `needs_user`.

## Datos guardados

- **Esquema:** una tabla nueva, `memory_conflict`. No se toca ninguna columna de `memory`.
- **Conversión:** cada id que nombra un `disputed_against` válido entra como par `escalated` con `distance = NULL`. Las filas conservan `status`, `disputed_against` y `last_disputed_at`, así que lo que ya estaba en ACTIVE CONFLICTS sigue ahí.
- **Antes de migrar** se escribe la copia `pre-migration` (`crates/clawg-core/src/memory/db.rs`).
- **Rollback:** un binario anterior abre la base, porque refinery acepta migraciones aplicadas que no conoce. Ignora la tabla y lee `memory` como siempre: los pares escalados siguen `disputed` y los pendientes se ven `active`, igual que si el detector no los hubiera visto. Para quitar la tabla basta con `DROP TABLE memory_conflict; DELETE FROM refinery_schema_history WHERE version = 15;`.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- un save cercano a otra fila de tipo `semantic_fact`, sin un identificador compartido, deja las dos `active` y un par `pending` con su distancia;
- un par juzgado no vuelve a anotarse y `compatible` y `scoped` no cambian el ranking;
- `supersedes` conserva R1 y R3 de `specs/disputes`;
- la base sembrada en V14 con disputas abre con sus pares `escalated` y las mismas filas `disputed`;
- la respuesta del save nombra las candidatas, y un save sin candidatas responde como hoy;
- `memory_status` cuenta los pendientes.

`rag-eval --diff`: el ancla debe dar cero casos movidos, porque el ranking no cambia. La compuerta que la exploración propone para el detector se mide con `conflict-eval --real`, fijada antes de correrla: menos del 5 % de pares escalados por save en el flujo de hechos distintos, y que la clase contradicción no baje de 0,8 de candidatas. Hasta que se corra queda pendiente en [`verify.md`](verify.md).
