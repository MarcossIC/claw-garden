# docs-index-status — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (y transversal: ver lo que el daemon hace en segundo plano) | S | —; se coordina con [read-connections](../../read-connections/exploration.md) (57) | propuesta; las tres decisiones abiertas de la exploración se cerraron con su opción recomendada |

Las citas son `ruta:línea` de `develop` en `0716645`.

## Problema

La [exploración](exploration.md) muestra que un refresco del índice de un proyecto corre dentro del daemon sin dejar nada que mirar hasta que termina:

- el estado por proyecto es `running`, `last_started` y `last_error` (`Progress`, `crates/clawg-core/src/memory/project_docs/mod.rs:219`), y lo único que lo lee desde afuera es el booleano `indexing` de una búsqueda (`crates/clawg-cli/src/daemon/api.rs:3318`);
- el log escribe sus dos líneas después de que el refresco vuelve (`refresh_docs`, `api.rs:3243`), así que uno que no termina no deja rastro;
- `/health` lleva `ingest` e `import` y no nombra al índice (`HealthBody`, `api.rs:582`);
- el refresco conoce su plan después de recorrer el árbol (`plan_refresh`, `mod.rs:513`) y lo recorre archivo por archivo (`mod.rs:527`), pero ni lo planeado ni lo hecho salen de la función.

En el caso observado, 36 archivos cambiados de 901 tardaron 42,4 s y tres búsquedas seguidas contestaron «todavía se está indexando», sin forma de saber cuánto faltaba.

## Decisiones tomadas

Las que la exploración dejó abiertas, con su opción recomendada:

- **Dónde se ve:** en `/health` y, por eso, en `clawg status`. `clawg projects list` (camino C) queda fuera.
- **El agente no ve el avance.** El aviso de `search_project` (camino D) no cambia, y el contrato fijado del agente tampoco.
- **Se cuentan archivos**, que es lo que el plan conoce.

Y dos más:

- **No hay esquema nuevo.** El estado del refresco que corre y el del último que terminó viven en la memoria del daemon. Lo durable ya existe: cuándo se indexó cada archivo está en `project_doc_file.indexed_at` (`crates/clawg-core/migrations/V12__project_docs.sql:16`).
- **`/health` lee de memoria**, como hace con `import` (`ImportJobs::active`, `crates/clawg-cli/src/daemon/imports.rs:387`): no espera detrás de un escritor ocupado.

## Alcance

**A. El estado del índice en `/health`.**

- En `crates/clawg-core`, `DocIndex` guarda por proyecto el refresco que corre (quién lo disparó, cuándo empezó, cuántos archivos del plan lleva y cuántos son) y el último que terminó (cuándo empezó y terminó, y qué hizo o por qué falló). El refresco avisa su plan y cada archivo que termina.
- En `crates/clawg-cli`, `GET /health` suma `docs_index`: una entrada por cada proyecto con un refresco en esta ejecución del daemon.
- `clawg status` lo muestra: el `Health` del cliente (`crates/clawg-cli/src/client.rs:146`) suma el campo, que un daemon anterior no manda.

**B. Una línea de log al empezar**, después del plan: de qué proyecto, quién lo disparó (un registro o una búsqueda) y cuántos archivos va a indexar, cuántos deja y cuántos saca.

También `docs/reference/daemon.md` y `docs/reference/commands.md`, donde describen `/health`, el log del refresco y `clawg status`.

## Fuera de alcance

- Lo que la exploración deja fuera: cancelar o pausar un refresco, un watcher del sistema de archivos y el progreso de una importación.
- Los caminos C (`clawg projects list`), D (números en el aviso de `search_project`) y E (una tabla con la historia).
- `memory_status`: la exploración dice que puede sumar una línea, pero es una herramienta del agente, y la decisión es que el agente no ve el avance.
- Leer el store para `/health`. Después de reiniciar, `docs_index` nombra un proyecto recién cuando empieza su primer refresco en esa ejecución, que es lo que hace el registro del puente al abrir una sesión. Mostrar lo ya indexado desde `project_doc_file` es el camino C.
- Subir la versión y archivar.

## Datos guardados

Ninguno cambia: ni el esquema, ni los identificadores, ni lo que el índice guarda. No hay migración ni vuelta atrás que preparar. `/health` gana un campo y no pierde ninguno: un cliente anterior lo ignora, y el cliente nuevo lee un daemon anterior como «sin proyectos».

## Cómo se mide el éxito

Tests que fallan hoy, sin métrica de `rag-eval` (no cambian el ranking ni lo que se indexa):

- con un refresco detenido a mitad, `/health` nombra el proyecto, dice que corre y cuántos archivos lleva de cuántos;
- eso mismo con la única conexión del store ocupada: `/health` contesta sin esperarla;
- al terminar, dice cuándo terminó y qué hizo, y un fallo deja su causa;
- el log tiene una línea al empezar;
- `clawg status` muestra lo mismo.

El contrato fijado del agente (`the_agent_contract_matches_its_file` y su par en español) sigue pasando sin regenerar sus archivos.

## Riesgos

- **Una línea más de log por refresco.** Una búsqueda dispara uno cada 60 s como mucho (`REFRESH_INTERVAL`, `mod.rs:34`), y cada uno ya escribe dos líneas al terminar.
- **`docs_index` crece con los proyectos que el daemon toca.** Es una entrada chica por proyecto y se vacía al reiniciar.
- **El estado se pierde al reiniciar.** Aceptado: es el estado de un trabajo en curso, y el log y `metrics.db` guardan la historia.
