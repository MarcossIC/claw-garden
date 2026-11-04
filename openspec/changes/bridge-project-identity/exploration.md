# bridge-project-identity — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | M | una prueba con Cursor real, con una persona presente | exploración |

**Tesis:** el puente decide una sola vez a qué proyecto pertenece todo, con el directorio donde arrancó. Eso vale para un cliente que abre un proceso por repositorio, pero en Cursor un proceso puede atender varias ventanas y una ventana varios repositorios. Sin resolver esto, las memorias de Cursor pueden caer en el proyecto equivocado sin que nadie se entere.

## Problema (verificado)

- `ClawgMcp::pending` fija `default_project` con `std::env::current_dir()` al crearse (`crates/clawg-cli/src/mcp/server.rs:530`). Es lo que usa cada herramienta sin `project` ni `project_path`.
- El puente no pide MCP roots: no hay `roots/list` en `crates/clawg-cli/src`.
- La [documentación de MCP de Cursor](https://cursor.com/docs/context/mcp) no dice qué directorio recibe un servidor stdio.
- En un log de Cursor del 2026-11-03, una ventana abierta a las 10:04 conectó su servidor con `connected=true` sin ningún `connecting stdio` nuevo: reutilizó el proceso que ya estaba. Otra ventana tenía un workspace con 4 repositorios. Es un caso observado, no una regla documentada.
- `${workspaceFolder}` es «la carpeta que contiene `.cursor/mcp.json`»: en la configuración global no resuelve nada de esto.

## La prueba que falta (local, 10 minutos, con una persona)

Un servidor MCP stdio mínimo, sin commitear, que escriba en un archivo su directorio actual, su pid y lo que responde `roots/list`, y que lo registre en cada llamada. Se agrega a `~/.cursor/mcp.json` y se mira:

1. una ventana con un repositorio;
2. una segunda ventana con otro repositorio;
3. un workspace con varios repositorios;
4. si `notifications/roots/list_changed` llega al agregar una carpeta.

De ahí sale qué enfoque sirve.

## Enfoques

**A. Pedir roots por llamada, con el directorio como respaldo (recomendado si la prueba lo permite).**
- **A favor:** es el estándar de MCP, sirve para cualquier cliente y no necesita configuración por repositorio.
- **En contra:** si un proceso compartido responde con las carpetas de una sola ventana, no alcanza. Con varias raíces hay que elegir.

**B. Exigir `project_path` cuando la raíz es ambigua.**
- **A favor:** ya existe en cada herramienta, y el agente sabe en qué archivo está trabajando. Resuelve el workspace con varios repositorios, donde no hay una respuesta única.
- **En contra:** depende de que el modelo lo pase, y hay que decírselo en las instrucciones, que pesan en cada sesión.

**C. Un `.cursor/mcp.json` por repositorio con `${workspaceFolder}`.**
- **A favor:** la identidad queda fija y no hace falta adivinar.
- **En contra:** hay que configurar cada repositorio, y no resuelve el workspace con varios.

Lo probable es A con B para varias raíces: una carpeta se resuelve sola y varias se preguntan con un error que le diga al agente qué pasar. Eso es mejor que guardar en el proyecto equivocado.

## Cómo se mide

Tests con un cliente MCP de prueba que declara roots: una raíz, varias, ninguna, un cambio de raíces, y un cliente sin esa capacidad, que tiene que seguir como hoy. Después, en local, un save desde Cursor en un workspace con dos repositorios tiene que caer en el proyecto correcto.

## Decisiones abiertas

- [ ] Qué enfoque, según la prueba.
- [ ] Si con varias raíces un save sin `project_path` se rechaza o queda sin proyecto. Recomendado: se rechaza, con un error que explique qué pasar.
- [ ] Si `search_memory` con varias raíces busca en todas o exige elegir.
