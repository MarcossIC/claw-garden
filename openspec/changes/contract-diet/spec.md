# contract-diet — Especificación (el bloque `lean`)

Suma una cuarta variante del bloque del protocolo, `lean`, que `clawg setup claude-code --protocol lean` escribe con su sello, y el brazo del arnés que la mide contra `full`. `full` sigue siendo el valor por defecto y su texto no cambia. Las instrucciones del servidor, las descripciones de las herramientas, el daemon, los hooks y los datos guardados no cambian (CD6).

## Definiciones

- **Bloque, sello, cuerpo, versión propia:** como en [`specs/setup`](../../specs/setup/spec.md) (PB1) y [`specs/doctor`](../../specs/doctor/spec.md) (PB2).
- **Variante:** `full`, `short`, `lean` u `off`.
- **Cabeza** de un protocolo: su texto hasta la primera línea que empieza por `## `, es decir el título y la apertura (`opening`, `crates/clawg-cli/src/mcp/protocol.rs:57`).
- **Sección** `## H` de un protocolo: desde esa línea hasta la siguiente que empieza por `## `, o el final.
- **Nombres** de un protocolo: sus tramos entre comillas invertidas sin espacio, o con `<` (los de `names_and_markers`, `protocol.rs`); un tramo con espacio y sin `<` es un ejemplo de texto guardado.
- **Viñeta de `topic_key`:** la viñeta de la sección «How to write a save» («Cómo escribir un save») que empieza por «A decision or a preference that may change later gets a `topic_key`» («Una decisión o una preferencia que puede cambiar lleva un `topic_key`»).

## CD1. El texto de `lean`

`protocol::lean(language)` MUST devolver, con finales de línea `\n`:

- la cabeza de `protocol::text(language)` sin cambiar un carácter, de modo que `opening(lean(l)) == opening(text(l))` para `None`, `Some(En)` y `Some(Es)`;
- seguida de las secciones de `protocol.lean.md`, o de `protocol.lean.es.md` para `Some(Es)`;
- con la preferencia `en`, la regla del idioma del save reescrita como la reescribe `text` (`SAVE_RULE`, `protocol.rs:28`), que MUST aparecer una vez en `protocol.lean.md`.

El texto MUST cumplir, en las dos lenguas:

- los mismos encabezados `## ` que `text(language)`, en el mismo orden;
- la viñeta de `topic_key` de `text(language)` entera, y las secciones «Session summary» y «After a compaction» («Resumen de sesión», «Después de una compactación») iguales a las de `text(language)`;
- los mismos nombres que `text(language)`, y los mismos marcadores de texto plano (`ACTIVE CONFLICTS`, `LAST SESSION`, `supersedes`, `conflicts`, `inferred`, `imported`, `conclusion`, `merge`, `apart`);
- como mucho el 70 % de los bytes de `text(language)`.

`protocol.md` y `protocol.es.md` MUST NOT cambiar: `text` y `short` devuelven lo mismo que antes de este cambio.

#### Escenario: la apertura intacta
- GIVEN cualquier preferencia de idioma, o ninguna
- WHEN se arma `lean`
- THEN su cabeza es la de `full` y `opening` de los dos es la misma
- AND son las instrucciones que el servidor manda en `initialize`

#### Escenario: lo que queda entero
- GIVEN la preferencia `es`
- WHEN se arma `lean`
- THEN contiene la viñeta de `topic_key` de `protocol.es.md`, `## Resumen de sesión` con `Objetivo:` … `Validación:` y `## Después de una compactación`, tal como están en `full`

#### Escenario: inglés por preferencia
- GIVEN la preferencia `en`
- WHEN se arma `lean`
- THEN dice «Save memories and search in English:» en la apertura y «still in Spanish, search again with the query in Spanish.» en «How to write a save»

**Tests:** `the_lean_protocol_keeps_the_head_of_the_full_one`, `the_lean_protocol_keeps_the_topic_key_rule_and_the_session_summary`, `the_lean_protocol_names_what_the_full_one_names`, `a_language_rewrites_both_rules_of_the_lean_protocol` y `the_lean_protocol_is_at_most_seventy_percent_of_the_full_one` (`protocol.rs`). Hoy fallan: `lean` no existe.

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

## CD5. El brazo `a3-lean`

`agent-eval` MUST tener el brazo `a3-lean`, junto a `a3-short` en `ARMS`: `a3` con el bloque que planea `setup::plan` con `--protocol lean`, y todo lo demás (servidor, hooks, daemon, memorias, argumentos y variables del cliente) igual a `a3`. `a3` MUST seguir escribiendo el bloque `full`.

#### Escenario
- GIVEN una tarea con memorias
- WHEN se planean `a3` y `a3-lean`
- THEN sus planes son iguales salvo el `CLAUDE.md`, que en `a3-lean` es el bloque `lean` con `protocol=lean`

**Tests:** `a3_lean_is_a3_with_the_lean_block` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`), y el brazo en las listas de brazos con daemon, con `setup` y con bloque; hoy falla: el brazo no existe.

## CD6. Lo que no cambia

La variante por defecto MUST seguir siendo `full`. `protocol.md`, `protocol.es.md`, las instrucciones del servidor (`server.rs:2081`) y la lista de herramientas MUST NOT cambiar: `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs:3468`, `:3491`) MUST pasar sin regenerar los archivos con `CLAWG_BLESS=1`. `crates/clawg-cli/src/daemon/` y `crates/clawg-cli/src/hook.rs` MUST NOT cambiar.

**Tests:** los dos del contrato y `without_a_language_the_protocol_is_the_file` (`protocol.rs`), que ya pasan y tienen que seguir pasando.

## Specs que se consolidan al archivar

- CD1, CD3 y CD4 en [`specs/setup`](../../specs/setup/spec.md), junto a PB5 y PB6 (la lista de variantes y `--protocol`) y PB1 (el sello ilegible).
- CD4, la parte de `freshness`, en [`specs/doctor`](../../specs/doctor/spec.md), junto a PB2.
- CD5 en [`specs/eval`](../../specs/eval/spec.md), junto a PB7.
- CD2 y CD6 no se consolidan.
