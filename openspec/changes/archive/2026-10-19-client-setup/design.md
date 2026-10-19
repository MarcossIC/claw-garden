# client-setup — Diseño

## D1. Un plan puro sobre el estado leído

`crates/clawg-cli/src/setup.rs` sigue el molde de `user_path.rs` y de `update/plan.rs`: primero se lee todo (`State`: el `.claude.json`, el `settings.json` y el `CLAUDE.md` de Claude Code como texto, si `claude` está en el PATH, la ruta del binario y el perfil), después una función pura arma el plan (`plan(&State, Mode) -> Result<Plan, SetupError>`), y por último un borde fino lo aplica. Los tests del plan no tocan disco ni procesos.

- **Todo o nada al leer.** Si un archivo no se puede leer o no es JSON válido, o el `CLAUDE.md` tiene un marcador sin su par, `plan` falla y no se escribe nada en ningún archivo. Igual que `clawg sharing` con un `sharing.json` inválido: lo que el usuario dejó a medio editar no se pisa.
- **`--dry-run` imprime el plan** con los mismos renglones, en condicional, y no escribe ni corre `claude mcp add|remove`.
- **Sin abstracción de cliente.** `CLAUDE_CODE` es una constante con el nombre del cliente y su agente; un segundo cliente pasaría a una tabla cuando exista.

## D2. Registro por el CLI de Claude Code; lectura de `.claude.json`

**Elegido:** escribir con `claude mcp add --scope user <servidor> -- <exe> mcp --profile P --agent claude-code` y `claude mcp remove --scope user <servidor>`; leer el estado previo de `mcpServers.<servidor>` en `.claude.json`.

**Alternativas:** escribir `.claude.json` a mano (B de la exploración: Claude Code lo reescribe mientras corre, guarda mucho más que servidores y su formato no promete estabilidad; un error ahí daña la configuración entera); usar `claude mcp get` para leer (imprime texto para personas, y además lanza el servidor para chequear su salud, así que tardaría y arrancaría un daemon).

- Leer no tiene el riesgo de escribir: si el formato cambia, lo peor es no reconocer una entrada y que `claude mcp add` la rechace por existir (sale con 1 y se reporta).
- Verificado con Claude Code 2.1.303 en un home temporal: `add` escribe `mcpServers.<nombre> = {type: "stdio", command, args, env: {}}` en `.claude.json` (o en `$CLAUDE_CONFIG_DIR/.claude.json` si está definido), rechaza un nombre que ya existe con código 1, y `remove --scope user` de un nombre ausente sale con 1.
- **Se usa `add` y no `add-json`:** los argumentos van como argv, sin un JSON entre comillas que pase por un `claude.cmd` en Windows.
- **Coincide** cuando `command` es la ruta del plan y `args` son los del plan (`type` ausente o `stdio`; `env` no cuenta). **Es de clawg** cuando el nombre de archivo de `command` es `clawg` (con o sin `.exe`) y el primer argumento es `mcp`: sólo eso quita `--remove`.
- **Sin `claude` en el PATH** (se busca `claude`, y en Windows también `claude.exe` y `claude.cmd`) se imprime el comando exacto para copiar, se aplica lo demás y sale con 1: el alta no quedó completa.

## D3. Identidad fija `claude-code`

Siempre `--agent claude-code`. Es el único nombre observado de Claude Code (2.1.296, `openspec/changes/archive/2026-10-13-mcp-protocol-upgrade/verify.md:57`), así que quien venía sin `--agent` sigue con sus filas, su workspace y sus reglas de `sharing.json` ([`specs/visibility`](../../../specs/visibility/spec.md), R6). Tomar el nombre de `clawg memories stats` necesitaría un daemon corriendo y elegir entre varios; un usuario con otro nombre lo ve ahí y en `memory_status`, y el README lo dice. Renombre y alias quedan fuera (exploración, opciones a y b).

## D4. Hooks en el `settings.json` de usuario

Una entrada por evento, agregada al final del arreglo del evento:

```json
{ "matcher": "startup|resume|clear|compact",
  "hooks": [ { "type": "command", "command": "<exe> hook session-start --agent claude-code --profile P", "timeout": 5 } ] }
```

y la de `UserPromptSubmit` sin `matcher` (el evento no lo admite).

- **Nivel usuario**, como el servidor: los hooks se suman entre niveles, así que agregar uno no desplaza los del proyecto.
- **Un hook es de clawg para un evento y un perfil** cuando su `command`, partido como lo partiría un shell simple, empieza por un programa llamado `clawg`, sigue con `hook <evento>` y lleva `--profile P`. Uno idéntico al plan no se toca; uno de clawg distinto (otra ruta, otro agente) es un conflicto, igual que el servidor: se reporta, y `--remove` seguido de `setup` lo rehace. Uno sin `--profile` no se reconoce como de ningún perfil y queda como ajeno.
- **`--profile` siempre explícito**, también en el servidor: el perfil por defecto depende del tipo de build y de `CLAWG_PROFILE`, que el proceso que lanza Claude Code no tiene por qué compartir.
- **Forma de shell, no `args`.** La forma `args` (sin shell) es nueva en Claude Code; uno que no la conozca correría el binario solo, sin subcomando, y clap saldría con 2, que bloquea la sesión. En la forma de shell (`sh -c`, o Git Bash en Windows), la ruta va con `/` en Windows y entre comillas dobles sólo si tiene algo fuera de letras, dígitos y `/._-:`, escapando `"`, `$`, `` ` `` y `\`. Sin Git Bash, Claude Code usa PowerShell, donde una ruta entre comillas al principio no es un comando: con espacios en la ruta, esa combinación no funciona, y queda anotado como riesgo.
- **Las claves ajenas conservan su orden.** `serde_json` se compila con `preserve_order` (indexmap ya estaba en `Cargo.lock`), así un `settings.json` reescrito no reordena las claves del usuario. Lo que cambia es el formato (sangría de dos espacios), no el contenido.
- `--remove` saca sólo los hooks de clawg del perfil; un grupo que queda sin hooks se va, y también el arreglo del evento y `hooks` si quedan vacíos por esa remoción.

## D5. El protocolo en el `CLAUDE.md` de usuario

El texto completo de `protocol.md` entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`, al final del archivo, separado por una línea en blanco.

- **Una proyección generada**, no un texto editado: la fuente sigue siendo `protocol.md` compilado en el binario (22). Un bloque distinto del actual (otra versión de clawg) se reescribe; lo de afuera no cambia nunca.
- **Un marcador sin su par es un error** y no se escribe nada: no se sabe dónde termina lo de clawg.
- **El mismo bloque para los dos perfiles:** el texto no depende del perfil. `--remove` lo deja cuando el servidor del otro perfil sigue en `.claude.json`.
- **Ida y vuelta exacta:** `--remove` saca el bloque y la línea en blanco que se agregó; un archivo que queda vacío se borra (sólo tenía el bloque). Respeta `\r\n` si el archivo ya lo usa.
- **No se duplica con el hook:** `session-start` no repite el protocolo (D3 de 25). La apertura sí llega dos veces (bloque e instrucciones del servidor), 1.700 caracteres que se aceptan para que el bloque se lea solo.

## D6. Dónde está la configuración de Claude Code

`$CLAUDE_CONFIG_DIR` si está definido; si no, `~/.claude` para `settings.json` y `CLAUDE.md`, y `~/.claude.json` para los servidores. Con `CLAUDE_CONFIG_DIR`, `.claude.json` vive adentro (verificado con 2.1.303). El home sale de `clawg_core::paths::home_dir`, que en un build con `testing` respeta `CLAWG_TEST_HOME`.

## D7. Escritura

Temporal en el mismo directorio y `rename`, como `sharing::save`, para que Claude Code lea el archivo viejo o el nuevo y nunca la mitad. Antes de cambiar un archivo que existía, se copia a `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>`; el reporte nombra la copia. El orden es servidor, hooks, bloque; si `claude mcp add` falla, lo demás se aplica igual y se reporta la falla (cada paso es independiente y cada uno se puede repetir).

## D8. Ruta del binario

`std::env::current_exe()`, sin el prefijo `\\?\` en Windows. Una ruta que no es absoluta se rechaza antes de planear: Claude Code lanza el comando con su propio directorio y PATH (lo dice el README en «Conectarlo a Claude Code»).

## D9. Códigos de salida y reporte

Un renglón por paso (`mcp server clawg: added`, `hooks: unchanged`, …). Sale con 0 cuando todo quedó como el plan (o ya estaba), y con 1 ante un conflicto, el paso manual sin `claude`, un `claude` que falló o un error de lectura.
