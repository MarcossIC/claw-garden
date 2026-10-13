# dispute-resolution — Diseño

Las decisiones de producto (opción a, `superseded` fuera del workspace, `disputed` adentro) están en la [propuesta](proposal.md). Acá van las decisiones de implementación.

## D1. Qué significa `disputed_against`

| Opción | A favor | En contra |
|---|---|---|
| **a. Lista de disputas pendientes: al cerrar una disputa se saca el id** | "Valen las dos" queda registrado sin columna nueva. El resolver muerto ya hacía lo mismo (`disputed_against=NULL` al resolver, `crates/clawg-core/src/memory/resolver.rs:158-160`) | Se pierde qué disputas hubo y se cerraron como "valen las dos" |
| b. Historial: nunca se saca nada, y el cierre se guarda aparte | Conserva todo | Necesita una columna o una tabla nueva, o sea migración, para registrar "valen las dos" |
| c. Historial, y "valen las dos" se deduce del estado | Sin migración | Con dos disputas, cerrar una como "valen las dos" y la otra después deja a la fila `disputed` para siempre: no hay forma de saber cuál se cerró |

**Elegida a.** Lo que se pierde es metadato de la disputa, no una memoria: `last_disputed_at` sigue diciendo que hubo una, y `superseded_by` dice quién reemplazó a quién. Para borrar y reemplazar no hace falta editar la lista: una fila borrada o reemplazada deja de contar porque no está viva (R4), así que esas operaciones sólo recalculan estados.

## D2. El estado sale de una regla, no de cada operación

Toda fila no reemplazada que toca este cambio termina en su **estado de reposo** (spec, Definiciones), calculado en SQL con la misma expresión:

```sql
CASE WHEN EXISTS (
  SELECT 1 FROM json_each(memory.disputed_against) j
  JOIN memory p ON p.id = j.value
  WHERE p.deleted_at IS NULL AND p.status <> 'superseded'
) THEN 'disputed' ELSE 'active' END
```

Tres sentencias la usan, y difieren sólo en qué filas eligen: una fila por id (resolución), las `disputed` que nombran a X (borrar o reemplazar X) y todas las `disputed` (mantenimiento). Todas filtran `status <> <expresión>`, así que una fila que no cambia no se escribe ni mueve `last_modified_at`, y el barrido diario no genera escrituras cuando no hay nada que arreglar.

La alternativa, que cada operación escriba el estado final que calcula en Rust, repartía la regla en tres lugares y obligaba a leer las filas vecinas antes de escribir.

## D3. Lo puro y lo que toca la base

- **Puro** (`dispute.rs`): validar el par. Recibe las dos filas ya leídas (`id`, `status`, `superseded_by`, `disputed_against`, borrada o no) y devuelve el error tipado o el permiso. Ahí viven "misma fila", "no enlazadas" y "reemplazada por una tercera". Se prueba sin base.
- **Con base:** leer las dos filas, validar, y aplicar R1 o R2 dentro de la misma transacción (`conn.call` con `transaction()`), para que nadie vea a medias un par resuelto. `MemoryStore` tiene una sola conexión serializada, así que leer y escribir en la misma transacción no compite con nadie.
- **Errores:** `ResolveError` con `thiserror`, separado de `MemoryError` porque el daemon mapea cada variante a un código distinto (400, 404, 409) y `MemoryError` es siempre 500.

## D4. Huérfanas existentes: barrido de mantenimiento, no migración

| Opción | A favor | En contra |
|---|---|---|
| **a. Paso en la pasada diaria de mantenimiento** | Sin migración. Es el mismo patrón que el barrido de vectores huérfanos (`crates/clawg-core/src/memory/maintenance.rs:105-115`). Idempotente | La primera pasada corre 5 minutos después de arrancar (`maintenance.rs:23`), no al abrir |
| b. Migración V9 con un `UPDATE` | Corre una vez, al abrir | Hay otros cambios en curso (storage, project-identity) que pueden sumar V9: la numeración y la lista `PINNED` (`crates/clawg-core/src/memory/migrations.rs:44`) chocarían. Además, una migración no arregla las huérfanas que dejara después un camino que no pase por `delete_memory_core` |
| c. Al abrir el store, siempre | Inmediato | Una escritura en cada arranque, en el camino crítico del arranque |

**Elegida a.** Cinco minutos de espera no cambian nada para el usuario: una fila `disputed` puntúa igual que una activa y su conflicto ya no se muestra.

## D5. Pedazos hermanos: presupuesto de candidatos

| Opción | A favor | En contra |
|---|---|---|
| **a. Pasar los ids hermanos al detector, filtrarlos y pedir k + hermanos** | Un save largo no gasta los 5 candidatos en sus propios pedazos | `detect_contradiction` gana un parámetro |
| b. Filtrar en el worker después del detector | No cambia la firma | Con 5 o más hermanos cerca, una disputa real queda fuera de los 5 candidatos y no se ve |
| c. Detectar contra lo que había antes del save (todos los pedazos contra el store previo) | Exacto | Exige cambiar el orden del loop de inserción, que también usa la deduplicación |

**Elegida a.** Sólo se excluyen los pedazos que el evento **insertó**. Un pedazo absorbido por la deduplicación se fusionó con una fila anterior al save, y esa fila sigue siendo una contraparte legítima.

## D6. Forma de la API

- **Ruta propia** (`POST /v1/disputes/resolve`) y no un `PATCH` sobre una memoria: la operación toca dos filas (y las vecinas) y su resultado depende del par, no de una fila.
- **Sin `kind`:** el detector sólo compara filas de `memory` (`crates/clawg-core/src/memory/contradiction.rs:108-116`), así que no hay disputas en las otras tablas.
- **`keep_both` booleano** y no un enum de salidas: hay dos salidas y "una gana" es la que se espera casi siempre.
- **Sin scope por agente,** igual que `delete` y `PATCH` hoy. visibility-policy lo agrega a todas a la vez.
