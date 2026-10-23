# language-preference — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4 | L, partido en tres cambios: A (M–L, esta carpeta), B (79, M–L) y C (80, L) | nada pendiente: agent-authorship (76) y project-reassign (77) se archivaron el 2026-10-22, así que la migración de A es V22 y C sólo espera a A. B y C usan lo que deja A (`Lang` y `preferences.json`). history-import (44), que importa memorias de otras herramientas, recibe dos pedidos de este cambio (ver «Relación con otros cambios») y espera a A | decisiones cerradas con la opción recomendada; ver la [propuesta](proposal.md) |

**Tesis:** el idioma de clawg está fijo en inglés en tres planos que hoy no se distinguen: lo que el agente guarda y busca (`crates/clawg-cli/src/mcp/protocol.md:16-18`, `:79-80`), lo que clawg le escribe al agente (instrucciones, 12 descripciones de herramientas, 41 de argumentos, notas del contexto, respuestas del puente, hooks) y lo que le muestra a la persona (`doctor`, `update`, `setup`, `export`, `import`…). Se fijó así porque un idioma fijo es lo único que el agente puede cumplir sin saber cómo llegará la próxima consulta, y las herramientas ya estaban en inglés ([memory-protocol](../archive/2026-10-17-memory-protocol/design.md), línea 59). La medición de english-recall (21) mostró que lo que importa no es el inglés sino que consulta y fila estén en el mismo idioma: `es>es` 18/20 y `es>en` 6/19 en R@5, y con cualquiera de los dos lados traducido 19/19 y 18/19. Este cambio agrega un sistema de preferencias de la persona, un archivo por perfil como `sharing.json`, cuya primera preferencia es el idioma (`es` o `en`), y hace que esa preferencia gobierne las cuatro cosas: en qué idioma se guarda y se busca, en qué idioma le habla el agente a la persona, qué idioma usan las salidas del CLI y qué idioma lee el agente en sus herramientas, instrucciones y contexto. Lo guardado no se toca. Lo no obvio es que, mientras convivan los dos idiomas, nada sabe en cuál está cada fila (no hay columna que lo diga) y sólo el agente puede traducir una consulta, así que repetir la búsqueda en el idioma viejo exige contar las filas vivas de ese idioma, decírselo al agente cuando la primera búsqueda trae poco y dejar de decírselo cuando ya no quedan.

## Premisas que se verificaron

Cada cita se leyó en el código el 2026-10-22. No se compiló ni se corrió ningún test, y no se abrió ningún perfil de datos. Los conteos de caracteres y de literales salen de recorrer los archivos con un script fuera del repositorio y son un orden de magnitud: la lista exacta de textos la fija el diseño. Las cifras de un store con unas 880 memorias en inglés vienen de un recuento anterior y no se verificaron acá.

| Premisa | Resultado |
|---|---|
| El inglés se pide en dos lugares del protocolo | **Cierta.** `protocol.md:16-18` (la apertura, que es lo que llega como instrucciones: «Write memories and search queries in English, whatever language the conversation is in») y `:79-80` (la sección «How to write a save»: «Write it in English, and search in English too»). Las dos dicen «search» sin distinguir `search_memory` de `search_project`, cuyos pasajes están en el idioma de los docs del proyecto |
| La razón de esa regla | **Cierta.** [`design.md:59`](../archive/2026-10-17-memory-protocol/design.md): inglés «para el protocolo, para los saves y para las búsquedas»; un idioma fijo es lo único que el agente puede cumplir sin saber cómo va a llegar la próxima consulta. [`exploration.md:112`](../archive/2026-10-17-memory-protocol/exploration.md): «un agente que lee tres idiomas en una misma salida no gana nada»; `:127`: el idioma ideal es «el mismo en que se va a buscar»; `:163` y `:167`: «inglés, sin versión en español», «inglés fijo». Este cambio revierte esa decisión y conserva su argumento de los tres idiomas como razón para localizar también lo que lee el agente |
| Lo que rinde es el mismo idioma, no el inglés | **Cierta.** [english-recall, `verify.md:100-102`](../archive/2026-10-17-english-recall/verify.md): en `similarity`, R@5 `es>es` 18/20, `en>en` 6/7 y `es>en` 6/19; en agent-view, `search.found` 19/20, 6/7 y 11/19 (`:111-113`). Con la consulta traducida `es>en` pasa de 6/19 a 19/19, y con la fila traducida a 18/19 (`:188-189`). [`config.yaml`](../../config.yaml), líneas 21-24, lo resume |
| El cruce es asimétrico | **Parcial.** En `golden-cross.json` (48 casos) `en>es` da 21/22 y `es>en` 19/26 en R@5 de `similarity` (`verify.md:119-120`); los intervalos [0,782, 0,992] y [0,539, 0,863] se tocan. Sugiere que una consulta en inglés alcanza mejor filas de los dos idiomas que una en español. El n es chico y no se midió sobre un store mezclado, así que no decide en qué idioma conviene la segunda búsqueda (ver «Cómo se mide») |
| La descripción de `search_memory` promete lo contrario de lo medido | **Cierta.** `server.rs:1101-1102` y `agent_contract.json:303`: «it finds memories written in other words or in another language». Con `es>en` en 6/19 (coseno) y 11/19 (`search_memory`), la frase es optimista; la preferencia tiene que cambiarla |
| Nada le dice al agente en qué idioma hablarle a la persona | **Cierta.** Una búsqueda de `reply`, `answer in`, `respond in` y `language` en `protocol.md`, `hook.rs`, `server.rs` y `tools.rs` sólo encuentra las tres frases de arriba y comentarios de tests |
| La apertura casi no tiene margen | **Cierta.** `server.rs:2002-2008` fija 2.048 caracteres (Claude Code anterior a la v2.1.296 corta ahí las instrucciones); la apertura mide 2.028 y tiene 337 palabras, así que quedan 20 de margen ([`specs/mcp`](../../specs/mcp/spec.md), «Las instrucciones caben en 2.048 caracteres»). `agent_contract.json:2` guarda su copia. Una apertura en español que diga lo mismo más la frase de cómo hablarle a la persona no entra sin reescribirla; el protocolo completo mide 8.211 caracteres y 1.415 palabras en 161 líneas |
| Las descripciones son texto fijo de compilación | **Cierta.** 12 `#[tool(description = "…")]` (`server.rs:1077-1283`) con 5.359 caracteres, y 41 descripciones de argumentos que salen de comentarios `///` de 11 structs (`server.rs:156-376`) a través de schemars, con 4.676. `agent_contract.json` fija las 53 en 23.013 caracteres. `#[tool_handler(router = self.tool_router)]` (`server.rs:1288`) genera `list_tools` sobre un router que se arma una sola vez (`server.rs:452`, comentario en `:1286-1287`), así que cambiar de idioma exige reemplazar `description` y la de cada propiedad después de `list_all()`, o armar el router por idioma |
| Las instrucciones se entregan una vez por sesión | **Cierta.** `get_info` las arma de la apertura (`server.rs:1290-1302`) y `discover` usa `get_info` (`:1306-1312`). El puente es un proceso stdio que lanza el cliente (`server.rs:3-6`): una preferencia nueva llega a las instrucciones y a las herramientas en el siguiente `initialize`, no a una sesión abierta. `setup` ya dice lo mismo de sus cambios: «Open a new Claude Code session for the change to take effect» (`setup.rs:952`) |
| Las notas y los encabezados del contexto son constantes | **Cierta.** `prompt_builder.rs:24-30` (siete encabezados), `:35-36` (`SHARED_NOTE`), `:62` (`TRUNCATED`), `:594-615` (`format_cut`), `:741-744` (`CONFLICT_NOTE`), `:799` (conflictos omitidos); `workspace.rs:460-468` (tres encabezados del `workspace.md`). Tests los fijan: `prompt_builder.rs:2194-2197` y `workspace.rs:1414-1422` |
| El medidor de agent-view lee esos textos | **Cierta.** `rag-eval` parte el contexto en sus líneas `=== TÍTULO ===` y reconoce `WORKSPACE`, `LAST SESSION`, `RECENT CONTEXT` y `LONG-TERM KNOWLEDGE` por su nombre (`agent_view/parse.rs:70`, `:87-89`), compara con `no memories found` y con `NO_RELEVANT_MEMORIES` (`parse.rs:30`, `:34`) y llama a `format_search` y `format_context` (`agent_view/runner.rs:12`, `:143`, `:152`). Su documentación lo dice: si el formato cambia, el parser falla en voz alta (`parse.rs:3-7`) |
| Ninguna fila dice su idioma | **Cierta.** Las migraciones V1 a V19 (`crates/clawg-core/migrations/`) no tienen una columna de idioma. Dos caminos insertan en `memory`: el save (`store_insert.rs:553`) y la importación (`portability/import.rs:254`). El índice de texto sirve a los dos idiomas con `unicode61 remove_diacritics 2` (`V8__memory_fts.sql:31-35`, `:43`) |
| El golden ya etiqueta el idioma | **Cierta.** `CorpusRow.lang` (opcional, `rag-eval/golden.rs:50`) y `RetrievalCase.query_lang` (`:93`); `specs/eval`, «Métricas por pareja» ([`spec.md:258-266`](../../specs/eval/spec.md)), reparte `retrieval`, `similarity` y agent-view por pareja consulta>fila. Da con qué validar un detector y con qué medir |
| No hay dependencia para detectar idioma ni para catálogos de mensajes | **Cierta.** Ninguna de `whatlang`, `lingua`, `fluent`, `gettext` ni `rust-i18n` figura en los manifiestos. La regla del proyecto es no sumar una sin un uso real ([`.claude/CLAUDE.md`](../../../.claude/CLAUDE.md), «Code conventions») |
| La preferencia puede seguir el molde de `sharing.json` | **Cierta.** `sharing.rs:17-18` (versión), `:65-74` (`deny_unknown_fields`), `:171-190` (`load` y `save` por archivo temporal y `rename`), `paths.rs:267` (`sharing_file`); el daemon lo lee en cada pedido (`api.rs:1305-1318`) y el puente en `memory_status` (`server.rs:1053`). Los campos nuevos de una respuesta son seguros en las dos direcciones: el cliente los lee con `#[serde(default)]` (`client.rs:156-162`, el precedente es `withheld`) |
| El hook no puede hablar sin daemon | **Cierta.** `hook.rs:154` (`DaemonClient::from_data_dir(base_dir)?`) devuelve `None` sin daemon y [`specs/hooks`](../../specs/hooks/spec.md) R3 exige que no escriba nada. `session_start_text` devuelve `None` si no hay nada que decir (`hook.rs:60-88`; test `an_empty_context_prints_nothing`, `:268-270`), y lo que suma el hook tiene que ocupar menos de 1.000 bytes (`:273-278`) |
| `doctor` tiene ids estables y una frase por hallazgo | **Cierta.** `check_id` y `reason_code` son `&'static str` (`doctor/finding.rs:20-29`) y la spec dice «`message`: una frase en inglés para una persona» ([`specs/doctor`](../../specs/doctor/spec.md), R1, línea 21). Son 52 hallazgos con `message` (46 en `checks.rs`, 6 en `mcp.rs`) y 10 `safe_next_step`, todos comandos (`checks.rs:95`, `:160`, `:240`, `:252`, `:307`, `:325`, `:340`; `mcp.rs:42`, `:75`, `:98`) |
| Los errores tipados reenvían su texto | **Cierta.** Hay 125 `#[error(` en `clawg-core` y 82 en `clawg-cli`. `ApiError::internal` los reenvía tal cual (`api.rs:157-163`), `ApiError.message` mezcla códigos que el cliente ramifica (`project_root_collision`, `embedder_not_ready`, `embedder_failed`, `ingest_queue_full`, `ingest_failed`: `server.rs:58`, `:565`, `:659`, `:672`, `:676`) con frases (`api.rs:599`, `:1864`), y el puente reenvía esas frases al agente (`server.rs:671`) |
| Contar las filas de un idioma sin una columna nueva | **No se puede barato.** `memory_fts` es de contenido externo y sólo indexa `content`, `agent_id`, `project_id` y `memory_type` (`V8__memory_fts.sql:36-44`); `content` es de sólo escritura (`:90-93`). El conteo de filas vivas de un idioma pide una columna propia o recorrer el texto de cada fila en cada búsqueda |
| El `CLAUDE.md` del usuario lleva el protocolo entero | **Cierta.** `setup.rs:36-38` incluye `protocol.md`, `block` lo envuelve entre marcadores (`:557-562`) e `install_block` lo reescribe si difiere (`:583-606`; [`specs/setup`](../../specs/setup/spec.md) R4). Cambiar el idioma cambia ese bloque, pero sólo cuando `setup` vuelve a correr |

## Problema (verificado contra el código)

### Cómo se llegó a la regla de hoy

- **memory-protocol (22)** fijó el inglés para el protocolo, los saves y las búsquedas ([`design.md:57-61`](../archive/2026-10-17-memory-protocol/design.md)). Lo hizo como contrato y no como medida de calidad: el agente no puede saber en qué idioma llegará la próxima consulta, y las herramientas, las descripciones y las notas del servidor ya estaban en inglés. Lo ya guardado en español «se queda así» (`design.md:61`).
- **english-recall (21)** midió que lo que pierde las filas es el cruce de idioma y no el modelo ni el piso de coseno, y que con los dos lados en el mismo idioma la recuperación vuelve (`verify.md:188-189`, `:208`). Dejó la decisión de «en qué idioma guardar» a 22.
- **Lo que queda abierto.** La regla es de la instalación y no de la persona. Una persona que trabaja en español obliga a su agente a traducir cada consulta al inglés; cuando lo olvida el cruce es `es>en`. Una preferencia lo resuelve: el idioma lo elige la persona y el protocolo lo dice con todas las letras, que es lo que 22 pedía para que el agente pudiera cumplirlo.

### El inventario

Cada lugar donde un texto depende del idioma, agrupado en las cuatro cosas que cambia la preferencia. Los conteos son literales de prosa (tres palabras o más) fuera de tests, logs y SQL, fragmentos incluidos.

**Parte 1. Memorias y búsquedas: en qué idioma se guarda y se busca**

| # | Lugar | Dónde | Hoy | Con la preferencia |
|---|---|---|---|---|
| 1 | Apertura / instrucciones | `protocol.md:3-32` (la regla en `:16-18`); `server.rs:1300`; `agent_contract.json:2` | «Write memories and search queries in English…» | El idioma elegido. Sigue dentro de 2.048 caracteres en cada idioma |
| 2 | «How to write a save» | `protocol.md:79-80` | «Write it in English, and search in English too» | Lo mismo, y distingue las búsquedas de memorias de `search_project` |
| 3 | Descripción de `search_memory` | `server.rs:1101-1102`; `agent_contract.json:303` | Dice que encuentra memorias «in another language» | Dice que se busca en el idioma de lo guardado y qué hacer cuando hay filas en el otro |
| 4 | Línea del otro idioma | no existe; `SearchBody` (`api.rs:647-652`), `format_search` (`tools.rs:80`), `format_recall` (`tools.rs:522`) | — | Una línea en `search_memory` y `recall_context` con las memorias vivas del otro idioma, sólo cuando hay y la búsqueda trajo poco |

Son 3 textos que existen y 1 que se agrega.

**Parte 2. Cómo le habla el agente a la persona**

Hoy no hay ningún texto (premisa de arriba). Se agregan tres puntos de entrega: una frase en la apertura (`protocol.md:3-32`), un párrafo en el protocolo completo (que llega por el bloque de `CLAUDE.md`, `setup.rs:557-562`) y una línea en el `session-start` (`hook.rs:60-88`). Son 0 textos y 3 puntos.

**Parte 3. Lo que clawg le muestra a la persona**

| Grupo | Dónde | Textos | Qué no cambia |
|---|---|---|---|
| `doctor` | `doctor/checks.rs`, `doctor/mcp.rs`, `doctor/mod.rs` | 61 literales; 52 hallazgos con `message` | `check_id`, `severity`, `reason_code`, `evidence` y los 10 `safe_next_step` (comandos) |
| `update` | `update/report.rs` (44), `effects.rs` (20), `client.rs` (7), `release.rs` (6), `eligibility.rs` (5), `mod.rs` (5), `signature.rs` (4), `lock.rs` (2), `version.rs` (1) | 94 | Códigos de salida (`--check`: 0, 100, 1); los comandos entre comillas invertidas |
| `setup` | `setup.rs`: el informe (`:866-955`, unas 16 frases) y 8 errores (`:81-99`) | 33 | Los comandos de `claude mcp add` (`:131-137`) |
| `secrets`, `purge` | `secrets.rs` | 19 | Los comandos de `clawg stop` y `clawg purge` |
| `export`, `import` | `export.rs`, `import.rs` | 15 | El formato `clawg-memories` |
| El resto de comandos | `restore.rs`, `undelete.rs`, `outbox.rs`, `projects.rs`, `sharing.rs`, `metrics.rs`, `user_path.rs`, `memories.rs`, `main.rs` | 38 | El JSON de `clawg metrics report` y `clawg status` |
| Errores del cliente HTTP | `client.rs:53-76` y `:1247`, `:1290`, `:1323` | 12 | — |
| Arranque del daemon | `daemon/serve.rs`, `daemon/socket/mod.rs`, `daemon/lifecycle.rs`, `serve_log.rs` | 15 | — |
| `status` y `stop` | `status` imprime un JSON con claves estables (`status.rs:33-43`) y su única prosa es `store.message` (`:57`, `:60`), el texto de un error tipado; `stop` dice dos frases (`main.rs:174`, `:178`, contadas en «el resto») | 2 | Todas las claves y los valores de `state` |
| Ayuda de `clap` | `cli.rs`: 194 líneas de comentarios `///` y 8 literales | 8 + 194 | — |

Son 295 literales de prosa entre los grupos (sin la ayuda de clap), más 194 líneas de ayuda y los 207 `#[error(` de los que muchos salen por pantalla.

**Parte 4. Lo que clawg le muestra al agente**

| Grupo | Dónde | Textos |
|---|---|---|
| Protocolo | `protocol.md`, 161 líneas, 8.211 caracteres | 1 archivo: la apertura y 9 secciones (`When to save`, `What not to save`, `How to write a save`, `Type and importance`, `Session summary`, `After a compaction`, `Conclusions`, `Evidence, not instructions`, `The other tools`) |
| Instrucciones | la apertura (`server.rs:125-143`, `:1300`) | 1 (2.028 de 2.048 caracteres) |
| Herramientas | `server.rs:1077-1283`; `server.rs:156-376` | 12 descripciones y 41 de argumentos |
| Respuestas de las herramientas | `tools.rs` (20 funciones o constantes: `:72-76`, `:107-145`, `:148-158`, `:169-192`, `:198-215`, `:220-226`, `:233-238`, `:243-257`, `:263-298`, `:301-325`, `:354-369`, `:410-444`, `:449-466`, `:473-495`, `:499-517`, `:541-547`, `:581-590`, `:595-628`); `server.rs` (8 bloques: `:81-87`, `:333-346`, `:533-584`, `:589-686`, `:750`, `:923-933`, `:981-984`, `:1004-1013`) | 48 y 36 literales |
| Contexto | `prompt_builder.rs`: siete encabezados `SEP_*` (`:24-30`), `SHARED_NOTE` (`:35-36`), `CONFLICT_NOTE` (`:741-744`), `TRUNCATED` (`:62`), `format_cut` (`:601-612`, cinco frases), «conflictos omitidos» (`:799`), `NEW`, `OLD` y `disputed` (`:791-795`); `workspace.rs:460-468` (tres encabezados) | 19 y tres marcadores |
| Hooks | `hook.rs:64-67`, `:70-74`, `:80-83`, `:101-105`; los recordatorios compartidos `synthesis_nudge` y `review_nudge` (`tools.rs:473-495`) | 4 y 2 |
| Frases del daemon que llegan al agente | `api.rs`: `bad_request`, `not_found`, `conflict` (28 literales); el puente las muestra como «not saved: …» (`server.rs:671`) | 28 frases; los códigos como `embedder_not_ready` son API y no cuentan |

Total: el protocolo, las instrucciones, 53 descripciones, unos 130 literales de prosa fuera de las descripciones y 19 textos de contexto.

**Lo que no se traduce.** Los nombres de las 12 herramientas, los de sus argumentos (`query`, `project_path`, `topic_key`, `derived_from`, `user_answered`, `memory_type`…) y los siete valores de `memory_type` (`semantic_fact`, `user_preference`, `architectural_decision`, `session_summary`, `code_block`, `chat_message`, `tool_error`) son la API: el protocolo los nombra entre comillas invertidas y un test exige que existan (`the_protocol_names_only_what_exists`, `server.rs:2054-2076`), y el daemon rechaza un tipo distinto con un 400 que lista los siete (`api.rs:1124`). Tampoco se traducen `check_id`, `reason_code`, las claves de los JSON, los comandos, los códigos de salida ni los valores de `status`. Los textos de abajo son prosa; los de arriba son identificadores.

### Cuándo llega un cambio de preferencia

| Texto | Cuándo se lee | Llega |
|---|---|---|
| Instrucciones y herramientas | `initialize` y `tools/list` | En la sesión siguiente: el puente nuevo lee el archivo al arrancar. Una sesión abierta conserva las que recibió |
| Contexto y avisos de `recall_context` | cada pedido al daemon | Al pedido siguiente, si el daemon lee el archivo en cada uno como hoy lee `sharing.json` (`reach`, `api.rs:1305-1318`) |
| Hooks | cada evento (`startup`, `resume`, `clear`, `compact`) | Al evento siguiente |
| Respuestas del puente | cada llamada | Al arrancar el puente, si lo fija por sesión; en cada llamada, si relee (decisión abierta 8) |
| Bloque de `CLAUDE.md` | cuando corre `clawg setup claude-code` | Al volver a correr `setup` |
| Salida del CLI | cada comando | De inmediato |

No hace falta reiniciar el daemon. Un cambio sí pide una sesión nueva del cliente para las instrucciones y las herramientas, porque `instructions` no se reenvía después de `initialize` (`server.rs:1290-1312`).

### Qué pasa con un store lleno

- **Al cambiar de idioma no se escribe nada en `memory.db`.** La migración que suma la columna de idioma deja las filas como están, con el idioma sin clasificar hasta que un relleno lo calcule. Las 880 memorias en inglés de un store como el de partida siguen vivas y alcanzables.
- **Las filas nuevas nacen en español** si el agente obedece. Una consulta en español alcanza mal las viejas (`search.found` `es>en` 11/19, `verify.md:113`), y ahí entra la segunda búsqueda.
- **Qué retira filas viejas:** borrarlas, un veredicto `supersedes`, o guardar una versión nueva bajo su `topic_key`. La clave es ASCII y no depende del idioma (`topic.rs:19`, `:33`), así que una versión en español reemplaza a la inglesa del mismo tema y la deja `superseded`, fuera de la búsqueda. **Qué no las retira es el tiempo:** el mantenimiento sólo baja de tier por inactividad (`maintenance.rs:1-5`, `:53-69`), no borra.
- **Consecuencia.** Las filas sin clave no desaparecen solas, así que la segunda búsqueda puede estar encendida durante meses. «Se apaga cuando ya no quedan memorias vivas en el idioma viejo» es cierto y rara vez se cumplirá; el costo se controla con cuándo dispara (sólo si la primera trajo poco) y no con cuándo se apaga.
- **Dos idiomas, una misma cosa.** El dedup compara `content_hash` (`store_insert.rs:300-307`), así que un hecho guardado en inglés y vuelto a guardar en español queda dos veces. El detector de contradicciones mide cercanía dentro de la banda y se calibró con pares de un idioma (`conflict-eval` etiqueta cada par con su `lang`, `measure.rs:53`); no se sabe si empareja un hecho con su traducción (ver «Cómo se mide»).

## Lo que ya existe y se reutiliza

- **`sharing.rs` como molde** (`:17-190`): versión, `deny_unknown_fields`, `load` que tolera el archivo ausente, `save` por archivo temporal y `rename` en un directorio privado, y el patrón «el daemon lo lee en cada pedido» (`api.rs:1305-1318`). `clawg sharing` (`crates/clawg-cli/src/sharing.rs`) es el molde del comando.
- **`Finding`** (`doctor/finding.rs:20-29`): el sobre separa los identificadores estables (`check_id`, `reason_code`, `evidence`) del único campo de prosa (`message`), así que localizar `doctor` es cambiar el texto de cada hallazgo y no su forma.
- **Los guardas del texto del agente:** `opening` y `code_spans` (`server.rs:125-143`, `:1969-1978`), los tests de 2.048 caracteres y de nombres existentes, y el contrato con `CLAWG_BLESS=1` (`the_agent_contract_matches_its_file`, `server.rs:2420`; [`specs/quality`](../../specs/quality/spec.md)). Se repiten por idioma.
- **El golden con idioma y los cortes por pareja** (`golden.rs:50`, `:93`; `specs/eval`), con agent-view, que ya mide `search.found`, `context.found` y tokens por pareja (`verify.md:109-113`).
- **El mantenimiento del daemon** (`maintenance.rs:20-28`: una pasada 5 minutos después de arrancar y cada 24 horas), que ya actualiza columnas derivadas de `memory` (`tier`, `last_modified_at`) sin tocar el contenido (`maintenance.rs:43-51`): el precedente para completar `lang` en las filas anteriores.
- **El molde de una migración con datos previos:** `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` (`db.rs:1320`) y los checksums fijados (`published_migration_checksums_are_pinned`, `migrations.rs:43`).
- **El bloque de `CLAUDE.md`:** `install_block` ya distingue «igual» de «distinto» y reescribe (`setup.rs:583-606`), así que un protocolo en otro idioma lo reemplaza sin código nuevo.
- **`topic_key`** (`topic.rs`): encadena versiones sin importar el idioma.
- **Campos aditivos en las respuestas del daemon** (`client.rs:156-162`).

## Enfoques posibles

### I. Una preferencia por perfil que elige entre textos fijos por idioma (elegido)

Un tipo `Lang` en `clawg-core`, un archivo `<data>/preferences.json`, y cada texto como una función pura de `Lang`. Los textos con datos son funciones con un `match` exhaustivo sobre `Lang` (el compilador exige las dos versiones); los largos y estáticos (el protocolo, las descripciones) son archivos por idioma con tests de paridad. Parsear y decidir queda separado de leer el archivo, como pide el proyecto.

### II. Dejar los textos en inglés y decirle al agente que hable y guarde en español

Es el tramo A solo. Cubre las partes 1 y 2 y no las 3 y 4, que se eligieron. Se conserva como primer paso: A deja las frases nuevas en inglés, y C las traduce con el resto.

### III. Un binario por idioma, con una feature de compilación

El idioma pasaría de ser de la persona a ser de la instalación; duplicaría los binarios del release, su firma y `clawg update`. Se descarta.

### IV. Traducir lo guardado al cambiar de idioma

No hay modelo de traducción local (el único modelo es el de embeddings, `multilingual-e5-small`, [`config.yaml`](../../config.yaml)), de modo que lo haría el agente a un costo de tokens por fila, y `content` es de sólo escritura: nada lo actualiza y un trigger de texto sería deuda (`V8__memory_fts.sql:90-93`). Se descarta, de acuerdo con lo decidido.

### V. Resolver la búsqueda del otro idioma dentro del daemon

Imposible sin traducir la consulta, y sólo el agente traduce. La variante «el agente manda las dos consultas y el daemon las funde» queda como opción de la decisión abierta 4.

## Decisiones tomadas

Cerradas el 2026-10-22.

- **Hay un sistema de preferencias de la persona, y la primera es el idioma, `es` o `en`.** Una preferencia gobierna cuatro cosas: en qué idioma se guardan y se buscan las memorias; en qué idioma le habla el agente a la persona; qué idioma usan las salidas que clawg muestra a la persona (`doctor`, `status`, los reportes de `export` e `import`, los avisos del CLI); y qué idioma usan los textos que clawg le muestra al agente (descripciones de las herramientas, instrucciones del servidor, notas del contexto).
- **Guardar y buscar van siempre en el mismo idioma.** Es lo que midió english-recall: el cruce pierde las filas, el idioma elegido no.
- **También se localiza lo que lee el agente, aunque cueste.** Los costos conocidos son tokens por sesión, un segundo texto que mantener para cada frase y el tope de 2.048 caracteres de la apertura. Se acepta porque un agente que lee dos idiomas en una misma salida no gana nada ([`exploration.md:112` de 22](../archive/2026-10-17-memory-protocol/exploration.md)).
- **Los identificadores no se traducen:** nombres de herramientas y de argumentos, los siete valores de `memory_type`, `check_id`, `reason_code`, las claves de los JSON, los comandos.
- **Lo guardado no se traduce ni se toca.** Mientras convivan los dos idiomas el agente busca en el preferido y, si encuentra poco, repite en el idioma viejo. La segunda búsqueda se apaga cuando no quedan memorias vivas en el idioma viejo.
- **El destino inmediato es el español**, sobre un store de unas 880 memorias en inglés.

Las resolvió la evidencia:

- **El inglés fijo ya no es la regla razonable:** lo que rinde es el mismo idioma (`verify.md:100-102`, `:188-189`).
- **La traducción de lo guardado no es una opción:** no hay modelo local y `content` es de sólo escritura.
- **Ninguna fila dice su idioma hoy** (migraciones V1 a V19), así que la búsqueda del idioma viejo necesita un dato nuevo.
- **Sólo el agente puede traducir una consulta**, de modo que el daemon puede informar la necesidad de repetir y ejecutarla, pero no producirla.

## Decisiones abiertas

- [ ] **1. ¿Dónde vive la preferencia?**
  - **a. `<data>/preferences.json`, uno por perfil, y `clawg preferences get|set|unset` (recomendado).** Como `sharing.json`: `{"version": 1, "language": "es"}`. El daemon, el puente, los hooks y el CLI lo leen del directorio de datos que ya conocen; `dev` y `prod` quedan aislados. Cada preferencia futura es una clave más con su valor por defecto, y el valor se valida contra un conjunto cerrado: nunca es texto libre que termine en un prompt. Se escribe por archivo temporal y `rename` (`sharing.rs:183-190`). Con `deny_unknown_fields` y `version`, como `sharing.json`, una clave mal escrita se ve en `doctor` en lugar de dejar el inglés sin aviso, y una versión mayor la rechaza un binario anterior, que usa el valor por defecto.
  - **b. Un flag de `clawg mcp` que escribe `setup`.** Sólo llega al puente: el CLI, el daemon y los hooks no lo ven. Es un valor por registro de cliente y el idioma es de la persona. Cambiarlo exige reescribir la configuración del cliente, que `setup` evita para `.claude.json` (`setup.rs:7-9`).
  - **c. Una variable de entorno.** Es por proceso: el cliente lanza el puente y los hooks con su entorno, no con el del terminal donde se la definió, y en Windows una variable de usuario no llega a un cliente ya abierto. Sirve de anulación en pruebas, como `CLAWG_TEST_*`, no de fuente.
  - **d. Una memoria `user_preference`.** Entra en backups y export, pero sólo la leen quienes abren el store: `doctor` con el store ilegible, `update`, `setup` y el hook sin daemon no podrían. Una memoria además se reemplaza, decae, se oculta con `sharing` y se compara con otras, ciclo que una configuración no debe tener. Puede existir como espejo para que el agente la vea en el WORKSPACE, no como fuente.
- [ ] **2. ¿Qué hace clawg sin `preferences.json`, o con uno inválido?**
  - **a. Sin archivo, lo de hoy, byte a byte (recomendado).** Las instrucciones, la lista de herramientas, el contexto, los hooks y el CLI son idénticos a los de hoy, así que `agent_contract.json` y los tests actuales no se mueven, y un cliente que no usa la preferencia no paga nada. La frase de cómo hablar y la línea del otro idioma aparecen sólo con una preferencia escrita. Un archivo inválido usa `en` y lo dice `doctor`; nunca falla un comando por él. `doctor` suma el hallazgo `preferences` sólo si el archivo existe, y `memory_status` dice el idioma sólo con una preferencia puesta. La columna de idioma sí se completa siempre (es dato, no texto).
  - **b. Sin archivo equivale a `language: "en"` con todas las frases nuevas.** Más simple de explicar. Cambia la salida de todos los clientes en la actualización, mueve las bases de agent-view que `config.yaml` fija (761,4 tokens de `search_memory`, 1.037,3 de `recall_context`) y exige volver a bendecir el contrato.
- [ ] **3. ¿Cómo se sabe el idioma de una fila?**
  - **a. Una columna `memory.lang` con `en`, `es` o `und`, calculada por un detector puro sin dependencias y completada por el mantenimiento (recomendada).** El detector mira el texto ya enmascarado: quita bloques de código, identificadores y rutas, cuenta palabras funcionales de cada idioma y marcas (`ñ`, `¿`, `¡`) y devuelve `und` si hay poca evidencia (texto corto, código, nombres propios). Se escribe en los dos caminos que insertan (`store_insert.rs:553`, `import.rs:254`), así que no cambia el save ni el outbox. Las filas anteriores quedan con `lang` NULL («sin clasificar») y una tanda acotada del mantenimiento las completa sin tocar el contenido ni `last_modified_at`. `und` no cuenta como ningún idioma ni dispara el aviso (típicamente es código o identificadores, que el canal léxico encuentra sin importar el idioma). Mide, no declara: una sesión que sigue con las instrucciones viejas, o una cita textual de la persona en otro idioma, se cuenta por lo que es. Es un cambio de esquema con su prueba de datos previos, su plan de consulta y su vuelta atrás; su número de migración lo fija quien aterrice después de 76 (que usa V20) y 77.
  - **b. Una columna con el idioma declarado al guardar (el de la preferencia).** No necesita detector. Se equivoca cuando el agente guarda en otro idioma que el pedido, y las filas anteriores no tienen declaración. Con las 880 existentes en NULL, «las del idioma viejo» sólo se podría contar como «sin declarar», y la segunda búsqueda no se apagaría nunca.
  - **c. Sin columna, con una función escalar de SQLite que detecta en cada consulta.** Recorre todas las filas vivas del alcance en cada búsqueda: crece con el store (las 36.000 filas que estima 44 son el caso) y no se midió.
  - **d. Una dependencia de detección (trigramas u otra).** La regla es no sumar una sin uso real. Para dos idiomas una lista de palabras funcionales alcanza si el golden etiquetado lo confirma (M4 de «Cómo se mide»); si no llega a la barra, se reabre esta opción.
- [ ] **4. ¿Cómo se repite la búsqueda en el idioma viejo, y cuándo se apaga?**
  - **a. El daemon cuenta y avisa; el agente repite (recomendado).** `POST /v1/search` y `POST /v1/context` suman un campo opcional, `other_language` con el idioma y las memorias vivas alcanzables (propias y de los agentes que `sharing.json` deja leer, no las ocultas, no las borradas ni las reemplazadas). El puente agrega una línea a la respuesta sólo si hay preferencia, el conteo es mayor que cero y la búsqueda trajo poco; el agente traduce la consulta y vuelve a llamar. Con conteo cero la línea no sale, que es la forma de «apagarse». Se aplica a `search_memory` y `recall_context` con consulta; no a `search_project` (los docs tienen su propio idioma) ni a los hooks. Un daemon anterior no manda el campo y el puente no avisa (`client.rs:156-162`). «Poco» se calibra con M2: ninguna memoria relevante (`nothing_relevant`, `tools.rs:74-76`), o menos de tres hits mostrados. La línea se escribe en inglés en A y se traduce en C. Costo estimado, sin medir: la línea, de unos 35 tokens como la de `topic_key_warning` (`openspec/README.md`, fila 68), más la segunda llamada, cuyo resultado pesa entre 354 y 843 tokens en agent-view (`verify.md:111-113`).
  - **b. Sólo una instrucción en el protocolo («si encuentra poco, repite en inglés»).** No sabe cuándo parar: un store sin filas viejas, o que ya las perdió, paga una llamada de más en cada búsqueda pobre, y el protocolo, que no depende del store, no puede decirlo.
  - **c. Un argumento con la consulta ya traducida, que el daemon funde con la primera.** Ahorra una ida y vuelta, pero el agente escribe la traducción en cada llamada (tokens de salida) y la fusión es un cambio de ranking, que se valida con `rag-eval --diff` antes de ser el comportamiento por defecto ([`config.yaml`](../../config.yaml), «design»). Candidata para después, si M2 muestra que la ida y vuelta pesa.
  - **d. Buscar siempre en los dos idiomas.** Dobla el costo de cada búsqueda por una necesidad ocasional.
- [ ] **5. ¿Qué se localiza dentro del contexto y de las respuestas: los marcadores o sólo la prosa?**
  - **a. Los marcadores quedan estables y la prosa se traduce (recomendado).** Marcadores: los siete encabezados `=== … ===` (`prompt_builder.rs:24-30`), los tres de `workspace.md` (`workspace.rs:460-468`), `NEW`, `OLD`, `disputed`, `piece=K/N`, `by`, `topic=`, `score=`, `imp=`, `acc=`, `by=#N`, `from=`, `live=`, `lost=`, `origin=`. El protocolo nombra varios entre comillas invertidas, `rag-eval` los parsea (`parse.rs:43-63`, `:70`, `:87-89`) y son el equivalente de `check_id` para el texto del agente. Prosa: `SHARED_NOTE`, `CONFLICT_NOTE`, `format_cut`, `retrieval_notice`, los avisos, las guías de `synthesize`, los mensajes de «nada». Costo: el texto en español queda con encabezados en inglés, algo feo pero inocuo para un modelo.
  - **b. También se traducen los encabezados.** Consistente a la vista. El medidor de agent-view tiene que reconocer los nombres de los dos idiomas (o correr siempre con uno), y los tests que fijan los textos (`prompt_builder.rs:2194-2197`, `workspace.rs:1414-1422`) se duplican. Los textos de «vacío» (`no memories found`, `no relevant memories for this query`, `parse.rs:30`, `:34`) se aceptan en los dos idiomas en cualquier caso.
- [ ] **6. ¿Qué pasa con los errores tipados, las causas externas, la ayuda de clap, los logs y el instalador?**
  - **a. Se localiza lo que clawg compone y se deja en inglés lo que viene de afuera (recomendado).** Una frase propia enmarca la causa técnica: «no se guardó: {causa}», con la causa tal cual. Quedan en inglés el `Display` de los 207 errores tipados (la biblioteca no conoce el idioma), el `message` de `ApiError` (mezcla códigos con frases, `api.rs:126-164`), los errores de SQLite, del sistema y de HTTP, la ayuda y los errores de `clap` (`cli.rs`: 194 líneas de `///` en la API derive), unas 170 llamadas a `log::*` y `scripts/install.ps1`, que corre antes de que exista una preferencia. Los registros y la ayuda en inglés se pegan en una incidencia y se buscan.
  - **b. Se traducen también los errores tipados de `clawg-cli`** (82, por ejemplo `SetupError`) con un método `texto(lang)`. Más completo; duplica 82 mensajes y deja a los 125 de `clawg-core` en inglés a medias.
  - **c. Todo, incluida la ayuda de clap.** Pasaría de la API derive a la de construcción y duplicaría 194 líneas; la ayuda de un CLI suele leerse en el idioma de su documentación.
- [ ] **7. ¿Cómo se guardan los textos largos y qué pasa con el tope de 2.048 caracteres?**
  - **a. Un archivo por idioma para el protocolo y un complemento por idioma para las descripciones, con tests de paridad (recomendado).** `protocol.es.md` junto a `protocol.md`; las descripciones y los argumentos se reemplazan después de `list_all()` desde una tabla `(herramienta, propiedad) → texto`. La paridad exige las mismas secciones, los mismos textos entre comillas invertidas (`code_spans`), las mismas herramientas y propiedades, y una apertura de 2.048 caracteres o menos en cada idioma. Hay un segundo contrato, `agent_contract.es.json`, bendecido con el mismo `CLAWG_BLESS=1`. La apertura en español se escribe para entrar en el tope, pasando frases a las secciones, que llegan por el bloque de `CLAUDE.md`; el tope no se sube sin saber qué clientes anteriores a la v2.1.296 hay en uso.
  - **b. Una plantilla con marcadores (`{idioma}`) y una sola fuente.** Evita la divergencia, pero el español no es el inglés palabra por palabra (orden, concordancia, largo) y los marcadores quedarían en el `CLAUDE.md` del usuario.
  - **c. Traducir sólo las frases nuevas y dejar el protocolo en inglés.** Es el tramo A, no la parte 4.
- [ ] **8. ¿El puente fija el idioma al arrancar o relee en cada llamada?**
  - **a. Lo fija al arrancar y `memory_status` avisa si el archivo cambió (recomendado).** Un solo `Lang` en `ClawgMcp`, puro y fácil de probar, y una sesión coherente: instrucciones, herramientas y respuestas en el mismo idioma. `memory_status` dice «idioma de este puente: en; preferencia actual: es; se aplica en una sesión nueva». El daemon y los hooks, que no tienen sesión, siguen el archivo.
  - **b. Relee en cada llamada**, como `format_sharing` (`server.rs:1053`). Las respuestas siguen la preferencia nueva con las instrucciones viejas: una sesión en dos idiomas.
- [ ] **9. ¿Qué tan estricta es la frase sobre cómo habla el agente?**
  - **a. Un idioma por defecto con salvedad (recomendado):** «Responde a la persona en español; si escribe en otro idioma, en el suyo». El cliente ya puede tener su propia regla de idioma en el `CLAUDE.md` del usuario, y una frase absoluta de clawg pelea con ella. Se entrega por la apertura, el protocolo y el hook.
  - **b. Siempre en el idioma de la preferencia.** Más predecible; falla cuando la persona escribe en otro idioma por una vez.
- [ ] **10. ¿Se parte el cambio?**
  - **a. Tres cambios: A, B y C (recomendado, ver «Alcance candidato»).** Lo central no espera a lo caro, y B y C son mecánicos y se pueden partir más. Costo: A deja en inglés los textos que lee el agente (parte 4) hasta que llegue C: el agente lee en inglés una instrucción que dice «guarda en español», y funciona.
  - **b. Uno solo.** Una sola spec y un solo contrato, pero 300 mensajes, 130 literales, 53 descripciones, una migración y dos mediciones de eval en un cambio que no se puede archivar hasta que termina el último.

## Relación con otros cambios

| Cambio | Relación |
|---|---|
| **76 agent-authorship** | Reescribe las mismas cosas que C traduce: las cuatro descripciones con «Only your own memories» (`server.rs:1166-1228`), `agent_contract.json:5`, `:152`, `:200`, `:417`, `protocol.md:69-76`, `:146-148`, `:158-159`, `SHARED_NOTE` (`prompt_builder.rs:35-36`) y las descripciones y los argumentos de `list_memories`, y suma V20. **Orden: 76, luego C.** Si C fuera antes, 76 editaría dos copias de cada frase y bendeciría dos contratos. A toca otras líneas (`protocol.md:16-18`, `:79-80`, `server.rs:1101-1102`, `agent_contract.json:2`, `:303`) y puede ir antes o después: sólo coinciden en el archivo del contrato, que se vuelve a bendecir, y en el margen de 20 caracteres de la apertura, que se reparte entre los dos (quien aterriza segundo corrige su texto). El número de la migración de A es el siguiente al que use 76 |
| **44 history-import** | Importa memorias de otras herramientas. Depende de la parte A, que va antes en el orden de ejecución: el idioma del store se decide antes de importar. Recibe dos pedidos. (1) Medir el idioma de lo que trae cada fuente con un script que imprima sólo conteos (`es`, `en`, `und`, por tipo y con o sin proyecto), nunca un texto, un título ni una clave, usando la misma lista de palabras funcionales de A; el export no trae un campo de idioma (`exploration.md:19` de 44), y esos conteos dicen cuánto pesará el otro idioma: las 36.000 filas que 44 estima pueden dejar encendida la segunda búsqueda para siempre. (2) Definir si las filas importadas llevan su idioma: se recomienda que sí, calculado por el mismo detector puro en el camino de inserción (`import.rs:254`) y llevado por el formato `clawg-memories` como un campo más, que un lector anterior ignora (`CLAUDE.md`, «clawg export»); el reporte de la importación cuenta por idioma, sin texto |
| **77 project-reassign** | No se cruzan. Su exploración deja el número de migración a quien aterrice después (`exploration.md:272`) |
| **22 memory-protocol** y **21 english-recall** | Este cambio revierte el «inglés, sin versión en español» de 22 (`exploration.md:163`, `:167`) con la evidencia de 21. Las specs `mcp`, `retrieval`, `eval`, `doctor`, `setup` y `hooks` que dicen «en inglés» se reescriben |
| **68 topic-key-warning** | La línea `topic_key_warning` (`tools.rs:233-238`) está atada a la medida de 34 tokens que midió 48: un test de `revision-eval` la compara con `WARNING_LINE` (`cost.rs:338`). En `en` no cambia; su versión en español pierde esa medida y la repite C |

## Alcance candidato

Partido en tres cambios, de modo que lo central no espere a lo más caro.

**A. Preferencia, memorias, búsquedas y cómo habla el agente** (el cambio que nombra esta carpeta; M–L; objetivos 1 y 4)

1. `clawg_core::preferences`: `Lang`, la lectura y la validación puras, y `load`/`save` con `paths::preferences_file` (decisiones 1 y 2).
2. `clawg preferences get|set|unset [--profile P]`, con su propia salida ya en el idioma elegido. `memory_status` dice el idioma vigente cuando hay una preferencia. `clawg doctor` suma un hallazgo `preferences` (inválido, versión, idioma desconocido) y avisa si el bloque del protocolo del `CLAUDE.md` está en otro idioma que la preferencia (`safe_next_step`: `clawg setup claude-code`).
3. `memory.lang`: la columna, un índice parcial, el detector puro, la escritura en los dos caminos de inserción, el campo del formato `clawg-memories`, el relleno de las filas anteriores por el mantenimiento y la migración con su prueba de datos previos y su escenario de `clawg update` (decisión 3).
4. `other_language` en las respuestas de `/v1/search` y `/v1/context`, respetando `sharing.json`; la línea en `search_memory` y `recall_context`; el mensaje de la segunda búsqueda en inglés (decisión 4).
5. La regla de idioma en la apertura y en el protocolo, la frase de cómo hablar (apertura, protocolo, `session-start` también sin daemon), la descripción de `search_memory` corregida, y el contrato bendecido; la apertura dentro de 2.048 caracteres en los dos idiomas.
6. En `clawg-eval`: un golden pareado, el apareo por id en `--diff`, agent-view con el aviso, y pares entre idiomas en `conflict-eval`.
7. Las specs `mcp`, `retrieval`, `storage`, `hooks`, `doctor`, `setup`, `eval`, `portability` y `.claude/CLAUDE.md`.

**B. Textos del CLI** (M–L; objetivo 1; depende de A)

1. El mecanismo de mensajes del CLI (`match` exhaustivo sobre `Lang` por frase, sin dependencias) y la lectura del idioma una vez por comando.
2. Las frases de `doctor` (los 52 `message`), `update`, `setup`, `secrets`, `purge`, `export`, `import`, `restore`, `undelete`, `outbox`, `projects`, `sharing`, `metrics`, `path`, `memories`, `stop`, `status`, los errores del cliente HTTP y el arranque del daemon (decisión 6).
3. Un test por grupo que recorre los hallazgos de `doctor` y comprueba que los `check_id`, `reason_code`, `severity` y `safe_next_step` son los mismos en los dos idiomas. Tests de concordancia para las frases con plural (`secrets.rs:34-49`, `tools.rs:487-493`, `:617-626`).
4. Las specs `doctor` (R1: `message` deja de ser «en inglés»), `setup` (R7), `self-update`, `secrets`, `portability`.

**C. Textos que lee el agente** (L; objetivo 1; depende de A y va después de 76)

1. `protocol.es.md`, la tabla de descripciones y de argumentos en español, `agent_contract.es.json` y los tests de paridad (decisión 7).
2. Las respuestas del puente (`tools.rs`, `server.rs`), las notas del contexto, los hooks y los recordatorios, con los marcadores estables (decisión 5), y el medidor de agent-view que acepta los mensajes de vacío de los dos idiomas.
3. Las frases que A dejó en inglés: la línea del otro idioma y la de cómo hablar.
4. La medición de tokens de las instrucciones y de `tools/list` por idioma (M3).
5. Las specs `mcp`, `retrieval`, `hooks`, `quality`, `eval`. Si es demasiado grande, se parte en C1 (frases compuestas en Rust) y C2 (protocolo, descripciones y contrato).

### No-alcance

- **Los diálogos de permisos de Claude Code:** no son de clawg.
- **El código, los comentarios, los mensajes de commit y los nombres de los tests:** van en inglés por convención del repositorio; los artefactos de openspec se escriben como hasta ahora.
- **Traducir lo guardado:** filas, resúmenes de sesión, `topic_key`.
- **Más idiomas que `es` y `en`.** La estructura los admite (`Lang` es un `enum` con `match` exhaustivo), pero cada uno pide el texto completo y un detector propio.
- **Un modelo de traducción local, y fundir en el daemon dos consultas** (decisión 4.c), mientras M2 no lo pida.
- **Cambios de ranking o de embedder.**
- **Otras preferencias que el idioma.** La estructura queda lista; no se agrega ninguna.
- **El idioma de `search_project` y de los documentos del proyecto.**
- **Corregir a mano el idioma de una fila** con un comando.
- **La ayuda de `clap`, los logs, el `Display` de los errores tipados, las causas externas y el instalador** (decisión 6.a).

## Cómo se mide

Tests RED primero. Fallan hoy, y salvo aviso deben pasar en `en` y en `es`:

1. **Preferencias:** un archivo ausente da el valor por defecto; la versión, un campo desconocido y un idioma que no existe se rechazan; el ida y vuelta de `to_json` y `parse`; `clawg preferences set` escribe por archivo temporal y respeta `--profile`; un archivo inválido nunca hace fallar un comando y lo dice `doctor` (`preferences.rs`, `cli.rs`, `tests/binary.rs`).
2. **Sin archivo, nada cambia:** las instrucciones, la lista de herramientas, un contexto, un hook y las frases de la CLI son idénticos, byte a byte, a los de hoy (el contrato actual y los tests actuales pasan sin tocarlos); `doctor` y `memory_status` suman su línea sólo si hay una preferencia puesta.
3. **Instrucciones y paridad:** `the_instructions_fit_older_clients` (`server.rs:2005`) para cada idioma; `the_protocol_names_only_what_exists` (`:2055`) sobre `protocol.es.md`; mismas secciones y mismas comillas invertidas en los dos protocolos; la tabla de descripciones cubre exactamente las herramientas y propiedades de `list_all()`; `the_agent_contract_matches_its_file` (`:2420`) para el contrato en español.
4. **El detector:** casos de prosa en cada idioma, mezclados, de sólo código, cortos y con identificadores dan `en`, `es` o `und` como corresponde; sobre las filas etiquetadas del golden (`golden.rs:50`) acierta en al menos el 95 % de las de ocho palabras o más (a fijar en la propuesta) y el porcentaje de `und` se informa.
5. **La migración:** un store sembrado en la versión anterior con filas en los dos idiomas, sin autor y con claves se abre con `MemoryStore::open`, conserva cada fila alcanzable, trae `lang` en NULL, y su relleno clasifica sin tocar el contenido, el hash ni `last_modified_at`; el plan de consulta del conteo usa el índice sin `TEMP B-TREE`; un escenario de `clawg update` sobre una base de la versión anterior. Un save y una importación marcan `lang`; un export sin el campo se lee.
6. **El aviso:** sale cuando el conteo es mayor que cero y la búsqueda trajo poco; no sale con conteo cero, con una búsqueda que trajo lo suficiente ni sin preferencia; el conteo excluye filas borradas, reemplazadas y de autores ocultos con `sharing hide`; un daemon anterior no lo manda y el puente no avisa.
7. **Hooks:** `session-start` dice cómo hablar con la preferencia puesta, también sin daemon (cambia [`specs/hooks`](../../specs/hooks/spec.md) R3), y lo que suma sigue en menos de 1.000 bytes.
8. **`setup`:** con un idioma distinto, el bloque pasa a `Rewritten` y `--dry-run` lo dice.
9. **`doctor` y el CLI:** los `check_id`, `reason_code` y `severity` de un mismo hecho son iguales en los dos idiomas; cada frase existe en los dos (lo exige el compilador).

Medidas con el modelo real, sobre el golden y su versión pareada (los números de `verify.md` y `config.yaml` son los puntos de partida):

- **M1. Paridad.** El golden de 46 casos (y el expandido de 154) con todas sus filas y consultas en inglés y la misma versión en español, apareados por id de caso. Hoy `--diff` aparea por `query` (`verify.md:59` lo resolvió a mano), de modo que hace falta un apareo por id. Métricas primarias: R@5 y `search.found` (`rag-eval --diff`); `context.found` y tokens, informativas. Umbral a fijar en la propuesta: ninguna pérdida con p < 0,05 de español contra inglés (`--fail-on-regression` sale con 6), más un tope de casos peores. Hoy sólo hay `es>es` (n = 20, 18/20) contra `en>en` (n = 7, 6/7), que no alcanza.
- **M2. Store mezclado y costo de la segunda búsqueda.** Un corpus con filas de los dos idiomas (partiendo de las variantes `golden-e-*`), con tres políticas en agent-view: P1 sólo español, P2 español y, si la primera trajo poco, inglés, P3 sólo inglés. Se informa `search.found`, `context.found`, los tokens sumando la segunda llamada, la probabilidad de que el aviso salga dado un acierto y dado un fallo (calibra «poco», decisión 4) y el contraste `en>es` contra `es>en` que sugiere `verify.md:119-120`.
- **M3. Tokens por sesión.** Las instrucciones y `tools/list` en cada idioma (hoy 2.028 y 5.359 + 4.676 caracteres), y la línea del aviso, con el contador de tokens del modelo que las lee; el cambio medio de `search_memory` (761,4 tokens) y de `recall_context` (1.037,3) con el aviso encendido ([`config.yaml`](../../config.yaml)).
- **M4. El detector** contra las filas etiquetadas del golden expandido (1.280 filas): matriz de confusión y porcentaje de `und`. Sobre un perfil real, sólo conteos.
- **M5. Dos idiomas, un hecho.** `conflict-eval` con pares y cadenas de `revision-eval chains` traducidos: si el detector empareja un hecho con su traducción y si una versión en otro idioma bajo la misma clave retira la anterior (M1 y M2 de `revision-eval` siguen cumpliéndose).
- **M6. A mano y con autorización, sobre el perfil `dev`:** cambiar el idioma, abrir una sesión nueva de Claude Code, comprobar cuatro cosas (idioma de las memorias guardadas, de la respuesta del agente, de `clawg doctor` y de las descripciones), una búsqueda que dispara el aviso y una que no, `clawg setup claude-code` con el bloque en el otro idioma, y que el agente con el protocolo en español siga llamando a las herramientas con los mismos argumentos.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| La apertura en español no entra en 2.048 caracteres, o deja a los clientes anteriores a la v2.1.296 con las instrucciones cortadas | Alta | Test por idioma; frases que pasan a las secciones, que llegan por el bloque de `CLAUDE.md`; la apertura se escribe para el tope, no se traduce a ciegas |
| Costo en tokens por sesión: más texto en español, no medido, en cada sesión | Media | M3 antes de archivar C; C se puede partir y entregar por tramos |
| Mantener dos textos para cada frase para siempre | Media | `match` exhaustivo en las frases y tests de paridad en los archivos; un contrato por idioma hace visible cualquier cambio de texto en el diff |
| La segunda búsqueda dura meses porque las filas sin clave no mueren | Alta | Dispara sólo si la primera trajo poco; el umbral se calibra con M2; la revisión por `topic_key` retira filas a medida que se reescriben los temas |
| El detector se equivoca y el conteo del otro idioma miente | Media | M4 antes de fijar la barra; `und` para lo dudoso; el aviso dice «unas N»; un perfil real se mide sólo con conteos |
| El agente no obedece (guarda en inglés con la preferencia en español, o al revés) | Media | El idioma de la fila se detecta, no se declara, así que el conteo refleja la realidad; el protocolo y la línea del aviso lo repiten |
| Una sesión abierta antes del cambio sigue con las instrucciones viejas | Baja | Decisión 8; `clawg preferences set` y `memory_status` dicen que se aplica en una sesión nueva |
| Un hecho en dos idiomas queda dos veces porque el dedup compara hashes y el detector se calibró con pares de un idioma | Media | M5; el aviso de revisión y el veredicto `supersedes` quedan como el camino manual |
| El medidor de agent-view y los tests atados a texto en inglés fallan en español | Media | Marcadores estables (decisión 5.a); el parser acepta los mensajes de vacío de los dos idiomas; las pruebas existentes pasan sin cambios en `en` |
| Concordancia y plural en frases construidas con datos (`tools.rs:487-493`, `:617-626`, `secrets.rs:34-49`) | Baja | Formas neutras donde se pueda («Conclusiones a revisar: 2») y tests de las dos formas |
| Versiones distintas de puente y daemon | Baja | Los campos nuevos son opcionales y se toleran en las dos direcciones (`client.rs:156-162`); sin el campo, el puente no avisa |
| Una preferencia mal escrita deja el inglés sin que nadie lo note | Baja | `deny_unknown_fields` y `version`, y un hallazgo de `doctor` |
| La preferencia se usa para meter texto en un prompt | Baja | El valor es un elemento de un conjunto cerrado; ninguna preferencia es texto libre |
| Muchos tests fijan texto en inglés y se pueden dejar a medias | Media | Con `en` y sin archivo la salida es idéntica a la de hoy, así que los tests actuales no se tocan; cada texto nuevo trae su par en español |
| El relleno de `lang` compite con una importación larga de 44 en la única conexión de escritura | Baja | Tandas acotadas del mantenimiento; 44 marca `lang` al insertar y el relleno sólo toca las filas en NULL |
| Un protocolo en español cambia lo que el agente hace, no sólo cómo lo dice | Media | M6 en `dev`: mismos argumentos, mismos tipos de memoria, mismas llamadas a `resolve_conflict` y a `synthesize` |
