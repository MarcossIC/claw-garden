# client-setup-cursor — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | M | — | exploración; decisiones cerradas, lista para aplicar |

**Tesis:** `clawg setup` sólo conoce a Claude Code (`specs/setup`), y `clawg hook` sólo habla su formato. Conectar Cursor hoy es editar archivos a mano, y aun así el hook no funcionaría. Este cambio agrega `clawg setup cursor` y le enseña al hook el formato de Cursor. A qué proyecto va cada memoria lo resuelve [bridge-project-identity](../2026-11-07-bridge-project-identity/exploration.md).

## Lo que dice Cursor (documentación consultada el 2026-11-03, Cursor 3.28)

Fuentes: [Hooks](https://cursor.com/docs/hooks), [Third Party Hooks](https://cursor.com/docs/reference/third-party-hooks), [MCP](https://cursor.com/docs/context/mcp).

- **MCP:** `~/.cursor/mcp.json`, con `mcpServers.<nombre>` y `type`, `command`, `args`, `env`.
- **Hooks de usuario:** `~/.cursor/hooks.json`. Corren en `~/.cursor/`, y en Windows la aplicación los corre por PowerShell con el JSON por stdin.
- **Entrada:** `conversation_id`, `generation_id`, `model`, `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`, `transcript_path`. `sessionStart` suma `session_id`, igual a `conversation_id`. No hay `cwd`.
- **Salida de `sessionStart`:** `additional_context`, que va al contexto inicial del sistema.
- **`beforeSubmitPrompt`:** devuelve `continue` y `user_message`. No puede inyectar contexto.
- **Salida del proceso:** 2 bloquea, otro código distinto de 0 falla abierto.
- **Configuración de terceros:** con "Include Third-Party Plugins, Skills, and Other Configs", encendido por defecto, Cursor también carga los hooks de `~/.claude/settings.json` y de `.claude/settings*.json`, y los plugins de Claude Code. Los logs lo confirman: `Claude user config path` y `loadClaudePlugin`.

## Problema (verificado contra el código)

- `HookInput` lee `cwd`, `source`, `transcript_path` y `session_id` (`crates/clawg-cli/src/hook.rs:39`). Sin `cwd` usa el directorio actual (`:241`), que para un hook de usuario de Cursor es `~/.cursor/`, y registraría esa carpeta como proyecto.
- `envelope` imprime `hookSpecificOutput.additionalContext` (`:171`), que Cursor no documenta.
- Un hook de `clawg setup claude-code` que Cursor cargue por la configuración de terceros correría con `--agent claude-code`. La documentación no dice si respeta `args`: en cualquiera de los dos casos el resultado está mal.

## Decisiones (cerradas)

1. **`clawg setup cursor [--profile P] [--dry-run] [--remove]`**, con la misma forma que `setup claude-code`: un `plan` puro sobre lo leído y después `apply`, copia en `backups/setup/`, escritura atómica, y lo ajeno intacto. Un servidor o hook de clawg que difiere se informa y no se pisa.
2. **`mcp.json`:** el servidor del perfil, con la ruta absoluta del binario y `mcp --profile P --agent cursor`.
3. **`hooks.json`:** sólo `sessionStart`. No se instala `beforeSubmitPrompt`, porque no puede inyectar el recordatorio.
4. **`clawg hook session-start --agent cursor --client cursor [--profile P]`:** con `--client cursor` lee `workspace_roots` y `session_id`, e imprime `{"additional_context": …}`. Sin `--client`, sigue como hoy.
5. **La carpeta sale de la entrada, nunca del directorio actual.** Con una sola raíz, esa es el proyecto. Con varias o ninguna, no adivina: imprime sólo la línea de idioma, si hay.
6. **El comando se escribe para PowerShell en Windows:** `& "<ruta>" hook …`, también con una ruta con espacios.
7. **Un hook de Claude Code que corre Cursor no hace nada:** sin `--client cursor`, una entrada con `cursor_version` imprime nada y sale con 0.
8. **Sin archivo de reglas.** Cursor ya recibe las instrucciones del servidor (`mcps/<servidor>/INSTRUCTIONS.md`) y deja las herramientas en archivos.
9. **`setup cursor` avisa** que hay que apagar la configuración de terceros, porque no se puede leer de forma estable.

## Alcance

**Dentro:** `crates/clawg-cli/src/setup.rs` (o un módulo propio si `setup.rs` lo pide), `cli.rs`, `hook.rs`, `specs/setup`, `specs/hooks`, `docs/reference/commands.md` y el README.

**Fuera:** el proyecto por defecto del puente, `prompt_builder.rs` (lo toca session-start-context en paralelo), el contrato del agente, subir la versión y archivar.

## Cómo se mide

Tests en rojo primero, con directorios temporales y JSON de entrada copiado de la documentación:

- un `mcp.json` con otros servidores queda con esos servidores intactos, y `--remove` deja el archivo como estaba;
- una segunda corrida no cambia nada;
- un JSON inválido no escribe nada;
- una entrada de `sessionStart` con una raíz imprime `additional_context` y no registra `~/.cursor/`;
- una con dos raíces no registra ningún proyecto;
- una entrada con `cursor_version` en un hook sin `--client cursor` no imprime nada y sale con 0.

## Por qué va en la nube, y qué queda en local

El código se prueba en Linux. Lo que un CI no reproduce queda en `verify.md` como pendiente en local, con Cursor instalado y el perfil `dev`: que el hook corra por PowerShell sin abrir ventanas, que `additional_context` llegue al agente y que la decisión 7 sea cierta para la entrada que Cursor manda de verdad.
