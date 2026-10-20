# doctor-checks — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | 26 client-setup y 29 project-root-guard (archivados); se coordina con 53 (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`2773c88`). Desde la exploración cambiaron tres cosas: `purge` ya vacía el `context` de los pedazos de su save (`crates/clawg-core/src/memory/secrets.rs:221-224`), `to_prune` ya conserva el más nuevo de cada motivo (`crates/clawg-core/src/memory/backup.rs:246-251`) y el spill sólo lo escribía un clawg anterior: el actual lo pasa al outbox al arrancar (`crates/clawg-core/src/memory/spill.rs:1-4`).

`clawg doctor` responde una sola pregunta: qué `clawg` resuelve el PATH (`analyze`, `crates/clawg-cli/src/doctor.rs:46`; `run`, `doctor.rs:112`, que no lee nada del directorio de datos). Lo que deja a un usuario con una memoria que "no anda" se descubre por accidente:

1. **Estado que nada comprueba.** El esquema no impone las reglas de `memory`: sin claves foráneas ni `CHECK` sobre `status`, `superseded_by`, `revived_from` ni `disputed_against` (`crates/clawg-core/migrations/V1__init.sql:24-27`; «No foreign keys» en `V11__memory_piece.sql:8` y `V15__memory_conflict.sql:8`), y los índices de tema y de dedup no son únicos (`V16__topic_key.sql:9-15`, `V6__content_hash.sql:9-16`). Dos versiones vivas de un tema, un `superseded_by` hacia una fila que ya no existe o dos filas vivas con el mismo hash no se ven en ningún lado.
2. **Diagnósticos que exigen el daemon.** El contrato de embedding distinto sólo se ve en `/health` y `clawg status` con el daemon vivo; con el daemon parado no hay forma de saber si va a arrancar con la memoria apagada.
3. **Cosas sin salida visible.** El último snapshot de `<data>/backups` puede tener meses (sólo `clawg backup` saca uno manual); `ingest-pending.rejected.jsonl` son saves de alguien que nada borra (`spill.rs:13-15`); un `sharing.json` inválido no comparte nada y sólo lo dicen el log, `memory_status` y `clawg sharing`; los saves `failed` del outbox sólo los cuenta `/health`; un registro MCP que apunta a un binario borrado sólo se ve en el cliente.

## Objetivo que sirve

Transversal: protege el activo más importante, la base del usuario, diciendo qué está mal, por qué, con qué evidencia y cuál es el siguiente paso seguro, sin escribir nada. Va antes de 44 (history-import): sirve para revisar el store antes y después de una importación.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Store:** `clawg doctor` lo lee siempre, sin flag, en sólo lectura y sin lock (enfoque B), como `clawg secrets`. Sin `memory.db` lo dice y sigue.
- **Código de salida:** 1 sólo si hay un hallazgo `error`; 0 con `ok`, `info` o `warn`. El PATH sombreado o duplicado sigue siendo `error`, así que lo que hoy sale 1 sigue saliendo 1.
- **MCP:** ni herramienta nueva ni línea en `memory_status`. `doctor` es del usuario.
- **Reparación:** ninguna. `doctor` nunca escribe; cada hallazgo nombra el comando que existe para resolverlo, o ninguno.
- **Umbral de backup viejo:** 30 días, y aviso cuando un store con filas no tiene ningún snapshot.
- **Salida:** sólo JSON, como hoy. Las claves de hoy quedan; se suman `data_dir`, `checks` y `summary`.
- **Invariantes:** corren en cada `doctor`. La severidad de cada una está en la [spec](spec.md#r6-invariantes-del-estado-de-memory).
- **Poda de snapshots:** ya resuelta en `to_prune`; queda fuera.

## Alcance

1. `clawg-core`: `memory::inspect`, una función pública que abre el store con `open_for_probe` y devuelve hechos (esquema, contrato, agentes y conteos, proyectos, outbox, violaciones de invariantes por id), sin contenido de filas; `backup::parse_snapshot_time` (pura) y `embedder::expected_contract`.
2. `clawg-cli`: `doctor` pasa a ser un módulo con el sobre uniforme (`Finding`), chequeos puros sobre hechos y la lectura en `run`: PATH, directorio de datos, socket, titular del store, esquema, contrato, backups, spill, `sharing.json`, outbox, agentes, proyectos, las seis invariantes y el registro MCP de Claude Code.
3. `main.rs`: el código de salida sale del resumen.
4. Docs: `.claude/CLAUDE.md` (el `doctor` ya no es "no abre el store"), y al archivar una spec nueva, `openspec/specs/doctor`.

## Fuera de alcance

- Reparar nada (`apply`), migrar, restaurar o re-embeber desde `doctor`.
- Una herramienta MCP o una línea en `memory_status`; un endpoint nuevo.
- Fusionar o renombrar agentes; registrar el servidor MCP (eso es `clawg setup`).
- Backups periódicos automáticos.
- Una salida de texto además del JSON.
- Ranking: no toca retrieval, no se corre `rag-eval`.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- el JSON conserva `exe`, `version`, `profile`, `installs`, `winner`, `shadowed`, `duplicates` y suma `checks` y `summary`;
- cada chequeo, sano y roto, sobre hechos fabricados;
- cada una de las seis invariantes, violada en una base fabricada, da su hallazgo, y la base sana ninguno; los pedazos de un save no cuentan como dos versiones de un tema;
- `inspect` sobre un store con el daemon vivo y sobre uno en una versión anterior no cambia `memory.db` ni su WAL;
- una fila con un secreto no aparece en la salida de `doctor`;
- `clawg doctor` sale 0 con un `warn` y 1 con un `error`.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding: `doctor` sólo lee, sin lock, con `open_for_probe`, que abre `immutable` un store parado y en sólo lectura uno con WAL (`backup.rs:531-543`). Un store de una versión más nueva o divergente no se lee más allá de su historia de migraciones. Volver atrás es volver al binario anterior.
