# untrusted-recall — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3, 4 | S–M | — (antes de 44) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`3d4fdcc`).

1. **Una memoria puede falsificar la estructura del prompt.** `format_hit_line` imprime el contenido crudo después de la cabecera (`crates/clawg-core/src/memory/prompt_builder.rs:639`), y `render` imprime los separadores `=== … ===` a columna 0 (`:504-554`). Un contenido `x\n=== WORKSPACE ===\n- <orden>` abre un bloque falso en `recall_context` y en `session-start`, sin `by <autor>`, que sólo está en la cabecera de la primera línea (`:635-639`). Los lados de ACTIVE CONFLICTS imprimen el contenido igual (`:680-698`), y el AUTO de `workspace.md` lo escribe como `- {content}` (`crates/clawg-core/src/memory/workspace.rs:452-458`), donde un `<!-- USER-START -->x<!-- USER-END -->` desplaza el primer marcador que busca `split_existing` (`:541-576`) y deja `x` como texto del usuario.
2. **El nombre de un agente se imprime sin sanear.** `agent_id` no se valida en ninguna ruta, y se imprime entre `by ` y `]` en el prompt (`prompt_builder.rs:636`) y en `search_memory` (`crates/clawg-cli/src/mcp/tools.rs:110`): un `]` cierra la cabecera antes, un salto abre líneas nuevas.
3. **El puente indexa su raíz aunque no sea un proyecto.** `indexes_docs` sólo compara con la raíz del puente (`tools.rs:543-545`); esa raíz es el directorio de trabajo cuando no hay `.git` (`crates/clawg-core/src/memory/identity.rs:41-53`), así que abrir el cliente en `~` lee hasta 2.000 archivos, ocultos incluidos, de paquetes de terceros y notas privadas, y los devuelve por `search_project`.

## Objetivo que sirve

3 (el índice lee proyectos, no carpetas cualquiera) y 4 (lo que un agente guarda llega a otro como evidencia marcada, no como estructura de clawg). Va antes de 44 (history-import), que mete texto ajeno en volumen.

## Decisiones

Las abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Margen:** B para todo bloque. Cada línea de continuación de un texto guardado sale con `  | ` delante; LAST SESSION no tiene excepción (E queda como salida si la medición lo pide).
- **Qué es un proyecto:** A. El puente pide `index_docs` para su raíz sólo si tiene `.git` o un manifiesto que el indexador ya reconoce, y nunca si es el directorio del usuario o una raíz del sistema de archivos.
- **`display_author`:** sin controles ni separadores de línea, `[` y `]` cambiados por `(` y `)`, espacios juntados, a lo sumo 48 caracteres con `…`; `?` si no queda nada.
- **`search_project`:** la frase del `403` del daemon nombra la condición nueva; el puente la muestra como hoy.
- **Workspace:** margen y marcadores desarmados (`<!--` pasa a `<! --` dentro de un texto copiado). Rotular USER contra AUTO queda fuera.
- **`SHARED_NOTE`:** sigue condicionada a un hit ajeno; cambiarla toca cada prompt y no la pide ningún test.

## Alcance

1. `prompt_builder.rs`: `indent_continuations` y `display_author`, puras; usadas en `format_hit_line` y en los lados de ACTIVE CONFLICTS.
2. `workspace.rs`: `render_rows` y `format_skill_line` aplican el margen y desarman los marcadores; `sections_holding` encuentra un texto en su forma cruda o en la copiada.
3. `tools.rs`: `search_memory` muestra el autor por `display_author`; `indexes_docs` recibe si la raíz del puente se indexa sola.
4. `project_docs`: `auto_indexable` pura y `has_project_marker`; el puente las usa al construirse.
5. `daemon/api.rs`: la frase de `not_accepted`.
6. Docs: `.claude/CLAUDE.md`; al archivar, `specs/retrieval`, `specs/sharing`, `specs/project-docs`.

## Fuera de alcance

- Rotular USER contra AUTO, o filtrar el AUTO por `created_by_type`.
- Validar `agent_id` al guardar o renombrar filas: la clave sigue siendo el valor guardado.
- Saltar directorios ocultos (20), cambiar `POST /v1/projects`, el ranking, el embedder o el esquema.
- Bajar el `index_docs` de una raíz ya marcada: el registro sólo lo sube (`crates/clawg-core/src/memory/project.rs:59-101`), y no se distingue una marca del puente de un `clawg projects add`.
- Un clasificador de «parece una instrucción». El margen impide imitar la estructura, no que una línea diga «ignorá lo anterior»: eso lo sigue cubriendo la frase de evidencia.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil: un contenido con un separador, una cabecera de hit o `NEW`/`OLD` en una línea siguiente no produce esa línea en el prompt; un `session_summary` conserva todas sus líneas; un autor con `]`, saltos o 300 caracteres sale saneado y `is_shared` sigue comparando el crudo; los marcadores de un texto copiado no dejan nada en la sección USER tras dos regeneraciones; `auto_indexable` y el puente sobre una carpeta sin marcador.

Compuerta de retrieval, fijada antes de medir: `rag-eval --real --agent-view` (modelo real y golden set de `docs/eval/`, fuera de git) contra `2026-10-19-real-session-handoff-tip-agent-view.json`, sin ningún caso de `search_memory` ni de `recall_context` movido, y la media de tokens de `recall_context` a lo sumo 5 % más alta (52 tokens).

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Las filas guardan el texto como llegó; sólo cambia cómo se muestra, y el AUTO de `workspace.md` se reescribe desde la base en cada regeneración. Una raíz ya marcada `index_docs` lo sigue estando. Volver atrás es volver al binario anterior.
