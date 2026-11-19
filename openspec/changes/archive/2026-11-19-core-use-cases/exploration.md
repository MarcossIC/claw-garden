# core-use-cases — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M–L, por rutas | memory-module-layout (aplicado en `develop`) | ver [`state.yaml`](state.yaml) |

**Tesis:** las reglas de qué es un guardado válido viven en el handler HTTP. El daemon debería traducir HTTP y delegar, y el núcleo debería decidir. Hoy, para probar una regla de dominio hay que levantar un router, y un puerto de almacenamiento no tendría un caso de uso que lo consuma.

## Problema (verificado contra el código)

Revisado el 2026-11-12 sobre `develop`, después de store-boundary (archivado, 0.84.0) y de memory-module-layout (las carpetas de `crates/clawg-core/src/memory/`). Las cifras del borrador del 2026-11-01 eran de antes de esos dos cambios; donde cambiaron, la de hoy va primero y la del borrador entre paréntesis.

- `crates/clawg-cli/src/daemon/api.rs` tiene 4.396 líneas, 3.412 de producción (3.196), 126 funciones de producción (117) y 68 usos de `bad_request` (68).
- `save_memory` (`api.rs:1420-1552`, 133 líneas; antes `:1331-1456`, 126) decide reglas de dominio:
  - `user_stated` no puede ir con `derived_from` (`:1440-1444`).
  - Las fuentes de una conclusión tienen que existir y estar vigentes (`:1445-1466`).
  - Con el embedder en falla no se acepta el guardado (`:1467-1475`).
- El tipo se valida y se tira: `raw.parse::<MemoryType>()` (`:1427-1429`) descarta el resultado, se vuelve a parsear tirando el error para la métrica de la sesión (`:1482`), y al evento viaja el `String` original (`:1495`).
- Las reglas de formato también están en el daemon, como funciones puras de `api.rs`: `check_content` (`:703`), `check_operation_id` (`:722`, que también usan las rutas del outbox, `:2089`), `distinct_sources` (`:331`), `missing_sources` (`:345`) y `replaced_list` (`:350`).
- Otros handlers largos con la misma forma: `context` (`:1690-1779`, 90 líneas; 87), `register_root` (`:2687-2757`, 71; 71, en `:2498`) y `locate_project` (`:3097-3145`, 49; 59).
- **Corregido:** `api.rs` ya no ejecuta SQL. La consulta que el borrador citaba en `api.rs:2920-2928` la sacó store-boundary a `registry::own_folder_count` (`crates/clawg-core/src/memory/projects/registry.rs:816`), y `locate_project` la llama en `api.rs:3131`. Lo que queda en el handler es la decisión que toma con ese número: con más de una carpeta responde `409 folder_shared`.
- `MemoryRuntime::accept` (`crates/clawg-core/src/runtime.rs:251`; antes `:245`) ya es la entrada natural del guardado en el núcleo.

## Por qué importa aunque haya un solo cliente HTTP

El puente MCP va por HTTP, así que hoy no hay dos caminos que puedan divergir en producción. Lo que sí pasa:

- `clawg-eval` entra por `clawg_core::eval_api` (`crates/clawg-core/src/eval_api.rs`, 1.951 líneas de producción; 1.967) y no pasa por estas reglas: `EvalIngest` encola el evento sin `accept` (`eval_api.rs:727`).
- Una regla de dominio sólo se prueba por HTTP.
- Sin casos de uso en core, los puertos de storage-ports no tienen quién los pida: terminan llamados desde handlers.

## Forma candidata

Separar lo que es de cada capa. El handler conserva lo de HTTP: leer el cuerpo y traducir un error a un código de estado. El caso de uso conserva lo demás:

```rust
// en core: parsear en vez de validar
let save = NewSave::parse(raw)?;          // SaveRejected, sin saber de HTTP
let accepted = runtime.accept(save).await?;

// en el handler: sólo traducción
let rejected_to_status = |e: SaveRejected| ApiError::bad_request(e.to_string());
```

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Caso de uso por operación en core, ruta por ruta, empezando por el guardado** | Entrega valor pronto, y cada ruta se verifica contra los tests de API que ya existen | Un tiempo conviven los dos estilos |
| B. Una capa `application` entera de una vez | Queda uniforme | Un cambio enorme sin puntos de control |
| C. Sólo partir `api.rs` en archivos por ruta | Fácil y útil para navegar | No mueve ninguna regla: el olor sigue, repartido |

**Recomendado: A**, y partir `api.rs` en `daemon/routes/` como paso mecánico al final de la primera tanda. C sola no alcanza.

## Alcance candidato

1. `POST /v1/memories`: tipos de entrada validados y errores de dominio en core.
2. Las rutas que escriben: disputas, actualizar y restaurar memorias, proyectos.
3. Partir `api.rs` por dominio.

### No-alcance

- Cambiar códigos de estado, cuerpos de respuesta o mensajes de error. Ninguna respuesta existente cambia, ni con campos agregados.
- Las rutas de sólo lectura sin reglas, salvo que el diseño encuentre alguna.
- El contrato MCP.

## Decisiones abiertas

1. Qué errores de dominio llevan un texto ya listo y cuáles un dato que el handler traduce. Hay mensajes en dos idiomas en la CLI (`Lang`).
2. Si `check_content` y `check_operation_id` (formato de la entrada) son de HTTP o de dominio. Un límite de tamaño es del producto (`specs/ingest`), no de HTTP.
3. Si `eval_api` pasa a usar estos casos de uso. Sería más fiel a producción, pero podría mover mediciones.

## Cómo se mide

- **Conducta:** `crates/clawg-cli/tests/api.rs` pasa sin editarse. Ese es el contrato de que nada cambió.
- **Test en rojo:** cada regla movida tiene un test en core, sin router, que antes no se podía escribir.
- `api.rs` baja de líneas, y ningún handler decide una regla de dominio.

## Interacciones

- **domain-types:** los tipos validados de este cambio son el primer lugar donde entran `MemoryType` y los newtypes.
- **storage-read-side y storage-write-side:** los casos de uso son quienes abren `store.read` y `store.write`.
- **cli-dispatch:** sin relación, pero comparte la idea: un punto de entrada traduce y delega.
