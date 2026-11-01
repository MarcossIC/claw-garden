# native-memory-import — Diseño

Decisiones con alternativas. Requisitos en la [spec](spec.md) (NM1 a NM7). Lo puro (el frontmatter, el índice, las notas, el hallazgo) está separado de lo que hace I/O (listar y leer el directorio, el store).

## D1. Quién lee el directorio (NM1, NM6)

| Opción | A favor | En contra |
|---|---|---|
| **a. El daemon, con la ruta que manda la CLI** (elegida) | Es como funciona hoy todo import por la CLI: el job recibe una ruta y lee en `spawn_blocking` (`read_export`, `crates/clawg-cli/src/daemon/api.rs:2227-2251`); el progreso, Ctrl-C y el límite de una sola importación a la vez valen sin cambios | El daemon lee un directorio además de un archivo |
| b. La CLI arma el documento y lo manda por `POST /v1/import` (la exploración) | El daemon lee un solo texto | Pierde el job: sin progreso, sin cancelación, y el cuerpo viaja entero; la exploración es anterior a import-job |

El documento existe igual: es lo que la lectura arma (`{"projects": [{"name", "entries": [{"name", "text", "modified_ms"}]}]}`) y lo que `POST /v1/import` acepta como `text` con `from: "claude-code"`. Así el adaptador es una función pura del documento, que los tests arman a mano, y las dos rutas del daemon traducen lo mismo.

## D2. Cómo se lee el frontmatter (NM2)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un lector propio de claves de una línea, un mapa anidado de un nivel y valores de bloque** (elegida) | Es todo lo que el cliente escribe; sin dependencia nueva (el workspace no tiene YAML) | No es YAML completo: una lista o un anidamiento más profundo se cuentan, no se leen |
| b. Un crate de YAML | Lee cualquier frontmatter | Una dependencia para cuatro claves; un YAML inválido haría fallar el archivo en vez de contarlo |

Lo que el lector no entiende nunca rechaza el archivo: se cuenta en `unplaced` por clave, y lo que se lee se lee.

## D3. Dónde se decide el tipo (NM4)

| Opción | A favor | En contra |
|---|---|---|
| **a. `translate` mira la fuente de la exportación** (elegida) | El informe muestra el tipo como lo escribió la fuente (`user`, `feedback`), y el mapeo de cada fuente queda junto | `memory_type_of` recibe la fuente |
| b. El adaptador traduce a los tipos de engram (`preference`) | `translate` no cambia | El informe mentiría sobre el tipo de origen, y un tipo de engram con el mismo nombre que uno del cliente se mezclaría |

## D4. Cómo sabe `doctor` qué falta importar (NM7)

| Opción | A favor | En contra |
|---|---|---|
| **a. Comparar orígenes:** `claude-code:<proyecto>/<archivo>` de cada archivo listado contra los `created_by_ref` de las filas `source = 'import'` (elegida) | Lee nombres, no textos (las reglas de `doctor`); el aviso se apaga solo cuando se importa y vuelve si aparece una memoria nueva | Un archivo reescrito después del import no se avisa (su origen ya está); un segundo import lo trae como versión nueva |
| b. Avisar siempre que haya archivos | Más simple | No se apaga nunca: un aviso que no cambia con lo que el usuario hace deja de leerse |

Una fila importada que el usuario borró sigue contando como importada: el usuario ya la vio.

## D5. La versión del formato

El cliente no versiona su formato, y el documento es de clawg. `ForeignExport.version` queda vacía para `claude-code` y el informe dice `from claude-code:` en lugar de inventar una versión.

## D6. El nombre de un proyecto que empieza con guion (NM6)

El nombre del directorio de una ruta absoluta empieza con `-` (`/work/app` → `-work-app`). Sin más, `--project-map -work-app=/work/app` lo toma clap como una opción. `--project-map` acepta valores con guion inicial (`allow_hyphen_values`): un valor de `--project-map` siempre lleva `=`, así que no se confunde con otra opción.
