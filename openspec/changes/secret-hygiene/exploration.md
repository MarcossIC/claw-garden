# secret-hygiene — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | S | — (conviene antes de visibility-policy) | exploración (borrador) |

**Tesis:** un secreto que un agente guarda por error (una API key, un token, una contraseña en un connection string) queda en texto plano **para siempre** y no hay forma de sacarlo sin tocar la base a mano. Hoy queda encerrado en el agente y proyecto que lo guardó. Cuando visibility-policy abra el cerebro compartido (objetivos 2 y 4), ese secreto va a viajar al contexto de otros agentes y otros proyectos, y de ahí al proveedor de cada uno.

## Problema (verificado)

### No hay ningún filtro de contenido al guardar

- El worker trocea `evt.content` tal cual llega (`crates/clawg-core/src/memory/worker.rs:269-274`).
- Después calcula el hash de dedup sobre el chunk (`worker.rs:98`), lo embebe y lo inserta en `memory`, el vector y el FTS en una sola transacción (`crates/clawg-core/src/memory/store_insert.rs:329-372`).
- No hay `redact`, ni patrones de secretos, en ningún crate.

### Borrar no saca el secreto

- El delete es lógico: el texto queda con `deleted_at` (`crates/clawg-core/src/memory/admin.rs:205-251`).
- No hay purga (`crates/clawg-core/src/memory/maintenance.rs:229-238`).
- Resultado: el único camino para eliminar un secreto guardado es SQL a mano sobre `memory.db`.

### Copias que se multiplican

- `workspace.md` vuelca contenido de memorias a disco en cada `recall_context` (`crates/clawg-core/src/memory/workspace.rs:89`, `:202-246`).
- data-preservation parte B va a agregar backups: cada uno es otra copia en claro de toda la base.

### Hoy el alcance está contenido, mañana no

- search/recall filtran por `m.agent_id = ?3` (`crates/clawg-core/src/memory/search.rs:119-122`). Hoy un secreto sólo vuelve al agente que lo guardó.
- `user_preference` y las filas sin proyecto ya cruzan proyectos (mismo predicado).
- visibility-policy va a compartir entre agentes. Un secreto guardado por un agente puede terminar en el contexto de otro cliente con otro proveedor.

## Por qué no cifrado en reposo

El cifrado (SQLCipher) protege contra el robo del disco. Eso ya lo cubre el cifrado del sistema operativo (BitLocker), y en la misma máquina los `.env` de los repos ya están en claro. Lo que **sí es propio** de Claw Garden es que **mueve** el secreto entre proyectos y agentes. Ese es el riesgo que ataca este cambio.

## Alcance candidato

1. **Enmascarado al ingerir.** Una función pura `redact(&str) -> (String, Vec<RedactionKind>)` que corre antes del chunking (`worker.rs:269`), así el hash, el embedding, el FTS y el texto guardado nunca ven el secreto. Reemplaza el match por `[REDACTED:<tipo>]`.
2. **Patrones de alta precisión, sin entropía.** Prefijos conocidos (AWS `AKIA`/`ASIA`, GitHub `ghp_`/`github_pat_`, Anthropic `sk-ant-`, OpenAI `sk-`, Slack `xox?-`, Google `AIza`, Stripe `sk_live_`), bloques PEM de clave privada, JWT y `scheme://user:password@host`. **Nada de heurísticas de entropía:** marcarían hashes de commit, UUIDs y checksums, que sí son memoria legítima.
3. **Purga explícita, sólo por CLI.** `clawg purge <kind> <id>`: borra el texto, el vector y el FTS de una fila. Nunca como herramienta MCP: destruir datos es una decisión del usuario, no del agente (regla de `config.yaml`). Se coordina con data-preservation parte E.
4. **Reporte de lo que ya está guardado.** Un escaneo de solo lectura (por ejemplo dentro de `clawg doctor` o un comando aparte) que lista `(kind, id, tipo)` de filas con patrones de secreto, **sin imprimir el secreto**. No modifica nada: el usuario decide si purga.

**Fuera de alcance:** cifrado en reposo; redactar datos personales en general; reescribir automáticamente memorias ya guardadas (violaría la regla de no tocar la base del usuario).

## Decisiones abiertas

- [ ] **¿Enmascarar o rechazar el save?** Enmascarar conserva el resto del contenido; rechazar obliga al agente a reformular. Si ingest-durability elige el save síncrono, la respuesta puede informar qué se enmascaró.
- [ ] **Dependencia.** `regex` ya está en el árbol como transitiva (`Cargo.lock`). ¿Se agrega como directa o se escriben los matchers a mano? Los prefijos fijos no necesitan regex; el PEM y el connection string son más cómodos con regex.
- [ ] **Purga y backups.** Un secreto purgado sigue en los backups hasta que roten. ¿Se documenta, o la purga también reescribe los backups retenidos?
- [ ] **¿El escaneo va en `clawg doctor`?** Hoy `doctor` no abre el store (CLAUDE.md), así que probablemente sea un comando aparte.

## Cómo se mide

Tests RED primero:

1. Un save con una key de cada tipo: el texto guardado tiene el placeholder y una búsqueda FTS por la key no devuelve nada.
2. Negativos: un hash de commit, un UUID y un identificador largo de código **no** se enmascaran.
3. Dedup: dos saves que sólo difieren en el secreto enmascarado se deduplican.
4. Purga: después de `clawg purge`, el texto no está en `memory`, ni el vector, ni la fila FTS.
5. El escaneo reporta la fila sin incluir el secreto en su salida.

`rag-eval --real` con métricas idénticas: el golden set no tiene secretos.
