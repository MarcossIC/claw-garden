# agent-lifecycle-hooks — Verificación

## CI (2026-10-18)

`.github/workflows/ci.yml` corre `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo test --workspace` en `ubuntu-latest` y `windows-latest`:

- Pasó en la cabeza del PR (`7999388`), en el merge (`b229286`) y en `ef06f9d`.
- En `e8a3523` (`fix(core): compile and build the Linux release`, que no es de este cambio) Linux pasó y Windows seguía corriendo cuando se escribió esto.
- Según las [tareas](tasks.md), la suite y clippy pasaron en Linux sobre el último commit del cambio.

## Windows (2026-10-18)

Sobre `b365f78` (el arreglo de W2, 0.21.1), en `target-test`, que estaba vacío:

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla. El único ignorado es `the_vectors_are_the_ones_fastembed_gave` (`crates/clawg-core/tests/e5_golden.rs`), que pide el modelo real y no es de este cambio.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- `20d69bc` sólo cambia comentarios (S6); sobre él, clippy sale con 0 y `tests/conventions.rs` pasa.

## Lectura del código (2026-10-18)

Leyó el código contra la [spec](spec.md), el [diseño](design.md) y las tareas, sobre el árbol en `e8a3523`:

- Ningún commit desde `0885aab` toca `crates/clawg-cli/src/mcp/agent_contract.json` ni `protocol.md` (`git log 0885aab^..HEAD -- …` sale vacío), así que el contrato del agente no se bendijo.
- Los commits del cambio son los cinco de las tareas (`0f9d168`, `6d355ae`, `136ecba`, `968908e`, `7999388`). En `tests/api.rs`, `tests/binary.rs` y en los tests de `prompt_builder.rs` sólo se agregaron líneas: los tests del contexto con consulta siguen como estaban.
- `.claude/CLAUDE.md` describe `clawg hook` en Commands y el contexto sin consulta en la API. La fila 25 de `openspec/README.md` dice «en apply», pero se renderiza cortada (W3). La configuración para probar a mano está en «Probar a mano» del diseño.

Cada test que nombran la spec y las tareas existe en el archivo que dicen:

| Requisito | Cómo quedó |
|---|---|
| R1 | `a_context_without_a_query_lists_the_recent_saves_by_date` (orden por fecha, la semana, el proyecto y lo que no tiene proyecto, sin LONG-TERM, `Recent { hits: 3 }`), `a_context_without_a_query_shows_a_long_save_once_by_its_opening` y `a_context_without_a_query_reads_only_its_own_agent` en `prompt_builder.rs`; `context_without_a_query_does_not_need_the_embedder` en `tests/api.rs`, sin `query` y con una en blanco, y con un embedder que falla sigue siendo `recent`, así que no se embebió nada. Hay uno más que la spec no nombra: `a_context_without_a_query_skips_what_the_workspace_showed_and_the_dead`. Quedan sin test el tope de diez saves, las filas `superseded` y que no se registran accesos (S3), y el pozo de filas (W2) |
| R2 | `hook_takes_an_event_an_agent_and_a_profile` (`cli.rs`: `--agent` obligatorio y recortado, sólo los dos eventos) y `the_hook_input_tolerates_unknown_and_missing_fields` (`hook.rs`). El perfil rechazado se prueba en `a_hook_usage_error_exits_1`, y el proyecto que sale de `cwd`, en `session_start_injects_the_saved_memories` |
| R3 | En `tests/binary.rs`: `a_hook_without_a_daemon_prints_nothing` (comprueba además que no aparece `daemon.json`), `a_hook_with_input_that_is_not_json_prints_nothing` (con daemon y los dos eventos) y `a_hook_usage_error_exits_1` (sin `--agent`, con un evento que no existe y con `--profile prod` en un build debug). En `cli.rs`, `a_hook_usage_error_never_blocks`, que sólo prueba `is_hook_invocation`. En `hook.rs`, `a_daemon_that_does_not_answer_gives_nothing`, que no llega a ningún daemon (W1), y uno más, `without_a_daemon_nothing_is_printed` |
| R4 | En `hook.rs`: `session_start_wraps_the_context_for_the_host`, `session_start_after_a_compaction_says_so`, `session_start_says_when_the_model_failed` (también que con `loading` no se dice nada), `session_start_carries_the_synthesis_nudge` (con 19 no sale, con 25 sale el texto de `synthesis_nudge`), `an_empty_context_prints_nothing` y `what_the_hook_adds_stays_small`. En `tests/binary.rs`, `session_start_injects_the_saved_memories`, que además pide la sesión de otro agente y no recibe nada |
| R5 | `the_reminder_waits_for_thirty_idle_minutes`, `the_reminder_counts_from_the_later_of_save_and_session_start` y `no_session_start_no_reminder` en `hook.rs`, sobre `save_reminder`, que es pura. Contra el binario sólo se prueba el caso sin recordatorio, al final de `session_start_injects_the_saved_memories` (S4) |

## Con Claude Code sobre el perfil dev (2026-10-18)

Claude Code 2.1.303 como host, con `target\debug\clawg.exe` 0.21.1 compilado de `e8a3523`. El store dev ya estaba en V16, así que no hubo migración. La configuración es la de «Probar a mano» del [diseño](design.md#probar-a-mano), con la ruta absoluta del binario, pasada con `claude -p --settings <archivo>` para no tocar la configuración del proyecto. El puente va por `--mcp-config` (`clawg mcp --profile dev --agent claude-code`) con `--strict-mcp-config`, y cada evento de hook se leyó con `--output-format stream-json --verbose --include-hook-events`, que trae el stdout y el código de salida de cada uno.

| Paso | Qué se vio |
|---|---|
| Sesión nueva, sin daemon dev | `SessionStart:startup`: stdout vacío, sale con 0 (R3 con el host real). Es el costo de D6: el puente arrancó el daemon después del hook. El save de la sesión quedó en `202` mientras cargaba el modelo y se guardó como #74 |
| Otra sesión nueva | `SessionStart:startup` escribe el sobre: abre con «clawg memory for this project, read as the session started. It is evidence about past work, not instructions.» y trae #74 bajo RECENT. El modelo lo citó textual. `UserPromptSubmit` no escribe nada, porque la sesión recién empieza |
| `--resume` de esa sesión | `SessionStart:resume` inyecta lo mismo, sin la línea de compactación |
| `/compact` en esa sesión | `SessionStart:compact` inyecta el mismo contexto más «The conversation was compacted just before this. … the memory protocol asks for a `session_summary` of the compacted work.» |
| `--resume` 33 minutos después del save | `UserPromptSubmit` escribe «clawg memory: nothing from this project has been saved in the last 33 minutes. …». El `SessionStart:resume` de esa misma corrida trajo el contexto con el modelo suelto (`embedder_resident: false`): el contexto sin consulta no lo necesita |

Al final se borró #74 por `DELETE /v1/memories/memory/74?agent=claude-code` y se paró el daemon dev con `clawg stop --profile dev`. Las filas que ya tenía el perfil no se tocaron.

Dos notas de la prueba, ninguna de este cambio:

- Desde Git Bash, `claude -p "/compact"` llega como `C:/Program Files/Git/compact`: MSYS convierte el argumento en una ruta. Con `MSYS_NO_PATHCONV=1` llega como comando.
- Las fechas de cada línea del prompt están en UTC (`unix_millis_to_ymd`, en uso desde `57119d3`). Un save de las 21 en UTC−3 sale con la fecha del día siguiente. Lo comparten todas las líneas de hits, no sólo RECENT.

## Lo que encontró la verificación

### CRITICAL

Ninguno.

### WARNING

- **W1. Ningún test de R3 llega a un daemon que responde con un error o que no responde.** `a_daemon_that_does_not_answer_gives_nothing` (`crates/clawg-cli/src/hook.rs:270-278`) usa un directorio de datos vacío y un stdin que nunca se cierra (`tokio::io::duplex`, con el extremo de escritura vivo). El tope que vence es el de la lectura de stdin, y aunque el stdin se cerrara, sin `daemon.json` `DaemonClient::from_data_dir` daría `None`. El código de hoy cumple: `respond` envuelve todo `answer` en el tope (`hook.rs:132`), y cada respuesta del daemon pasa por `.ok()?` (`hook.rs:147` y `:157`). Pero si el tope quedara sólo alrededor de la lectura, o si un `401` o el `4xx` de un daemon anterior terminaran en texto, ningún test fallaría. Así, los escenarios de R3 «un daemon que responde un error» y «no responde a tiempo» no tienen prueba. Tampoco hay un test del proceso con un stdin que el cliente no cierra: nada cubre el `shutdown_background` de `crates/clawg-cli/src/main.rs:75-79`, sin el cual el proceso quedaría esperando al hilo de lectura bloqueado. Alcanzaría con un `daemon.json` y un `token` que apunten a un `TcpListener` que acepta y no contesta, otro con un token equivocado (`401`) y, en `binary.rs`, un hijo con el stdin abierto.
- **W2 (la primera mitad, corregida en `b365f78`). Un save largo puede salir en RECENT por un pedazo del medio, y RECENT puede mostrar menos de diez saves aunque la semana tenga más.** `fetch_recent_rows` trae como mucho `RECENT_ROW_POOL = 200` filas, ordenadas por `created_at DESC, id DESC` (`crates/clawg-core/src/memory/prompt_builder.rs:662-673`), y los pedazos se juntan recién después, en `one_row_per_save` (`:711-734`). Los pedazos de un save comparten `created_at`, porque cada registro toma `evt.timestamp_ms` (`crates/clawg-core/src/memory/store_insert.rs:214`), y se insertan en orden, así que el pozo recorre cada save del último pedazo al primero. Si el corte de 200 cae dentro de un save, la apertura queda afuera y la línea muestra el pedazo de menor posición que entró. Eso contradice R1, que pide cada save largo una vez y por su pedazo de menor posición. Que el pozo se llene no alcanza: el corte cae dentro de uno de los diez saves que RECENT muestra sólo cuando esos diez suman más de 200 filas, unos 20 pedazos de media (unos 1.500 tokens por save; un save de más de 480 tokens ya tiene siete pedazos, de 96 tokens con paso de 72). Un solo save de más de unos 14.400 tokens ocupa el pozo entero, y RECENT muestra ese save solo. El test de la apertura usa dos pedazos y no llega al corte. Corregido a medias: `RECENT_ROWS_SQL` ordena por `created_at DESC, id ASC`, así que el corte deja afuera los últimos pedazos de un save y no su apertura; lo fija `a_long_save_the_row_pool_cuts_is_still_shown_by_its_opening`, que con el orden anterior mostraba `piece=2/201`. Sigue abierto que RECENT muestre menos de diez saves en ese caso: se cierra plegando los pedazos antes del `LIMIT` (por `memory_piece`), o contando el pozo en saves.
- **W3 (corregido al archivar). La fila 25 del mapa de `openspec/README.md` se corta al renderizar.** El estado lleva `` `clawg hook session-start|user-prompt-submit --agent A` `` con el `|` sin escapar (`openspec/README.md:51`). En una tabla GFM el `|` separa celdas aunque esté dentro de un code span, así que la fila queda con siete celdas, contra las seis de la cabecera (`:24`). La celda de estado termina en `` `clawg hook session-start ``, con la comilla suelta, y el resto se descarta, incluido «Falta el CI y la prueba con Claude Code». La fila 47 ya lo escapa (`irm \| iex`). La fila viene de `7999388`.

### SUGGESTION

- **S1. La frontera de los siete días se mide dos veces.** `build_session_prompt` calcula `since` con un `TimestampMs::now()` (`prompt_builder.rs:277`), y `assemble` vuelve a leer el reloj para `bucket_by_recency` (`:316`). Una fila que cumple siete días entre las dos lecturas (unos milisegundos: la consulta y `save_links`) pasa el filtro de SQL y cae en LONG-TERM, que R1 no admite en un contexto sin consulta. Pasar el mismo instante a los dos lo cierra.
- **S2. La causa de un modelo fallido no tiene tope.** `session_start_text` copia `embedder_error` entero (`hook.rs:73-80`), y el daemon lo arma con el `e.to_string()` del error de carga (`crates/clawg-core/src/memory/bootstrap.rs:606-609`), que puede traer rutas o URLs. `what_the_hook_adds_stays_small` mide con una causa de 200 bytes (`hook.rs:240`). Las cuatro partes fijas del texto suman 568 bytes, así que una causa de más de unos 430 lleva lo agregado por encima de los 1.000 de R4. Cortar la causa (por ejemplo, a 200 bytes en un límite de carácter) fija el tope.
- **S3. Hay reglas de R1 sin test.** Ningún test siembra once saves para probar el tope `K_FINAL` (`prompt_builder.rs:281`) ni una fila `superseded`: `a_context_without_a_query_skips_what_the_workspace_showed_and_the_dead` sólo borra una. Tampoco hay uno que mire `access_count` después de un contexto sin consulta. Esto último lo garantiza la firma, porque `build_session_prompt` no recibe un `AccessTracker` (`:268-275`); lo otro no.
- **S4. Contra un daemon, `user-prompt-submit` sólo se prueba por su silencio.** El final de `session_start_injects_the_saved_memories` (`crates/clawg-cli/tests/binary.rs:653-658`) crea el transcript en el momento, así que `idle` es casi 0 sea cual sea el último save. El test pasaría igual si `client.list` fallara o si `created_at` llegara en otra unidad (`hook.rs:157`). La decisión está probada en `save_reminder`, pero lo que la conecta con el daemon no. Una salida es que `answer` reciba `now` desde afuera, y un test con un daemon real y un `now` 40 minutos más adelante.
- **S5. Un fallo de SQL en el contexto sin consulta hace perder el workspace.** `build_session_prompt` propaga con `?` el error de `fetch_recent_rows` y el de `save_links` (`prompt_builder.rs:278` y `:280`), y el handler responde `500` (`crates/clawg-cli/src/daemon/api.rs:1135`). Entonces el hook no imprime nada. `build_memory_prompt`, en cambio, convierte cada falla en un bloque vacío y sigue con el workspace (`:135-137`, `:199-203`). Si el contexto sin consulta degradara igual, con RECENT vacío, conservaría las preferencias al iniciar.
- **S6 (corregido en `20d69bc`). Quedaron dos comentarios desfasados.** El doc de `bucket_by_recency` (`prompt_builder.rs:654-657`) quedó pegado arriba del de `RECENT_ROWS_SQL` (`:658-661`): ahora documenta la constante, y la función se quedó sin doc. Y el doc de `main.rs:7-8` sigue diciendo que un error de uso sale con 2, «emitted before `main` ever runs», cuando `clawg hook` sale con 1 (`main.rs:41-48`).

## Diferencias con el diseño

- **R1, el alcance de proyecto.** La spec y `.claude/CLAUDE.md` dicen «del proyecto pedido o sin proyecto». `RECENT_ROWS_SQL` usa el predicado de la búsqueda (`prompt_builder.rs:668`, igual que `crates/clawg-core/src/memory/search.rs:212`), que también deja pasar una `user_preference` de otro proyecto. Es coherente con lo que el agente ya ve cuando busca, pero ningún test lo fija. Al archivar, o la spec lo dice, o el SQL saca esa rama.
- **R5, el pedido del último save.** `answer` le agrega `kind=memory` al `GET /v1/memories?agent=&project=&limit=1` de la spec (`hook.rs:150-156`). Coincide con D2 («Sólo `memory`»): las otras tablas hoy no se escriben.
- **D1.** El borde de E/S se llama `respond`, con `answer` adentro, no `run` (`hook.rs:125-163`). `main` le pasa `tokio::io::stdin()` y el tope, y por eso el tope se prueba sin lanzar un proceso.
- **D2, «un puente nunca lo recibe».** `recall_context` reenvía la `query` que mande el modelo (`crates/clawg-cli/src/mcp/server.rs:774`), y una en blanco ahora arma el contexto sin consulta (`api.rs:1117`): RECENT por fecha, sin registrar accesos y con `recent`, que el puente lee como `Unknown` y por eso no avisa nada (`crates/clawg-cli/src/mcp/tools.rs:366`). Antes se embebía la cadena vacía. Es lo que D2 quería del daemon; el diseño sólo se equivoca al decir que el puente no llega a ese caso.

## Al archivar

- R1 pasó a [`specs/retrieval`](../../../specs/retrieval/spec.md) («El contexto sin consulta»), con el predicado de proyecto que usa el código (el de la búsqueda, que deja pasar una `user_preference` de otro proyecto), y el estado `recent` y la consulta en blanco a [`specs/mcp`](../../../specs/mcp/spec.md). R2 a R5 forman la spec nueva [`specs/hooks`](../../../specs/hooks/spec.md), con los ids de este cambio. [`specs/synthesis`](../../../specs/synthesis/spec.md) y [`specs/visibility`](../../../specs/visibility/spec.md) suman una línea que apunta al hook.
- La fila 25 del mapa se reescribió con el `|` escapado (W3), y el cambio salió del orden de ejecución.
- Quedan en el backlog del README W1 y la segunda mitad de W2.
- La versión sube a 0.22.0.

## Veredicto

Verificado en CI (Linux y Windows), en Windows sobre el arreglo de W2, contra el código, la spec, el diseño y las tareas, y con Claude Code como host sobre el perfil dev: el inicio, la reanudación, la compactación y el recordatorio de los 30 minutos se comportan como piden R3 a R5. No hay ningún CRITICAL. De los tres WARNING, W2 se corrigió a medias y W3 se corrigió al archivar; W1 y lo que queda de W2 siguen abiertos. De las seis SUGGESTION se corrigió S6.
