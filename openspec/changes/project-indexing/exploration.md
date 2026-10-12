# project-indexing — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 | decisión de producto (después, uno o más cambios) | eval-golden-expansion | exploración (borrador) |

**Tesis:** el objetivo es que "la IA entienda cómo funciona cada proyecto". El análisis lo traduce directo a "chunking AST de código fuente", pero eso es **una** respuesta posible, no la pregunta. Antes de una spec hay que decidir qué se indexa.

## Estado actual (verificado)

### Nada produce memorias de código

- `ChunkerKind::Code` está marcado "never returned" (`crates/clawg-core/src/memory/chunker/mod.rs:44-55`).
- `MemorySource::CodeFile` está "reserved" (`crates/clawg-core/src/memory/events.rs:44`).
- `code_block` es sólo una etiqueta de `memory_type`.

### Ya existe un chunker AST, muerto

- `crates/clawg-core/src/memory/chunker/code.rs` usa tree-sitter para rs, ts y py. Arma una cabecera "kind (lang)" y sub-divide a 480 tokens sin solapamiento.
- Se construye en `crates/clawg-core/src/runtime.rs:219`, pero nunca se alcanza.
- Aunque se alcanzara, el worker le pasa `language = None` (`crates/clawg-core/src/memory/worker.rs:274`), y caería al splitter plano.

### No hay ingesta de repositorios

- Ningún comando ni watcher lee archivos de un repo.

### Límites del embedder

- e5-small trunca a 512 tokens (`crates/clawg-core/src/memory/embedder.rs:116-123, 243`). Muchas funciones completas no entran.
- e5-small no es un modelo de embeddings de código, y su calidad sobre código no está medida.

### Lo que el análisis tenía mal

- **Las cifras no salen del paper cAST.** Similitud 0,25 → 0,57, Hit@5 42,1 % → 89,4 %, "340 caracteres", "2,28×": ninguna aparece en él ([arXiv 2506.15655](https://arxiv.org/abs/2506.15655), Findings of EMNLP 2025).
- **El paper reporta mejoras de un dígito:**
  - +4,3 de Recall@5 en RepoEval;
  - +2,67 de Pass@1 en SWE-bench.
  - Además, mide los chunks en caracteres sin espacios, hasta 2000.

## La pregunta de producto

El agente ya tiene `grep` y `read` sobre el repo. Indexar código crudo duplica lo que hace con ripgrep, y agrega el problema de mantener el índice al día cuando el código cambia. ¿Qué le falta que una memoria sí le puede dar?

| Opción | Qué se indexa | A favor | En contra |
|---|---|---|---|
| a. Código crudo con chunking AST (cAST) | funciones y clases | Reusa `code.rs` | Duplica a ripgrep. Hay que invalidar por hash o commit. e5-small no es de código |
| b. Conocimiento del proyecto | README, `docs/`, ADRs, `CLAUDE.md`, manifiestos | Barato, alto valor, cambia poco | No explica el código que no está documentado |
| c. Mapa generado por módulo | un resumen de "qué hace y cómo se conecta" cada módulo | Responde justo al "cómo funciona" | Necesita un LLM (el agente o un proveedor) y refresco cuando cambia el código |
| d. Mapa de símbolos | archivo → tipos → firmas, salido de tree-sitter | Sin LLM, y tree-sitter ya es dependencia | Es estructura, no comprensión |

## Decisiones abiertas

- [ ] **¿Qué significa "entender cómo funciona"?** Conviene definirlo con 5–10 preguntas reales que hoy un agente no puede contestar bien.
- [ ] **¿Qué opciones se combinan?** Por ejemplo, b + d primero y c después.
- [ ] **¿Cómo se invalida el índice?** Por hash de archivo, por commit, o al pedido.
- [ ] **¿El código indexado convive con las memorias o va en otra tabla?** Afecta el ranking y la visibilidad.

## Cómo se mide

- **Un subconjunto del golden set** con las preguntas reales de "cómo funciona X".
- **Hit rate en `--agent-view`** antes y después.
- **Costo:** tokens y tiempo de indexado por repositorio.
