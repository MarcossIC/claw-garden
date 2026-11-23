# contract-diet — Especificación (el bloque `lean`)

Suma una cuarta variante del bloque del protocolo, `lean`, que `clawg setup claude-code --protocol lean` escribe con su sello, y el brazo del arnés que la mide contra `full`. `lean` es el valor por defecto (CD7, agregado después de D11), y `full` sigue elegible con `--protocol full` sin que su texto cambie. Las instrucciones del servidor, las descripciones de las herramientas, el daemon, los hooks y los datos guardados no cambian (CD6).

## Definiciones

- **Bloque, sello, cuerpo, versión propia:** como en [`specs/setup`](../../../specs/setup/spec.md) (PB1) y [`specs/doctor`](../../../specs/doctor/spec.md) (PB2).
- **Variante:** `full`, `short`, `lean` u `off`.
- **Cabeza** de un protocolo: su texto hasta la primera línea que empieza por `## `, es decir el título y la apertura (`opening`, `crates/clawg-cli/src/mcp/protocol.rs:57`).
- **Sección** `## H` de un protocolo: desde esa línea hasta la siguiente que empieza por `## `, o el final.
- **Nombres** de un protocolo: sus tramos entre comillas invertidas sin espacio, o con `<` (los de `names_and_markers`, `protocol.rs`); un tramo con espacio y sin `<` es un ejemplo de texto guardado.
- **Viñeta de `topic_key`:** la viñeta de la sección «How to write a save» («Cómo escribir un save») que empieza por «A decision or a preference that may change later gets a `topic_key`» («Una decisión o una preferencia que puede cambiar lleva un `topic_key`»).

## CD1. El texto de `lean`

`protocol::lean(language)` MUST devolver, con finales de línea `\n`:

- el título de `protocol::text(language)`, su primera línea, sin la apertura: `opening(lean(l))` MUST ser vacío para `None`, `Some(En)` y `Some(Es)`, porque la apertura la ponen en cada pedido las instrucciones del servidor, `opening(text(l))` (enmendado el 2026-11-20 por [D11](design.md#d11-después-de-la-campaña-lean-sin-la-apertura); antes era la cabeza entera);
- seguida de las secciones de `protocol.lean.md`, o de `protocol.lean.es.md` para `Some(Es)`;
- con la preferencia `en`, la regla del idioma del save reescrita como la reescribe `text` (`SAVE_RULE`, `protocol.rs:28`), que MUST aparecer una vez en `protocol.lean.md`.

El texto MUST cumplir, en las dos lenguas:

- los mismos encabezados `## ` que `text(language)`, en el mismo orden;
- la viñeta de `topic_key` de `text(language)` entera, y las secciones «Session summary» y «After a compaction» («Resumen de sesión», «Después de una compactación») iguales a las de `text(language)`;
- junto con las instrucciones del servidor, los mismos nombres que `text(language)` y los mismos marcadores de texto plano (`ACTIVE CONFLICTS`, `LAST SESSION`, `supersedes`, `conflicts`, `inferred`, `imported`, `conclusion`, `merge`, `apart`);
- como mucho el 70 % de los bytes de `text(language)`.

`protocol.md` y `protocol.es.md` MUST NOT cambiar: `text` y `short` devuelven lo mismo que antes de este cambio.

#### Escenario: sin la apertura
- GIVEN cualquier preferencia de idioma, o ninguna
- WHEN se arma `lean`
- THEN su primera línea es el título de `full` y `opening(lean)` es vacío
- AND la apertura es la que las instrucciones del servidor mandan en `initialize`

Enmendado el 2026-11-20 por D11, como la viñeta de arriba: antes la cabeza de `lean` era la de `full`.

#### Escenario: lo que queda entero
- GIVEN la preferencia `es`
- WHEN se arma `lean`
- THEN contiene la viñeta de `topic_key` de `protocol.es.md`, `## Resumen de sesión` con `Objetivo:` … `Validación:` y `## Después de una compactación`, tal como están en `full`

#### Escenario: inglés por preferencia
- GIVEN la preferencia `en`
- WHEN se arma `lean`
- THEN dice «still in Spanish, search again with the query in Spanish.» en «How to write a save», y no dice «Save memories and search in English:»
- AND las instrucciones del servidor dicen «Save memories and search in English:» y «Reply to the user in English unless»

**Tests:** `the_lean_protocol_keeps_the_title_and_leaves_the_opening_to_the_servers_instructions`, `the_lean_protocol_keeps_the_topic_key_rule_and_the_session_summary`, `the_lean_protocol_and_the_servers_instructions_name_what_the_full_one_names`, `a_language_rewrites_the_save_rule_of_the_lean_protocol_and_the_server_says_the_other` y `the_lean_protocol_is_at_most_seventy_percent_of_the_full_one` (`protocol.rs`), con los nombres que tienen desde D11. Antes de este cambio fallaban: `lean` no existía.

## CD2. Las pruebas del protocolo valen para `lean`

Cada test que fija una regla del texto del protocolo MUST correr también sobre `lean`, en las lenguas en que corre sobre `full`:

- en `server.rs`: `the_protocol_says_where_a_key_is_shown`, `the_protocol_describes_the_session_summary`, `the_spanish_protocol_describes_the_session_summary` y `the_protocol_names_only_what_exists` (que nombra cada herramienta y cada tipo de memoria, y nada que no exista);
- en `protocol.rs`: `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`, `the_protocol_says_which_preferences_open_every_session` y `the_protocol_says_where_a_call_without_a_project_goes`.

#### Escenario
- GIVEN una herramienta renombrada en el router
- WHEN corre `the_protocol_names_only_what_exists`
- THEN falla por `full` y por `lean`

**Tests:** los siete, con `lean` agregado; hoy no compilan con `lean`.

## CD3. El bloque `lean` y su sello

`block` MUST armar, para `Variant::Lean`, `protocol::lean(language)` entre los marcadores, después del sello. `Variant::ALL` MUST ser `full`, `short`, `lean`, `off`, y `Variant::parse("lean")` MUST dar `Lean`. `setup claude-code --protocol lean` MUST escribir el bloque `lean` con `protocol=lean` en el sello; sin la opción, sobre un bloque con `protocol=lean`, MUST conservarlo (PB6). `freshness` MUST dar `current` a un bloque `lean` con su sello, en su idioma, y `stale` en el otro idioma. El error de uso de un `--protocol` desconocido MUST nombrar las cuatro variantes. El reporte MUST decir `(protocol=lean)`.

#### Escenario: elegir `lean` y conservarlo
- GIVEN un `CLAUDE.md` con el bloque `full` de antes del sello
- WHEN corre `clawg setup claude-code --protocol lean --profile dev`
- THEN el bloque es el de `lean` con `<!-- clawg X.Y.Z protocol=lean -->`, y el texto fuera de los marcadores no cambió
- AND una segunda corrida sin `--protocol` no escribe nada
- AND `clawg doctor --profile dev` da `protocol_block` `ok` `healthy`

**Tests:** `each_variant_is_its_block`, `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it` y `a_new_version_with_the_same_text_rewrites_nothing` (`setup.rs`), con `lean`; `a_variant_is_named_by_its_option` (`protocol.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`cli.rs`), con `--protocol lean` y el mensaje; `doctor_marks_an_old_block_stale_without_a_preference` (`tests/binary.rs`), con `--protocol lean` entre las elecciones que dan `ok`. Hoy fallan: la variante no existe.

## CD4. Un sello con una variante desconocida

Un sello cuya variante este binario no conoce (la `lean` leída por un binario anterior, o una variante posterior leída por este) MUST leerse como un sello ilegible (PB1): `Stamp::parse` da `None`, la línea cuenta como parte del cuerpo, y `freshness` MUST dar `stale`, no `newer`, aunque la versión del sello sea posterior a la propia, porque esa versión no se llega a leer. `setup` sin `--protocol` sobre ese bloque escribe la variante por defecto.

#### Escenario
- GIVEN un bloque con el sello `<!-- clawg 0.99.0 protocol=leaner -->` y la versión propia `0.88.0`
- WHEN se clasifica
- THEN es `stale`
- AND su sello se lee como ausente

**Tests:** `a_stamp_with_an_unknown_variant_reads_as_stale` (`setup.rs`). Es un requisito sobre conducta que ya existe: el test fija que `lean` no la cambia.

## CD5. El brazo `a3-full`

Enmendado el 2026-11-20: con `lean` por defecto (CD7), `a3` ya lleva el bloque `lean`, y el brazo `a3-lean` de la campaña pasa a ser `a3-full`. `agent-eval` MUST tener el brazo `a3-full`, junto a `a3-short` en `ARMS`: `a3` con el bloque que planea `setup::plan` con `--protocol full`, y todo lo demás (servidor, hooks, daemon, memorias, argumentos y variables del cliente) igual a `a3`. `a3` MUST escribir el bloque por defecto.

#### Escenario
- GIVEN una tarea con memorias
- WHEN se planean `a3` y `a3-full`
- THEN sus planes son iguales salvo el `CLAUDE.md`, que en `a3` es el bloque `lean` y en `a3-full` el entero con `protocol=full`

**Tests:** `a3_full_is_a3_with_the_whole_block` y `a3_full_reads_and_prints_by_its_name` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`), y el brazo en las listas de brazos con daemon, con `setup` y con bloque.

## CD6. Lo que no cambia

La variante por defecto la fija CD7. `protocol.md`, `protocol.es.md`, las instrucciones del servidor (`server.rs:2081`) y la lista de herramientas MUST NOT cambiar: `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs:3468`, `:3491`) MUST pasar sin regenerar los archivos con `CLAWG_BLESS=1`. `crates/clawg-cli/src/daemon/` MUST NOT cambiar, y `crates/clawg-cli/src/hook.rs` sólo en el bloque de su test (`written_block`), que es el de por defecto.

## CD7. `lean` es el valor por defecto

Agregado el 2026-11-20, después de que `lean` sin la apertura pasara la regla de [D11](design.md#d11-después-de-la-campaña-lean-sin-la-apertura) ([verify](verify.md#d11-lean-sin-la-apertura)). `setup claude-code` sin `--protocol` MUST escribir `lean` sin `protocol=` en el sello cuando el bloque instalado no trae `protocol=`, y MUST mantener la variante elegida cuando lo trae (PB6). Un bloque `full` sin `protocol=` MUST leerse `stale`, y `setup` sin la opción MUST reescribirlo como `lean`. `--protocol full` MUST seguir eligiendo el entero.

#### Escenario: una máquina con el bloque entero de antes
- GIVEN un `CLAUDE.md` con el bloque `full` sin `protocol=`
- WHEN corre `clawg setup claude-code --profile dev` sin `--protocol`
- THEN el bloque es el de `lean`, sin `protocol=` en el sello, y el texto fuera de los marcadores no cambió
- AND el reporte dice `(protocol=lean)`

**Tests:** `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_stamp_with_an_unknown_variant_reads_as_stale`, `each_variant_is_its_block`, `a_block_is_current_stale_or_newer` y `the_block_follows_the_preferred_language` (`setup.rs`); `session_start_says_when_the_block_is_stale` (`hook.rs`).

**Tests:** los dos del contrato y `without_a_language_the_protocol_is_the_file` (`protocol.rs`), que ya pasan y tienen que seguir pasando.

## Specs que se consolidan al archivar

- CD1, CD3, CD4 y CD7 en [`specs/setup`](../../../specs/setup/spec.md), con CD7 en lugar de la frase «la de por defecto MUST ser `full`», junto a PB5 y PB6 (la lista de variantes y `--protocol`) y PB1 (el sello ilegible).
- CD4, la parte de `freshness`, en [`specs/doctor`](../../../specs/doctor/spec.md), junto a PB2.
- CD5 en [`specs/eval`](../../../specs/eval/spec.md), junto a PB7.
- CD2 y CD6 no se consolidan.
