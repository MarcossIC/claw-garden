# secret-hygiene — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | S–M | — (tiene que cerrar antes de visibility-policy) | propuesta |

Sale de la [exploración](exploration.md). Las cuatro decisiones abiertas quedan tomadas acá y en `design.md`.

## Problema

Un secreto que un agente guarda por error (una API key, un token, una contraseña dentro de una URL) queda en texto plano para siempre, y la única forma de sacarlo es SQL a mano sobre `memory.db`.

- **No hay filtro al guardar.** `process_event` trocea `evt.content` tal cual llega (`crates/clawg-core/src/memory/worker.rs:422-433`). Después calcula el hash de dedup de cada chunk (`collapse_duplicate_chunks`, `worker.rs:134`), lo embebe (`worker.rs:485`) y lo inserta en `memory`, el vector y el FTS en una sola transacción (`crates/clawg-core/src/memory/store_insert.rs:329-372`). No hay `redact` ni patrones de secretos en ningún crate.
- **Borrar no lo saca.** `delete_memory_core` sólo pone `deleted_at` (`crates/clawg-core/src/memory/admin.rs:202`, `:223-264`): el texto queda en la fila. El trigger de borrado lógico resta los términos del índice FTS (`crates/clawg-core/migrations/V8__memory_fts.sql:73-78`), pero FTS5 lo registra como una marca nueva en otro segmento: los términos siguen en `memory_fts_data` hasta que un merge los descarte. Nada borra filas de `memory` (`crates/clawg-core/src/memory/maintenance.rs:249-251`).
- **El riesgo crece con el cerebro compartido.** Hoy search y recall filtran por `agent_id` (`crates/clawg-core/src/memory/search.rs:119-122`), así que un secreto vuelve sólo al agente que lo guardó. visibility-policy va a compartir memorias entre agentes, y un secreto llegaría al contexto de otro cliente, con otro proveedor.
- **Las copias se multiplican.** `workspace.md` vuelca memorias de tier 1 a disco en cada `recall_context` (`crates/clawg-cli/src/daemon/api.rs:714-723`, `crates/clawg-core/src/memory/workspace.rs:106`), y cada backup de storage-safety-net es otra copia completa de la base (`crates/clawg-core/src/memory/backup.rs:275`).

## Alcance

1. **Enmascarado al ingerir.** Una función pura `redact(&str)` reemplaza cada secreto reconocido por `[REDACTED:<tipo>]` y dice qué tipos encontró. Corre en `process_event` antes del chunking, así que el hash de dedup, el embedding, el FTS y el texto guardado nunca ven el secreto. Se enmascara en lugar de rechazar el save: el resto del contenido es memoria legítima.
2. **Patrones de alta precisión, sin entropía.** Prefijos conocidos (AWS `AKIA`/`ASIA`, GitHub `ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`/`github_pat_`, Anthropic `sk-ant-`, OpenAI `sk-`, Slack `xox?-`, Google `AIza`, Stripe `sk_live_`/`rk_live_`), bloques PEM de clave privada, JWT y la contraseña de `esquema://usuario:contraseña@host`. Nada de heurísticas de entropía: marcarían hashes de commit, UUIDs y checksums, que sí son memoria.
3. **El save dice qué enmascaró.** `POST /v1/memories` agrega `redacted: ["github_token", ...]` a la respuesta confirmada, y `save_memory` se lo cuenta al agente. Es posible porque ingest-durability hizo el save síncrono ([`specs/ingest`](../../specs/ingest/spec.md)).
4. **Escaneo de lo ya guardado: `clawg secrets [--profile P]`.** Lista `memory <id> <tipos>` de las filas de `memory`, vivas o borradas, que tienen un patrón de secreto, **sin imprimir el secreto**. Sólo lee: no toma el lock y funciona con el daemon andando. No va en `clawg doctor`, que no abre el store (`.claude/CLAUDE.md`).
5. **Purga explícita, sólo por CLI: `clawg purge <ID> --yes [--profile P]`.** Borra para siempre una fila de `memory`: el texto, el vector y los términos del FTS, también de las páginas libres y del WAL. Pide `--yes` porque no tiene vuelta atrás. Con el daemon parado, como `clawg restore`. Nunca como herramienta MCP ni por la API HTTP: destruir datos es una decisión del usuario, no del agente (`openspec/config.yaml`, regla general sobre la base del usuario).
6. **La purga nombra los backups que todavía tienen la fila.** No los reescribe: son la red de seguridad del usuario, y tocarlos es exactamente lo que esa red evita. El usuario decide si los borra.

**Enfoque:** detección y decisiones puras (`redact`, `find_secrets`, el armado de los mensajes); el I/O en un módulo nuevo de `clawg-core`, `memory::secrets`, con el mismo patrón de lock y conexión directa que `backup_offline` y `restore`. Alternativas en `design.md`.

## Fuera de alcance

- **Cifrado en reposo.** Descartado en el README (lo cubre el cifrado del sistema operativo).
- **Datos personales en general** (mails, teléfonos, nombres). No son secretos de acceso y hay mucha memoria legítima que los nombra.
- **Reescribir automáticamente lo ya guardado.** Violaría la regla de no tocar la base del usuario. El escaneo informa y el usuario purga.
- **Purgar `episodic_trajectory` y `procedural_skill`.** Ningún código de producción las escribe (README, "Sobre el estado actual": extractor, miner y resolver inactivos). Se suman cuando knowledge-synthesis les dé un escritor.
- **`workspace.md`.** Su sección AUTO se regenera desde la base en el próximo `recall_context` de ese agente y proyecto (`crates/clawg-cli/src/daemon/api.rs:714-723`). La sección USER la escribe el usuario.
- **El volcado `ingest-pending.jsonl`.** Guarda eventos aceptados que todavía no pasaron por el worker, y se borra al reinyectarlos (`crates/clawg-core/src/memory/spill.rs:44-57`). El enmascarado ocurre al ingerirlos.
- **El vector de la fila.** Ya se borra con el delete lógico (`admin.rs:246`); la purga también lo borra. Un embedding no es el texto.
- **Bloques libres del sistema de archivos.** Lo que el disco guarde por debajo de SQLite (por ejemplo, después de truncar el WAL) es del sistema operativo.
- **Un secreto partido entre dos saves**, o escrito con espacios en el medio. Ningún patrón de alta precisión lo ve.

## Éxito

`cargo test --workspace` en verde. Cada uno de estos tests falla hoy:

| # | Test | Falla hoy porque |
|---|---|---|
| 1 | Un save con una key de cada tipo guarda el placeholder, y una búsqueda FTS por la key no devuelve nada | se guarda tal cual |
| 2 | Un hash de commit, un UUID, un SHA-256, un identificador largo de código, `sk-learn` y una URL con `${DB_PASSWORD}` no se enmascaran | no existe `redact` |
| 3 | Dos saves que sólo difieren en el secreto enmascarado se deduplican (`reinforced`) | se insertan dos filas |
| 4 | La respuesta de `POST /v1/memories` lista los tipos enmascarados, y `save_memory` lo dice | no hay campo |
| 5 | `clawg secrets` reporta filas vivas y borradas con secretos, sin el secreto en la salida, con el daemon andando | no existe |
| 6 | Después de `clawg purge`, ni `memory`, ni el vector, ni el FTS tienen la fila, y los bytes del secreto no están en `memory.db` ni en `memory.db-wal` | no existe |
| 7 | `clawg purge` se niega sin `--yes`, con el daemon andando, con un id que no existe y con una base que no está al día | no existe |
| 8 | La purga nombra los backups que tienen la fila | no existe |

**`rag-eval --real`:** métricas idénticas a la base (R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693). El golden set no tiene secretos, así que el enmascarado no cambia ningún texto. Necesita `docs/eval/`, que está en la máquina del usuario.

## Datos guardados

- **Esquema:** no cambia. No hay migración.
- **Filas existentes:** no se tocan. El enmascarado sólo corre sobre saves nuevos; lo viejo se ve con `clawg secrets` y se saca con `clawg purge`.
- **`clawg purge`** es el único camino nuevo que destruye datos, y sólo una fila por vez, con `--yes`, con el daemon parado. No deja snapshot previo a propósito: sería otra copia del secreto que se quiere sacar.
- **Rollback:** revertir los commits. Las filas guardadas con placeholder siguen siendo memoria válida para cualquier binario; el secreto original nunca se guardó y no se recupera.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Falso positivo: se enmascara algo que no era secreto | Baja | Sólo prefijos con formato fijo y estructura (PEM, JWT con `eyJ` en dos partes, URL con contraseña). Las keys de forma libre (`sk-`) piden letras y dígitos. Tests negativos con hashes, UUIDs, identificadores y `${VAR}` |
| Falso negativo: un secreto sin formato conocido (una contraseña suelta) | Media | Es el límite de no usar entropía. Para eso están `clawg purge` y la instrucción al agente de no guardar secretos |
| La purga deja el secreto en un backup | Media | La purga lista los backups que tienen la fila. Borrarlos es decisión del usuario |
| `optimize` del FTS en una base grande tarda | Baja | Es un comando manual, sobre un store personal. Reescribe el índice una vez |
| El regex cuesta en cada save | Baja | `regex` ya está compilado en el binario (lo traen `tokenizers`, `refinery-core` y `tree-sitter`); los patrones se compilan una vez. Al lado de un embedding, es ruido |
