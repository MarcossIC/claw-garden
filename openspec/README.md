# openspec — cambios de Claw Garden

Cada cambio vive en su carpeta, `changes/<nombre>/`. No hay un documento único con todo.

Este archivo es solamente el **mapa**: qué cambios hay, en qué orden y de dónde salieron.

## Cómo se trabaja un cambio

```
exploration -> proposal -> spec (+ design si hace falta) -> tasks -> apply -> verify -> archive
```

- **`exploration.md`**: el punto de partida. Tiene el problema verificado contra el código (con `ruta:línea`), los enfoques posibles y las decisiones abiertas.
- **`proposal.md` / `spec.md` / `design.md` / `tasks.md`**: se escriben cuando le toca al cambio. El detalle es proporcional al cambio: un fix chico es una propuesta de una página más las tareas.
- **`state.yaml`**: en qué fase está el cambio.
- **Archivo**: cuando un cambio se cierra, pasa a `changes/archive/AAAA-MM-DD-<nombre>/`. Sus requisitos se consolidan en `specs/<dominio>/spec.md`, la fuente de verdad del comportamiento actual.
- **Versión**: al archivar un cambio se sube el minor de `[workspace.package] version` en el `Cargo.toml` raíz (los crates la heredan), se corre `cargo update --workspace` para llevarla a `Cargo.lock` y se commitean los dos juntos como `chore: bump version to X.Y.Z`. Un fix suelto, fuera de openspec, sube el patch.
- **Reglas del proyecto para cada fase**: están en `config.yaml`.

## Mapa de cambios (orden recomendado)

El número identifica al cambio y no se reasigna. La tabla está por número, no por prioridad: el orden en que conviene ejecutar lo pendiente está en [ROADMAP.md](ROADMAP.md).

La tabla tiene sólo lo que todavía no está en `changes/archive/`. Las filas de los cambios archivados están en [changes/archive/README.md](changes/archive/README.md), con el mismo formato y los mismos números: al archivar un cambio, su fila pasa ahí.

| # | Cambio | Objetivo | Tamaño | Depende de | Estado |
|---|--------|----------|--------|------------|--------|
| 17 | [model-reembed](changes/model-reembed/exploration.md) | 1 | S (sólo fijar el contrato) – L (cambio de dimensión) | — (su modelo destino, si lo hay, sale de 97) | exploración; urgencia baja: no hay ningún cambio de modelo planeado. Fija el contrato de producción y arregla el re-embebido de arranque, que hoy es todo o nada |
| 37 | [knn-prefilter](changes/knn-prefilter/exploration.md) | 2, 4 | M | — (33, archivado, dejó fuera el corpus multiagente: lo construye este cambio) | puerta medida ([verify](changes/knn-prefilter/verify.md)): abre por el pool de 30; el top 10 y el detector de contradicciones quedan bajo sus umbrales. Los filtros de agente, proyecto y estado corren después del corte k de vec0. El paso siguiente pide la propuesta de la variante A. Es la parte D de 16 |
| 57 | [read-connections](changes/read-connections/exploration.md) | transversal | M | 37 y el volumen real de 44; se coordina con 55 y 53 | exploración; el daemon tiene una sola conexión de SQLite y una operación larga frena guardados, búsquedas y hooks. Empieza por una puerta de medición; si la espera aparece, un lector de sólo lectura |
| 68 | [topic-key-warning](changes/topic-key-warning/proposal.md) | 1 | S | 48 | en verificación; `save_memory` avisa cuando un save sin clave parece la revisión de una fila de su mismo agente y proyecto. Provisional: no se archiva hasta medirlo en casos reales de un clawg con la marca ([tareas](changes/topic-key-warning/tasks.md)) |
| 69 | keyed-revision-rank (sin carpeta; [origen](changes/archive/2026-10-21-revision-eval/verify.md#history-import-44-mapea-a-topic_key-las-claves-de-tema-m1-m2-no-se-cumple)) | 1 | S–M | 48 (antes de 44) | cerrado sin cambio del producto por 70, el 2026-10-22: lo que separa a `keyed` del control es la historia del store, no las versiones viejas. Detalle en [revision-recheck](changes/archive/2026-10-22-revision-recheck/tasks.md) |
| 91 | [tokenizer-footprint](changes/tokenizer-footprint/exploration.md) | transversal | M–L | 92 (archivado); se coordina con 62 y 17 | exploración (borrador); con el modelo suelto, el piso del daemon es casi todo el tokenizer. Un paso 0 sin sesión de ONNX mide de qué está hecho |
| 97 | [embedder-bakeoff](changes/embedder-bakeoff/exploration.md) | medición (1, 3) | M, por puertas | — para memorias; el golden y los resultados de 20 y la memo de 94, archivados, para docs; alimenta a 17 | exploración; decide midiendo si conviene dejar `multilingual-e5-small`, contra dos candidatos de granite de 384 dimensiones, con el umbral fijado antes |
| 98 | [memory-anchors](changes/memory-anchors/exploration.md) | 1 (y 3) | M–L, por pasos | — para los pasos 0 a 2; el índice de docs de 15 (archivado) sólo para la tabla de nombres; se coordina con 99, 96 y 105 | exploración; propone anclar cada save a los archivos y nombres que cita y marcar `gone=` al leerlo cuando uno ya no existe. El paso 0 mide sobre memorias reales si vale la pena |
| 99 | [session-lifecycle](changes/session-lifecycle/exploration.md) | 1 y 4 | M, por partes | el paso 1 de 98 para la parte A; 96 mide B, C y D | exploración; el traspaso entre sesiones se rompe con sesiones en paralelo en worktrees del mismo proyecto. Parte A: traspaso por checkout; B a D: aviso antes de compactar, `compact_summary`, `SubagentStart` |
| 107 | [domain-types](changes/domain-types/proposal.md) | transversal | L, por tandas | 106 (archivado) | tareas escritas para la tanda 1: el tipo y el estado de una memoria, de la base al cliente, con `Other(String)` para lo que este código no conoce y sin cambiar lo que se guarda. Las tandas 2 a 5 (ids, `ProjectId`, structs de parámetros, el predicado de alcance), cada una en un cambio propio |
| 108 | [storage-read-side](changes/storage-read-side/exploration.md) | transversal | L, por dominio | 104, 105, 107; se coordina con 57 | exploración; un `ReadTx` que no puede escribir y consultas por caso de uso, todavía sin traits |
| 109 | [storage-write-side](changes/storage-write-side/exploration.md) | transversal | L–XL, por entregas | 104, 105, 107; conviene después de 103 | exploración; agregados que son únicos escritores de sus tablas y una unidad de trabajo para lo transversal |
| 110 | [search-index-ports](changes/search-index-ports/exploration.md) | medición (1, 3) | M–L, con puerta | 37, 108, 109 | exploración; dos puertos angostos, `VectorIndex` y `LexicalIndex`, para comparar motores con `rag-eval --diff`. Sin un candidato concreto, cierra sin código |
| 111 | [storage-ports](changes/storage-ports/exploration.md) | transversal | L, más el adaptador; con puerta | 108, 109 | exploración; traits por capacidad extraídos de 108 y 109 y una batería de contrato. Su puerta pide nombrar qué base se prueba; sin eso, cierra sin código |
| 120 | [claude-code-plugin](changes/claude-code-plugin/exploration.md) | transversal | M | 96 para medir el bloque del protocolo; sigue a setup-local-hooks (archivado) | exploración (borrador); distribuir servidor y hooks como plugin de Claude Code. Recomienda plugin con `setup` sólo para el bloque; Cursor es el punto sin verificar |
| 122 | [session-tokens](changes/session-tokens/exploration.md) | medición | M–L | 121; contrasta con 96 | exploración (borrador); clawg no ve tokens. Si se hace, un receptor OTLP `http/json` en el daemon que guarde sólo sesión, modelo, tokens y costo |
| 123 | [project-groups](changes/project-groups/exploration.md) | 2 | M–L, por partes | —; se coordina con 37 y 121 | exploración (borrador); una memoria es de un proyecto o de todos, sin nada en el medio. Propone un árbol de proyectos (`project.parent_id`) para que una búsqueda lea también lo de sus ancestros |
| 125 | [project-concept-map](changes/project-concept-map/exploration.md) | 3 | L, por pasos y con puerta | —; se coordina con 98 y alimenta a 126 | exploración (borrador); prioridad baja. Pregunta si vale registrar un mapa del proyecto; empieza por una puerta sin código |
| 126 | [project-architecture-map](changes/project-architecture-map/exploration.md) | 3 | sin estimar | 125 | exploración (borrador); pregunta de viabilidad, prioridad baja. Se retoma cuando 125 tenga su puerta medida |
| 131 | [repeated-errors](changes/repeated-errors/proposal.md) | 1 y 5 | M | — para el producto; 129 para la campaña | tareas escritas. Hooks de Bash avisan de los programas que no existen en la máquina y `session-start` los nombra; puerta de latencia p95 ≤ 60 ms |
| 133 | [contract-diet](changes/contract-diet/proposal.md) | transversal | S (el ruido del esquema) – M (el bloque `lean`) | 132 (archivado); 134 para sacar las secciones del resumen | en verificación ([verify](changes/contract-diet/verify.md)); el ruido del esquema, hecho (−1.856 caracteres en las 13 herramientas). La variante `lean` del bloque (la apertura intacta y cada regla, el 66 % de `full`), `--protocol lean` y el brazo `a3-lean`, aplicados; el valor por defecto sigue en `full` hasta la campaña paga contra `a3` |
| 134 | [memory-links](changes/memory-links/exploration.md) | 1 y 5 | S (las dos puertas) – sin estimar | — | exploración (borrador). Dos preguntas con puerta medida antes de cualquier código: qué aporta un `session_summary` que no aporten las memorias sueltas, y si los enlaces entre memorias traen algo que la búsqueda no trae |

Objetivos del producto:

1. Guardar y clasificar memorias de forma eficiente.
2. Separarlas por proyecto y también usarlas de forma global.
3. Indexar proyectos para que la IA entienda cómo funciona cada uno.
4. Ser compartido entre agentes, con control del usuario sobre qué ve cada uno.
5. Extraer conclusiones con el tiempo.

## Orden de ejecución de lo pendiente

La idea que lo ordena: primero poder medir, después que lo que se recupera y se guarda sea bueno, y recién entonces que llegue solo al agente. Si un contexto malo se inyecta en cada sesión, es peor que no inyectar nada. Y un cambio de ranking por vez: si dos se miden juntos, no se sabe cuál movió el número. Lo que entra en volumen (una importación) va después de lo que lo vuelve sano: el enmascarado, los conflictos, las revisiones y las piezas largas.

El import del historial en prod se hizo el 2026-10-24 con el release 0.62.0, que trajo 83, 84 y 88. Los ordenó una regla: lo que cambia qué se escribe va antes del import, y lo que cambia cómo se lee puede ir después, porque un re-import encuentra presente cada nota y no la vuelve a escribir ni a partir. La excepción fue 88, que no cambia lo que se escribe pero va antes porque mide: el import en prod es el primero con un binario release y no se repite, y su log deja el reparto de cada fase en release.

El orden concreto, con qué va en local y qué en la nube, está en [ROADMAP.md](ROADMAP.md), el único lugar que ordena lo pendiente.

## Más documentos

- [ideas.md](ideas.md): ideas y afirmaciones evaluadas contra el código, con las descartadas y su razón, y el porqué histórico del orden de los primeros cambios.
- [backlog.md](backlog.md): arreglos sueltos que no tienen spec.
- [changes/archive/README.md](changes/archive/README.md): las filas de los cambios ya archivados.
