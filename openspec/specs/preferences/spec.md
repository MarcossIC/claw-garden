# preferences — Especificación

Fuente de verdad del comportamiento actual de las preferencias del usuario: el archivo `preferences.json` de un perfil, cómo se lee y se escribe, qué pasa sin él o con uno roto, y `clawg preferences`. La única preferencia es el idioma, `es` o `en`. Lo que el idioma cambia vive en las specs de cada parte: el idioma de cada fila en [`specs/storage`](../storage/spec.md) (LP4, LP5) y [`specs/portability`](../portability/spec.md) (LP6), el conteo del otro idioma en [`specs/retrieval`](../retrieval/spec.md) (LP7), el puente en [`specs/mcp`](../mcp/spec.md) (LP8 a LP11), `session-start` en [`specs/hooks`](../hooks/spec.md) (LP12), `setup` en [`specs/setup`](../setup/spec.md) (LP13), `doctor` en [`specs/doctor`](../doctor/spec.md) (LP14) y `rag-eval --lang-detect` en [`specs/eval`](../eval/spec.md) (LP15).

**Origen:** [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23. Los ids de requisito son los de esa spec; acá están LP1 a LP3.

## Definiciones

- **Preferencia:** `<data>/preferences.json` de un perfil (`paths::preferences_file`), leído con éxito y con `language` puesto. Un archivo ausente, inválido o sin `language` es **sin preferencia**.
- **Idioma preferido:** el `language` de la preferencia, `es` o `en` (`Lang`, `crates/clawg-core/src/preferences.rs:21`). El **otro idioma** es el que no es el preferido.

## LP1. La preferencia y su archivo

`preferences.json` MUST tener la forma `{"version": 1, "language": "es"|"en"}`, con `language` opcional (`Preferences`, `preferences.rs:89`). Leerlo (`load`, `preferences.rs:113`) MUST rechazar una versión distinta de 1, un campo desconocido y un idioma que no sea `es` ni `en`. Un archivo ausente MUST dar «sin preferencia» sin error. `language_in` (`preferences.rs:124`) MUST loguear un archivo inválido y leerlo como sin preferencia. Escribirlo (`save`, `preferences.rs:137`) MUST pasar por un archivo temporal y un `rename` dentro del directorio de datos, creado privado como el resto (`private_dir`). El archivo MUST NOT estar en `memory.db`, en un snapshot ni en un export.

#### Escenario: ida y vuelta
- WHEN se escribe la preferencia `es` y se vuelve a leer
- THEN da `es`, y el texto es `{"version": 1, "language": "es"}` con salto de línea final

#### Escenario: lo que se rechaza
- WHEN el archivo dice `"version": 2`, `"lenguaje": "es"` o `"language": "fr"`
- THEN la lectura falla con un error que nombra el archivo y la causa, y quien lo lea como idioma obtiene «sin preferencia»

**Test:** `a_missing_file_is_no_preference`, `a_preference_round_trips`, `an_unknown_version_field_or_language_is_refused`, `an_invalid_file_reads_as_no_preference` y `saving_goes_through_a_temporary_file` (`preferences.rs`).

## LP2. Sin preferencia, lo de antes

Sin preferencia, las instrucciones del servidor, la lista de herramientas (salvo la descripción de LP10, [`specs/mcp`](../mcp/spec.md)), las respuestas de las herramientas, el contexto, lo que imprimen los hooks, el bloque que escribe `setup` y los hallazgos de `doctor` MUST ser los de antes de las preferencias. Un archivo inválido MUST NOT hacer fallar ningún comando, herramienta ni hook: el puente, `session-start` y `setup` lo leen con `language_in`, y sólo `doctor` lo nombra (LP14).

#### Escenario: nada cambia
- GIVEN un perfil sin `preferences.json`
- THEN `get_info().instructions` es la apertura de `protocol.md`, `session_start_text` da lo de antes, `setup` deja el bloque instalado `Unchanged` y `doctor` no tiene hallazgos `preferences` ni `protocol_block`

#### Escenario: un archivo roto
- GIVEN un `preferences.json` que no es JSON
- THEN el puente arranca y sus instrucciones son las de sin preferencia, `session-start` imprime lo de sin preferencia y `doctor` lo marca

**Test:** `without_a_preference_the_instructions_are_the_opening` (`server.rs`); `without_a_preference_session_start_says_what_it_said` (`hook.rs`); `an_invalid_file_reads_as_no_preference` (`preferences.rs`), la lectura que hacen todos; los tests de `server.rs`, `hook.rs`, `setup.rs`, `prompt_builder.rs` y `the_agent_contract_matches_its_file` con sus textos esperados de antes. Ningún test arranca el puente con un archivo roto.

## LP3. `clawg preferences`

`clawg preferences [--profile P]` MUST mostrar la preferencia o decir que no hay y que clawg usa el inglés. `clawg preferences set language es|en` MUST escribirla y `clawg preferences unset language` MUST quitarla, y las dos MUST decir que se aplica a las sesiones del cliente que se abran después y que `clawg setup claude-code` reescribe el bloque del protocolo. Un archivo inválido MUST hacer fallar `set` y `unset` con salida 1, nombrando la causa, sin tocarlo. Las frases del comando MUST salir en el idioma preferido después del cambio (en inglés sin preferencia), un `match` sobre `Lang` por frase (`crates/clawg-cli/src/preferences.rs`). MUST NOT necesitar el daemon ni abrir el store.

#### Escenario: fijar y quitar
- WHEN se corre `clawg preferences set language es --profile dev`
- THEN `preferences.json` del perfil dev dice `es`, la salida está en español y nombra la sesión nueva y `clawg setup claude-code`
- WHEN se corre después `clawg preferences unset language --profile dev`
- THEN el archivo no tiene `language` y la salida está en inglés

**Test:** `preferences_parses_set_and_unset` (`cli.rs`); `the_preferences_command_speaks_the_language_it_sets` (`preferences.rs` de `clawg-cli`); `preferences_set_and_unset_write_the_profiles_file` (`tests/binary.rs`).
