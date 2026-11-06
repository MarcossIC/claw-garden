# bridge-project-identity — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | M | — | exploración cerrada con la prueba de Cursor; sigue la propuesta |

**Tesis:** el puente decide una sola vez a qué proyecto pertenece todo, con el directorio donde arrancó. Eso vale para un cliente que abre un proceso por repositorio, pero en Cursor el proceso arranca en el directorio personal del usuario y recibe las carpetas de todas las ventanas abiertas. Sin resolver esto, las memorias de Cursor caen en el proyecto equivocado sin que nadie se entere.

## Problema (verificado)

- `ClawgMcp::pending` fija `default_project` con `std::env::current_dir()` al crearse (`crates/clawg-cli/src/mcp/server.rs:535`). Es lo que usa cada herramienta sin `project` ni `project_path`.
- El puente no pide MCP roots: no hay `roots/list` en `crates/clawg-cli/src`.
- La [documentación de MCP de Cursor](https://cursor.com/docs/context/mcp) no dice qué directorio recibe un servidor stdio.
- En un log de Cursor del 2026-11-03, una ventana abierta a las 10:04 conectó su servidor con `connected=true` sin ningún `connecting stdio` nuevo: reutilizó el proceso que ya estaba. Otra ventana tenía un workspace con 4 repositorios. Es un caso observado, no una regla documentada.
- `${workspaceFolder}` es «la carpeta que contiene `.cursor/mcp.json`»: en la configuración global no resuelve nada de esto.

## Resultado de la prueba con Cursor (2026-11-05 y 2026-11-06)

Una sonda MCP stdio mínima, sin commitear y registrada en `~/.cursor/mcp.json`, anotó en cada arranque su directorio, su pid, su proceso padre, las variables de entorno de Cursor y lo que respondió `roots/list`. Se usó Cursor 3.28.7 en Windows, con una ventana de un repositorio y un workspace de 5 repositorios, abiertos juntos y por separado.

| Caso | Qué pasó |
|---|---|
| 1. Una ventana con un repositorio | El directorio del proceso es el directorio personal del usuario, no el repositorio. `roots/list` da una raíz: el repositorio. |
| 2. Una segunda ventana con otro repositorio | Cursor arranca un proceso nuevo, pero su `roots/list` da las carpetas de todas las ventanas abiertas (6, con la nueva al final). El proceso de la ventana que ya estaba sigue vivo. |
| 3. Un workspace con varios repositorios | `roots/list` da sus 5 carpetas. |
| 4. `notifications/roots/list_changed` al cambiar las carpetas | No llegó nunca. Cursor declara `roots.listChanged: false` en cada `initialize`. |

Además:
- El directorio es el personal en los 14 arranques, con cualquier combinación de ventanas.
- `WORKSPACE_FOLDER_PATHS` trae siempre las mismas carpetas que `roots/list`, separadas por comas.
- El `uri` de cada raíz llega como ruta de Windows con la unidad en minúscula (`c:\...`), no como una URI `file:`.
- Cada apertura reinicia el servidor 2 o 3 veces en pocos segundos.
- El cliente es `cursor-vscode` 1.0.0, con protocolo `2025-11-25`.

**Falta medir:** nadie llamó a la herramienta de la sonda desde el chat, así que no se sabe si un proceso arrancado antes que otra ventana ve las carpetas nuevas cuando se le vuelve a pedir `roots/list`. Las decisiones de abajo no dependen de eso: si las raíces se piden en cada llamada, las dos respuestas posibles quedan cubiertas.

**Qué se deduce:** las raíces no dicen desde qué ventana llega una llamada, y el directorio nunca es el repositorio. El enfoque A sólo da una respuesta cuando hay abierta una sola ventana con un solo repositorio.

## Enfoques

**A. Pedir roots por llamada, con el directorio como respaldo.**
- **A favor:** es el estándar de MCP, sirve para cualquier cliente y no necesita configuración por repositorio.
- **En contra:** en Cursor, con dos ventanas abiertas, las raíces son las de todas y no dicen cuál es la de la llamada. Con varias raíces hay que elegir.

**B. Exigir el proyecto en la llamada.**
- **A favor:** `project` y `project_path` ya existen en cada herramienta, y el agente sabe en qué archivo está trabajando. Resuelve el workspace con varios repositorios, donde no hay una respuesta única.
- **En contra:** depende de que el modelo lo pase, y hay que decírselo en lo que ve, que pesa en cada sesión.

**C. Un `.cursor/mcp.json` por repositorio con `${workspaceFolder}`.**
- **A favor:** la identidad queda fija y no hace falta adivinar.
- **En contra:** hay que configurar cada repositorio, y no resuelve el workspace con varios.

## Decisiones

- [x] **Qué enfoque.** B para guardar y A para leer; C queda descartado porque no resuelve el workspace con varios repositorios y obliga a configurar cada uno.
  - **Un save dice siempre a qué proyecto va,** con cualquier cliente: la carpeta (`project_path`), un proyecto por nombre (`project`, que crea uno nuevo si ninguno se llama así) o `all_projects`. Sin ninguno, el save se rechaza con un error que dice qué pasar y lista como opciones las raíces que declaró el cliente. Así ningún save cae en un proyecto adivinado, tampoco en Claude Code, donde una conversación puede tratar de un proyecto distinto del de su directorio.
  - **Lo que lee usa las raíces que declara el cliente,** pedidas en cada llamada porque Cursor no avisa cuando cambian. Con un cliente que no declara roots, usa el directorio donde arrancó, salvo que sea el directorio personal del usuario, que nunca es un proyecto.
- [x] **Si con varias raíces un save sin proyecto se rechaza o queda sin proyecto.** Se rechaza, y no sólo con varias raíces: todo save sin proyecto se rechaza. Una memoria sin proyecto la lee cada proyecto, así que un dato de un repositorio aparecería en todos los demás.
- [x] **Si `search_memory` con varias raíces busca en todas o exige elegir.** Busca en todas, y cada resultado dice de qué proyecto es. Leer no ensucia nada, y el agente no tiene que acertar el repositorio antes de buscar. Hoy una búsqueda va a un proyecto o a todos: buscar en un grupo de proyectos es nuevo.

## Para la propuesta

- Qué hacen con varias raíces las otras herramientas que leen sin `project` ni `project_path`. Lo coherente es lo mismo que `search_memory`.
- Exigir el proyecto en `save_memory` cambia lo que ve el modelo: se actualizan `crates/clawg-cli/src/mcp/agent_contract.json` y `agent_contract.es.json`.
- Una raíz puede llegar como URI `file:` o, en Cursor, como ruta de Windows: las dos tienen que resolver al mismo proyecto.

## Cómo se mide

Tests con un cliente MCP de prueba:
- un save sin `project`, `project_path` ni `all_projects` se rechaza con las raíces como opciones, con roots y sin roots;
- una búsqueda sin proyecto con una raíz busca en su proyecto, con varias busca en todas, y sin roots usa el directorio;
- el directorio personal del usuario nunca es un proyecto;
- una raíz como ruta de Windows y como URI `file:` dan el mismo proyecto;
- un cambio de raíces entre dos llamadas se ve en la segunda.

Después, en local con Cursor real: un save desde una ventana con dos repositorios cae en el proyecto que nombró el agente, y una búsqueda con dos ventanas abiertas trae memorias de los dos.
