# doctor-checks — Diseño

## D1. Hechos en `clawg-core`, decisiones en `clawg-cli` (R3, R6)

**Elegido:** `clawg_core::memory::inspect::inspect(base_dir) -> Inspection` abre el store con `open_for_probe` (`crates/clawg-core/src/memory/backup.rs:531`), el mismo camino sin lock de `secrets::scan` (`crates/clawg-core/src/memory/secrets.rs:53`) y de `backup::probe` (`backup.rs:509`), y devuelve **hechos**: el `StoreProbe` del esquema y, sólo cuando el esquema está al día o con migraciones pendientes, un `StoreFacts` con el contrato guardado, los agentes y sus conteos, las filas de `project`, el outbox (cantidad `pending`, cada `failed` por `operation_id`) y las violaciones de las seis invariantes por id. Las severidades, los mensajes y los siguientes pasos son funciones puras en `crates/clawg-cli/src/doctor/` sobre esos hechos.

Las invariantes son consultas SQL, así que su prueba es contra una base fabricada en `inspect.rs`; lo que se prueba sin base es cómo cada hecho se vuelve un hallazgo.

| Alternativa | Por qué no |
|---|---|
| Abrir `open_for_probe` a `pub` y leer desde `clawg-cli` | Abre visibilidad de la capa de almacenamiento a un crate que hoy no escribe SQL; `secrets::scan` ya marcó el patrón de una función pública que devuelve hechos |
| Leer por la API del daemon | Exige el daemon vivo, que es justo lo que `doctor` no debe exigir, y un endpoint nuevo por algo legible sin él |
| Severidad en `clawg-core` | Es una política de presentación del CLI, no del motor |

**Esquema que no es el de hoy.** Con migraciones pendientes cada consulta comprueba antes que su tabla y sus columnas existan (`topic_key` desde V16, `revived_from` y `supersede_cause` desde V17, `memory_piece` desde V11, `ingest_outbox` desde V14, `project` desde V9) y, si faltan, deja ese hecho vacío. Un store más nuevo o divergente no se lee: sus columnas pueden significar otra cosa.

**Sin contenido.** `StoreFacts` no tiene ningún campo de texto de fila: sólo ids, agentes, proyectos, `operation_id`, conteos y el contrato (constantes del modelo). Lo que no se lee no se puede filtrar.

## D2. El sobre y el código de salida (R1, R2)

**Elegido:** `Finding { check_id, severity, reason_code, message, evidence, safe_next_step, requires_confirmation }`, con `evidence` como `serde_json::Value` (un objeto distinto por chequeo, sin un tipo por chequeo que nadie más lee). `Report` suma `data_dir`, `checks` y `summary` sin tocar sus claves aplanadas. El código de salida es 1 si `summary.error > 0`.

`message` se suma al sobre de la exploración: el JSON se pega en issues y chats, y una frase dice más que un código a quien no conoce el código.

| Alternativa | Por qué no |
|---|---|
| 2 para `warn` | Un script que hoy mira `!= 0` empezaría a fallar por un backup viejo |
| Un `enum` de evidencia por chequeo | Diecinueve tipos para un JSON que sólo se imprime |
| Reemplazar `installs`/`winner` por el hallazgo `path` | Rompe a quien lee las claves de hoy |

## D3. Severidades

`error` es para lo que hoy deja la memoria sin funcionar o el PATH equivocado: PATH sombreado o duplicado, contrato distinto, esquema más nuevo, divergente o ilegible, una ruta de socket inusable (el daemon no arranca sin `--port`, y la autoarrancada no lo pasa) y un registro MCP a un binario que no existe. `warn` es lo que pierde o esconde datos sin apagar la memoria (invariantes 1, 2 por revisión, 4 y 5; backups; saves `failed` o rechazados; `sharing.json` inválido; agente de respaldo; proyecto con id que no corresponde). `info` es lo que se arregla solo o es un efecto buscado (migración pendiente, re-embebido de pooling, spill pendiente, disputa sin rival hasta la limpieza diaria, `superseded_by` de un veredicto cuyo ganador se purgó, `revived_from` de una fila purgada).

**Umbral de backup: 30 días.** Un punto de partida, no medido: un mes de memorias es lo que se está dispuesto a perder sin aviso. Sin ningún snapshot en un store con filas también es `warn`, porque el único que se saca solo es el de una migración, un restore, un re-embed o una importación (`Reason`, `backup.rs:127-139`).

## D4. El registro MCP

**Elegido:** leer el mismo `.claude.json` que `clawg setup claude-code` (`locations`, `crates/clawg-cli/src/setup.rs:625`) y decidir con una función pura del texto. Sólo se mira la entrada del servidor del perfil; se compara su `command` con el binario que corre con la misma identidad de ruta que el chequeo del PATH (mayúsculas, barras y `\\?\` en Windows). Un `command` relativo (por nombre en el PATH) lo cubre el chequeo del PATH, así que sólo se mira `--agent`.

## D5. Lo que no se hace

Ni reparación ni snapshot: `doctor` nunca escribe. El único arreglo que la exploración justificaba, las cabeceras de `memory_piece` que dejaba `purge`, ya lo hace `purge` de origen (`secrets.rs:221-224`); el chequeo 5 sólo encuentra lo que dejó un clawg anterior, y repararlo sería código nuevo sobre datos del usuario para un caso que ya no se produce.
