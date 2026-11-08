# claude-code-plugin — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (instalar, prender por repo y actualizar el cliente de uso diario; toca 1 y 4 por el ruido de contexto y por el reparto entre clientes) | M (el plugin y la detección en `setup` son S; la medición del protocolo es M) | [setup-local-hooks](../archive/2026-11-08-setup-local-hooks/proposal.md) (archivado); la medición usa [agent-eval](../agent-eval/proposal.md) (96) | exploración (borrador) |

**Tesis.** Un plugin de Claude Code sirve para la caja (servidor MCP y hooks) y no resuelve el binario ni el protocolo. Lo que da de verdad es un solo interruptor por repositorio para servidor y hooks, y que Claude Code gestione la actualización. Hoy eso lo hacen tres escrituras de `clawg setup claude-code` que el usuario no ve, y un `--local` que prende sólo los hooks. El costo está en la coexistencia: si el plugin y `setup` conviven, los hooks corren dos veces, y eso sí está documentado. El bloque del protocolo es un tema aparte: no se decide leyendo, se decide midiendo, y el arnés ya tiene los brazos para hacerlo con una sola incorporación.

Las citas de Claude Code son de https://code.claude.com/docs/en, leídas el 2026-11-08 (páginas `plugins-reference`, `hooks`, `mcp`, `plugins/loading`, `plugins/marketplace-reference`). Lo que la documentación no dice queda marcado «sin verificar».

## El problema verificado contra el código

1. **`setup claude-code` escribe tres cosas a nivel usuario**: el servidor MCP en `~/.claude.json`, dos hooks en `~/.claude/settings.json` y un bloque de protocolo en el `CLAUDE.md` del usuario. El servidor sale de `Server::planned` (`crates/clawg-cli/src/setup.rs:127-133`: `command` = ruta absoluta del exe, `args` = `mcp --profile P --agent claude-code`). Los hooks son los eventos de `HOOK_EVENTS` (`setup.rs:51`: `SessionStart` y `UserPromptSubmit`) en forma exec, `command` + `args` (`hook_args`, `setup.rs:329`; entrada en `:336`). El bloque se arma con `protocol::text` entre marcadores (`setup.rs:36-37`, `:600-605`).
2. **Todo se ata a una ruta absoluta del exe.** `exe_text` rechaza una ruta relativa (`setup.rs:252-257`), y una entrada con otro exe es `Conflict` (`:227`) o una migración de hooks (`HooksStep.migrated`, `:184`). Mover o reinstalar el binario obliga a correr `setup` otra vez (el roadmap lo dice: «el servidor ya registrado sin `--agent` no se actualiza solo», `ROADMAP.md:44`).
3. **Prender por repo hoy es `--local`**: `setup/local.rs` escribe sólo los dos hooks en `.claude/settings.local.json` de una carpeta (`LOCAL_FILE`, `local.rs:22`). El servidor sigue siendo global, así que las herramientas de clawg están en todos los repositorios y la memoria sólo se inyecta en los que tienen los hooks.
4. **Qué hace cada pieza al modelo.** `session-start` inyecta idioma y memoria del proyecto, no el protocolo; `user-prompt-submit` registra la sesión y recuerda guardar tras 30 minutos sin saves (`crates/clawg-cli/src/hook.rs:47-49`, `:172`). El protocolo le llega por las `instructions` del servidor: son la apertura del protocolo (`server.rs:2011`, `protocol::opening`, `protocol.rs:33`) y un test las mantiene en 2048 caracteres por los clientes viejos (`server.rs:2890-2896`). El resto del protocolo (`protocol.md` 10.891 bytes, `protocol.es.md` 11.654) sólo llega por el bloque de `CLAUDE.md`: reglas de escritura, tipos, el resumen de sesión con sus seis secciones (`protocol.md:115-131`).
5. **Los nombres de las herramientas no están en el contrato del agente.** `agent_contract.json` y `.es.json` fijan las `instructions` y las herramientas del servidor, y ninguno contiene `mcp__`. Fuera de `clawg-eval`, tampoco el código ni `docs/` nombran `mcp__clawg…`.
6. **`agent-eval` ya distingue lo que hay que distinguir.** El brazo `a2` es el servidor solo («no hooks, no protocol in `CLAUDE.md`») y `a3` es lo que instala `setup` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:21-23`, `:64-68`; `docs/reference/testing.md:59`). Falta un brazo con los hooks y sin el bloque.

## Qué dice Claude Code, pregunta por pregunta

### 1. El binario

- Un plugin no trae el exe (por plataforma, unos 38 MB). `${CLAUDE_PLUGIN_ROOT}` apunta al directorio **de la versión instalada**, que cambia en cada actualización (`plugins-reference`, «Environment variables»), así que no sirve para un binario que se instala aparte.
- **Los hooks de plugin soportan la forma exec.** «A command hook runs as exec form when `args` is set»: Claude Code «resolves `command` as an executable on `PATH` and spawns it directly», sin shell (`hooks`, «Exec form and shell form»). En Windows exige que `command` resuelva a un ejecutable real, como un `.exe`: `clawg.exe` lo es. `${CLAUDE_PLUGIN_ROOT}` y `${CLAUDE_PLUGIN_DATA}` se resuelven en `command` y `args` de un hook.
- **`userConfig` existe**, con tipo `file` y `directory`, y `${user_config.KEY}` se sustituye en la configuración del servidor MCP y en los `args` de un hook exec (`plugins-reference`, «User configuration»). Para `command` de un hook no lo dice: sin verificar. Por eso la ruta del exe en un `userConfig` podría servir al servidor MCP pero no, por documentación, al `command` de los hooks.
- **`bin/` del plugin** pone sus archivos en el `PATH` de la herramienta Bash mientras el plugin está activo; no dice nada de hooks ni servidores MCP. No sirve.
- Un servidor MCP de plugin con `command` sin ruta (`node` en los ejemplos de la documentación) se lanza con el mismo entorno que uno manual. Que el servidor se resuelva por `PATH` como un hook, la documentación no lo escribe: sin verificar, y se comprueba en local con `claude --plugin-dir`.
- El instalador ya pone el binario en el `PATH` del usuario: `scripts/install.ps1:307` corre `clawg path add --dir` y avisa de otro `clawg.exe` que lo tape (`:123-136`). `clawg path` edita el `PATH` sólo en Windows (`docs/reference/commands.md:38`); en Linux y macOS no hay instalador equivalente en el repositorio.
- **Efecto:** con `clawg` por `PATH`, el plugin deja de depender de dónde está el exe, y reinstalar el binario no obliga a tocar nada. Pierde una cosa que hoy `setup` da: un exe absoluto no puede ser tapado por otro `clawg` del `PATH`.

### 2. Duplicación

- **Hooks:** «When a plugin is enabled, its hooks merge with your user and project hooks». La deduplicación vale entre archivos de settings; «a plugin's or skill's copy of the same handler stays separate» (`hooks`, «Duplicate handling»). Con el plugin y `setup claude-code` instalados, `session-start` corre dos veces (dos inyecciones de contexto) y `user-prompt-submit` dos veces. Es el riesgo más serio y está documentado.
- **Servidor:** los servidores manuales ganan a los de plugin; un servidor de plugin que apunta al mismo comando que uno manual habilitado cuenta como duplicado y no se conecta (`mcp`, «Deduplication»). Pero los dos comandos no serían iguales (`C:\...\clawg.exe mcp …` frente a `clawg mcp …`), y la página no dice que se comparen resueltos: sin verificar. Lo seguro es suponer dos puentes.
- **Qué puede hacer `setup` al detectar el plugin.** Ya lee `settings.json` entero (`State.settings`, `setup.rs:81`) y su `plan` es puro (`:215`). Detectar una clave `clawg@…` en `enabledPlugins` con valor `true` cabe ahí sin I/O nuevo. Con plugin activo, `setup claude-code` debería:
  - no agregar servidor ni hooks (`ServerStep` y `HooksStep` en «ya cubierto»), y avisar si hay un servidor o hooks propios que lo duplican, proponiendo `--remove`;
  - seguir escribiendo el bloque de `CLAUDE.md`, que un plugin no escribe («A `CLAUDE.md` at the plugin root isn't loaded as context», `plugins-reference`, «Standard layout»), mientras la medición de abajo no lo descarte;
  - `--remove` sigue sacando sólo lo suyo.
- **`--local`** queda redundante: el plugin en alcance `local` prende servidor y hooks en ese repositorio con la misma escritura (`.claude/settings.local.json`, clave `enabledPlugins`). No se borra: lo usa quien no quiere el servidor en cada repositorio sin plugin, y el roadmap del piloto lo cita.

### 3. El protocolo y el ruido

- Costo fijo hoy: el bloque son unos 11 KB en `CLAUDE.md` de usuario, en todas las sesiones de todos los proyectos, instalado o no el servidor. Lo paga cada sesión, use o no la memoria: es el ruido que conviene bajar.
- Tras una compactación el protocolo no necesita reimprimirse en un hook: `CLAUDE.md` sobrevive a la compactación y el `session-start` de clawg ya reinyecta la memoria del proyecto tras `compact` (`setup.rs:43`, `hook.rs:254`). Que también sobrevivan las `instructions` de un servidor, la documentación consultada no lo afirma: sin verificar.
- **Qué falta saber** (no se decide por lectura): si el modelo guarda y busca igual con sólo las `instructions` de 2048 caracteres. Lo que el bloque trae de más es la forma del resumen de sesión, las reglas de escritura largas y la tabla de tipos. Las herramientas llevan sus descripciones, pero el contrato del agente no cubre la forma del resumen.
- **Medición propuesta** (con `agent-eval`, de 96):
  - brazo nuevo `a3-noblock`: `a3` sin el bloque (hooks, sin protocolo), junto a `a2` (nada) y `a3` (todo). La comparación `a3-noblock` contra `a3` aísla el bloque; `a2` contra `a3-noblock` aísla los hooks;
  - métricas fijadas antes de correr: tasa de saves por tarea que lo pide, tasa de búsqueda previa en las tareas `K1` a `K3`, calidad de los saves por la rúbrica determinista (`rubric.rs`), presencia y seis secciones del `session_summary` al cerrar, y tokens y costo por sesión (el bloque son unos 2.700 tokens por sesión, a tomar del `stream-json`);
  - el umbral, escrito antes: el bloque se saca si ninguna de las tasas de arriba baja más que un margen no inferior fijado de antemano; si baja sólo el resumen, se evalúa el bloque reducido a esa sección o como skill del plugin (se carga por descripción, no entera);
  - se corre también con el brazo `a3-eager` (herramientas cargadas por adelantado), porque con la búsqueda de herramientas del cliente las `instructions` pueden llegar distinto: sin verificar si siempre se inyectan.

### 4. Nombres de las herramientas

- Pasan de `mcp__clawg__*` a `mcp__plugin_<plugin>_<servidor>__*`: con plugin `clawg` y servidor `clawg`, `mcp__plugin_clawg_clawg__save_memory` (`mcp`, «Tool naming»). El servidor se registra como `plugin:clawg:clawg`.
- **No se rompe:** el contrato del agente (sin `mcp__`, punto 5 de arriba), las `instructions` (hablan de `save_memory`, no del prefijo) ni `stream::is_tool`, que compara por sufijo (`stream.rs:90-92`).
- **Se rompe o cambia:**
  - `agent-eval` pasa `--allowedTools mcp__<servidor>` (`arm.rs:251`) y sus tests lo fijan (`arm.rs:445-487`): un brazo de plugin necesita el prefijo nuevo. Además arma cada brazo con `--mcp-config` y `--strict-mcp-config` y su `CLAUDE_CONFIG_DIR` (`docs/reference/testing.md:59`); cargar un plugin sería con `--plugin-dir` (origen `@inline`, `plugins/loading`). Si `--strict-mcp-config` deja afuera a los servidores de plugin, la documentación consultada no lo dice: sin verificar.
  - Las reglas de permiso de los usuarios (`mcp__clawg__*` o `mcp__clawg`) dejan de valer. `setup` no escribe permisos (no hay `permissions` en `setup.rs` ni en `setup/`), así que son reglas que el usuario puso a mano: hay que avisarlo en las notas de migración.
  - Un matcher de hook escrito contra `mcp__clawg__…` nunca dispara con el servidor de plugin («A hook matcher written against the bare server key … never fires for a plugin-bundled server»). clawg no tiene ninguno propio.
  - Dos formas de tener las herramientas (manual y plugin) con el mismo servidor: el modelo vería dos juegos si no se deduplica.

### 5. Perfiles dev y prod

- Hoy el nombre del servidor sale del perfil (`Profile::mcp_server_name`: `clawg` y `clawg-dev`); un build de debug rechaza `prod` (`CLAUDE.md` del proyecto, «Data profiles»).
- **Un plugin con un `userConfig` `profile`** (cadena con `options: ["prod", "dev"]`, `default: "prod"`; las `options` piden Claude Code 2.1.271 o más) y `${user_config.profile}` en los `args` del servidor y de los hooks (ambos lo permiten, ver 1). Un solo plugin para los dos, pero no pueden estar los dos perfiles en la misma sesión: el servidor es `plugin:clawg:clawg` en ambos casos.
- **Dos plugins** (`clawg` y `clawg-dev`, mismo repositorio): se pueden tener activos a la vez, cada uno con sus herramientas, como hoy con los dos servidores. Duplica los archivos del plugin.
- **Sólo `prod` en el plugin**, y `dev` sigue por `setup --profile dev` o `claude --plugin-dir` (un plugin `@inline` por sesión, que además tapa a uno instalado del mismo nombre). Es lo que usa quien desarrolla clawg, que es una minoría y ya sabe de perfiles.
- **`--agent claude-code` fijo.** El puente toma `--agent` como opcional (`cli.rs:57`); los hooks lo piden (`cli.rs:277`). En el plugin de Claude Code, `claude-code` fijo en los dos es lo correcto. El problema aparece si Cursor carga el mismo plugin (6).

### 6. Cursor

- `setup cursor` escribe un servidor con `--agent cursor` y un hook `sessionStart` con `--client cursor` (`setup/cursor.rs:27`, `:139`, `:209`). Cursor manda `workspace_roots` y lee `additional_context`; Claude Code manda `cwd` y lee `hookSpecificOutput` (`hook.rs:11-13`, `:198-201`, `:213-214`).
- Si Cursor carga el plugin, correría los hooks con `--client claude-code` (el predeterminado) y `--agent claude-code`: sin `cwd` y con otro formato de salida, el hook caería en `current_dir` (`hook.rs:213`), que en Cursor no es el proyecto (`hook.rs:206-208`). Y su servidor del plugin se registraría como `claude-code`, no `cursor`, mezclando la autoría que 4 separa. Si Cursor traduce el formato de los hooks de Claude Code a su propio protocolo, no está verificado: sin verificar, y hay que probarlo en local con Cursor real (la afirmación de que lo carga viene del encargo, no de un archivo del repositorio).
- Con `setup cursor` instalado, además, los dos servidores y los dos hooks de sesión estarían duplicados dentro de Cursor.
- Salidas: (a) aceptar que Cursor ve el plugin y detectarlo en `setup cursor`, igual que en 2; (b) un `.mcp.json` sin `--agent` y que el puente use el `clientInfo.name` (el modo existe: el roadmap habla de «el servidor ya registrado sin `--agent`»), con hooks que reconocen el cliente por la entrada (`workspace_roots` presente) en vez de por flags; (c) no publicar el plugin donde Cursor lo lea. (b) cambia `hook.rs`, que es lo único del plan que toca producción.

### 7. Distribución y versión

- El repositorio puede ser el marketplace: `.claude-plugin/marketplace.json` en la raíz, con una entrada de fuente de ruta relativa (`./plugin`), que se resuelve desde la raíz del marketplace (`marketplace-reference`, «Relative path plugin source»). El usuario corre `claude plugin marketplace add <owner>/<repo>` y después instala. El nombre del plugin no puede empezar con `claude-` ni `anthropic-` (`plugins-reference`, `name`); `clawg` está libre.
- **Versión.** Una entrada con ruta relativa en un marketplace de git, si ni el manifiesto ni la entrada fijan `version`, se versiona por «el SHA del commit del directorio instalado» (`plugins/loading`, «How Claude Code computes the version»). Fijar `version` en el manifiesto retiene a todos hasta que cambie la cadena. Opciones:
  - **omitir `version`**: el plugin se actualiza con los commits del marketplace (la rama predeterminada, `main`), sin segundo lugar donde guardar una versión; coherente con «la versión vive sólo en `[workspace.package]`» (`CLAUDE.md` del proyecto, «Versioning»). Qué cuenta como «commit del directorio» (el del repositorio o el que toca `plugin/`) la página no lo precisa: sin verificar;
  - **fijarla** a `[workspace.package] version`: segundo lugar, y el bump de versión de `chore: bump version` tendría que tocar `plugin.json`, además del `Cargo.toml` y el `Cargo.lock`, o un test de `clawg-cli` que falle cuando difieran.
- El plugin no lleva el binario: su versión y la del exe pueden divergir. No importa para los archivos del plugin, que son un `.mcp.json` y un `hooks/hooks.json` que cambian poco. Importa para el contrato entre `clawg hook`/`clawg mcp` y esos archivos, porque los argumentos son una interfaz. El repositorio ya detecta un daemon viejo (`version_warning`, `docs/reference/release.md`, punto 8); un exe viejo con un plugin nuevo se vería igual.
- El flujo de publicación no cambia: se publica al fusionar el bump a `main` (`release.yml`); el marketplace lee `main` por su cuenta. Un test fija que `hooks/hooks.json` es el que `hook_args` produce (así el plugin y `setup` no se separan): es el único acoplamiento que justifica código.

### 8. Prender y apagar por repositorio

- `enabledPlugins` se resuelve por alcance: `user`, `project` (`.claude/settings.json`, se versiona), `local` (`.claude/settings.local.json`) y otros; gana el de mayor precedencia (`plugins/loading`, «Find where a plugin is enabled»). `claude plugin disable clawg@… --scope local` apaga el plugin en un repositorio habiéndolo prendido a nivel usuario.
- Frente a `--local`: un interruptor para servidor y hooks juntos, que el cliente muestra en `/plugin` y gestiona (actualizar, desinstalar, `/reload-plugins`). Con `--local` el servidor sigue en todos lados. Contrapartida: prender el plugin sólo en un repositorio saca las herramientas de los demás (menos esquema en el contexto, y también sin memoria ahí).
- El alcance `project` permite que un repositorio se lo pida a quien lo clona; una entrada con ruta relativa no necesita registro de instalación, pero los servidores MCP pasan por la aprobación por servidor y los monitores no cargan (`plugins/loading`, «Trust check»). Es útil sólo para equipos; no es el caso de esta etapa.
- `settings.local.json` puede no estar ignorado por git: `setup/local.rs:87` ya lo comprueba y avisa. El alcance `local` del plugin tiene el mismo cuidado.

## Enfoques

| | Qué es | A favor | En contra | Tamaño |
|---|---|---|---|---|
| **A** | Seguir con `setup` y mejorarlo: un `clawg setup claude-code --update`, aviso de exe tapado, Linux y macOS en el instalador | Sin superficie nueva; el exe sigue absoluto; el brazo `a3` mide exactamente lo que se instala | La instalación sigue siendo tres escrituras fuera del cliente; no hay interruptor por repositorio para el servidor; el usuario no ve actualizaciones ni apagado en la interfaz del cliente | S–M |
| **B** | Plugin con servidor y hooks, `setup` sólo para el bloque de `CLAUDE.md` y para detectar el plugin | Un interruptor por repositorio; actualización gestionada por el cliente; deja intacto el protocolo, así que no apuesta nada a la medición; vuelta atrás: desinstalar el plugin | Dos mecanismos conviven; el protocolo sigue costando ~11 KB por sesión; el exe pasa a resolverse por `PATH`; nombres de herramientas nuevos | M |
| **C** | Plugin solo; el protocolo sólo en las `instructions` del servidor | La instalación es un comando y desaparece `setup claude-code`; sin ruido de `CLAUDE.md` | Apuesta el comportamiento del agente a 2048 caracteres sin medir; pierde la forma del resumen de sesión si la medición lo requería; Cursor y los permisos siguen a resolver | M, con la medición como puerta |
| **D** | B o C con el resto del protocolo como skill del plugin | El texto largo se carga sólo cuando el modelo lo pide (descripción siempre, cuerpo bajo demanda) | Depende de que el modelo invoque la skill justo al guardar; hay que medirlo igual; un plugin «solo para esta skill» en Cursor es otra incógnita | M |
| **E** | Plugin por plataforma con el exe adentro (fuente `archive`) | Instalación completa en un paso | Una entrada de marketplace no elige por sistema operativo; 38 MB por plataforma; el exe firmado por minisign pasa a ser parte de otro canal de publicación. Descartado | L |

## Recomendación

**B ahora, con C como destino si la medición lo permite.** El orden:

1. Medir primero el bloque (brazo `a3-noblock`), porque es lo único que cambia entre B y C y no se decide leyendo. No bloquea el plugin.
2. Publicar el plugin con servidor y hooks sobre `clawg` por `PATH`, desde un directorio `plugin/` y un `marketplace.json` en el repositorio, sin `version` propia, y hacer que `setup claude-code` y `setup cursor` detecten un plugin activo y no dupliquen.
3. Si la medición no ve diferencia, `setup claude-code` deja de escribir el bloque (y se llega a C); si la ve, el bloque queda o pasa a skill (D).

Por qué no A sola: no da un interruptor por repositorio ni actualización sin tocar archivos a mano, y la mejora del instalador (Linux, macOS, aviso de exe tapado) se necesita igual. Por qué no C de entrada: la única evidencia disponible de que 2048 caracteres alcanzan es que el servidor ya los lleva; nadie midió qué pierde el agente sin el resto. B no cierra ninguna puerta y su costo extra (una detección pura en `plan`) es chico.

## Decisiones abiertas

- **Cómo encuentra el plugin el binario.**
  - Si es sólo `PATH`, el plugin no depende de dónde está el exe y se actualiza solo, pero un `clawg` ajeno anterior en el `PATH` lo tapa (hoy `install.ps1` avisa) y en Linux y macOS falta cómo ponerlo.
  - Si el servidor usa además un `userConfig` de tipo `file`, el servidor resuelve una ruta exacta y los hooks siguen por `PATH`: dos mecanismos para lo mismo, y el prompt de configuración es una pregunta más al instalar.
- **Perfiles.**
  - Si es un `userConfig` `profile`, un plugin sirve a los dos perfiles pero uno por vez, y la ayuda de «mi build de debug rechaza prod» pasa a ser un error del servidor del plugin.
  - Si son dos plugins, conviven pero hay dos juegos de archivos que sincronizar.
  - Si el plugin es sólo `prod`, `dev` sigue en `setup` y queda una asimetría documentada.
- **Qué hace `setup` con un plugin activo.**
  - Si omite servidor y hooks y avisa, el usuario nunca ve duplicados, pero un `enabledPlugins` en otro alcance (proyecto) que `setup` no lea pasa inadvertido.
  - Si se niega a instalar, es más estricto y obliga a `--remove` primero.
  - Si migra solo (saca lo suyo), es cómodo y toca archivos del usuario sin que lo pida.
- **El bloque del protocolo.**
  - Si la medición no ve diferencia, se saca y `setup claude-code` queda para migrar; el costo de ~11 KB por sesión desaparece.
  - Si ve diferencia sólo en el resumen de sesión, queda esa sección o una skill.
  - Si ve diferencia en búsqueda y guardado, el bloque se queda y B es el destino.
- **Versión del plugin.**
  - Si se omite, no hay segundo lugar donde guardarla y el plugin sigue a `main`; no se sabe con certeza qué cambia el SHA.
  - Si se fija a `[workspace.package]`, hay una versión explícita para soporte, a costa de tocarla en cada bump y de un test que la compare.
- **Cursor.**
  - Si se acepta que Cursor lo cargue, hay que cambiar `hook.rs` (reconocer el cliente por la entrada) y quitar `--agent` del `.mcp.json`.
  - Si se evita, el plugin queda sólo donde Cursor no lo lea, lo que no está verificado como posible.
  - En cualquier caso `setup cursor` debe detectar el plugin.
- **Dónde vive el plugin en el repositorio.** Si es `plugin/` con `marketplace.json` en la raíz, el árbol del repositorio queda con un directorio más y los tests de convenciones (`tests/conventions.rs`) tienen que conocerlo; si es la raíz misma como plugin, el marketplace trae todo el repositorio al cache del usuario.

## Qué mirar antes de la propuesta

1. **En local, con Claude Code 2.1.319 y `claude --plugin-dir`** (sin instalar nada ni tocar `~/.claude`): que un `.mcp.json` con `command: "clawg"` arranque por `PATH`; que `--strict-mcp-config` no deje afuera al servidor del plugin (para el arnés); qué pasa con el hook de plugin y el de `setup` a la vez (la duplicación documentada); y qué `cwd` y formato recibe el hook cuando lo corre Cursor.
2. **Con `agent-eval`:** agregar el brazo `a3-noblock` y la variante del prefijo de herramientas; el piloto de 48 corridas pide autorización de gasto.
3. Contar los tokens reales de `CLAUDE.md` con el bloque y de las `instructions` desde el `stream-json`, en vez de estimarlos.

## Fuera de alcance

Un instalador de binarios por plataforma dentro del plugin; capturar en `Stop` y `SubagentStop` (decisión 4 del corte en `ROADMAP.md`); publicar en el directorio oficial de Anthropic; un plugin para Codex o Gemini.
