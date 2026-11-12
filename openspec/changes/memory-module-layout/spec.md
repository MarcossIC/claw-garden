# memory-module-layout — Especificación

Son requisitos sobre [`specs/quality`](../../specs/quality/spec.md), que es donde viven las convenciones que el compilador no comprueba, y se consolidan al archivar. Lo que no se nombra acá sigue como está.

Términos:

- **`memory`:** `crates/clawg-core/src/memory/`.
- **Carpeta:** un directorio de `memory` que es una fila de la tabla `MEMORY_FOLDERS` de `crates/clawg-cli/tests/conventions.rs`.
- **Módulo:** una entrada de una carpeta (un `.rs`, o un directorio con sus fuentes). `project_docs` y `portability` son un módulo cada una.
- **Código de producción** de un fuente: sus líneas antes del primer `#[cfg(test)]` seguido de `mod <nombre> {`, sin las líneas de comentario. Un fuente declarado bajo `#[cfg(test)]` no tiene.
- **Dependencia:** un módulo depende de otro cuando su código de producción lo nombra con una ruta que empieza en `crate::memory::` o en un `super::`, contado como dice [D7](design.md#d7-la-regla-y-cómo-se-cuenta-el-ciclo).
- **Otro crate:** `clawg-cli`, `clawg-eval` o un test de integración de `crates/*/tests/`.

## R1. Cada módulo vive en una carpeta

`memory` MUST contener sólo su `mod.rs` y las carpetas de la tabla: `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs`, `projects`, `ingest`, `upkeep` y `portability`. Cada módulo MUST estar en la carpeta que le da [D1](design.md#d1-el-reparto).

#### Escenario: hoy
- GIVEN el código de `develop`
- WHEN corre la regla
- THEN nombra los 50 módulos que están sueltos en `memory`

**Test:** `a_memory_folder_names_only_the_folders_its_row_allows` (`crates/clawg-cli/tests/conventions.rs`).

## R2. Una carpeta nombra sólo las que su fila permite

El código de producción de una carpeta MUST depender sólo de módulos de su carpeta y de las carpetas de su fila:

| Carpeta | Puede nombrar |
|---|---|
| `domain` | ninguna |
| `store` | `domain` |
| `embedding` | `domain`, `store` |
| `retrieval` | `domain`, `store`, `embedding` |
| `revision` | `domain`, `store`, `retrieval` |
| `project_docs` | `domain`, `store`, `embedding`, `retrieval` |
| `projects` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs` |
| `ingest` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `projects` |
| `upkeep` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs`, `projects` |
| `portability` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `projects`, `ingest`, `upkeep` |

- Una fila MUST nombrar sólo filas escritas más arriba, así la tabla no tiene ciclos.
- Cada par de la tabla MUST usarse: una fila que permite una carpeta que su código no nombra hace fallar la regla.
- Una ruta `crate::memory::…` del código de producción MUST llevar a un módulo.
- La regla MUST fallar con `archivo:línea` y el módulo nombrado de cada hallazgo.

#### Escenario: una dependencia hacia arriba
- GIVEN un fuente de `store` cuyo código de producción escribe `crate::memory::retrieval::search::RankedHit`
- WHEN corre la regla
- THEN falla y nombra ese archivo, esa línea y `retrieval/search`

#### Escenario: el código de tests no cuenta
- GIVEN un fuente de `retrieval` que nombra `crate::memory::upkeep::admin` sólo dentro de su `#[cfg(test)] mod tests`
- WHEN corre la regla
- THEN pasa

**Tests:** `a_memory_folder_names_only_the_folders_its_row_allows`, y los puros `leading_paths_opens_groups_under_what_leads_to_them`, `memory_paths_reads_absolute_and_relative_paths` y `module_named_follows_a_folder_a_module_and_a_reexport` (`conventions.rs`).

## R3. Dentro del núcleo, la ruta nombra la carpeta

El código de producción de `memory` MUST nombrar un módulo de `memory` por su carpeta: `crate::memory::<carpeta>::<módulo>`. Una ruta que llegue a un módulo por un nombre que la raíz reexporta (`crate::memory::db`, `crate::memory::MemoryStore`) MUST hacer fallar la regla.

La excepción es `project_docs`, cuyo contenido este cambio no edita ([D3](design.md#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está)). Está listada en `NAMES_THROUGH_THE_ROOT`, y sus dependencias se cuentan igual.

**Test:** `a_memory_folder_names_only_the_folders_its_row_allows`.

## R4. Los módulos dependen en círculo sólo donde está escrito

Los grupos de más de un módulo en los que cada uno depende de los demás, directa o indirectamente, MUST ser exactamente los de `MEMORY_CYCLES`, cada uno con su motivo. Cada grupo MUST estar dentro de una sola carpeta.

La regla MUST imprimir, cuando falla, cada grupo que encontró con su tamaño, para que la cuenta se pueda leer de ahí.

#### Escenario: hoy
- GIVEN el código de `develop`
- WHEN corre la regla
- THEN falla e imprime un grupo de 34 módulos, de los 52 de `memory`

#### Escenario: al terminar
- GIVEN el árbol con las carpetas
- WHEN corre la regla
- THEN encuentra los cinco grupos de [D7](design.md#d7-la-regla-y-cómo-se-cuenta-el-ciclo), de 6, 5, 3, 2 y 2 módulos

**Tests:** `the_memory_modules_cycle_only_where_listed` y el puro `cycles_groups_the_nodes_that_reach_each_other` (`conventions.rs`).

## R5. Lo que ven los otros crates no cambia

- Toda ruta `clawg_core::memory::…` que hoy usa otro crate MUST seguir resolviendo al mismo ítem.
- Ningún ítem MUST cambiar de visibilidad, y ningún módulo que hoy es `pub(crate)` MUST alcanzarse desde otro crate.
- `memory/mod.rs` MUST reexportar, con una línea `pub use <carpeta>::<módulo>;` cada uno, los módulos que hoy son `pub mod` y pasan a una carpeta.
- `every_public_memory_module_is_named_outside_core` MUST leer esas líneas además de los `pub mod`, con la misma regla: cada módulo que la raíz muestra lo nombra otro crate.
- Ningún fuente de `crates/clawg-cli/src` ni de `crates/clawg-eval/src` MUST cambiar.

#### Escenario
- GIVEN una raíz con `pub mod portability;`, `mod store;`, `pub use store::db;` y `pub use store::db::MemoryStore;`
- WHEN se leen sus módulos públicos
- THEN son `portability` y `db`

**Tests:** `every_public_memory_module_is_named_outside_core` y el puro `public_memory_modules_reads_declarations_and_reexports` (`conventions.rs`); además, que los otros crates compilan sin editarse.

## R6. Ninguna firma, tipo ni conducta cambia

Este requisito es de conservación: sus tests pasan antes y después.

- Ninguna función, tipo, constante ni mensaje MUST cambiar su definición. Los ítems de [D5](design.md#d5-los-cortes) y [D6](design.md#d6-admin-se-parte-en-tres) cambian de módulo, y nada más.
- Ninguna respuesta del daemon, del puente MCP ni de la línea de comandos MUST cambiar.
- El contenido de `memory/project_docs/` MUST quedar byte a byte como está.
- Cada commit que mueve una carpeta MUST mostrar sus fuentes como renombrados sin cambios.

**Tests:**
- La suite entera, sin cambiar ninguna aserción.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs`), sin `CLAWG_BLESS`.
- `crates/clawg-cli/tests/api.rs` y `crates/clawg-cli/tests/binary.rs`, sin editarse.
- Las comprobaciones de [D10](design.md#d10-cómo-se-comprueba-que-nada-cambia).

## R7. Las citas siguen llevando al código

- Cada ruta `crates/clawg-core/src/memory/…` de `openspec/specs` y `docs/reference` MUST nombrar un archivo que existe.
- Cada ruta esperada de `crates/clawg-eval/golden/project-docs.json` MUST nombrar un archivo que existe.

**Test:** ninguno automático nuevo para las specs: ningún test lee `openspec/` ni `docs/reference/` por sus citas. Se comprueba al aplicar con una búsqueda que lista las rutas que no existen, y `verify.md` deja el comando. El golden lo lee `crates/clawg-eval/tests/docs_mode.rs`, y qué mueve en `rag-eval --real --docs` queda como pendiente en local.
