# project-indexing — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | decisión de producto (después, uno o más cambios) | eval-golden-expansion | exploración (borrador) |

**Tesis:** el objetivo es que "la IA entienda cómo funciona cada proyecto". La respuesta obvia es "chunking AST de código fuente", pero es **una** respuesta posible, no la pregunta. Antes de una spec hay que decidir qué se indexa.

## Estado actual (verificado)

### Nada produce memorias de código

- `ChunkerKind::Code` está marcado "never returned" (`crates/clawg-core/src/memory/chunker/mod.rs:44-55`).
- `MemorySource::CodeFile` está "reserved" (`crates/clawg-core/src/memory/events.rs:44`).
- `code_block` es sólo una etiqueta de `memory_type`. Un `save_memory` con ese tipo entra con `Role::User` (`crates/clawg-core/src/runtime.rs:262`) y lo trocea el chunker de chat, como cualquier otro save.

### Ya existe un chunker AST, muerto, y no es cAST

- `crates/clawg-core/src/memory/chunker/code.rs` usa tree-sitter para rs, ts y py.
- Se construye en `crates/clawg-core/src/runtime.rs:219`, pero nunca se alcanza.
- Aunque se alcanzara, el worker le pasa `language = None` (`crates/clawg-core/src/memory/worker.rs:274`), y caería al splitter plano.

Y aunque se conectara, no implementa cAST (dividir el árbol de arriba abajo y fusionar los nodos chicos vecinos hasta llenar el tope):

- **Recorre sólo los hijos directos de la raíz** (`code.rs:76`). No baja a los métodos ni fusiona nodos chicos: cada `use` y cada comentario sale como un chunk propio, y un doc comment (`///`) queda separado de la función que documenta.
- **Un nodo más grande que el tope cae al splitter de bytes**, sin solapamiento (`code.rs:93`). En Rust eso es casi cualquier `impl`: corta métodos por la mitad, que es justo la falla que el chunking AST viene a resolver.
- **La cabecera no sirve y además se pierde.** Es `"<kind> (<lang>)"`, sin ruta, nombre, firma ni ámbito (`code.rs:86-87`). Se escribe en `code.rs:94` y nadie la lee: el worker embebe sólo `chunk.text` (`worker.rs:321`).
- **Mide en tokens o bytes.** cAST mide en caracteres no blancos, para que el tope no dependa del formateo.

Lo reusable son los tres parsers y el fallback. El algoritmo de troceo se reescribe.

### No hay ingesta de repositorios

- Ningún comando ni watcher lee archivos de un repo.

### Límites del embedder

- e5-small trunca a 512 tokens (`crates/clawg-core/src/memory/embedder.rs:116-123, 243`). Muchas funciones completas no entran.
- e5-small no es un modelo de embeddings de código, y su calidad sobre código no está medida.

### Lo que dice el paper cAST

- **Hay cifras que circulan y no salen del paper** ([arXiv 2506.15655](https://arxiv.org/abs/2506.15655), Findings of EMNLP 2025): similitud 0,25 → 0,57, Hit@5 42,1 % → 89,4 %, "340 caracteres", "2,28×". Ninguna aparece en él.
- **El paper reporta mejoras de un dígito:**
  - +4,3 de Recall@5 en RepoEval;
  - +2,67 de Pass@1 en SWE-bench.
  - Además, mide los chunks en caracteres sin espacios, hasta 2000.
- **Y no mejora siempre.** Con retrievers de texto general (BGE-base, BGE-large) empata o pierde: nDCG@5 71,1 contra 71,3 en RepoEval, y Pass@8 30,3 contra 33,7 en SWE-bench con Gemini. Ninguno de los seis retrievers que prueba es multilingüe como e5-small.

## Qué se sabe de cada enfoque

- **Índice de código servido al agente.** Es la mejor evidencia a favor de indexar código. CodeNib ([arXiv 2607.25431](https://arxiv.org/abs/2607.25431)) combina un índice léxico, uno denso y un grafo, y lo compara contra un agente que sólo usa `grep` y `read`:
  - en SWE-bench Verified, con Claude Haiku 4.5 y un tope de 1 USD por issue, resuelve 79 de 200 contra 66 (McNemar p = 0,029);
  - con el mismo recall, gasta entre 50 % y 87 % menos tokens de trayectoria.

  Pero usa embedders de código de 0,6B a 7B parámetros (Qwen3-Embedding, Jina-Code, SweRank) más un reranker, y precarga el contexto en el prompt. Un servidor MCP no controla el prompt: lo más cercano es `recall_context`.
- **Cabecera de contexto en cada chunk.** Antes de embeber, se antepone al código la ruta, el ámbito que lo contiene (clase, `impl`), la firma y los imports. Así el vector "sabe" que `getUser` es un método de `UserService`. Las mediciones publicadas son de vendors y no aíslan la cabecera:
  - AST con cabecera: 70,1 % de R@5 contra 42,4 % de ventana fija, en un benchmark propio;
  - 70 % contra 59 % de respuestas correctas en 46 preguntas, con un juez LLM. Los autores admiten que no saben si la ganancia viene de que los chunks son más grandes.
- **Docs troceados por encabezado.** Es la misma idea aplicada a Markdown: se corta en cada encabezado y se antepone el breadcrumb (`[Guía > Instalación > macOS]`), para que un sub-chunk no quede huérfano de su sección. No necesita LLM, y es texto natural en castellano, justo donde e5-small encaja.
- **Contexto generado por LLM.** Una frase por chunk que lo sitúa en el documento ("contextual retrieval") reduce entre 35 % y 49 % los fallos de recuperación. Necesita una llamada a un LLM por chunk y depende de la misma decisión de proveedor que knowledge-synthesis.
- **Mapa del repo precargado.** En la misma comparación de CodeNib, precargar el repo map de Aider resolvió 56 issues: **menos** que no tener índice (66).
- **Invalidación.** Reusar los vectores de los archivos cuyo hash de contenido no cambió da el mismo resultado que reconstruir en el 90 % de los casos, y es entre 25× y 40× más rápido.

## La pregunta de producto

El agente ya tiene `grep` y `read` sobre el repo. Indexar código crudo duplica lo que hace con ripgrep, y agrega el problema de mantener el índice al día cuando el código cambia. ¿Qué le falta que una memoria sí le puede dar?

| Opción | Qué se indexa | A favor | En contra |
|---|---|---|---|
| a. Código crudo con chunking AST (cAST) y cabecera de contexto | funciones y clases | Es lo único con mejora medida en agentes. Reusa los parsers de `code.rs` | Duplica a ripgrep. La mejora medida usa embedders de código, no e5-small. El troceo de `code.rs` se reescribe |
| b. Conocimiento del proyecto, troceado por encabezado | README, `docs/`, ADRs, `CLAUDE.md`, manifiestos | Barato, alto valor, cambia poco. Sin LLM. e5-small encaja | No explica el código que no está documentado |
| c. Mapa generado por módulo | un resumen de "qué hace y cómo se conecta" cada módulo | Responde justo al "cómo funciona" | Necesita un LLM (el agente o un proveedor) y refresco cuando cambia el código |
| d. Mapa de símbolos | archivo → tipos → firmas, salido de tree-sitter | Sin LLM, y tree-sitter ya es dependencia | Es estructura, no comprensión. Precargado empeora el resultado. Bajo demanda, compite con el `grep` y el LSP que el agente ya tiene |

**Recomendación:** b primero. a, sólo si el subconjunto "cómo funciona X" del golden set muestra que b + `grep` no alcanza, y midiendo e5-small sobre código antes de construir nada. d se descarta.

## Descartado

Para no retomarlo por error:

| Idea | Por qué no |
|---|---|
| Expandir los resultados por el grafo de llamadas (callers y callees, con SCIP o Stack Graphs) | No tiene ganancia medida: en CodeNib, File Success@10 va de −4,8 a +7,1 y todos los intervalos de confianza incluyen cero. Además, necesita un indexador externo por lenguaje, y `github/stack-graphs` está archivado |
| Reanálisis incremental de tree-sitter | Sirve a un editor que conserva el árbol entre ediciones. Un índice en lote se invalida por hash de archivo |
| Mapa de símbolos o repo map precargado (opción d) | Precargado, resolvió menos issues que no tener índice |
| Reparar el grafo de símbolos por diff en lugar de reconstruirlo | Falla en Rust y en TS/JS: 0 de 9 transiciones coinciden con la reconstrucción |
| Indexar por el camino de `save_memory` | Ver la decisión sobre la tabla, abajo |

## Decisiones abiertas

- [ ] **¿Qué significa "entender cómo funciona"?** Conviene definirlo con 5–10 preguntas reales que hoy un agente no puede contestar bien.
- [ ] **¿Qué opciones se combinan?** La recomendación de arriba es b primero y a condicionada a la medición.
- [x] **¿Cómo se invalida el índice?** Por hash del contenido de cada archivo. Un commit no sirve como llave, porque el agente trabaja sobre cambios sin commitear.
- [x] **¿El contenido indexado convive con las memorias o va en otra tabla?** En otra tabla, derivada y reconstruible desde el disco. Si entrara por el camino de `save_memory`:
  - quedaría como `semantic_fact` y pasaría por el detector de contradicciones, que abriría disputas contra memorias reales;
  - el decaimiento por último acceso y los tiers no significan nada para un archivo;
  - el piso de coseno 0,8793 está calibrado sobre memorias, no sobre docs ni código.

  Además, por la regla de producto, borrar o reconstruir el índice no puede tocar una sola memoria.

## Cómo se mide

- **Un subconjunto del golden set** con las preguntas reales de "cómo funciona X".
- **Hit rate en `--agent-view`** antes y después.
- **Costo:** tokens y tiempo de indexado por repositorio.
- **Las métricas de memorias no se mueven:** el baseline de `rag-eval --real` tiene que quedar igual, porque el índice nuevo no puede desplazar memorias de los resultados.
