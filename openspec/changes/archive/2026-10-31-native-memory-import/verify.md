# native-memory-import — Verificación

**Veredicto: PASS.** La suite y los dos clippy pasan sobre el último commit del código (`c6fc8a3`), en Linux. Cada requisito de la [spec](spec.md) tiene sus tests, con fixtures armados a mano y el embedder falso; ninguno necesita el modelo real, la red ni un perfil de datos. No se corre `rag-eval`: el cambio no toca el ranking ni lo que ve el agente.

## Suite y clippy

- `cargo test --workspace --no-fail-fast`: pasa entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: sin avisos.

## Requisitos

| Requisito | Qué lo prueba |
|---|---|
| NM1, el directorio | Los tres tipos de DIR dan la misma nota; un directorio sin memoria, un archivo y uno que no existe se rechazan, también por `POST /v1/imports` antes de crear el trabajo; un `.md` que no es UTF-8, otro archivo y un subdirectorio se listan sin leerse y se cuentan como `entry`; el límite de bytes; `POST /v1/import` con el documento |
| NM2, el frontmatter | `type` arriba y anidado en `metadata`, el de arriba gana; comillas simples y dobles, bloques `|` y `>-`, un valor plano en dos líneas; claves desconocidas, una lista y una clave anidada desconocida contadas; un frontmatter sin cierre es cuerpo y se cuenta |
| NM3, la nota | Origen, título (de `name` o del archivo), contenido (descripción y cuerpo), fecha (`modified`, `metadata.modified` con zona, una fecha ilegible que cae a la hora del archivo, ninguna) |
| NM4, los tipos | `user` y `feedback` → `user_preference`; `project`, `reference`, `decision`, `preference` y sin tipo → `semantic_fact`; los de engram sin cambio (`each_source_type_lands_on_its_memory_type` sigue pasando) |
| NM5, el índice | Dos líneas sin archivo contadas; un encabezado y las líneas que enlazan (con `./` y `#ancla`) no |
| NM6, el import | Por el binario: `--dry-run` no escribe; el import guarda tres notas con `migrated`, dos preferencias y ningún `architectural_decision`; un segundo import y su `--dry-run` no agregan nada; `--undo 1` las quita; `--map-root` sale con 2; `--project-map -sandbox-app=…` con guion inicial. El informe dice `from claude-code:` en los dos idiomas |
| NM7, `doctor` | Sin servidor registrado no hay hallazgo; con él, `info` `not_imported` con los conteos y el paso, sin el texto de ninguna memoria en la salida; después del import, `ok`; un archivo nuevo lo vuelve a nombrar. Los orígenes importados se leen del store sin el lock, borrados incluidos, y sólo los de `source = 'import'` |

## Fuera de alcance, sin cambio

- Apagar la memoria nativa: espera a que agent-eval compare clawg contra ella.
- `autoMemoryDirectory`: `doctor` mira el lugar por defecto; un directorio movido se importa nombrándolo.
