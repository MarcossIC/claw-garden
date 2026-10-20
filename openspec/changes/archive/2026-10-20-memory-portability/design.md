# memory-portability — Diseño

## D1. Un camino de importación propio, no el save

**Elegido:** `memory::portability::import` inserta filas completas en transacciones propias.

**Alternativa:** `POST /v1/memories` por fila. Pierde estados, accesos, tier, cadenas y piezas (`store_insert.rs:507-586`), pasa cada fila por el detector, que abriría pares entre filas que ya convivían, y por el dedup, que refuerza en vez de saltar.

**Costo:** un segundo camino de escritura. Cada garantía del worker tiene su test en el import: el enmascarado (`redact`), el `content_hash`, el índice de texto (una fila viva indexada una vez; una borrada se inserta viva y se marca borrada en la misma transacción, para que el trigger de V8 la saque), una sola viva por texto y una sola save viva por clave de tema.

## D2. Tres pasadas

1. **Decidir**, en una transacción que se descarta: para cada fila, si es inválida, si ya está, si se funde con una viva igual, si una clave tomada la reemplaza o si se inserta. Se inserta de verdad (sin vector) para que las filas del mismo archivo se vean entre sí, y se deshace. Esto es todo `--dry-run`.
2. **Embeber** fuera de toda transacción, en lotes de 32 (`embed_batch`), sólo las filas que se insertan y no están borradas.
3. **Escribir** en transacciones de a 256 filas, volviendo a decidir dentro de cada una (el worker escribe en paralelo), y una última transacción con los vínculos.

Si el proceso muere entre transacciones, las filas escritas están y los vínculos no. Volver a importar el mismo archivo las encuentra presentes (R5) y completa sólo los vínculos vacíos, así que el import se repite sin daño.

**Alternativa:** una sola transacción. Bloquea la única conexión del daemon mientras escribe miles de filas, y el re-embebido no puede ir dentro.

## D3. Qué es «la misma fila»

Presente = mismo agente (después de `--agent`), mismo proyecto de destino, mismo hash del texto enmascarado y mismo `created_at`. El hash sólo no basta: el origen puede tener la misma frase guardada dos veces (una reemplazada y otra viva, o una borrada y otra guardada de nuevo), y son filas distintas. Con la fecha en milisegundos, dos filas iguales en todo eso son la misma.

## D4. `source` se conserva

`provenance::mark` muestra `imported` para `source = 'import'`. Este cambio no lo escribe: mover memorias entre perfiles no cambia quién las dijo, y una conclusión tiene que seguir siendo `conclusion`. `import` queda para lo que llega de otra herramienta (history-import, 44).

## D5. Export sin el daemon

El export lee con la conexión de sólo lectura de `secrets` (`open_for_probe`, `backup.rs:528`): funciona con o sin daemon y no toma el lock. Lo que lee es lo confirmado; las saves que esperan en el outbox no viajan y el comando lo dice. Una base de un esquema anterior se lee con las columnas que tenga (`PRAGMA table_info`), sin migrarla, que pediría escribir.

**Alternativa:** una ruta del daemon, como `backup`. Exige un daemon para una lectura y pasa el archivo entero por HTTP.

## D6. El import entra por HTTP con el archivo entero

`POST /v1/import` recibe el texto del archivo en el cuerpo (tope `MAX_IMPORT_BYTES`, 256 MiB, sólo en esa ruta) y lo parsea el núcleo con una función pura. El CLI resuelve `--map-root NEW` como un puente (`resolve_project_root`), porque el daemon no tiene por qué leer rutas del usuario, y valida el tamaño antes de enviar.
