# openspec — cambios de Claw Garden

Cada cambio vive en su carpeta, `changes/<nombre>/`. No hay un documento único con todo.

Este archivo es solamente el **mapa**: qué cambios hay, en qué orden y de dónde salieron.

## Cómo se trabaja un cambio

```
exploration (en cola) -> plan -> apply -> verify -> archive
```

- **`exploration.md`**: cómo entra un cambio a la cola. Tiene el problema verificado contra el código (con `ruta:línea`), los enfoques posibles y las decisiones abiertas. Un cambio se queda en exploración hasta que [ROADMAP.md](ROADMAP.md) dice que le toca.
- **`plan.md`**: se escribe cuando el cambio se toma, y es su único documento de trabajo: el problema, qué significa «hecho», los pasos, el registro y la verificación. La spec del área se edita en el mismo cambio, sin una copia en su carpeta. Un cambio chico no lleva plan. Qué tamaño le corresponde a cada cambio y qué secciones lleva el plan está en `.claude/skills/change-workflow/SKILL.md`.
- **`proposal.md` / `spec.md` / `tasks.md`**: un cambio que ya los tiene se implementa desde ellos, sin reescribirlos como plan.
- **`state.yaml`**: en qué fase está el cambio.
- **Archivo**: cuando un cambio se cierra, pasa a `changes/archive/AAAA-MM-DD-<nombre>/`. `specs/<dominio>/spec.md` es la fuente de verdad del comportamiento actual.
- **Versión**: al archivar un cambio se sube el minor de `[workspace.package] version` en el `Cargo.toml` raíz (los crates la heredan), se corre `cargo update --workspace` para llevarla a `Cargo.lock` y se commitean los dos juntos como `chore: bump version to X.Y.Z`. Un fix suelto, fuera de openspec, sube el patch.
- **Reglas del proyecto**: están en `config.yaml`.

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
| 99 | [session-lifecycle](changes/session-lifecycle/exploration.md) | 1 y 4 | M, por partes | el paso 1 de 98 para la parte A; 96 mide B, C y D | exploración; el traspaso entre sesiones se rompe con sesiones en paralelo en worktrees del mismo proyecto. Parte A: traspaso por checkout; B a D: aviso antes de compactar, `compact_summary`, `SubagentStart` |
| 141 | [conflicts-on-demand](changes/conflicts-on-demand/proposal.md) | 1, y el criterio del piloto sobre LAST SESSION | M, por partes; no toca `memory.db` | —; se coordina con 99 y 134 (el traspaso entre sesiones) y sigue la regla de la apertura de 133 (archivado) | tareas (15 tareas sobre la spec CO1 a CO13; tope de 10 pares en la lista a pedido); ACTIVE CONFLICTS sale del contexto sin consulta, donde el presupuesto cortaba LAST SESSION antes que los pares (134 de 250 contextos sin resumen en el piloto). Cada fila en disputa que clawg muestra lleva `conflict=#N`, `recall_context` con consulta conserva el bloque cortándolo antes que el resumen, y `list_memories` con `conflicts` lista a pedido los pares que esperan a la persona |
| 142 | [summary-on-stop](changes/summary-on-stop/exploration.md) | 1, y el criterio de la etapa sobre los resúmenes | M; no toca `memory.db` | —; se coordina con 141 y 99 | exploración; un hook `Stop` que, si la sesión guardó algo sin `session_summary`, se lo pide al agente una sola vez y lo deja terminar. Sólo 27 de 122 sesiones con prompts guardaron uno |

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
