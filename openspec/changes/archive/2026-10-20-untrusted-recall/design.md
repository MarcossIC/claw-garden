# untrusted-recall — Diseño

## D1. Margen en las líneas de continuación

**Elegido:** `  | ` delante de cada línea de un texto guardado después de la primera, en todo bloque (B de la exploración). Ninguna línea de contenido empieza a columna 0, así que no imita `=== … ===`, `## `, `[fecha #id …]`, `NEW`, `OLD` ni `- `, ni lo que el prompt agregue mañana. La sangría del código y las secciones de un resumen se conservan.

**Alternativas:** aplanar como `truncate_content` (rompe `code_block` y `session_summary`, que son multilínea por naturaleza); una lista negra de prefijos (no cubre la cabecera de un hit ni lo futuro); cercar cada hit (el contenido repite un cerco fijo, y uno variable cuesta más tokens que el margen); una regla propia para LAST SESSION (E: sólo si la medición la pide, y sin medición previa se parte de una sola regla).

**Saltos:** además de `\n` y `\r\n`, un `\r` solo, VT, FF, NEL, U+2028 y U+2029: un tokenizador o un visor puede leerlos como salto, y no cuestan nada.

**Costo:** unos 2 tokens por línea extra; un resumen de seis secciones suma del orden de 12.

## D2. `display_author` al mostrar, no al guardar

`agent_id` es clave de alcance, de `sharing.json` y de la carpeta del workspace. Rechazarlo al guardar perdería saves de un cliente con un nombre raro y no repararía las filas ya guardadas. Sanear al mostrar no esconde nada: `is_shared` y el alcance comparan el crudo. Dos nombres distintos pueden mostrarse igual; la vista no es la clave. 48 caracteres alcanzan para los nombres de cliente conocidos (`claude-code`, `codex`, `cursor`).

`display_author` vive en `prompt_builder.rs`, junto a `SHARED_NOTE`, que `tools.rs` ya importa de ahí.

## D3. El workspace

El margen no basta en `workspace.md`: `split_existing` busca los marcadores con `find`, a mitad de línea. Dentro de un texto copiado, `<!--` pasa a `<! --`: desarma los cuatro marcadores y cualquier comentario HTML, y se lee igual. El AUTO se regenera desde la base, así que un archivo viejo con un marcador falso dentro del AUTO se repara en la próxima regeneración salvo lo que ya hubiera movido a la sección USER, que es del usuario y no se toca.

`sections_holding` (lo que usa `clawg purge` para nombrar los archivos) busca el texto crudo y su forma copiada: un archivo de una versión anterior tiene el crudo, uno nuevo la copia.

## D4. Qué raíz se indexa sola

**Elegido:** A de la exploración. `auto_indexable(root, home, has_marker)` es pura; `has_project_marker(root)` mira el disco (`.git` o un manifiesto de `MANIFESTS` en la raíz). El puente la calcula una vez al construirse (`own_root_to_index`) y le pasa a `indexes_docs` su raíz sólo si la acepta, así que `indexes_docs` no cambia. `home` se canonicaliza como la raíz (`canonicalize` y `strip_verbatim`, que ya es canónica) para que un enlace simbólico o el prefijo `\\?\` de Windows no lo esquiven.

**Alternativas:** sólo excluir `$HOME` y `/` (B: no cubre `~/Documents`); validar en `POST /v1/projects` (rompe la decisión de que el token puede indexar cualquier raíz); saltar ocultos (cambia el corpus de todos, es de 20).

**Costo:** una carpeta de notas sin `.git` ni manifiesto deja de indexarse sola; la frase de `search_project` dice cómo pedirla.
