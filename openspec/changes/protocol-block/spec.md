# protocol-block — Especificación

Cambia cómo `clawg setup claude-code` escribe el bloque del protocolo en el `CLAUDE.md` del usuario, cómo `clawg doctor` lo revisa y qué dice `clawg hook session-start` cuando el bloque no es el de este binario. Suma la opción de elegir la variante del bloque y el brazo del arnés que la mide. El valor por defecto sigue siendo el bloque entero. Las instrucciones del servidor, las descripciones de las herramientas, el daemon y los datos guardados no cambian (PB8).

## Definiciones

- **Bloque:** el texto del `CLAUDE.md` del usuario desde `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` hasta el final de `<!-- END CLAWG MEMORY PROTOCOL -->` (`BEGIN_MARKER`, `END_MARKER`, `crates/clawg-cli/src/setup.rs:36-37`; `find_block`, `setup.rs:612`). El archivo es el que lee `setup` (`locations`, `setup.rs:670`).
- **Sello:** la línea que sigue al marcador de apertura, `<!-- clawg X.Y.Z -->` o `<!-- clawg X.Y.Z protocol=V -->`, con `X.Y.Z` como lo lee `Version::parse` (`crates/clawg-cli/src/update/version.rs:21`) y `V` una de las variantes.
- **Variante:** `full`, `short` u `off` (PB5).
- **Cuerpo:** el bloque sin el sello, con los finales de línea pasados a `\n`.
- **Versión propia:** la de `clawg_core::build_info::version()`.

## PB1. El sello

`setup claude-code` MUST escribir el sello en la línea siguiente al marcador de apertura, con la versión propia, y con ` protocol=V` sólo cuando la variante se eligió (PB6). Los dos marcadores MUST NOT cambiar, así que `find_block` y `remove_block` de un binario anterior encuentran y quitan un bloque con sello. Un bloque sin sello MUST leerse como uno sin versión y sin variante elegida. Un sello que no se puede leer (otra forma, una versión mal escrita, una variante desconocida) MUST leerse como si no estuviera, y la línea cuenta como parte del cuerpo.

#### Escenario: el sello de un bloque nuevo
- GIVEN un `CLAUDE.md` que no existe y la versión propia `0.88.0`
- WHEN `setup claude-code` planea instalar sin `--protocol`
- THEN el archivo empieza por `<!-- BEGIN CLAWG MEMORY PROTOCOL -->`, sigue con `<!-- clawg 0.88.0 -->` y después `# Claw Garden memory protocol`

#### Escenario: un bloque de antes del sello
- GIVEN un bloque sin sello
- WHEN se lee su sello
- THEN no tiene versión ni variante elegida

**Tests:** `the_block_carries_the_version_that_wrote_it` y `a_stamp_reads_its_version_and_its_choice` (`setup.rs`); hoy fallan: el bloque no tiene sello.

## PB2. La frescura del bloque

Una función pura MUST clasificar un bloque instalado, dado el idioma preferido y la versión propia, contra el bloque que este binario escribiría para la variante que dice el sello (o la de por defecto, sin variante elegida):

- `current`, si los cuerpos son iguales;
- `newer`, si difieren y el sello tiene una versión posterior a la propia;
- `stale`, si difieren en cualquier otro caso, también sin sello o con un sello que no se lee.

La clasificación MUST NOT mirar la versión cuando los cuerpos son iguales, de modo que un bloque que escribió una versión anterior con el mismo texto es `current`.

#### Escenario: mismo texto, otra versión
- GIVEN un bloque `full` con sello `0.87.0` cuyo cuerpo es el que escribe `0.88.0`
- WHEN se clasifica con la versión propia `0.88.0`
- THEN es `current`

#### Escenario: el bloque de antes de verdict-on-replace
- GIVEN un bloque sin sello con el texto que pide juzgar cada cercana como `compatible` o `scoped`
- WHEN se clasifica con la versión propia `0.88.0`, sin preferencia
- THEN es `stale`

#### Escenario: un bloque de un binario posterior
- GIVEN un bloque con sello `0.90.0` y otro cuerpo
- WHEN se clasifica con la versión propia `0.88.0`
- THEN es `newer`

**Tests:** `a_block_is_current_stale_or_newer` (`setup.rs`); hoy falla: la función no existe.

## PB3. `doctor` revisa el bloque con preferencia o sin ella

`doctor` MUST sumar el chequeo `protocol_block` siempre que el `CLAUDE.md` del usuario tenga un bloque, haya preferencia de idioma o no, con la clasificación de PB2:

- `current`: `ok` `healthy`;
- `stale`: `warn` `stale`, con `safe_next_step` `clawg setup claude-code --profile P` y un `message` que dice que el bloque no es el que escribe este clawg y que el agente puede leer ahí otras reglas que en las instrucciones del servidor, sin nombrar el idioma preferido cuando no lo hay;
- `newer`: `info` `newer`, sin `safe_next_step`, con un `message` que dice que lo escribió un clawg posterior;
- un archivo que no se lee o un marcador suelto: `warn` `unreadable`, como hoy.

`evidence` MUST llevar `path`, `written_by` (la versión del sello, o `null`) y `this_version` (la versión propia). Sin bloque, `doctor` MUST sumar `info` `not_installed` sólo con preferencia, como hoy, y sin preferencia MUST NOT sumar ningún hallazgo `protocol_block`. Cada `message` MUST estar en los dos idiomas (CL4 de [`specs/doctor`](../../specs/doctor/spec.md)).

Esto reemplaza en LP14 de `specs/doctor` «Con preferencia, MUST sumar el chequeo `protocol_block`» y, en LP2 de [`specs/preferences`](../../specs/preferences/spec.md), «`doctor` no tiene hallazgos … `protocol_block`» pasa a «`doctor` no tiene hallazgos `preferences`, y `protocol_block` sólo cuando hay un bloque».

#### Escenario: sin preferencia, un bloque viejo
- GIVEN un perfil sin `preferences.json` y un `CLAUDE.md` con un bloque sin sello y otro texto
- WHEN corre `clawg doctor --profile dev`
- THEN hay un hallazgo `protocol_block` `warn` `stale` con `safe_next_step` `clawg setup claude-code --profile dev`
- AND su `evidence.written_by` es `null`

#### Escenario: sin preferencia, sin bloque
- GIVEN un perfil sin `preferences.json` y un `CLAUDE.md` sin marcadores
- WHEN corre `clawg doctor`
- THEN no hay hallazgos `protocol_block`

#### Escenario: después de `setup`
- GIVEN el `CLAUDE.md` que deja `clawg setup claude-code --profile dev`, con cualquier variante
- WHEN corre `clawg doctor --profile dev`
- THEN `protocol_block` es `ok` `healthy`

**Tests:** `protocol_block_findings` (`doctor/checks.rs`), con `newer` y con el mensaje sin preferencia; `the_findings_are_the_same_in_both_languages_but_their_message` (`checks.rs`), con los hallazgos nuevos; `doctor_marks_an_old_block_stale_without_a_preference` (`tests/binary.rs`), que hoy falla: sin preferencia no hay hallazgo. `doctor_names_a_broken_preferences_file` (`tests/binary.rs:2387`) sigue igual: su sandbox no tiene bloque.

## PB4. `session-start` avisa del bloque que difiere

`clawg hook session-start` con `--client claude-code` MUST leer el bloque del `CLAUDE.md` del usuario (el de `locations`, con el `CLAUDE_CONFIG_DIR` de su entorno) y, cuando PB2 lo da `stale`, MUST sumar la línea del aviso: que el bloque del protocolo del `CLAUDE.md` del usuario no es el que escribe este clawg, con su versión, que puede contradecir las instrucciones del servidor, que donde difieran siga las del servidor, y que le diga una vez a la persona que `clawg setup claude-code --profile P` lo reescribe, con el perfil del hook. Los textos son los de [design](design.md#d3-el-aviso-en-cada-inicio-de-sesión), en el idioma preferido (AL6 de [`specs/hooks`](../../specs/hooks/spec.md)).

- La línea MUST ir después de la del idioma y antes del contexto.
- MUST imprimirse en cada `session-start`, con cualquier `source`, también sin daemon, con un daemon que falla o no responde, o con el contexto vacío, como la línea del idioma (R3 y R7 de `specs/hooks`).
- MUST NOT imprimirse con un bloque `current` o `newer`, sin bloque, con un archivo que no se lee o con un marcador suelto, ni en `user-prompt-submit`, ni con `--client cursor`, ni cuando el hook de Claude Code lo corre Cursor (CU8).
- La lectura MUST ir en un hilo propio, dentro de la misma espera que la preferencia (`LANGUAGE_WAIT`, `crates/clawg-cli/src/hook.rs:57`); un archivo que no se leyó a tiempo cuenta como sin bloque, y MUST NOT demorar la respuesta más allá del tope de R3.
- Lo que el hook suma al prompt MUST ocupar menos de 2.000 bytes (antes 1.500, R4 de `specs/hooks`), y el contexto (8.000) más eso MUST seguir dentro de los 10.000 que el host inyecta.
- El hook MUST NOT escribir el `CLAUDE.md`.

#### Escenario: un bloque viejo, sin daemon
- GIVEN un `CLAUDE.md` con un bloque `stale`, sin preferencia y sin daemon
- WHEN corre `clawg hook session-start --agent claude-code --profile dev` con un `cwd`
- THEN imprime el sobre de `SessionStart` con sólo la línea del aviso, que nombra `clawg setup claude-code --profile dev`
- AND sale con 0

#### Escenario: con todo lo demás
- GIVEN un bloque `stale`, la preferencia `es`, un contexto, una compactación, el modelo caído, memorias sin revisar y sugerencias pendientes
- WHEN se arma el texto de `session-start`
- THEN abre con la línea del idioma, sigue con el aviso en español y después con el contexto
- AND lo que suma ocupa menos de 2.000 bytes

#### Escenario: nada que avisar
- GIVEN un bloque `current`, uno `newer`, ninguno, o uno con un marcador suelto
- WHEN corre `session-start`
- THEN el texto es el de hoy

**Tests:** `session_start_says_when_the_block_is_stale` y `what_the_hook_adds_stays_small` (`hook.rs`), con el aviso; `a_block_that_never_reads_costs_the_notice_and_not_the_memory` (`hook.rs`), con una lectura que no termina; `session_start_names_an_old_protocol_block` (`tests/binary.rs`). Hoy fallan: el hook no lee el bloque.

## PB5. Las variantes del bloque

`block` MUST armar, entre los marcadores y después del sello:

- `full`: `protocol::text(language)`, como hoy (`crates/clawg-cli/src/mcp/protocol.rs:58`);
- `short`: el título del protocolo del idioma, la línea fija de [design](design.md#d4-tres-variantes-de-un-solo-archivo) y las secciones «Session summary» y «After a compaction» (en español, «Resumen de sesión» y «Después de una compactación») tal como están en ese mismo texto, cada una hasta la sección siguiente;
- `off`: nada; el bloque son los dos marcadores y el sello.

El bloque `short` MUST ocupar menos de 2.000 bytes en los dos idiomas, y MUST NOT contener la apertura.

#### Escenario: el bloque corto en español
- GIVEN la preferencia `es`
- WHEN se arma el bloque `short`
- THEN contiene `## Resumen de sesión` con sus secciones (`Objetivo:` … `Validación:`) y `## Después de una compactación`
- AND no contiene `## Cómo escribir un save` ni la primera oración de la apertura

**Tests:** `the_short_protocol_keeps_the_session_summary_in_both_languages` y `the_short_protocol_has_no_opening` (`protocol.rs`); `each_variant_is_its_block` (`setup.rs`). Hoy fallan: no hay variantes.

## PB6. `--protocol`

`clawg setup claude-code` MUST tomar `--protocol full|short|off`. Con la opción MUST escribir esa variante, con `protocol=V` en el sello. Sin la opción MUST mantener la variante del sello del bloque instalado cuando trae `protocol=`, y si no, escribir la de por defecto sin `protocol=`. La de por defecto MUST ser `full`. `--protocol` MUST ser un error de uso con `--remove`, con `--local` o `--path`, y con `setup cursor`, sin escribir nada. El reporte MUST nombrar la variante que quedó.

Con el bloque instalado igual al planeado salvo por la versión del sello, el paso MUST ser `Unchanged` y el sello MUST quedar como estaba (R4 y R5 de [`specs/setup`](../../specs/setup/spec.md)).

#### Escenario: elegir y conservar
- GIVEN un `CLAUDE.md` con el bloque `full` de antes del sello
- WHEN corre `clawg setup claude-code --protocol short --profile dev`
- THEN el bloque es el `short` con `protocol=short` en el sello, y el texto fuera de los marcadores no cambió
- AND una segunda corrida sin `--protocol` no escribe nada

#### Escenario: sin elección, la de por defecto
- GIVEN un bloque `full` sin `protocol=` en el sello
- WHEN corre `clawg setup claude-code` sin `--protocol`
- THEN el bloque queda `Unchanged`

#### Escenario: con `--local`
- WHEN corre `clawg setup claude-code --local --protocol short`
- THEN sale con un error de uso y no escribe nada

**Tests:** `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_new_version_with_the_same_text_rewrites_nothing` (`setup.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`cli.rs`); `setup_protocol_short_writes_the_short_block` (`tests/binary.rs`). Hoy fallan: la opción no existe.

## PB7. El brazo `a3-short`

`agent-eval` MUST tener el brazo `a3-short`: `a3` con el bloque `short` que planea `setup::plan` con `--protocol short`, y todo lo demás (servidor, hooks, daemon, memorias, argumentos del cliente) igual a `a3`. `a3` MUST seguir escribiendo el bloque `full` y `l0` ninguno.

#### Escenario
- GIVEN una tarea con memorias
- WHEN se planean `a3` y `a3-short`
- THEN sus planes son iguales salvo el `CLAUDE.md`, que en `a3-short` es el bloque corto con `protocol=short`

**Tests:** `a3_short_is_a3_with_the_short_block` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`); hoy falla: el brazo no existe.

## PB8. Lo que no cambia

Las instrucciones del servidor (`server.rs:2069`) y la lista de herramientas MUST NOT cambiar: `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs:3421`, `:3444`) MUST pasar sin regenerar los archivos con `CLAWG_BLESS=1`. `crates/clawg-cli/src/daemon/` MUST NOT cambiar. Ningún comando, hook ni el daemon MUST escribir el `CLAUDE.md`, salvo `setup claude-code`, como hoy.

**Tests:** los dos del contrato, que ya pasan y tienen que seguir pasando sin regenerarse; es el único requisito sin un test que falle antes, porque pide que nada cambie.

## Specs que se consolidan al archivar

- PB1, PB5 y PB6 en [`specs/setup`](../../specs/setup/spec.md), junto a R4 y LP13.
- PB2 y PB3 en [`specs/doctor`](../../specs/doctor/spec.md), reemplazando la frase de LP14; el cambio de LP2 en [`specs/preferences`](../../specs/preferences/spec.md).
- PB4 en [`specs/hooks`](../../specs/hooks/spec.md), con el tope nuevo de R4 y lo que suman R3 y R7.
- PB7 en [`specs/eval`](../../specs/eval/spec.md).
- PB8 no se consolida.
