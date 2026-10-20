# memory-portability — Verificación

## Linux (2026-10-20)

Sobre `23b44a7`, en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`dd6d369`, `f6fadbe`, `6424bca`, `665a8e6`, `ac5deac`), sacado por separado, pasa la suite y clippy.

## El binario real con el embedder falso

`export_and_import_round_trip_through_the_binary` (`crates/clawg-cli/tests/binary.rs`) corre el `clawg` de los tests con `CLAWG_TEST_HOME` en dos directorios temporales y `CLAWG_TEST_EMBEDDER=fake`: exporta con el daemon en marcha (el archivo queda `0600` y un segundo export al mismo archivo sale con 1), `import` sin daemon sale con 1 sin arrancar ninguno, con daemon importa las dos filas bajo `--agent moved`, y el export en Markdown del destino tiene su texto.

## Con el modelo real

`target/debug/clawg serve --profile dev --port 0` con `CLAWG_TEST_HOME` en un directorio temporal y sin `CLAWG_TEST_EMBEDDER`, que descargó `multilingual-e5-small` (466 MB) en ese home; 4 núcleos, build de debug. Un archivo de 500 filas de un agente, de 20 a 60 palabras cada una:

| Corrida | Resultado | Tiempo |
|---|---|---|
| `clawg import --dry-run` | `would import 500 of 500` | 0,29 s |
| `clawg import` | `imported 500 of 500`, snapshot `pre-import` | 7,7 s (unos 15 ms por fila, casi todo embebido) |
| `clawg import` otra vez | `imported 0 of 500`, `already here: 500` | 0,27 s |

Una búsqueda (`POST /v1/search`) con el texto de la fila 42 la devuelve primera. El home temporal se borró después.

## Tests y código

En cada tarea, los tests y el código se escribieron en la misma pasada y se corrieron juntos: no hay una corrida en rojo registrada antes de cada uno. Todos describen conducta que no existía (el módulo `portability`, la ruta `POST /v1/import` y los dos comandos son nuevos), así que ninguno pasaba antes del cambio. Las corridas que fallaron por el camino fueron de los tests contra el código nuevo: la cuenta de vectores de la ida y vuelta (la fila previa del destino no tenía vector), la clave `hits` de la búsqueda y el filtro por estado en `tests/api.rs`, y una pieza del ejemplo que apuntaba a una fila fuera del archivo. El enmascarado de la cabecera de una save (`ac5deac`) salió de repasar el diff, y su aserción entró con el arreglo.

## Lo que no cubre

- Windows: el CI del pull request corre la suite allí; el modo `0600` del archivo es sólo de Unix, como el de `token`.
- `rag-eval`: el ranking no cambia, así que no se corrió.
- Un reimport que no inserta nada igual deja un snapshot `pre-import`. Los automáticos se podan (`AUTO_KEEP`), y el más nuevo de cada motivo queda.
