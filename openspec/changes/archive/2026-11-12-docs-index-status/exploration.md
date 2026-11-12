# docs-index-status — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (y transversal: ver lo que el daemon hace en segundo plano) | S | —; se coordina con [read-connections](../read-connections/exploration.md) (57), que mide cuánto frena un refresco a lo demás | exploración (borrador); prioridad baja |

**Tesis.** El índice de un proyecto se arma y se pone al día en segundo plano, dentro del daemon y fuera del agente. Mientras corre, nada dice de qué proyecto es, cuánto lleva ni cuánto falta, y cuando termina sólo queda una línea en el log. La única señal en vivo es un booleano dentro de la respuesta de una búsqueda. Un trabajo que corre fuera del agente tiene que poder mirarse sin abrir el log ni adivinar.

## El problema verificado contra el código

1. **El estado que el daemon guarda por proyecto es mínimo y vive en memoria.** `Progress` tiene `running`, `last_started` (un `Instant`) y `last_error` (`crates/clawg-core/src/memory/project_docs/mod.rs:211-218`). No guarda cuándo terminó el último refresco, qué hizo ni cuánto lleva el que corre. Se pierde al reiniciar.
2. **Lo único que lee `running` desde afuera es la búsqueda.** `DocsSearchBody.indexing` (`crates/clawg-cli/src/daemon/api.rs:3326`, llenado en `:3390`) llega al agente como una línea sin números: «todavía se está indexando» (`crates/clawg-cli/src/mcp/tools.rs:732-738`).
3. **El log escribe sólo al terminar.** `refresh_docs` registra `indexed the docs of …` y los tiempos por fase después de que `guard.run` vuelve (`api.rs:3251-3267`). No hay línea al empezar, así que un refresco colgado no deja rastro.
4. **Las métricas también se registran al terminar:** `docs.refresh.duration` y `docs.refresh.files` (`api.rs:3271-3280`).
5. **`/health` no nombra al índice.** `HealthBody` lleva `ingest` e `import` (`api.rs:582-612`), y `clawg status` imprime lo que dice `/health` y nada más (`crates/clawg-cli/src/status.rs:24-34`). El campo `import` es el antecedente: un trabajo en curso leído de memoria (`JobView`, `crates/clawg-cli/src/daemon/imports.rs:46`).
6. **Un refresco empieza desde dos lugares y ninguno queda anotado:** un registro que lo pide (`api.rs:2745-2746`, `spawn_doc_refresh` en `:3201`) y una búsqueda cuando pasaron 60 s (`begin_if_due`, `api.rs:3372`; `REFRESH_INTERVAL`, `mod.rs:34`).
7. **El refresco ya sabe cuánto tiene por delante.** Después de recorrer el árbol arma el plan (`plan_refresh`, `mod.rs:504`) y recorre `plan.index` archivo por archivo (`mod.rs:518`): la cantidad planeada y la hecha existen, pero no salen de la función.

**Caso observado.** Un refresco de un repositorio de 901 archivos, con 36 cambiados, tardó 42,4 s, 38,0 de ellos embebiendo. Tres búsquedas seguidas dentro de esa ventana contestaron «todavía se está indexando» y no había cómo saber si faltaban segundos o minutos. La respuesta estaba en el log, y recién cuando terminó.

## Caminos

### A. El estado del índice en `/health` (Recomendado; S)

`Progress` suma la hora de comienzo, la de fin, el reporte del último refresco y, mientras corre uno, cuántos archivos del plan lleva. `HealthBody` lo expone por proyecto, leído de memoria como `import`, así no espera detrás de un escritor ocupado. `clawg status` lo imprime sin cambios propios, y `memory_status` puede sumar una línea.

Lo que se pierde al reiniciar es aceptable: la fecha del último indexado de cada archivo ya está en `project_doc_file.indexed_at` (`crates/clawg-core/migrations/V12__project_docs.sql:16`), así que «cuándo se indexó por última vez» se puede leer del store sin esquema nuevo.

### B. Una línea de log al empezar (Incorporar; XS)

Después del plan: de qué proyecto, cuántos archivos va a indexar y quién lo disparó (registro o búsqueda). Vuelve visible un refresco que no termina.

### C. El índice en `clawg projects list` (Opcional; S)

Por carpeta: si se indexa, cuántos archivos tiene el índice, cuándo se puso al día y si hay un refresco corriendo. Es donde una persona mira sus proyectos, pero ese listado lee el store y el estado en curso vive en memoria: junta dos fuentes.

### D. Números en el aviso de `search_project` (Opcional; XS)

«36 de 901 archivos por indexar» en lugar de la línea sin números. Cambia lo que lee el modelo, y un test fija esa línea (`search_project_warns_while_indexing`, `tools.rs:2160`).

### E. Una tabla con la historia de los refrescos (Descartado)

`metrics.db` ya guarda duración y archivos por refresco. Una tabla nueva repetiría eso con un esquema que mantener.

## Cómo se mide

Con tests que hoy fallan, sin métrica de `rag-eval`:

- con un refresco detenido a mitad, `/health` nombra el proyecto, dice que corre y cuántos archivos lleva de cuántos;
- al terminar, dice cuándo terminó y qué hizo, y un fallo deja su causa;
- el log tiene una línea al empezar;
- `clawg status` muestra lo mismo.

## Fuera de alcance

- Cancelar o pausar un refresco.
- Un watcher del sistema de archivos, que project-indexing ya descartó.
- El progreso de una importación, que ya tiene su `JobView`.

## Decisiones abiertas

- **Dónde se ve:** sólo `clawg status`, también `clawg projects list`, o los dos.
- **Si el agente ve el avance** (D) o sólo la persona.
- **Qué se cuenta:** archivos o pasajes. El tiempo se va en embeber pasajes, pero el plan se conoce en archivos.

## Esfuerzo e impacto

| Camino | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Recomendado | Medio: se ve qué indexa el daemon y cuánto falta | Bajo | Bajo |
| B | Incorporar | Bajo, pero es lo único que muestra un refresco colgado | Bajo | Mínimo |
| C | Opcional | Medio | Bajo | Bajo |
| D | Opcional | Bajo | Bajo (cambia un texto que lee el modelo) | Mínimo |
| E | Descartado | — | — | — |

Va en la nube: se prueba en Linux con el embedder falso.

## Fuentes

- Código de `develop` en `f99e0211`, con las rutas citadas.
