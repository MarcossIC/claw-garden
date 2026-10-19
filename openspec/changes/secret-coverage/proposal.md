# secret-coverage — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (va antes de memory-portability, 43, y history-import, 44) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y citas en la [exploración](exploration.md#problema-verificado-contra-el-código), verificada contra `develop` (`bf5c1e4`).

1. **El detector no ve claves que llegan por nombre o por prefijo.** Los patrones por contexto sólo se anclan en `password` y en `Authorization` (`crates/clawg-core/src/memory/redact.rs:101-108`): `OPENROUTER_API_KEY=…`, `AWS_SECRET_ACCESS_KEY=…`, `"client_secret": "…"`, `AccountKey=…` o `X-Api-Key: …` entran en claro, y con ellos un `.env` pegado entero. Faltan además formatos con prefijo fijo: Hugging Face (`hf_`), Slack `xapp-` y sus webhooks, el secreto de webhook de Stripe (`whsec_`) y la clave privada PGP (el patrón exige `PRIVATE KEY-----` pegado, `redact.rs:95`).
2. **`purge` saca de menos y de más.** Borra del outbox por subcadena y sin mirar el agente (`DELETE FROM ingest_outbox WHERE instr(content, ?1) > 0`, `crates/clawg-core/src/memory/secrets.rs:114`): un texto corto se lleva saves aceptados de otros agentes. Deja la cabecera de un save largo en `memory_piece.context` (`secrets.rs:103-111` no toca esa tabla) y no dice nada del `workspace.md` que copia el texto en su sección AUTO (`crates/clawg-core/src/memory/workspace.rs:224-235`).
3. **`purge` de la versión vigente de un tema esconde su historia para siempre.** `purge` no llama a la revival que sí corre un `delete` (`revive_after_delete_of_piece`, `crates/clawg-core/src/memory/admin.rs:355`), y la pasada de mantenimiento no la recupera porque sólo mira filas borradas que existen (`crates/clawg-core/src/memory/revival.rs:86-93`, `:156-159`).
4. **El `reason` de un veredicto se guarda sin enmascarar** (`judge_pair`, `crates/clawg-core/src/memory/dispute.rs:327`, `:383`) y `clawg secrets` sólo lee `memory.content` (`secrets.rs:41`).
5. **`last_error` del outbox no pasa por `redact`.** El texto viene de un error del embedder, del store o de serde (`crates/clawg-core/src/memory/worker.rs:766`, `:830`; `crates/clawg-core/src/memory/outbox.rs:378`). Leído el código, ninguno cita el contenido del save hoy: el evento del outbox lleva el contenido vacío (`V14__ingest_outbox.sql:6-7`) y los errores de ONNX Runtime, del tokenizer y de SQLite no repiten la entrada. Pero un error de serde sí repite valores del evento, y nada impide que una librería cite su entrada mañana; la columna sale en `/health`.

## Decisiones

Las abiertas de la exploración se respondieron en el hilo, con estas opciones:

- **Tipo `credential` aparte** (enfoque A). Un patrón por clave, anclado en nombres que terminan en `secret`, `token`, `api_key` (`apikey`, `api-key`), `access_key`, `secret_key`, `account_key` (`AccountKey`) o `auth_key`, con cualquier prefijo de palabra o con guiones (`X-Api-Key`, `OPENROUTER_API_KEY`, `client_secret`). Sólo el valor se enmascara, con la misma regla del valor literal de `password` (`is_literal_secret`, `redact.rs:162`: 6 caracteres, un dígito o un símbolo, ni variable ni llamada ni plantilla) y una más: un valor de sólo dígitos no es una credencial (`max_token=100000`). `password` no cambia de significado en `redacted` ni en `clawg secrets`.
- **Prefijos nuevos:** `huggingface_token` (`hf_` y 34 de `[A-Za-z0-9]` que mezclan mayúscula, minúscula y dígito, como `sk-`, para no tomar un nombre de clase), `xapp-` dentro de `slack_token`, `slack_webhook`, `whsec_` dentro de `stripe_key` y la cabecera `PRIVATE KEY BLOCK` de PGP dentro de `private_key`.
- **Workspace: nombrar, no reescribir** (D). Después de purgar, `purge` lee cada `workspaces/<agente>/<proyecto>/workspace.md` y nombra los que tienen el texto, diciendo si está en la sección AUTO (que se regenera en el próximo `recall_context` del agente en ese proyecto) o en la USER (que es del usuario). No escribe ningún archivo.
- **Outbox: filtrar y listar** (G + H), sin esquema nuevo. Una fila de `memory` sin vínculo en `memory_piece` es un save de un solo pedazo, así que su save tiene el mismo texto entero: se borra la fila del outbox de su agente con `content` igual. Un pedazo de un save largo sigue buscándose por subcadena, pero sólo en las filas de su agente. La salida nombra cada fila borrada por `operation_id`. Una fila de `memory` sin agente (`agent_id` NULL) no se filtra por agente.
- **Cabecera de un save largo** (J). Si la fila purgada es la posición 1 de un save, el `context` de todos los vínculos de ese save queda vacío. El vínculo de la fila queda: guarda un id, que `AUTOINCREMENT` no reusa, y marca el pedazo como faltante (`read_save_core` ya lo muestra así, `crates/clawg-core/src/memory/pieces.rs:133-160`). La salida nombra los otros pedazos del save: comparten con el purgado los tokens del solape.
- **Revivir en la transacción de `purge`** (O). La fila viva se marca borrada, corre la misma revival que un `delete` (sólo `revision`; lo que reemplazó un veredicto queda escondido) y recién después se borra. La salida lista lo que volvió. Lo ya purgado antes de este cambio queda como está (P no se eligió).
- **`reason`:** se enmascara al escribirlo (L), `clawg secrets` lo mira (M) y hay una forma de limpiar los viejos (N): `clawg secrets --mask-reasons --yes`, con el daemon parado, reescribe cada `reason` con secreto por su versión enmascarada, compacta la base y vacía el WAL como `purge`, sin snapshot previo.
- **`last_error` se enmascara siempre** al escribirse en el outbox, en los tres caminos. Es barato e idempotente, y no depende de que ninguna librería cite su entrada.
- **Restaurar filas viejas:** se acepta que una regla nueva bloquee `clawg undelete` de una fila borrada que ahora tiene un secreto (`RestoreError::HoldsSecret`, `admin.rs:397`). No se pierde nada: la fila sigue borrada, `clawg secrets` la lista y el mensaje dice cómo purgarla.

## Alcance

1. `credential`, `huggingface_token`, `slack_webhook`, y `xapp-`, `whsec_` y PGP en los tipos que ya existían, con tablas de casos en los dos sentidos.
2. `judge_pair` enmascara el `reason`.
3. `outbox_fail` y los otros dos escritores de `last_error` lo enmascaran.
4. `clawg secrets` lista los `reason` con secreto (`pair <new> <old> <tipos>`), y `clawg secrets --mask-reasons --yes` los limpia.
5. `purge`: outbox por agente e igualdad, con las filas borradas listadas; la cabecera de un save largo; los otros pedazos nombrados; la revival en la transacción; los `workspace.md` nombrados.
6. Docs: [`specs/secrets`](../../specs/secrets/spec.md) (en el archive), `.claude/CLAUDE.md` y el backlog de `openspec/README.md`.

## Fuera de alcance

- Entropía sin contexto, reescribir filas de `memory` al subir una regla, reescribir `workspace.md`.
- Una pasada de mantenimiento para los `superseded_by` que ya cuelgan hacia un id purgado (P): `doctor` (31) puede detectarlos.
- La fila `failed` del outbox sin salida (31, 53).
- Los backups: `purge` los nombra y no los toca.
- El índice de docs: se re-lee por sha256 o por modelo, no por regla; un pasaje viejo queda con la regla vieja hasta que su archivo cambie.

## Datos guardados

No cambia el esquema ni un identificador guardado. Cambia lo que entra desde ahora (más texto enmascarado) y lo que `purge` borra (menos filas del outbox, el `context` de un save cuyo primer pedazo se purga) o devuelve a la vista (las revisiones de un tema). `--mask-reasons` reescribe una columna guardada, sólo cuando el usuario lo pide con `--yes`; lo que reemplaza es un secreto por su placeholder.

Vuelta atrás: un binario anterior abre la base igual, sin migración que deshacer; lo enmascarado al entrar no vuelve, como con cada regla del detector.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- cada formato nuevo, sintético, queda enmascarado; prosa y código con las mismas palabras (`token = next_token()`, `secret: string`, `api_key: Option<String>`, `max_token=100000`) quedan iguales; `masking_twice_finds_nothing_new` sigue valiendo;
- un `reason` con un secreto se guarda enmascarado, y `clawg secrets` lista uno viejo y `--mask-reasons` lo limpia sin dejar sus bytes en disco;
- un `last_error` con un secreto se guarda enmascarado;
- `purge` deja la fila del outbox de otro agente con el mismo texto y la de un texto más largo del mismo agente, y nombra lo que borró;
- `purge` del pedazo 1 deja vacío el `context` de su save y nombra los otros pedazos;
- `purge` de la versión vigente de un tema revive la anterior y la lista; una reemplazada por veredicto sigue escondida;
- `purge` nombra un `workspace.md` con el texto en AUTO y otro con el texto en USER, y no los cambia.

Además, el corpus de prosa técnica del repositorio (`docs/`, `openspec/`, los `.md` de `crates/`) pasado por `find_secrets` antes y después: cada match nuevo se revisa a mano en [`verify.md`](verify.md). `rag-eval` no aplica: no cambia el ranking.
