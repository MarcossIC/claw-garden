# docs-refresh-background — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | S | — (antes de 20 docs-search-quality) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`b61aa4c`), donde las líneas se movieron:

1. **`search_project` puede colgarse.** Cuando el último refresco del proyecto empezó hace 60 s o más, `search_docs` hace `await` del refresco entero antes de buscar (`crates/clawg-cli/src/daemon/api.rs:1817-1822`), y el cliente no tiene plazo a propósito (`DaemonClient::search_docs`, `crates/clawg-cli/src/client.rs`; `only_health_sets_a_timeout`). Tras un checkout, un pull grande o un cambio de modelo, la herramienta dura lo que tarde en embeberse todo lo cambiado; si el anfitrión corta la llamada, el refresco se cancela a mitad.
2. **El error de un refresco de fondo se pierde.** `spawn_doc_refresh` sólo lo registra con `log::warn!` (`api.rs:1735`), y `Progress` no guarda nada más que `running` y `last_started` (`crates/clawg-core/src/memory/project_docs/mod.rs:129-133`).
3. **Un doc con un vector inservible frena a los demás.** `embed_passages(..).await?` (`mod.rs:388`) corta el bucle: los docs que siguen en el plan no se indexan y `remove_docs` (`mod.rs:394`) no corre, en cada refresco mientras ese doc no cambie.
4. **`specs/project-docs` se contradice**: «Búsqueda» pide refrescar antes de buscar como MUST y el invariante dice que la búsqueda no espera el refresco.

## Objetivo que sirve

3: indexar un proyecto para que el agente entienda cómo funciona, sin que la herramienta que lo consulta se cuelgue. Va antes de 20 (docs-search-quality), que mide la búsqueda de docs y no debe mezclar latencia de embebido con calidad.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Frescura contra latencia:** B. El refresco vencido corre en una tarea propia que no depende de la petición; la búsqueda que lo lanza lo espera a lo sumo `REFRESH_GRACE`, y si no terminó contesta con lo indexado e `indexing: true`.
- **El tope:** `REFRESH_GRACE` = 3 s, lo que tardó un doc en el único reindexado medido (645 s para 226 docs, unos 2,9 s por doc, en un binario debug). La espera del primer índice (20 s) no cambia.
- **Persistencia del error:** el error del último refresco del proyecto, también uno de fondo, se guarda y sale en `warning` en cada búsqueda hasta que un refresco termina bien.
- **Doc malo:** sólo un vector que `to_unit` rechaza (dimensión, valor no finito, norma cero) es de un doc. Una cuenta de vectores distinta de la de pasajes es del embedder y sigue abortando, como un fallo de `Embed` o de la tienda.
- **Dónde se informa:** un campo nuevo, `RefreshReport::failed`, con la ruta y la causa de cada doc salteado, y una línea `warn` del log. El `warning` del modelo no lo nombra: el índice se refrescó y el doc sigue con su versión anterior.
- **Cancelación:** no se toca `last_started`. Con la tarea propia, una petición que se va ya no cancela el refresco.

## Alcance

1. `project_docs`: `RefreshGuard::run`, que refresca con la marca tomada y deja el resultado en `Progress`; `DocIndex::begin_if_due` y `last_error`; `refresh` saltea un doc con `DocIndexError::Vector`, lo cuenta en `failed` y sigue; la cuenta distinta pasa a `DocIndexError::VectorCount`.
2. `daemon/api.rs`: `search_docs` lanza el refresco vencido en una tarea (la misma que usa el registro, extraída de `spawn_doc_refresh`) y lo espera `REFRESH_GRACE`, o `FIRST_INDEX_WAIT` con el índice vacío; `warning` sale de `last_error`.
3. `clawg-eval`: `EvalDocIndex::index` falla si un doc quedó en `failed`, como fallaba antes, para que una medición no pierda docs en silencio.
4. Docs: `specs/project-docs` (al archivar) y `.claude/CLAUDE.md` si nombra la conducta.

## Fuera de alcance

- Un watcher, una herramienta `index_project` o un plazo en el cliente.
- `REFRESH_INTERVAL`, `plan_refresh`, la prioridad de los docs dentro de un refresco.
- Paralelizar o acotar el CPU del embebido de docs (62, embedder-footprint).
- Recordar la huella de un doc que falló para no reintentarlo.
- Ranking: no se corre `rag-eval`; la búsqueda no cambia y `rag-eval --docs` llama a `refresh` directamente.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- con un embedder que no termina de embeber pasajes, una búsqueda que encuentra el proyecto vencido con docs cambiados vuelve dentro del plazo con `indexing: true` y lo ya indexado; al soltar el embedder, la búsqueda siguiente ve los docs nuevos. Hoy no vuelve;
- la petición que lanzó el refresco puede irse y el refresco termina igual;
- un registro cuyo refresco de fondo falla deja el error en el `warning` de la búsqueda siguiente; hoy no hay `warning`;
- un doc con un vector no finito no impide indexar los docs que le siguen ni quitar los borrados, y el reporte lo cuenta con su causa; una cuenta de vectores distinta sigue abortando.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding, y ninguna tabla de memorias. Las tablas del índice de docs se escriben igual que hoy, un doc por transacción. Volver atrás es volver al binario anterior.
