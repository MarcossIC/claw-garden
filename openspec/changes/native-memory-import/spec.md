# native-memory-import — Especificación

Suma requisitos a [`specs/portability`](../../specs/portability/spec.md) (la fuente `claude-code`) y a [`specs/doctor`](../../specs/doctor/spec.md) (el hallazgo `native_memory`). Origen: [proposal](proposal.md).

## NM1. El directorio

`clawg import --from claude-code DIR` MUST aceptar como DIR:

- un directorio de memoria de un proyecto: el que tiene `MEMORY.md` o se llama `memory`; su proyecto es el nombre del directorio que lo contiene;
- el directorio de un proyecto que tiene un `memory/`; su proyecto es su nombre;
- el directorio de proyectos del cliente: cada subdirectorio con `memory/` es un proyecto.

Un DIR que no es ninguno de los tres, o que no es un directorio, MUST rechazarse antes de que exista un job, con un 400 que lo diga. En cada directorio de memoria, cada archivo regular `*.md` que no es `MEMORY.md` MUST ser un archivo de tema; cualquier otra entrada (otro archivo, un subdirectorio, un `.md` que no es UTF-8) MUST contarse en `unplaced` como `entry`, sin leerse. La suma de lo leído MUST respetar el límite de un import (`MAX_IMPORT_BYTES`).

#### Escenario: los tres directorios
- GIVEN `projects/-work-app/memory/` con un archivo de tema
- WHEN se importa `projects/`, `projects/-work-app` o `projects/-work-app/memory`
- THEN las tres lecturas dan la misma nota, del proyecto `-work-app`

**Tests:** `a_memory_dir_a_project_dir_and_the_projects_dir_read_the_same_note`, `a_directory_without_memory_is_refused`, `entries_that_are_not_topic_files_are_counted`.

## NM2. El frontmatter

Un archivo de tema MAY abrir con un frontmatter entre dos líneas `---`. El adaptador MUST leer `name`, `description`, `type`, `metadata.type`, `modified` y `metadata.modified`, con o sin comillas, y un valor de bloque (`|` o `>`). `type` MUST ganar sobre `metadata.type`, y `modified` sobre `metadata.modified`. Cualquier otra clave MUST contarse en `unplaced` como `frontmatter.<clave>` (`frontmatter.metadata.<clave>` si está anidada), y un frontmatter sin la línea de cierre como `frontmatter.unclosed`, con el archivo entero tomado como cuerpo. Ninguna de estas cosas MUST rechazar el archivo ni el import.

#### Escenario: el tipo anidado
- GIVEN un archivo con `metadata:` y, debajo, `type: feedback`
- WHEN se lee
- THEN la nota tiene el tipo `feedback`

**Tests:** `the_type_reads_top_level_or_nested_under_metadata`, `quoted_and_block_values_read_as_text`, `unknown_keys_and_an_unclosed_frontmatter_are_counted`, `a_file_without_frontmatter_is_all_body`.

## NM3. La nota

Cada archivo de tema MUST dar una nota con:

- origen `claude-code:<proyecto>/<archivo>`;
- título `name`, o el nombre del archivo sin `.md` si no hay;
- contenido `description` y el cuerpo, separados por una línea, o el que haya;
- fecha `modified` si se lee como fecha, si no la hora de modificación del archivo, y si tampoco, ilegible (el import le da su propia hora);
- proyecto de origen, el del directorio (NM1).

#### Escenario: la fecha
- GIVEN un archivo con `modified: 2026-10-30T12:00:00Z` y otro sin `modified`
- WHEN se leen
- THEN el primero tiene esa fecha y el segundo la hora del archivo

**Tests:** `a_topic_file_is_a_note_named_by_its_project_and_file`, `the_date_is_modified_or_else_the_files_time`.

## NM4. Los tipos

Al guardarse, una nota de `claude-code` MUST tomar el tipo `user_preference` si su tipo es `user` o `feedback`, y `semantic_fact` si es `project`, `reference`, otro o ninguno. Ninguna MUST guardarse `architectural_decision`. Los tipos de engram MUST seguir como están.

**Tests:** `claude_code_types_are_preferences_or_facts`.

## NM5. El índice

`MEMORY.md` MUST NOT dar una nota. Cada línea suya que no está en blanco, no es un encabezado (`#`) y no tiene un enlace a un archivo de tema de su directorio MUST contarse en `unplaced` como `index_line`.

**Tests:** `index_lines_without_a_topic_file_are_counted`.

## NM6. El import

`--from claude-code` MUST importar como `--from engram`: el agente `migrated` salvo `--agent`, `--project-map NOMBRE=RUTA|NOMBRE=project:REF` con el nombre del directorio del proyecto (también uno que empieza con `-`), el diario, el snapshot `pre-import`, `--dry-run` y `--undo N`. `--map-root` MUST ser un error de uso. El informe MUST decir `from claude-code:` sin versión de formato. `POST /v1/import` MUST aceptar `from: "claude-code"` con el documento JSON que arma la lectura del directorio como `text`.

#### Escenario: importar dos veces y deshacer
- GIVEN un directorio de proyectos con tres archivos de tema
- WHEN se corre `--dry-run`, después el import, después el import otra vez, después `--undo` del primero
- THEN el `--dry-run` no escribe nada; el import guarda tres notas con el agente `migrated`; el segundo no agrega ninguna; el `--undo` deja el store sin ellas

**Tests:** `a_claude_code_memory_imports_through_the_binary`, `import_takes_a_source_and_project_maps`, `the_claude_code_document_imports_by_text` y los de `parse_input`.

## NM7. El aviso de `doctor`

`doctor` MUST listar los archivos de tema de `<dir de Claude Code>/projects/*/memory/` (el directorio es `$CLAUDE_CONFIG_DIR` o `~/.claude`) sin leer su texto, y compararlos por origen con los `created_by_ref` de las filas importadas (`source = 'import'`). Cuando el `.claude.json` registra el servidor del perfil:

- con algún archivo sin importar, MUST dar un hallazgo `native_memory` `info` `not_imported` con el directorio, cuántos proyectos y archivos hay y cuántos faltan, y el paso `clawg import --from claude-code <dir> --dry-run --profile <perfil>`;
- con todos importados, MUST dar `native_memory` `ok` `healthy`.

Sin servidor registrado o sin archivos de tema, MUST NOT dar el hallazgo.

**Tests:** `native_memory_is_named_until_every_file_is_imported`, `the_facts_name_the_origins_imported_from_claude_code`, `doctor_names_native_memory_not_imported`.
