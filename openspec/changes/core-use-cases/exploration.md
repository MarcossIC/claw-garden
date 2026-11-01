# core-use-cases — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M–L, por rutas | memory-module-layout | exploración (borrador) |

**Tesis:** las reglas de qué es un guardado válido viven en el handler HTTP. El daemon debería traducir HTTP y delegar, y el núcleo debería decidir. Hoy, para probar una regla de dominio hay que levantar un router, y un puerto de almacenamiento no tendría un caso de uso que lo consuma.

## Problema (verificado contra el código)

- `crates/clawg-cli/src/daemon/api.rs` tiene 3.196 líneas de producción, 117 funciones y 68 usos de `bad_request`.
- `save_memory` (`api.rs:1331-1456`, 126 líneas) decide reglas de dominio:
  - `user_stated` no puede ir con `derived_from` (`:1351-1355`).
  - Las fuentes de una conclusión tienen que existir y estar vigentes (`:1356-1377`).
  - Con el embedder en falla no se acepta el guardado (`:1380-1386`).
- El tipo se valida y se tira: `raw.parse::<MemoryType>()` (`:1338-1340`) descarta el resultado, y después viaja el `String` original (`:1404`).
- Otros handlers largos con la misma forma: `context` (87 líneas), `register_root` (71, `:2498`), `locate_project` (59).
- `api.rs:2920-2928` ejecuta SQL directamente. Lo saca store-boundary, y lo que esa consulta decide también debería bajar a core.
- `MemoryRuntime::accept` (`crates/clawg-core/src/runtime.rs:245`) ya es la entrada natural del guardado en el núcleo.

## Por qué importa aunque haya un solo cliente HTTP

El puente MCP va por HTTP, así que hoy no hay dos caminos que puedan divergir en producción. Lo que sí pasa:

- `clawg-eval` entra por `clawg_core::eval_api` (`crates/clawg-core/src/eval_api.rs`, 1.967 líneas de producción) y no pasa por estas reglas.
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
